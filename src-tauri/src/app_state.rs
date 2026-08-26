//! Orchestration: the background state machine that connects to Valorant when it appears,
//! reconnects on loss, and emits a `TrackerSnapshot` on every change. Never crashes when
//! the game isn't running — that is a normal `ValorantNotRunning` snapshot.

use crate::riot::assemble::{assemble_players, AssembleInput};
use crate::riot::constants::{game_mode_name, INTER_REQUEST_DELAY_MS};
use crate::riot::content;
use crate::riot::error::{Error, Result};
use crate::riot::loadout::{self, PlayerSkinIds};
use crate::riot::lockfile::{self, Lockfile};
use crate::riot::local_api::LocalClient;
use crate::riot::match_state::{self, MatchPlayer};
use crate::riot::names;
use crate::riot::presence::{self, PresenceInfo};
use crate::riot::rank::{parse_mmr, MmrResponse};
use crate::riot::remote_api::{build_hosts, Auth, RemoteClient};
use crate::riot::static_data::{self, StaticData};
use crate::riot::stats::{self, MatchContribution, MatchTotals, RecentStats, RrHistory};
use crate::riot::types::{AppStatus, MapInfo, PendingStats, SessionLoopState, TrackerSnapshot};
use crate::riot::websocket::Poke;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Something that receives snapshots (the Tauri layer implements this).
pub trait Emitter: Send + Sync + 'static {
    fn emit(&self, snapshot: &TrackerSnapshot);
}

/// Shared tracker state, managed by Tauri.
pub struct TrackerState {
    snapshot: Mutex<TrackerSnapshot>,
    started: AtomicBool,
}

impl Default for TrackerState {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(TrackerSnapshot::not_running(Some(
                "Waiting for Valorant...".into(),
            ))),
            started: AtomicBool::new(false),
        }
    }
}

impl TrackerState {
    /// Current snapshot (for the `get_tracker_state` command).
    pub fn snapshot(&self) -> TrackerSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Currently published status — cheap (no snapshot clone) because the event loop only
    /// needs it to decide whether another player's presence event is worth a rebuild.
    fn status(&self) -> AppStatus {
        self.snapshot.lock().unwrap().status
    }

    fn store(&self, snap: TrackerSnapshot) {
        *self.snapshot.lock().unwrap() = snap;
    }

    /// Mark the tracker as started. Returns true exactly once (the first call), so the
    /// caller spawns the loop only once — makes `start_tracker` idempotent.
    pub fn begin(&self) -> bool {
        !self.started.swap(true, Ordering::SeqCst)
    }
}

/// Emit + store a snapshot only if it differs from the last one (avoids UI churn). This
/// dedup is what makes the incremental emit safe: each progress snapshot differs from the
/// last (a stat group settles, so values fill in and a pending flag clears), while a step
/// that adds nothing (e.g. a pregame with no recent matches) is silently suppressed.
fn publish(state: &TrackerState, emitter: &Arc<dyn Emitter>, mut snap: TrackerSnapshot) {
    {
        let prev = state.snapshot.lock().unwrap();
        if same_content(&prev, &snap) {
            return;
        }
        #[cfg(debug_assertions)]
        log_publish(prev.status, &snap);
    }
    snap.last_updated = crate::riot::types::now_millis();
    state.store(snap.clone());
    #[cfg(debug_assertions)]
    debug_capture::write(&snap);
    emitter.emit(&snap);
}

/// One console line per emitted snapshot: a status change spells out the match context it
/// moved to, while a same-status update reports how much of the table is still outstanding.
#[cfg(debug_assertions)]
fn log_publish(prev_status: AppStatus, snap: &TrackerSnapshot) {
    if prev_status != snap.status {
        vlt_log!(
            "state",
            "{:?} -> {:?}  map={} mode={} players={} enriched={}",
            prev_status,
            snap.status,
            snap.map.as_ref().map_or("-", |m| m.name.as_str()),
            snap.mode.as_deref().unwrap_or("-"),
            snap.players.len(),
            snap.enriched
        );
    } else {
        vlt_log!(
            "state",
            "{:?} update  players={} enriched={} pending_rows={}",
            snap.status,
            snap.players.len(),
            snap.enriched,
            snap.players.iter().filter(|p| p.pending != PendingStats::default()).count()
        );
    }
}

/// Whether two snapshots carry the same content, ignoring `last_updated` (which differs on
/// every build). Compared in place, so the dedup costs no clone. The destructuring is
/// deliberate: a new snapshot field then fails to compile here rather than silently escaping
/// the comparison. Pure.
fn same_content(a: &TrackerSnapshot, b: &TrackerSnapshot) -> bool {
    let TrackerSnapshot {
        status,
        map,
        mode,
        own_team,
        players,
        enriched,
        last_updated: _,
        message,
    } = a;
    *status == b.status
        && *map == b.map
        && *mode == b.mode
        && *own_team == b.own_team
        && *players == b.players
        && *enriched == b.enriched
        && *message == b.message
}

/// Dev-only capture of what the tracker saw. Compiled out of release builds entirely, and
/// inert unless `VLT_DEBUG_CAPTURE` names a directory — see `docs/testing.md`.
#[cfg(debug_assertions)]
mod debug_capture {
    use super::TrackerSnapshot;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Capture counter, so captured files sort in the order they were written — snapshots
    /// and presence dumps share it, which is what makes the interleaving readable.
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Whether capture is on: `VLT_DEBUG_CAPTURE` set to a non-empty directory. Cheap, so
    /// callers can gate any work that capture would otherwise cost them.
    pub fn enabled() -> bool {
        std::env::var_os("VLT_DEBUG_CAPTURE").is_some_and(|dir| !dir.is_empty())
    }

    /// Write `snapshot` as pretty JSON to `$VLT_DEBUG_CAPTURE/snapshot-{n:04}-{status}.json`.
    pub fn write(snapshot: &TrackerSnapshot) {
        let Some(dir) = capture_dir() else {
            return;
        };
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        write_json(dir.join(format!("snapshot-{n:04}-{:?}.json", snapshot.status)), snapshot);
    }

    /// Write the raw `/chat/v4/presences` body of one rebuild as pretty JSON to
    /// `$VLT_DEBUG_CAPTURE/presences-{n:04}.json`. Undecoded on purpose: the point is to see
    /// exactly what the local roster held (e.g. whether match players appear in it at all).
    pub fn write_presences(body: &serde_json::Value) {
        let Some(dir) = capture_dir() else {
            return;
        };
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        write_json(dir.join(format!("presences-{n:04}.json")), body);
    }

    /// The capture directory, created if needed. `None` (so the caller does nothing) when
    /// capture is off or the directory is unusable.
    fn capture_dir() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("VLT_DEBUG_CAPTURE")?;
        if dir.is_empty() {
            return None;
        }
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Serialize `value` as pretty JSON to `path`. Best-effort: every failure (unwritable
    /// path, serialization) is ignored so capture can never affect the running app.
    fn write_json<T: serde::Serialize>(path: std::path::PathBuf, value: &T) {
        if let Ok(json) = serde_json::to_string_pretty(value) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Shared build context threaded through the build path so each phase can publish directly
/// and the enrichment can watch the poke channel to abort promptly on a state change.
struct BuildCtx<'a> {
    state: &'a Arc<TrackerState>,
    emitter: &'a Arc<dyn Emitter>,
    rx: &'a mut mpsc::Receiver<Poke>,
}

/// Everything a snapshot needs besides the cache: the fixed match context a build resolves
/// once and every emit of that build reuses. Kept apart from the cache so a phase can hold
/// the cache mutably and still publish.
struct SnapshotParts<'a> {
    players: &'a [MatchPlayer],
    parties: &'a HashMap<String, String>,
    own_team: Option<String>,
    map: Option<MapInfo>,
    mode: Option<String>,
    status: AppStatus,
    static_data: &'a StaticData,
    own_puuid: &'a str,
    season_id: &'a str,
    seasons: &'a [content::Season],
    /// INGAME — the only state where loadouts exist.
    ingame: bool,
}

/// How long a burst of settling stats may accumulate before the next progress snapshot goes
/// out. Every phase ends in a forced flush, so an emit the window swallowed is never the last
/// word and needs no dirty tracking.
const PROGRESS_COALESCE_MS: u64 = 250;

/// Paces the mid-build progress emits so a fast burst can't turn into an event per request.
#[derive(Default)]
struct ProgressGate {
    last_emit: Option<Instant>,
}

impl ProgressGate {
    /// Whether this emit goes out now, recording it when it does.
    fn take(&mut self, forced: bool) -> bool {
        if !should_emit(self.last_emit.map(|at| at.elapsed()), forced) {
            return false;
        }
        self.last_emit = Some(Instant::now());
        true
    }
}

/// Whether a progress emit goes out: a forced one always, the first one immediately, and the
/// rest only once the coalescing window has passed. Pure.
fn should_emit(elapsed: Option<Duration>, forced: bool) -> bool {
    forced || elapsed.is_none_or(|since| since >= Duration::from_millis(PROGRESS_COALESCE_MS))
}

/// The publishing side of a build: the fixed match context plus the emit pacing, so each
/// phase can hand the table to the UI as its stats settle instead of at the phase boundary.
struct Progress<'a, 'p> {
    ctx: &'a mut BuildCtx<'p>,
    parts: &'a SnapshotParts<'a>,
    gate: ProgressGate,
    /// Whether the match's loadouts have settled (they arrive for the whole roster at once).
    loadouts_fetched: bool,
}

impl Progress<'_, '_> {
    /// Publish what the cache holds so far. Never final: the rows that are still outstanding
    /// carry their pending flags, and the snapshot dedup drops a step that changed nothing.
    fn publish_progress(&mut self, cache: &MatchCache, forced: bool) {
        if !self.gate.take(forced) {
            return;
        }
        let snap = assemble_snapshot(self.parts, cache, self.loadouts_fetched, false);
        publish(self.ctx.state, self.ctx.emitter, snap);
    }
}

/// Agent-select poll cadence (see `poll_interval`).
const PREGAME_POLL_MS: u64 = 1000;

/// How long the loop may wait for a poke before rebuilding anyway, for a given status.
/// `Some` only in Pregame: Riot pushes no presence event when a lobby player picks or locks an
/// agent (a friend's event carries nothing about their pick, and cannot be told from any other
/// friend's), and our own presence doesn't change either, so agent select would sit
/// still for its whole ~100 s after the entry events. Polling the pregame endpoints once
/// a second keeps the roster live (vRY's main loop polls for the same reason).
///
/// `roster_locked` ends the tick early: once every ally has locked an agent, nothing in the
/// pregame payload can change again, and the pregame→ingame transition arrives as an
/// own-presence poke. The rest of agent select — usually its longest stretch — then costs
/// nothing. Every other status stays purely event-driven; `None` means "wait indefinitely".
/// Pure.
fn poll_interval(status: AppStatus, roster_locked: bool) -> Option<Duration> {
    (status == AppStatus::Pregame && !roster_locked)
        .then(|| Duration::from_millis(PREGAME_POLL_MS))
}

/// Backoff schedule for retrying a failed rebuild, in milliseconds. A build only
/// fails when the state could not be determined or a pregame/coregame fetch failed — the
/// Menus path cannot error — so this never turns the 0-request Menus steady state into a
/// poll. The schedule is finite on purpose: after the last delay the loop falls back to the
/// event-driven wait, so a permanently broken endpoint can't become a permanent poll loop.
const RETRY_BACKOFF_MS: [u64; 4] = [1000, 2000, 4000, 8000];

/// Delay before retry attempt `attempt` (0-based), or `None` once the schedule is exhausted
/// and the loop should go back to waiting for an event. Pure.
fn retry_delay(attempt: usize) -> Option<Duration> {
    RETRY_BACKOFF_MS.get(attempt).copied().map(Duration::from_millis)
}

/// Settle a snapshot no further attempt will improve: every group still marked pending is as
/// final as it is going to get, so the flags clear and the snapshot becomes `enriched` — a
/// cell whose data never arrived renders as N/A instead of a skeleton waiting on a rebuild
/// that isn't scheduled. Equivalent to re-assembling the same cache with finality, which is
/// where a settled snapshot otherwise comes from. Reports whether anything changed. Pure.
fn settle_pending(snap: &mut TrackerSnapshot) -> bool {
    if snap.players.iter().all(|p| p.pending == PendingStats::default()) {
        return false;
    }
    for player in &mut snap.players {
        player.pending = PendingStats::default();
    }
    snap.enriched = true;
    true
}

/// What one build attempt asks the session loop to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildOutcome {
    /// A snapshot was published (or deliberately skipped) — wait for the next poke/tick.
    Done,
    /// Enrichment aborted on a mid-burst poke — rebuild at once.
    Interrupted,
    /// The attempt failed before it could publish a table, or published a snapshot whose
    /// phase-2 stats are still incomplete — retry on a bounded timer, because the event
    /// that would otherwise retry it may never come.
    Retry,
}

/// What one phase-2 pass achieved for the current lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase2Outcome {
    /// Every player's heavy stats are settled — fetched, or definitively absent (no
    /// competitive history). Only this may mark the cache `enriched`.
    Complete,
    /// At least one player's fetch failed for a transient reason (HTTP error, timeout,
    /// exhausted 429 retry). What we have is publishable, but not final.
    Partial,
    /// A poke arrived mid-burst — abort now and rebuild for the current state.
    Interrupted,
}

/// How many phase-2 passes a single lobby may spend on transient failures before the
/// remaining gaps are accepted as final. Without this bound the 1 s agent-select poll would
/// refetch the failed players every tick for the whole of agent select.
const MAX_PHASE2_ATTEMPTS: u32 = 3;

/// Whether phase-2 data may be marked final (`MatchCache::enriched`). A complete pass is
/// final at once; a partial one only once the per-lobby retry budget is spent. Pure.
fn enrichment_is_final(complete: bool, failed_attempts: u32) -> bool {
    complete || failed_attempts >= MAX_PHASE2_ATTEMPTS
}

/// Drain the poke channel, reporting whether any poke was waiting — a burst of own-presence
/// events collapses into the one rebuild that follows. Mid-burst this is also the abort check:
/// a new own-presence event (possibly a dodge / state transition) means the loop should rebuild
/// for the current state rather than block behind the remaining ~500 KB match-details fetches
/// (HIGH-2).
fn drain_pokes(rx: &mut mpsc::Receiver<Poke>) -> bool {
    let mut any = false;
    while rx.try_recv().is_ok() {
        any = true;
    }
    any
}

/// Await `fut`, cutting it short as soon as a poke arrives; `None` then means "abandon this
/// attempt and rebuild for the current state". This is what keeps a transition from waiting out
/// an in-flight request (up to the 15 s HTTP timeout) or a 429 backoff.
async fn until_poke<F: Future>(rx: &mut mpsc::Receiver<Poke>, fut: F) -> Option<F::Output> {
    tokio::pin!(fut);
    tokio::select! {
        out = &mut fut => Some(out),
        poke = rx.recv() => match poke {
            Some(_) => None,
            // Websocket task gone — nothing can interrupt any more, so see it through.
            None => Some(fut.as_mut().await),
        },
    }
}

/// Everything needed once connected to a running client.
struct Session {
    lockfile: Lockfile,
    local: LocalClient,
    remote: RemoteClient,
    own_puuid: String,
    static_data: StaticData,
    /// The authoritative client version the static-data top-up is working on and when it last
    /// tried, so the top-up in `build_snapshot` runs on a cooldown instead of once per
    /// rebuild. `None` until own presence reports a version.
    static_top_up: Option<(String, Instant)>,
    season_id: String,
    /// Content-service season list, kept for the peak-rank act label.
    seasons: Vec<content::Season>,
    /// Names/MMR/stats cached per match id (+ state) so an in-match presence update (score
    /// changes every round) does not refetch them (L1).
    cache: MatchCache,
    /// HS% + KD cached across matches within the session, keyed by puuid + the player's
    /// newest competitive match id — so a returning player's ~500 KB match-details are not
    /// re-downloaded while their newest match is unchanged (phase 2 constraint).
    recent_stats_cache: RecentStatsCache,
    /// Parsed match-details keyed by match id, so a match several lobby players share is
    /// downloaded once and a retry pass refetches only the ids that failed.
    match_details: MatchDetailsCache,
    /// The backoff a 429 imposed, honored by every remote call for the rest of the session.
    rate_limit: RateLimitGate,
}

/// Per-match cache of the expensive per-player lookups. Keyed by match id AND whether the
/// data was gathered for the INGAME state; either a new match id or a pregame→ingame
/// upgrade (same GUID, but enemies + loadouts now available) invalidates it. `enriched`
/// tracks whether phase 2 (updates/HS/skins) has been gathered yet, so the two-phase emit
/// can publish the fast fields first and fill the rest in on a later event.
#[derive(Default)]
struct MatchCache {
    match_id: Option<String>,
    /// Whether the cached rows were gathered for the INGAME state (vs PREGAME).
    ingame: bool,
    /// Whether phase 2 (updates/HS/skins) has been gathered — only then is the cache "fresh"
    /// enough to skip all fetching.
    enriched: bool,
    /// Phase-2 passes this lobby has spent on transient failures, bounded by
    /// `MAX_PHASE2_ATTEMPTS`.
    phase2_attempts: u32,
    names: HashMap<String, String>,
    mmr: HashMap<String, MmrResponse>,
    /// puuid -> ΔRR + last-5 pips (phase 2).
    updates: HashMap<String, RrHistory>,
    /// puuid -> HS% + KD over recent matches (phase 2; inner None == "N/a").
    recent_stats: HashMap<String, RecentStats>,
    /// puuid -> equipped Vandal/Phantom skin uuids (phase 2, INGAME only).
    skins: HashMap<String, PlayerSkinIds>,
    /// The coregame match payload's roster, kept for INGAME only (see `cached_roster`).
    roster: Option<CachedRoster>,
    /// The agent-select match id, once resolved (see `pregame_id`).
    pregame_match_id: Option<String>,
    /// Whether the pregame build in progress saw every ally locked in — the poll tick then
    /// stops (see `poll_interval`). Cleared at the start of every rebuild, so only a build
    /// that got as far as parsing a fully locked roster keeps the tick paused.
    pregame_locked: bool,
    /// Whether the immediate 404 retry has already been spent on the current unresolved
    /// state-transition race (see `fetch_with_retry`). Cleared by the next match-id fetch that
    /// succeeds, and by any invalidation.
    id_retry_spent: bool,
}

/// What a snapshot needs out of the match payload. Once a match is running the roster, the
/// agents and the map are fixed for the rest of it, so an own-presence event can render from
/// this instead of re-downloading the coregame match. Agent select changes all three, so it is
/// never cached there.
struct CachedRoster {
    players: Vec<MatchPlayer>,
    own_team: Option<String>,
    map_id: Option<String>,
}

impl MatchCache {
    /// True only when the cache holds the FULLY enriched data for this exact match AND
    /// state — the one case that skips all fetching.
    ///
    /// The state guard is the HIGH-1 fix: pregame and coregame share the same match GUID,
    /// so keying freshness on the id alone let a cache built in PREGAME (5 allies, no
    /// loadouts) be reused verbatim in INGAME — enemies never rendered and skins stayed
    /// null all match. A pregame-built cache (`ingame == false`) is therefore treated as
    /// STALE once the current state is INGAME. A phase-1-only cache (`enriched == false`)
    /// is never fresh either — phase 2 must still run.
    fn is_fresh_for(&self, match_id: &str, ingame: bool) -> bool {
        self.match_id.as_deref() == Some(match_id)
            && self.enriched
            && !(ingame && !self.ingame)
    }

    /// The roster to render `match_id` from without fetching the match payload at all: only
    /// INGAME, and only for the fully enriched cache of that exact match. The cheap match-id
    /// GET stays the change detector, so a match that ended (or a new one) still lands on the
    /// full path.
    fn cached_roster(&self, match_id: &str, ingame: bool) -> Option<&CachedRoster> {
        if !ingame || !self.is_fresh_for(match_id, true) {
            return None;
        }
        self.roster.as_ref()
    }

    /// The agent-select match id to reuse instead of asking `pregame/v1/players` for it again:
    /// it is fixed for the lobby's lifetime, so the poll tick costs one request rather than
    /// two. Never serves INGAME — that id comes from the coregame endpoint, and the cheap
    /// coregame match-id GET is what detects the transition.
    fn pregame_id(&self, ingame: bool) -> Option<String> {
        if ingame {
            None
        } else {
            self.pregame_match_id.clone()
        }
    }

    /// Prepare the cache to (re)build `match_id` at `ingame`. Same-match data is KEPT so a
    /// pregame→ingame upgrade — or a phase-1 cache being enriched, or a burst resumed after
    /// an abort — reuses the already-fetched per-puuid rows and fetches only what's missing
    /// (HIGH-1: do not redo the whole burst on the upgrade). A different match id drops the
    /// stale data. `enriched` is reset so phase 2 runs again.
    fn begin_match(&mut self, match_id: &str, ingame: bool) {
        let new_match = self.match_id.as_deref() != Some(match_id);
        if new_match {
            self.names.clear();
            self.mmr.clear();
            self.updates.clear();
            self.recent_stats.clear();
            self.skins.clear();
            self.roster = None;
        }
        // A different match — or a pregame→ingame upgrade, which brings a new half-roster and
        // the loadouts along with it — restores the phase-2 retry budget.
        if new_match || (ingame && !self.ingame) {
            self.phase2_attempts = 0;
        }
        self.match_id = Some(match_id.to_string());
        self.ingame = ingame;
        self.enriched = false;
    }

    /// Lift the agent-select poll pause. Every rebuild starts here, because only the build
    /// that goes on to parse a fully locked pregame roster may set the flag again: a build
    /// that fails anywhere earlier (a 404 transition race, an unreadable payload) then leaves
    /// the 1 s tick running to recover on its own, instead of stranding the lobby on a pause
    /// nothing is scheduled to lift.
    fn clear_pregame_lock(&mut self) {
        self.pregame_locked = false;
    }

    /// Drop everything (transition to MENUS / not-running).
    fn invalidate(&mut self) {
        *self = Self::default();
    }
}

/// How many players the session keeps stats for. Ten per lobby, so this covers a long evening
/// of matches plus their repeat players; past that the least recently added player goes.
const RECENT_STATS_CACHE_CAP: usize = 128;

/// Session-lived match-details stat cache: puuid -> (newest competitive match id the stats
/// were computed from, the HS% + KD they yielded). Both figures come from the same payloads,
/// so they share one entry. Persists across matches (NOT cleared on MENUS) so it
/// self-invalidates only when a player's newest competitive match changes.
#[derive(Default)]
struct RecentStatsCache {
    map: HashMap<String, (String, RecentStats)>,
    /// Insertion order, so the cap evicts the player held longest.
    order: VecDeque<String>,
}

impl RecentStatsCache {
    /// Cached stats for `puuid` iff they were computed from the same `newest_match_id`.
    fn get(&self, puuid: &str, newest_match_id: &str) -> Option<RecentStats> {
        self.map
            .get(puuid)
            .filter(|(id, _)| id == newest_match_id)
            .map(|(_, stats)| *stats)
    }

    fn put(&mut self, puuid: &str, newest_match_id: &str, stats: RecentStats) {
        if self
            .map
            .insert(puuid.to_string(), (newest_match_id.to_string(), stats))
            .is_some()
        {
            return; // already queued for eviction under this puuid
        }
        self.order.push_back(puuid.to_string());
        while self.order.len() > RECENT_STATS_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

/// How one match id of a player's HS%/KD window is served, before any request is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowStep {
    /// Already parsed — this is what it contributes to the player.
    Cached(MatchTotals),
    /// It already failed earlier in this pass, for this player or another one. Lobby players
    /// share recent matches, so retrying it per player would multiply one dead id across the
    /// whole lobby; the window is simply left incomplete, which is what keeps the pass
    /// non-final so a later pass can try the id again.
    Failed,
    /// Not seen yet — download it.
    Fetch,
}

/// Decide how to serve `match_id` for `puuid` this pass, from the session cache and the ids
/// that have already failed in it. Pure.
fn window_step(
    cache: &MatchDetailsCache,
    failed: &HashSet<String>,
    match_id: &str,
    puuid: &str,
) -> WindowStep {
    match cache.totals_for(match_id, puuid) {
        Some(totals) => WindowStep::Cached(totals),
        None if failed.contains(match_id) => WindowStep::Failed,
        None => WindowStep::Fetch,
    }
}

/// How many parsed matches the session keeps. One full lobby's HS%/KD window spans at most
/// `10 * RECENT_MATCHES_FOR_HS` = 50 matches, so this covers a lobby plus its overlap with the
/// next one; past that the oldest entry goes.
const MATCH_DETAILS_CACHE_CAP: usize = 128;

/// Session-lived cache of parsed match-details, keyed by match id. Lobby players share recent
/// matches, so keying the download by match instead of by player collapses the duplicates —
/// and a pass that failed halfway refetches only the ids it missed. Only the handful of
/// per-player numbers HS%/KD need are kept; the ~500 KB payload is dropped as soon as it is
/// parsed.
#[derive(Default)]
struct MatchDetailsCache {
    map: HashMap<String, MatchContribution>,
    /// Insertion order, so the cap evicts the oldest match.
    order: VecDeque<String>,
}

impl MatchDetailsCache {
    /// What `match_id` contributes to `puuid`'s window, or `None` when the match is not held.
    /// A cached match that the player never appeared in contributes nothing, which is a hit.
    fn totals_for(&self, match_id: &str, puuid: &str) -> Option<MatchTotals> {
        self.map
            .get(match_id)
            .map(|c| c.get(puuid).copied().unwrap_or_default())
    }

    fn put(&mut self, match_id: &str, contribution: MatchContribution) {
        if self.map.insert(match_id.to_string(), contribution).is_some() {
            return;
        }
        self.order.push_back(match_id.to_string());
        while self.order.len() > MATCH_DETAILS_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

/// Top-level loop: wait for the client, connect, run until the connection drops, repeat.
pub async fn tracker_main(state: Arc<TrackerState>, emitter: Arc<dyn Emitter>) {
    loop {
        // Phase 1: wait for the lockfile.
        let lockfile = loop {
            match lockfile::read() {
                Ok(lf) => {
                    vlt_log!("conn", "lockfile found (port {})", lf.port);
                    break lf;
                }
                Err(_) => {
                    publish(
                        &state,
                        &emitter,
                        TrackerSnapshot::not_running(Some("Waiting for Valorant...".into())),
                    );
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        };

        // Phase 2: connect.
        match connect(lockfile).await {
            Ok(mut session) => {
                vlt_log!(
                    "conn",
                    "session up  region={} shard={} version={} season={}",
                    session.remote.hosts.region,
                    session.remote.hosts.shard,
                    session.remote.auth.client_version,
                    session.season_id
                );
                run_session(&mut session, &state, &emitter).await;
            }
            // Underscore-bound: the reason is a dev-log detail only, and the log is compiled
            // out of release builds.
            Err(_err) => {
                vlt_log!("conn", "connect failed: {:?}", _err);
                publish(
                    &state,
                    &emitter,
                    TrackerSnapshot::not_running(Some("Connecting to Valorant...".into())),
                );
            }
        }
        // Connection lost / not ready — brief pause then retry from the top.
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Build a `Session`: auth, hosts, static data, season id.
async fn connect(lockfile: Lockfile) -> Result<Session> {
    let local = LocalClient::new(lockfile.clone())?;
    let entitlements = local.entitlements().await?;
    let region = local.region_locale().await?.region;
    let hosts = build_hosts(&region);

    // Static data (public host, valid TLS).
    let public = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    // A failed/incomplete static fetch degrades to unresolved agent+map names and icons rather
    // than caching empty mappings under a valid version; `build_snapshot` retries it once the
    // authoritative client version arrives. The version itself is still needed for
    // the remote headers, so fall back to fetching it on its own.
    let static_data = static_data::fetch(&public).await.unwrap_or_default();
    let client_version = if static_data.version.is_empty() {
        static_data::fetch_version(&public).await.unwrap_or_default()
    } else {
        static_data.version.clone()
    };

    let auth = Auth {
        access_token: entitlements.access_token.clone(),
        entitlements_token: entitlements.token.clone(),
        client_version,
    };
    let remote = RemoteClient::new(hosts, auth)?;

    // Seasons from the content service: the current season id, plus the list itself for the
    // peak-rank act label.
    let seasons = match remote.content().await {
        Ok(content_json) => content::parse_seasons(&content_json),
        Err(_) => Vec::new(),
    };
    let season_id = content::current_season_id(&seasons).unwrap_or_default();

    Ok(Session {
        lockfile,
        local,
        remote,
        own_puuid: entitlements.subject,
        static_data,
        static_top_up: None,
        season_id,
        seasons,
        cache: MatchCache::default(),
        recent_stats_cache: RecentStatsCache::default(),
        match_details: MatchDetailsCache::default(),
        rate_limit: RateLimitGate::default(),
    })
}

/// Re-fetch tokens after a BAD_CLAIMS and update the remote client. The puuid is the same
/// account for the life of the session, so it is not overwritten here.
async fn refresh_tokens(session: &mut Session) -> Result<()> {
    let entitlements = session.local.entitlements().await?;
    session
        .remote
        .set_tokens(entitlements.access_token, entitlements.token);
    vlt_log!("conn", "token refresh");
    Ok(())
}

/// How long the static-data top-up waits before trying the same authoritative client version
/// again. Long enough that the 1 s agent-select poll costs at most one `/version` GET a minute,
/// short enough that a valorant-api catching up mid-session is picked up within one.
const STATIC_TOP_UP_COOLDOWN: Duration = Duration::from_secs(60);

/// Whether the static data is worth (re)fetching for the authoritative `presence_version`:
/// true when what we hold is incomplete or keyed to a different version, and no attempt for
/// that version is still on cooldown. `last_attempt` is the version the top-up is working on
/// plus how long ago it last tried. Pure.
fn needs_static_top_up(
    held_version: &str,
    held_complete: bool,
    presence_version: &str,
    last_attempt: Option<(&str, Duration)>,
) -> bool {
    if held_complete && held_version == presence_version {
        return false;
    }
    match last_attempt {
        Some((version, elapsed)) if version == presence_version => {
            elapsed >= STATIC_TOP_UP_COOLDOWN
        }
        _ => true,
    }
}

/// A cooled-down best-effort static-data (re)fetch for the authoritative client version.
/// Covers two cases: a bootstrap fetch that failed (nothing was cached, so names/icons are
/// unresolved) and a valorant-api version lagging the running client, where the cache is keyed
/// to the older patch. Only data that arrives complete AND keyed to `presence_version` settles
/// it — a failed fetch, or one that comes back still on the older patch, just starts the
/// cooldown again, so valorant-api catching up hours into a session is still picked up. The
/// cooldown is what keeps that from becoming a request loop under the 1 s agent-select poll.
async fn top_up_static_data(session: &mut Session, presence_version: &str) {
    let last_attempt = session.static_top_up.as_ref().map(|(v, at)| (v.as_str(), at.elapsed()));
    if !needs_static_top_up(
        &session.static_data.version,
        session.static_data.is_complete(),
        presence_version,
        last_attempt,
    ) {
        return;
    }
    session.static_top_up = Some((presence_version.to_string(), Instant::now()));
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(20)).build() else {
        return;
    };
    if let Ok(data) = static_data::fetch(&client).await {
        session.static_data = data;
    }
}

/// Run a connected session: emit initial state, then react to websocket presence events
/// until the client is gone (websocket task drops tx, ending the loop).
async fn run_session(session: &mut Session, state: &Arc<TrackerState>, emitter: &Arc<dyn Emitter>) {
    // Poke channel: the websocket sends a `Poke` per OWN Valorant presence event, and one after
    // every reconnect so we re-poll presence for any transition missed while the socket was
    // down (C2). The task ends (dropping tx) only when the client is gone.
    let (tx, mut rx) = mpsc::channel::<Poke>(32);
    let ws_lockfile = session.lockfile.clone();
    let own = session.own_puuid.clone();
    let ws_state = Arc::clone(state);
    let ws_emitter = Arc::clone(emitter);
    let ws_task = tokio::spawn(async move {
        let mut backoff = 2u64;
        loop {
            match crate::riot::websocket::run_listener(&ws_lockfile, &own, tx.clone()).await {
                // Connection had come up then dropped — reset the backoff (C8).
                Ok(()) => backoff = 2,
                // Never connected — publish a "reconnecting" snapshot at once so a stale
                // INGAME table can't linger while we retry (C3).
                Err(_) => publish(
                    &ws_state,
                    &ws_emitter,
                    TrackerSnapshot::not_running(Some("Reconnecting to Valorant...".into())),
                ),
            }
            // Bail immediately if the lockfile no longer matches the credentials this
            // session is using rather than burning the full backoff schedule on a dead
            // endpoint (C3). That covers a removed lockfile (client gone) and a client that
            // restarted fast enough to rewrite the file with a new pid/port/password —
            // retrying the old endpoint would otherwise never end, so the top level would
            // never re-read auth. Ending the task drops `tx`, which ends
            // `run_session` and sends the top-level loop back to the lockfile.
            if !lockfile::still_current(&ws_lockfile) {
                vlt_log!("conn", "lockfile stale/gone -> ending session");
                break;
            }
            // Re-poll after the drop so any transition during the outage is picked up (C2).
            if tx.send(Poke).await.is_err() {
                break; // receiver gone
            }
            vlt_log!("ws", "reconnecting in {backoff}s");
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    });

    // Initial snapshot from REST, then react to state-change pushes. A build that was
    // interrupted mid-enrichment (a poke arrived — possibly a dodge/transition) rebuilds at
    // once for the current state instead of waiting on the next event, so transitions are
    // never lost behind the burst. A build that FAILED gets a bounded backoff tick
    // instead: the published status may still say Menus/not-running, so the
    // Pregame-only poll tick would not cover it and the rebuild could wait forever for
    // another qualifying event.
    let mut retry_attempt = 0usize;
    loop {
        let outcome = {
            let mut ctx = BuildCtx { state, emitter, rx: &mut rx };
            build_and_publish(session, &mut ctx).await
        };
        if outcome == BuildOutcome::Retry {
            if let Some(delay) = retry_delay(retry_attempt) {
                retry_attempt += 1;
                if !wait_before_retry(&mut rx, delay).await {
                    break; // websocket task ended -> client gone
                }
                continue;
            }
            // Schedule exhausted — fall back to the event-driven wait below, after settling
            // the published table: the rebuild that would have filled its outstanding cells is
            // no longer scheduled, so leaving them pending would strand them on skeletons.
            // Only the view is settled; the cache keeps its own `enriched`/attempt bookkeeping,
            // so the next event still fetches what is missing.
            let mut snap = state.snapshot();
            if settle_pending(&mut snap) {
                vlt_log!("enrich", "retry budget exhausted; settling pending cells as final");
                publish(state, emitter, snap);
            }
        }
        // A success, an interruption, or an exhausted schedule all start the next failure
        // with a full retry budget.
        retry_attempt = 0;
        if outcome == BuildOutcome::Interrupted {
            vlt_log!("rebuild", "interrupted mid-enrichment, rebuilding now");
            let _ = drain_pokes(&mut rx);
            continue;
        }
        if !wait_for_rebuild_poke(&mut rx, state, session.cache.pregame_locked).await {
            break; // websocket task ended -> client gone
        }
    }

    ws_task.abort();
}

/// Wait for the next poke, collapsing a burst into one rebuild (L1). Returns false when the
/// websocket task ended.
///
/// In Pregame the wait is bounded by `poll_interval`, so an elapsed timeout triggers a rebuild
/// just like a poke would — that tick is what makes teammates' agent picks visible, since Riot
/// pushes no presence event for them. Identical rebuilds are suppressed by the snapshot dedup
/// in `publish`, and a tick can't stack with pokes: the rebuild drains the channel anyway. A
/// roster whose every ally has locked in has nothing left to tick for, so the wait goes back to
/// being purely event-driven.
async fn wait_for_rebuild_poke(
    rx: &mut mpsc::Receiver<Poke>,
    state: &TrackerState,
    roster_locked: bool,
) -> bool {
    match poll_interval(state.status(), roster_locked) {
        Some(tick) => match tokio::time::timeout(tick, rx.recv()).await {
            Ok(Some(_)) => {
                vlt_log!("rebuild", "poke");
                true
            }
            Ok(None) => false, // websocket task ended -> client gone
            Err(_) => {
                vlt_log!("rebuild", "pregame tick");
                true // poll tick -> rebuild agent select
            }
        },
        None => {
            let received = rx.recv().await.is_some();
            if received {
                vlt_log!("rebuild", "poke");
            }
            received
        }
    }
}

/// Wait out a retry backoff. The wait is cut short by any poke — a real event is a
/// better reason to rebuild than the timer, and the rebuild drains the channel anyway, so no
/// poke is lost. Returns false when the websocket task ended (client gone).
async fn wait_before_retry(rx: &mut mpsc::Receiver<Poke>, delay: Duration) -> bool {
    match tokio::time::timeout(delay, rx.recv()).await {
        Ok(Some(_)) => {
            vlt_log!("rebuild", "poke cut the {}ms retry backoff short", delay.as_millis());
            true
        }
        Ok(None) => false, // websocket task ended -> client gone
        Err(_) => {
            vlt_log!("rebuild", "{}ms retry backoff elapsed", delay.as_millis());
            true // backoff elapsed -> retry the build
        }
    }
}

/// Whether an error means the next attempt should run with freshly fetched tokens: stale
/// claims, and a rate limit that outlived its backed-off retry — vRY clears its cached headers
/// before retrying either (spec §10.3, §10.6). Pure.
fn warrants_token_refresh(err: &Error) -> bool {
    matches!(err, Error::BadClaims | Error::RateLimited(_))
}

/// Build a snapshot and publish it, routing a stale-token failure through a token refresh +
/// one retry (C7) and NotReady through a "Loading..." placeholder. Shared by the initial paint
/// and the event loop so both handle token expiry identically. Every path that did NOT publish
/// a table reports `Retry` so the caller schedules a bounded rebuild rather than
/// depending on an event that may never arrive.
async fn build_and_publish(session: &mut Session, ctx: &mut BuildCtx<'_>) -> BuildOutcome {
    match build_snapshot(session, ctx).await {
        Ok(outcome) => outcome,
        Err(Error::NotReady) => {
            publish(
                ctx.state,
                ctx.emitter,
                TrackerSnapshot::not_running(Some("Loading...".into())),
            );
            BuildOutcome::Retry
        }
        // Exactly one refresh per build attempt: the retry's own errors fall through to
        // `Retry`, so a persistently rejected token can't spin here.
        Err(err) if warrants_token_refresh(&err) => match refresh_tokens(session).await {
            Ok(()) => build_snapshot(session, ctx).await.unwrap_or(BuildOutcome::Retry),
            Err(_) => BuildOutcome::Retry,
        },
        // Transient (404 race, local hiccup) — retry on the backoff schedule.
        Err(_) => BuildOutcome::Retry,
    }
}

/// Fetch the presence roster for one rebuild. In dev capture mode the untouched response
/// body is dumped alongside the snapshots (see `docs/testing.md`); everywhere else it is a
/// plain roster fetch, with no extra clone or serialization.
async fn fetch_presences(local: &LocalClient) -> Result<Vec<presence::RawPresence>> {
    #[cfg(debug_assertions)]
    {
        let (presences, raw) = local.presences_with_raw(debug_capture::enabled()).await?;
        if let Some(raw) = raw {
            debug_capture::write_presences(&raw);
        }
        Ok(presences)
    }
    #[cfg(not(debug_assertions))]
    Ok(local.presences_with_raw(false).await?.0)
}

/// Build (and publish) a full snapshot for the current state (Menus / Pregame / Ingame).
/// The returned `BuildOutcome` tells the session loop what to do next (see
/// `build_and_publish`).
async fn build_snapshot(session: &mut Session, ctx: &mut BuildCtx<'_>) -> Result<BuildOutcome> {
    session.cache.clear_pregame_lock();
    let presences = fetch_presences(&session.local).await?;
    let own = presences
        .iter()
        .find(|p| p.puuid == session.own_puuid && p.is_valorant());
    // An absent own presence, or one whose `private` blob is still empty, means the client
    // hasn't published our presence yet — surface NotReady (poll again) rather than
    // defaulting to a Menus render (C10).
    let info = match own {
        Some(p) => presence::info_for(p)?,
        None => return Err(Error::NotReady),
    };

    // Once connected, the client version from own presence is authoritative over the
    // valorant-api.com bootstrap value, which can lag the real client (probe finding).
    if let Some(v) = info.client_version.as_deref() {
        if v != session.remote.auth.client_version {
            session.remote.set_client_version(v.to_string());
        }
        top_up_static_data(session, v).await;
    }

    match info.session_state {
        // Party grouping decodes every online friend's presence blob, and only a match table
        // renders it, so it is built here rather than for every Menus rebuild.
        Some(state @ (SessionLoopState::Pregame | SessionLoopState::Ingame)) => {
            let parties = presence::party_grouping(&presences);
            let ingame = state == SessionLoopState::Ingame;
            build_match_snapshot(session, &info, &parties, ingame, ctx).await
        }
        // MENUS or unknown -> menus snapshot. A new match always starts from menus, so this
        // is where the per-match cache is invalidated (L1 + pitfall §12).
        _ => {
            session.cache.invalidate();
            publish(
                ctx.state,
                ctx.emitter,
                TrackerSnapshot {
                    status: AppStatus::Menus,
                    map: None,
                    mode: None,
                    own_team: None,
                    players: Vec::new(),
                    enriched: true,
                    last_updated: crate::riot::types::now_millis(),
                    message: None,
                },
            );
            Ok(BuildOutcome::Done)
        }
    }
}

/// Fetch + assemble a pregame or coregame snapshot, publishing incrementally: the table
/// appears as soon as the names are in and each stat group fills its cells in as it settles,
/// paced by `PROGRESS_COALESCE_MS` and flushed at every phase boundary. Reports `Interrupted`
/// when enrichment aborted on a mid-burst poke, and `Retry` when phase 2 could not settle
/// every player — the snapshot is published either way, just not as `enriched`.
async fn build_match_snapshot(
    session: &mut Session,
    info: &PresenceInfo,
    parties: &HashMap<String, String>,
    ingame: bool,
    ctx: &mut BuildCtx<'_>,
) -> Result<BuildOutcome> {
    let own = session.own_puuid.clone();

    // Match id. On the RESOURCE_NOT_FOUND transition race, retry once — after ~5s for
    // coregame, immediately for pregame (spec §10.5, C9) — but only while that retry has not
    // already been spent on the same unresolved race, so a transition that outlasts several
    // backoff cycles costs one 404 per cycle instead of two. The 429 retry sits INSIDE the
    // 404-race retry so a rate limit is backed off once per attempt rather than being
    // multiplied by it. Agent select reuses the id it already resolved (`pregame_id`).
    let match_id = match session.cache.pregame_id(ingame) {
        // A cached id means no race is in progress, so the next one gets its retry.
        Some(id) => {
            session.cache.id_retry_spent = false;
            id
        }
        None => {
            let retry_404 = !session.cache.id_retry_spent;
            let id_fetch = if ingame {
                until_poke(
                    ctx.rx,
                    fetch_with_retry(Duration::from_secs(5), retry_404, || {
                        with_rate_limit_retry(&session.rate_limit, || {
                            session.remote.coregame_match_id(&own)
                        })
                    }),
                )
                .await
            } else {
                until_poke(
                    ctx.rx,
                    fetch_with_retry(Duration::ZERO, retry_404, || {
                        with_rate_limit_retry(&session.rate_limit, || {
                            session.remote.pregame_match_id(&own)
                        })
                    }),
                )
                .await
            };
            let Some(id_json) = id_fetch else {
                return Ok(BuildOutcome::Interrupted);
            };
            let resolved = id_json
                .and_then(|v| match_state::extract_match_id(&v).ok_or(Error::ResourceNotFound));
            match resolved {
                Ok(id) => {
                    session.cache.id_retry_spent = false;
                    if !ingame {
                        session.cache.pregame_match_id = Some(id.clone());
                    }
                    id
                }
                Err(err) => {
                    if matches!(err, Error::ResourceNotFound) {
                        vlt_log!("rebuild", "match id 404 (immediate retry spent={})", !retry_404);
                        session.cache.id_retry_spent = true;
                    }
                    return Err(err);
                }
            }
        }
    };

    let status = if ingame { AppStatus::Ingame } else { AppStatus::Pregame };
    let mode = if info.is_custom_game() {
        Some("Custom Game".to_string())
    } else {
        info.queue_id.as_deref().map(game_mode_name)
    };

    // The in-match steady state: the id above already confirmed we are still in the same
    // match, and everything the match payload would carry is fixed for its duration, so this
    // path renders an own-presence event (the score rides the presence data) with one GET.
    if let Some(roster) = session.cache.cached_roster(&match_id, ingame) {
        let parts = SnapshotParts {
            players: &roster.players,
            parties,
            own_team: roster.own_team.clone(),
            map: session.static_data.map(roster.map_id.as_deref()),
            mode,
            status,
            static_data: &session.static_data,
            own_puuid: &session.own_puuid,
            season_id: &session.season_id,
            seasons: &session.seasons,
            ingame,
        };
        let snap = assemble_snapshot(&parts, &session.cache, true, true);
        publish(ctx.state, ctx.emitter, snap);
        return Ok(BuildOutcome::Done);
    }

    // Match data (owned Values consumed by the extractors — no deep clone, L4).
    let match_fetch = if ingame {
        let call = || session.remote.coregame_match(&match_id);
        let fetch = with_rate_limit_retry(&session.rate_limit, call);
        until_poke(ctx.rx, fetch).await
    } else {
        let call = || session.remote.pregame_match(&match_id);
        let fetch = with_rate_limit_retry(&session.rate_limit, call);
        until_poke(ctx.rx, fetch).await
    };
    let Some(match_json) = match_fetch else {
        return Ok(BuildOutcome::Interrupted);
    };
    // A cached agent-select id that the match endpoint no longer knows means the lobby is gone
    // (dodged, or already transitioned): drop it so the next attempt resolves the id afresh
    // and the 404-race handling applies to it again.
    let match_json = match match_json {
        Ok(value) => value,
        Err(err) => {
            if matches!(err, Error::ResourceNotFound) {
                session.cache.pregame_match_id = None;
            }
            return Err(err);
        }
    };
    let (players, own_team, map_id): (Vec<MatchPlayer>, Option<String>, Option<String>) = if ingame
    {
        // A payload we can't read is an error, not an empty lobby: erroring here happens
        // BEFORE `begin_match`, so nothing is cached and the loop retries.
        let data = match_state::extract_coregame(match_json, &own)?;
        (data.players, data.own_team, data.map_id)
    } else {
        let data = match_state::extract_pregame(match_json, &own)?;
        (data.players, data.own_team, data.map_id)
    };

    // Agent select stops polling once nothing is left to see (see `poll_interval`).
    if !ingame && match_state::roster_fully_locked(&players) {
        vlt_log!("rebuild", "pregame roster fully locked; poll tick paused");
        session.cache.pregame_locked = true;
    }

    let map = session.static_data.map(map_id.as_deref());

    // Fully cached (enriched, correct state) -> a single snapshot, no fetch. This is the
    // common in-match path: the score changes every round but nothing here is refetched.
    if session.cache.is_fresh_for(&match_id, ingame) {
        let parts = SnapshotParts {
            players: &players,
            parties,
            own_team,
            map,
            mode,
            status,
            static_data: &session.static_data,
            own_puuid: &session.own_puuid,
            season_id: &session.season_id,
            seasons: &session.seasons,
            ingame,
        };
        let snap = assemble_snapshot(&parts, &session.cache, true, true);
        publish(ctx.state, ctx.emitter, snap);
        return Ok(BuildOutcome::Done);
    }

    // Not fresh: prepare the cache for this match (keeping any same-match data to reuse on a
    // pregame→ingame upgrade), then proactively refresh the token before the burst so a
    // BadClaims can't strand us mid-burst and force a redo (MEDIUM-2). Best-effort: if the
    // refresh fails we proceed with the current token and the BadClaims arm still covers it.
    session.cache.begin_match(&match_id, ingame);
    if ingame {
        session.cache.roster = Some(CachedRoster {
            players: players.clone(),
            own_team: own_team.clone(),
            map_id,
        });
    }
    let _ = refresh_tokens(session).await;

    let puuids: Vec<String> = players.iter().map(|p| p.puuid.clone()).collect();

    let parts = SnapshotParts {
        players: &players,
        parties,
        own_team,
        map,
        mode,
        status,
        static_data: &session.static_data,
        own_puuid: &session.own_puuid,
        season_id: &session.season_id,
        seasons: &session.seasons,
        ingame,
    };
    let mut progress = Progress {
        ctx,
        parts: &parts,
        gate: ProgressGate::default(),
        // A pass that already got the loadouts (and failed elsewhere) does not refetch them,
        // so they are settled before this build starts.
        loadouts_fetched: !session.cache.skins.is_empty(),
    };

    // === Phase 1: names + MMR (fast fields). Publishes as they land. ===
    let gate = &session.rate_limit;
    let phase1 =
        fetch_phase1(&session.remote, gate, &mut session.cache, &puuids, &mut progress).await;
    // The phase boundary flushes whatever the outcome: a pass that ended early still owes the
    // table what it managed to fetch, and that is what a giveup would settle on.
    progress.publish_progress(&session.cache, true);
    if !phase1? {
        // Phase-1 data stays cached (enriched == false), so the rebuild fetches only the rest.
        return Ok(BuildOutcome::Interrupted);
    }

    // === Phase 2: competitiveupdates + HS%/KD + loadout skins. Interruptible. ===
    let phase2 = fetch_phase2(
        &session.remote,
        &session.rate_limit,
        &mut session.cache,
        &mut session.recent_stats_cache,
        &mut session.match_details,
        &puuids,
        ingame,
        &match_id,
        &mut progress,
    )
    .await;
    let outcome = match phase2 {
        Ok(outcome) => outcome,
        // The pass that failed still owes the table what it fetched before it stopped; the
        // final snapshot below is what would otherwise have carried it.
        Err(err) => {
            progress.publish_progress(&session.cache, true);
            return Err(err);
        }
    };

    // A transient gap must NOT be published as `enriched` — the contract says an enriched
    // null means the player genuinely has no data. Publish what we have with the flag still
    // false and ask for a bounded retry, which refetches only the players that failed.
    // Once the budget is spent the remaining gaps are accepted as final, so a
    // broken endpoint can't turn the 1 s agent-select poll into a permanent refetch loop.
    let complete = match outcome {
        // Phase-1 data stays cached (enriched == false) so the immediate rebuild reuses it
        // and only finishes the missing phase-2 work.
        Phase2Outcome::Interrupted => return Ok(BuildOutcome::Interrupted),
        Phase2Outcome::Complete => true,
        Phase2Outcome::Partial => {
            session.cache.phase2_attempts += 1;
            false
        }
    };
    let is_final = enrichment_is_final(complete, session.cache.phase2_attempts);
    vlt_log!(
        "enrich",
        "phase2 outcome={outcome:?} attempts={}/{} final={is_final}",
        session.cache.phase2_attempts,
        MAX_PHASE2_ATTEMPTS
    );

    session.cache.enriched = is_final;
    let snap2 = assemble_snapshot(&parts, &session.cache, progress.loadouts_fetched, is_final);
    publish(progress.ctx.state, progress.ctx.emitter, snap2);
    Ok(if is_final { BuildOutcome::Done } else { BuildOutcome::Retry })
}

/// Assemble a `TrackerSnapshot` from whatever the cache currently holds. Every emit — the
/// progress ones, the final one and the fully-cached path — goes through here; a stat that
/// has not reached the cache yet is simply absent, and its row says so through `pending`.
///
/// `enriched` doubles as the assembler's finality, which is what makes the contract's
/// invariant hold by construction: an `enriched` snapshot carries no pending flags, so an
/// absent value on it is settled rather than in flight.
fn assemble_snapshot(
    parts: &SnapshotParts,
    cache: &MatchCache,
    loadouts_fetched: bool,
    enriched: bool,
) -> TrackerSnapshot {
    let rows = assemble_players(&AssembleInput {
        players: parts.players,
        names: &cache.names,
        mmr: &cache.mmr,
        parties: parts.parties,
        updates: &cache.updates,
        recent_stats: &cache.recent_stats,
        skins: &cache.skins,
        static_data: parts.static_data,
        own_puuid: parts.own_puuid,
        own_team: parts.own_team.as_deref(),
        current_season_id: parts.season_id,
        seasons: parts.seasons,
        finality: enriched,
        loadouts_fetched,
        ingame: parts.ingame,
    });
    TrackerSnapshot {
        status: parts.status,
        map: parts.map.clone(),
        mode: parts.mode.clone(),
        own_team: parts.own_team.clone(),
        players: rows,
        enriched,
        last_updated: crate::riot::types::now_millis(),
        message: None,
    }
}

/// Retry a fetch once on the RESOURCE_NOT_FOUND state-transition race (spec §10.5). `delay`
/// is the wait before the single retry (5s for coregame, zero for pregame — C9). `retry` is
/// false when an earlier attempt at the same still-unresolved race already spent that retry:
/// the outer backoff is then the only thing repeating the call, so a long transition costs one
/// 404 per cycle rather than a pair.
async fn fetch_with_retry<F, Fut>(delay: Duration, retry: bool, f: F) -> Result<serde_json::Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    match f().await {
        Err(Error::ResourceNotFound) if retry => {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            f().await
        }
        other => other,
    }
}

/// Backoff before the single 429 retry when the server sent no usable `Retry-After`
/// (spec §10.6, C5).
const RATE_LIMIT_BACKOFF_SECS: u64 = 6;

/// How long to wait before the single 429 retry: the server's `Retry-After` when it sent one
/// (already capped by `remote_api::parse_retry_after`), else the default backoff. Pure.
fn rate_limit_backoff(retry_after_secs: Option<u64>) -> Duration {
    Duration::from_secs(retry_after_secs.unwrap_or(RATE_LIMIT_BACKOFF_SECS))
}

/// The deadline a 429 imposed on the whole session. Riot rate-limits the client, not one
/// endpoint, so pd and glz share it. It lives beside the requests rather than inside them
/// because a request can be abandoned mid-backoff (a transition rebuilds at once): the sleep
/// goes with the cancelled future, the deadline does not, so the next attempt still waits out
/// what the server asked for instead of resending inside its window.
#[derive(Default)]
struct RateLimitGate {
    until: Mutex<Option<Instant>>,
}

impl RateLimitGate {
    /// How much of the deadline is left (`None` = free to send). Reaching it clears it.
    fn wait(&self) -> Option<Duration> {
        let mut until = self.until.lock().unwrap();
        let deadline = (*until)?;
        let remaining = deadline.checked_duration_since(Instant::now());
        if remaining.is_none() {
            *until = None;
        }
        remaining
    }

    /// Hold every request off for `backoff`. A deadline already further out wins — the
    /// stricter of two limits is the one to honor.
    fn arm(&self, backoff: Duration) {
        let deadline = Instant::now() + backoff;
        let mut until = self.until.lock().unwrap();
        match *until {
            Some(held) if held >= deadline => {}
            _ => *until = Some(deadline),
        }
    }
}

/// Run a remote fetch with one backed-off retry on a 429 (spec §10.6, C5) rather than
/// silently degrading. BadClaims and other errors pass straight through to the caller; so does
/// a 429 the retry could not clear, which `warrants_token_refresh` then routes into the token
/// refresh, since headers we keep sending are part of what the limiter counts.
/// Every pd AND glz call goes through here, including the match-id/match fetches the
/// 1 s agent-select poll drives: the wait happens INSIDE the build, so a rate-limited poll
/// tick stretches rather than stacking another attempt on top. A backoff still owed from an
/// earlier 429 — this call's or an abandoned one's — is waited out before anything is sent.
async fn with_rate_limit_retry<F, Fut>(gate: &RateLimitGate, f: F) -> Result<serde_json::Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    if let Some(owed) = gate.wait() {
        vlt_log!("net", "holding {}ms for earlier 429", owed.as_millis());
        tokio::time::sleep(owed).await;
    }
    match f().await {
        Err(Error::RateLimited(retry_after)) => {
            arm_rate_limit(gate, retry_after);
            if let Some(owed) = gate.wait() {
                vlt_log!("net", "holding {}ms for earlier 429", owed.as_millis());
                tokio::time::sleep(owed).await;
            }
            match f().await {
                // The retry's own Retry-After still binds later requests, even though no
                // third attempt is made here.
                Err(Error::RateLimited(retry_after)) => {
                    arm_rate_limit(gate, retry_after);
                    Err(Error::RateLimited(retry_after))
                }
                other => other,
            }
        }
        other => other,
    }
}

/// Hold the session off for what the 429 asked for (or the default backoff), announcing it.
fn arm_rate_limit(gate: &RateLimitGate, retry_after: Option<u64>) {
    let backoff = rate_limit_backoff(retry_after);
    vlt_log!("net", "429: backoff armed for {backoff:?} (retry_after={retry_after:?})");
    gate.arm(backoff);
}

/// Batch name resolution. Propagates the errors that want fresh tokens so the caller refreshes
/// tokens and retries once (C4) — an unspent rate limit included, since it is lobby-wide and
/// would otherwise settle the whole table on placeholder names. Other transport errors degrade
/// to an empty map (names render as placeholders rather than failing the whole table).
async fn fetch_names(
    remote: &RemoteClient,
    gate: &RateLimitGate,
    puuids: &[String],
) -> Result<HashMap<String, String>> {
    match with_rate_limit_retry(gate, || remote.names(puuids)).await {
        Ok(v) => match names::parse_name_response(&v) {
            Ok(map) => Ok(map),
            Err(e) if warrants_token_refresh(&e) => Err(e),
            Err(_) => Ok(HashMap::new()),
        },
        Err(e) if warrants_token_refresh(&e) => Err(e),
        Err(_) => Ok(HashMap::new()),
    }
}

/// A short pause between the per-player stat requests at match start (spec: bounded,
/// sequential-ish burst) — works alongside the existing 429 retry.
fn inter_request_delay() -> Duration {
    Duration::from_millis(INTER_REQUEST_DELAY_MS)
}

/// Phase 1 of the burst: resolve names (batch) + MMR (per player) into the cache,
/// fetching only the puuids not already cached from a same-match pregame build (HIGH-1
/// reuse). MMR is the WR source too, so this covers every "free" field. Stale claims and an
/// unspent rate limit propagate for the shared token refresh (C4/C7); any other single MMR
/// failure degrades that row to Unranked, which is the "ranks never error a row" contract for
/// a genuinely absent record — a lobby-wide rate limit is not that, and settling every row as
/// Unranked because of one would be wrong. The inter-request delay is applied only
/// BETWEEN requests (LOW: no trailing
/// sleep), and now covers the MMR fetches too (LOW: consistency with the spec's per-player
/// 120 ms). Returns false when a poke arrived while a request was in flight — whatever was
/// fetched stays cached, so the rebuild resumes rather than redoing it.
///
/// The names are the table's first paint, so they are flushed the moment they land; each MMR
/// then fills its row's rank cells in as it arrives.
async fn fetch_phase1(
    remote: &RemoteClient,
    gate: &RateLimitGate,
    cache: &mut MatchCache,
    puuids: &[String],
    progress: &mut Progress<'_, '_>,
) -> Result<bool> {
    // Names: one batch call for the puuids we don't already hold.
    let missing_names: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.names.contains_key(*p))
        .cloned()
        .collect();
    vlt_log!("enrich", "phase1: {} names to fetch", missing_names.len());
    if !missing_names.is_empty() {
        let Some(fetched) =
            until_poke(progress.ctx.rx, fetch_names(remote, gate, &missing_names)).await
        else {
            return Ok(false);
        };
        cache.names.extend(fetched?);
        progress.publish_progress(cache, true);
    }

    // MMR: per player, only the ones missing, spaced between requests.
    let missing_mmr: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.mmr.contains_key(*p))
        .cloned()
        .collect();
    vlt_log!("enrich", "phase1: {} mmr to fetch", missing_mmr.len());
    for (i, puuid) in missing_mmr.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(inter_request_delay()).await;
        }
        let Some(result) =
            until_poke(progress.ctx.rx, with_rate_limit_retry(gate, || remote.mmr(puuid))).await
        else {
            return Ok(false);
        };
        match result {
            Ok(v) => {
                cache.mmr.insert(puuid.clone(), parse_mmr(v));
                progress.publish_progress(cache, false);
            }
            Err(e) if warrants_token_refresh(&e) => return Err(e),
            Err(_) => { /* private profile / hiccup -> Unranked for this row only */ }
        }
    }
    Ok(true)
}

/// Phase 2 of the burst: competitiveupdates (ΔRR + last-5 + recent match ids),
/// HS% + KD (throttled + session-cached, both from the same match-details), and — INGAME
/// only — loadout skins. Only puuids not
/// already cached are fetched (HIGH-1 reuse of pregame data + resume-after-abort). Between
/// per-player requests it checks the poke channel and returns `Interrupted` to abort promptly
/// when a new presence event arrives, so a dodge/transition is not blocked behind the
/// remaining fetches (HIGH-2). Partial results are left in the cache so the rebuild only
/// finishes the missing work. BadClaims propagates for the shared refresh; a transient
/// per-player failure leaves that player's entry UNSET and reports `Partial`, so the caller
/// keeps the cache un-final and retries that player only — never publishing a transient gap
/// as a settled `enriched` null. The inter-request delay spaces requests only
/// BETWEEN them (LOW: no trailing sleep, none after a skip/cache-hit).
///
/// Each settled group is published as it lands (coalesced), so ΔRR, the last-5 pips, HS%/KD
/// and the skins reach the table one group at a time instead of all at the end.
#[allow(clippy::too_many_arguments)] // one caller; the parameters are the phase's inputs.
async fn fetch_phase2(
    remote: &RemoteClient,
    gate: &RateLimitGate,
    cache: &mut MatchCache,
    recent_stats_cache: &mut RecentStatsCache,
    match_details: &mut MatchDetailsCache,
    puuids: &[String],
    ingame: bool,
    match_id: &str,
    progress: &mut Progress<'_, '_>,
) -> Result<Phase2Outcome> {
    // Tracks whether any request has been issued yet, so the 120 ms delay only ever sits
    // *between* two real requests across the whole phase.
    let mut sent_any = false;
    // Set by any transient failure — the whole pass is then not final.
    let mut partial = false;
    // Match ids that failed this pass. Lobby players share recent matches, so without this a
    // single dead id would be retried once per player, every pass.
    let mut failed_matches: HashSet<String> = HashSet::new();

    // competitiveupdates: one request per player missing history.
    let missing_updates: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.updates.contains_key(*p))
        .cloned()
        .collect();
    for puuid in &missing_updates {
        if drain_pokes(progress.ctx.rx) {
            return Ok(Phase2Outcome::Interrupted);
        }
        if sent_any {
            tokio::time::sleep(inter_request_delay()).await;
        }
        sent_any = true;
        let Some(result) = until_poke(
            progress.ctx.rx,
            with_rate_limit_retry(gate, || remote.competitive_updates(puuid)),
        )
        .await
        else {
            return Ok(Phase2Outcome::Interrupted);
        };
        match result {
            Ok(v) => {
                cache.updates.insert(puuid.clone(), stats::parse_competitive_updates(v));
                progress.publish_progress(cache, false);
            }
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            // Transient: leave the entry unset so the retry refetches this player only.
            Err(_) => partial = true,
        }
    }

    // HS% + KD: up to RECENT_MATCHES_FOR_HS match-details per player missing them,
    // session-cached. Both figures come out of the same payloads — KD adds no request.
    for puuid in puuids {
        if cache.recent_stats.contains_key(puuid) {
            continue;
        }
        if drain_pokes(progress.ctx.rx) {
            return Ok(Phase2Outcome::Interrupted);
        }
        // No history entry at all means the competitiveupdates call above failed — a
        // transient gap, NOT "this player has no matches". Leave the stats unset.
        let Some(newest) = cache.updates.get(puuid).map(|h| h.newest_match_id().map(String::from))
        else {
            partial = true;
            continue;
        };
        let Some(newest) = newest else {
            // Definitively no recent competitive matches -> HS% is "N/a" and KD is null.
            cache.recent_stats.insert(puuid.clone(), RecentStats::default());
            progress.publish_progress(cache, false);
            continue;
        };
        // Session cache hit (same newest match) -> reuse, no match-details fetch.
        if let Some(stats) = recent_stats_cache.get(puuid, &newest) {
            vlt_log!("enrich", "stats cache hit for {}", crate::debug_log::short(puuid));
            cache.recent_stats.insert(puuid.clone(), stats);
            progress.publish_progress(cache, false);
            continue;
        }
        // Cache miss -> fetch up to N match-details and accumulate this player's hits.
        let match_ids = cache
            .updates
            .get(puuid)
            .map(|h| h.recent_match_ids.clone())
            .unwrap_or_default();
        let mut acc = MatchTotals::default();
        let mut any_failed = false;
        for mid in &match_ids {
            match window_step(match_details, &failed_matches, mid, puuid) {
                WindowStep::Cached(totals) => {
                    acc.add(totals);
                    continue;
                }
                WindowStep::Failed => {
                    any_failed = true;
                    continue;
                }
                WindowStep::Fetch => {}
            }
            if drain_pokes(progress.ctx.rx) {
                return Ok(Phase2Outcome::Interrupted);
            }
            if sent_any {
                tokio::time::sleep(inter_request_delay()).await;
            }
            sent_any = true;
            let Some(result) = until_poke(
                progress.ctx.rx,
                with_rate_limit_retry(gate, || remote.match_details(mid)),
            )
            .await
            else {
                return Ok(Phase2Outcome::Interrupted);
            };
            match result {
                Ok(v) => {
                    let contribution = stats::match_contribution(&v);
                    acc.add(contribution.get(puuid).copied().unwrap_or_default());
                    match_details.put(mid, contribution);
                }
                Err(Error::BadClaims) => return Err(Error::BadClaims),
                Err(_) => {
                    any_failed = true;
                    failed_matches.insert(mid.clone());
                }
            }
        }
        if any_failed {
            // A window missing some of its matches yields the WRONG HS%/KD, so neither figure
            // is cached for this player (the session entry would otherwise stay wrong until
            // their next competitive match). The matches that DID arrive stay in
            // `match_details`, so the retry only refetches the ones that failed.
            partial = true;
            continue;
        }
        let recent = acc.recent_stats();
        recent_stats_cache.put(puuid, &newest, recent);
        cache.recent_stats.insert(puuid.clone(), recent);
        progress.publish_progress(cache, false);
    }

    // Loadout skins: one request per match, INGAME only.
    if ingame && cache.skins.is_empty() {
        if drain_pokes(progress.ctx.rx) {
            return Ok(Phase2Outcome::Interrupted);
        }
        let Some(result) = until_poke(
            progress.ctx.rx,
            with_rate_limit_retry(gate, || remote.coregame_loadouts(match_id)),
        )
        .await
        else {
            return Ok(Phase2Outcome::Interrupted);
        };
        match result {
            Ok(v) => {
                cache.skins = loadout::parse_loadouts(&v);
                vlt_log!(
                    "enrich",
                    "loadouts: {} players, vandal skin/chroma {}/{}, phantom {}/{}",
                    cache.skins.len(),
                    cache.skins.values().filter(|s| s.vandal.skin.is_some()).count(),
                    cache.skins.values().filter(|s| s.vandal.chroma.is_some()).count(),
                    cache.skins.values().filter(|s| s.phantom.skin.is_some()).count(),
                    cache.skins.values().filter(|s| s.phantom.chroma.is_some()).count()
                );
                progress.loadouts_fetched = true;
                progress.publish_progress(cache, false);
            }
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            Err(_) => partial = true,
        }
    }

    if !partial {
        return Ok(Phase2Outcome::Complete);
    }
    vlt_log!(
        "enrich",
        "phase2 partial: updates missing for [{}], stats missing for [{}], skins missing={}",
        missing_ids(puuids, |p| !cache.updates.contains_key(p)),
        missing_ids(puuids, |p| !cache.recent_stats.contains_key(p)),
        ingame && cache.skins.is_empty()
    );
    Ok(Phase2Outcome::Partial)
}

/// Comma-separated truncated puuids of the players `is_missing` still holds nothing for.
#[cfg(debug_assertions)]
fn missing_ids(puuids: &[String], is_missing: impl Fn(&str) -> bool) -> String {
    puuids
        .iter()
        .filter(|p| is_missing(p))
        .map(|p| crate::debug_log::short(p))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pregame_polls_on_a_timer() {
        // Agent select gets a bounded wait (no presence events for non-friends' picks); every
        // other state stays purely event-driven.
        assert_eq!(
            poll_interval(AppStatus::Pregame, false),
            Some(Duration::from_millis(PREGAME_POLL_MS))
        );
        assert_eq!(poll_interval(AppStatus::Ingame, false), None);
        assert_eq!(poll_interval(AppStatus::Menus, false), None);
        assert_eq!(poll_interval(AppStatus::ValorantNotRunning, false), None);
    }

    #[test]
    fn a_fully_locked_agent_select_stops_ticking() {
        // Nothing in the pregame payload can change once every ally has locked in, and the
        // transition into the match arrives as an own-presence poke.
        assert_eq!(poll_interval(AppStatus::Pregame, true), None);
        assert_eq!(poll_interval(AppStatus::Ingame, true), None);
    }

    #[test]
    fn drain_collapses_a_burst_and_empties_the_channel() {
        let (tx, mut rx) = mpsc::channel::<Poke>(8);
        assert!(!drain_pokes(&mut rx));
        tx.try_send(Poke).unwrap();
        tx.try_send(Poke).unwrap();
        // The whole burst becomes the one rebuild that follows...
        assert!(drain_pokes(&mut rx));
        // ...and nothing is left to re-trigger it.
        assert!(!drain_pokes(&mut rx));
    }

    #[test]
    fn retry_backoff_is_increasing_and_finite() {
        // A failed rebuild always gets a bounded retry, but the schedule must end so
        // a permanently failing endpoint never becomes a permanent poll loop.
        let delays: Vec<Duration> =
            (0..RETRY_BACKOFF_MS.len()).map(|i| retry_delay(i).unwrap()).collect();
        assert_eq!(delays[0], Duration::from_millis(1000));
        assert!(delays.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(retry_delay(RETRY_BACKOFF_MS.len()), None);
        assert_eq!(retry_delay(usize::MAX), None);
    }

    #[test]
    fn a_complete_phase2_is_final_immediately() {
        // Only a pass where every player settled may publish `enriched: true`.
        assert!(enrichment_is_final(true, 0));
        assert!(enrichment_is_final(true, MAX_PHASE2_ATTEMPTS));
    }

    #[test]
    fn a_partial_phase2_stays_unfinal_until_the_budget_is_spent() {
        // Transient gaps must not be published as settled nulls...
        for attempts in 1..MAX_PHASE2_ATTEMPTS {
            assert!(!enrichment_is_final(false, attempts), "attempt {attempts} must retry");
        }
        // ...but the retries are bounded, so a broken endpoint can't make the 1 s agent-select
        // poll refetch forever.
        assert!(enrichment_is_final(false, MAX_PHASE2_ATTEMPTS));
        assert!(enrichment_is_final(false, MAX_PHASE2_ATTEMPTS + 1));
    }

    #[test]
    fn the_phase2_retry_budget_resets_per_lobby() {
        let mut c = MatchCache::default();
        c.begin_match("m", false);
        c.phase2_attempts = MAX_PHASE2_ATTEMPTS;

        // Another pass at the same pregame keeps the spent budget (no storm).
        c.begin_match("m", false);
        assert_eq!(c.phase2_attempts, MAX_PHASE2_ATTEMPTS);

        // The pregame→ingame upgrade brings new players + loadouts: budget restored.
        c.begin_match("m", true);
        assert_eq!(c.phase2_attempts, 0);

        c.phase2_attempts = MAX_PHASE2_ATTEMPTS;
        c.begin_match("m", true);
        assert_eq!(c.phase2_attempts, MAX_PHASE2_ATTEMPTS, "same match + state keeps it");

        // A different match is a fresh lobby.
        c.begin_match("other", true);
        assert_eq!(c.phase2_attempts, 0);
    }

    #[tokio::test]
    async fn retry_wait_ends_on_the_timer_a_poke_or_a_dead_client() {
        // Timer elapses with nothing waiting -> retry anyway (the case the published status
        // can't cover, since it may still say Menus/not-running). A 1 ms stand-in for the
        // real backoff keeps the test instant.
        let (tx, mut rx) = mpsc::channel::<Poke>(8);
        assert!(wait_before_retry(&mut rx, Duration::from_millis(1)).await);
        // A poke cuts the backoff short.
        tx.try_send(Poke).unwrap();
        assert!(wait_before_retry(&mut rx, Duration::from_secs(30)).await);
        // Websocket task gone -> end the session so the top level re-reads the lockfile.
        drop(tx);
        assert!(!wait_before_retry(&mut rx, Duration::from_secs(30)).await);
    }

    #[tokio::test]
    async fn the_immediate_404_retry_is_spent_once_per_race() {
        let calls = std::cell::Cell::new(0u32);
        let not_found = || {
            calls.set(calls.get() + 1);
            std::future::ready(Err(Error::ResourceNotFound))
        };

        // First cycle: the documented single retry on the transition race.
        assert!(fetch_with_retry(Duration::ZERO, true, not_found).await.is_err());
        assert_eq!(calls.get(), 2);

        // Later cycles of the same unresolved race carry `retry = false`, so the outer backoff
        // is the only thing repeating the call — one 404 per cycle instead of a pair.
        assert!(fetch_with_retry(Duration::ZERO, false, not_found).await.is_err());
        assert_eq!(calls.get(), 3);

        // A fetch that succeeds never retries, whatever the flag says — and it is what clears
        // the flag for the next race.
        let found = || std::future::ready(Ok(serde_json::json!({ "MatchID": "m" })));
        assert!(fetch_with_retry(Duration::ZERO, true, found).await.is_ok());
    }

    #[test]
    fn rate_limit_backoff_prefers_the_servers_retry_after() {
        // Honor what the server asked for, else fall back to the default backoff.
        assert_eq!(rate_limit_backoff(Some(3)), Duration::from_secs(3));
        assert_eq!(rate_limit_backoff(Some(0)), Duration::ZERO);
        assert_eq!(rate_limit_backoff(None), Duration::from_secs(RATE_LIMIT_BACKOFF_SECS));
    }

    #[test]
    fn static_top_up_stops_once_the_live_version_is_held() {
        // Complete and correctly keyed -> nothing to do (the steady state), however long ago
        // the attempt that got us there was.
        assert!(!needs_static_top_up("v2", true, "v2", None));
        assert!(!needs_static_top_up("v2", true, "v2", Some(("v2", Duration::ZERO))));
    }

    #[test]
    fn static_top_up_holds_off_for_the_cooldown_then_tries_again() {
        // A bootstrap fetch that failed, and a valorant-api still serving the older patch,
        // both leave the version unsatisfied. Inside the cooldown the 1 s agent-select poll
        // must not spend a request...
        for held in [("", false), ("v1", true)] {
            assert!(!needs_static_top_up(held.0, held.1, "v2", Some(("v2", Duration::ZERO))));
            assert!(!needs_static_top_up(
                held.0,
                held.1,
                "v2",
                Some(("v2", STATIC_TOP_UP_COOLDOWN - Duration::from_millis(1)))
            ));
            // ...and once it elapses the session tries again, with no lifetime ceiling, so a
            // valorant-api that catches up mid-session is still picked up.
            assert!(needs_static_top_up(held.0, held.1, "v2", Some(("v2", STATIC_TOP_UP_COOLDOWN))));
            assert!(needs_static_top_up(
                held.0,
                held.1,
                "v2",
                Some(("v2", STATIC_TOP_UP_COOLDOWN * 100))
            ));
        }
    }

    #[test]
    fn static_top_up_cooldown_is_per_version() {
        // A patch mid-session is a new version, so it never waits out the old one's cooldown.
        assert!(needs_static_top_up("v1", true, "v3", Some(("v2", Duration::ZERO))));
        assert!(needs_static_top_up("v1", true, "v2", None));
    }

    #[test]
    fn an_unspent_rate_limit_refreshes_tokens_like_bad_claims() {
        // A 429 that outlived its backed-off retry means the next attempt should carry fresh
        // headers, exactly as stale claims do.
        assert!(warrants_token_refresh(&Error::BadClaims));
        assert!(warrants_token_refresh(&Error::RateLimited(None)));
        assert!(warrants_token_refresh(&Error::RateLimited(Some(5))));
        // Everything else is an ordinary transient failure -> plain backoff, no refresh.
        assert!(!warrants_token_refresh(&Error::ResourceNotFound));
        assert!(!warrants_token_refresh(&Error::NotReady));
        assert!(!warrants_token_refresh(&Error::Http("boom".into())));
    }

    #[test]
    fn empty_cache_is_never_fresh() {
        let c = MatchCache::default();
        assert!(!c.is_fresh_for("m", false));
        assert!(!c.is_fresh_for("m", true));
    }

    #[test]
    fn fresh_only_when_enriched_same_match_and_state() {
        let mut c = MatchCache::default();
        c.begin_match("m", false);
        // Phase-1 only (not yet enriched) -> still must run phase 2.
        assert!(!c.is_fresh_for("m", false));
        c.enriched = true;
        assert!(c.is_fresh_for("m", false)); // same match + state, enriched
        assert!(!c.is_fresh_for("other", false)); // different match
    }

    #[test]
    fn pregame_cache_is_stale_when_ingame() {
        // HIGH-1 core rule: pregame and coregame share the same match GUID, but a cache
        // built in PREGAME lacks the enemy team and every loadout, so it must NOT be reused
        // in INGAME.
        let mut c = MatchCache::default();
        c.begin_match("m", false); // built in PREGAME
        c.enriched = true;
        assert!(!c.is_fresh_for("m", true), "pregame cache must be stale once INGAME");
        // ...but stays fresh while we remain in pregame.
        assert!(c.is_fresh_for("m", false));
    }

    #[test]
    fn ingame_cache_stays_fresh_ingame() {
        let mut c = MatchCache::default();
        c.begin_match("m", true);
        c.enriched = true;
        assert!(c.is_fresh_for("m", true));
    }

    #[test]
    fn begin_match_reuses_same_match_data_and_resets_enriched() {
        let mut c = MatchCache::default();
        c.begin_match("m", false);
        c.names.insert("ally".into(), "Ally#1".into());
        c.mmr.insert("ally".into(), MmrResponse::default());
        c.enriched = true;

        // pregame→ingame upgrade for the SAME match: keep the already-fetched ally row, but
        // force phase 2 to run again (enemies + loadouts still missing).
        c.begin_match("m", true);
        assert_eq!(c.names.get("ally").map(String::as_str), Some("Ally#1"));
        assert!(c.mmr.contains_key("ally"));
        assert!(!c.enriched);
        assert!(c.ingame);

        // A different match id drops everything.
        c.begin_match("other", true);
        assert!(c.names.is_empty());
        assert!(c.mmr.is_empty());
    }

    #[test]
    fn invalidate_clears_everything() {
        let mut c = MatchCache::default();
        c.begin_match("m", true);
        c.names.insert("p".into(), "x".into());
        c.enriched = true;
        c.pregame_match_id = Some("m".into());
        c.pregame_locked = true;
        c.id_retry_spent = true;
        c.invalidate();
        assert!(!c.is_fresh_for("m", true));
        assert!(c.names.is_empty());
        assert_eq!(c.match_id, None);
        // Leaving a match (MENUS) is what un-sticks the agent-select bookkeeping: the next
        // lobby resolves its own id, ticks again, and gets its own immediate 404 retry.
        assert_eq!(c.pregame_id(false), None);
        assert!(!c.pregame_locked);
        assert!(!c.id_retry_spent);
    }

    #[test]
    fn every_rebuild_lifts_the_agent_select_poll_pause() {
        let mut c = MatchCache::default();
        c.begin_match("m", false);
        c.pregame_locked = true;

        // A build that fails before it can parse a roster must not inherit the pause: the tick
        // is the only thing scheduled to rebuild agent select, so leaving it stopped would
        // freeze the lobby until an unrelated presence event arrived.
        c.clear_pregame_lock();
        assert!(!c.pregame_locked);
        assert!(poll_interval(AppStatus::Pregame, c.pregame_locked).is_some());

        // Nor does an INGAME build, which never sets the flag at all.
        c.pregame_locked = true;
        c.clear_pregame_lock();
        c.begin_match("m", true);
        assert!(!c.pregame_locked);
    }

    #[test]
    fn the_pregame_match_id_is_reused_only_during_agent_select() {
        let mut c = MatchCache::default();
        // Nothing resolved yet -> the id must be fetched.
        assert_eq!(c.pregame_id(false), None);

        c.pregame_match_id = Some("m".into());
        assert_eq!(c.pregame_id(false).as_deref(), Some("m"));
        // INGAME always asks the coregame endpoint — that GET is the transition detector.
        assert_eq!(c.pregame_id(true), None);
    }

    /// A roster row, enough for the ingame skip tests.
    fn roster_of(puuid: &str) -> CachedRoster {
        CachedRoster {
            players: vec![MatchPlayer {
                puuid: puuid.into(),
                team: "Blue".into(),
                character_id: None,
                selection_state: None,
                account_level: 0,
                incognito: false,
                hide_account_level: false,
            }],
            own_team: Some("Blue".into()),
            map_id: Some("/Game/Maps/Ascent/Ascent".into()),
        }
    }

    #[test]
    fn the_match_payload_is_skipped_only_for_an_enriched_ingame_match() {
        let mut c = MatchCache::default();
        c.begin_match("m", true);
        c.roster = Some(roster_of("p"));
        // Phase 2 hasn't settled yet -> the payload is still needed.
        assert!(c.cached_roster("m", true).is_none());
        c.enriched = true;
        assert!(c.cached_roster("m", true).is_some());
        // A different match id is exactly what the cheap match-id GET is there to catch.
        assert!(c.cached_roster("other", true).is_none());
        // Agent select changes the roster and the picks, so it always refetches.
        assert!(c.cached_roster("m", false).is_none());
    }

    #[test]
    fn a_new_match_drops_the_cached_roster() {
        let mut c = MatchCache::default();
        c.begin_match("m", true);
        c.roster = Some(roster_of("p"));
        c.enriched = true;
        c.begin_match("other", true);
        assert!(c.roster.is_none());
        assert!(c.cached_roster("other", true).is_none());
    }

    #[test]
    fn a_cached_match_serves_every_player_that_played_it() {
        let mut cache = MatchDetailsCache::default();
        assert_eq!(cache.totals_for("m1", "a"), None);

        let played =
            |kills, deaths| MatchTotals { kills, deaths, kd_matches: 1, ..Default::default() };
        let mut contribution = MatchContribution::new();
        contribution.insert("a".into(), played(20, 15));
        contribution.insert("b".into(), played(9, 18));
        cache.put("m1", contribution);

        // Both lobby members read the one download...
        assert_eq!(cache.totals_for("m1", "a").unwrap().kills, 20);
        assert_eq!(cache.totals_for("m1", "b").unwrap().kills, 9);
        // ...and a player who wasn't in it contributes nothing, which is still a hit.
        assert_eq!(cache.totals_for("m1", "c"), Some(MatchTotals::default()));
    }

    #[test]
    fn the_match_cache_evicts_the_oldest_past_its_cap() {
        let mut cache = MatchDetailsCache::default();
        for i in 0..MATCH_DETAILS_CACHE_CAP + 2 {
            cache.put(&format!("m{i}"), MatchContribution::new());
        }
        assert_eq!(cache.map.len(), MATCH_DETAILS_CACHE_CAP);
        assert_eq!(cache.order.len(), MATCH_DETAILS_CACHE_CAP);
        assert_eq!(cache.totals_for("m0", "p"), None);
        assert_eq!(cache.totals_for("m1", "p"), None);
        assert!(cache.totals_for("m2", "p").is_some());
        assert!(cache.totals_for(&format!("m{}", MATCH_DETAILS_CACHE_CAP + 1), "p").is_some());

        // Re-putting a held match must not queue a second eviction slot for it.
        cache.put("m2", MatchContribution::new());
        assert_eq!(cache.order.len(), MATCH_DETAILS_CACHE_CAP);
    }

    #[test]
    fn a_match_that_failed_this_pass_is_not_refetched_for_the_next_player() {
        let mut cache = MatchDetailsCache::default();
        let mut failed = HashSet::new();

        // Untouched so far -> download it.
        assert_eq!(window_step(&cache, &failed, "m1", "a"), WindowStep::Fetch);

        // It failed for the first player, so every other player who played it takes the
        // no-data path instead of spending a request of their own on the same dead id.
        failed.insert("m1".to_string());
        assert_eq!(window_step(&cache, &failed, "m1", "b"), WindowStep::Failed);
        // A different id in the same pass is unaffected.
        assert_eq!(window_step(&cache, &failed, "m2", "b"), WindowStep::Fetch);

        // A cached match always wins: a later pass that succeeds serves everyone.
        cache.put("m1", MatchContribution::new());
        assert_eq!(
            window_step(&cache, &failed, "m1", "b"),
            WindowStep::Cached(MatchTotals::default())
        );
    }

    #[test]
    fn a_rate_limit_deadline_outlives_the_request_that_earned_it() {
        let gate = RateLimitGate::default();
        // Nothing owed by default.
        assert_eq!(gate.wait(), None);

        // A 429's backoff holds off every later request, not just the retry that was cancelled.
        gate.arm(Duration::from_secs(30));
        let owed = gate.wait().expect("deadline still ahead");
        assert!(owed > Duration::from_secs(29) && owed <= Duration::from_secs(30));

        // A shorter backoff can't shorten what the server already asked for...
        gate.arm(Duration::from_secs(1));
        assert!(gate.wait().expect("longer deadline kept") > Duration::from_secs(29));
        // ...but a stricter one wins.
        gate.arm(Duration::from_secs(60));
        assert!(gate.wait().expect("stricter deadline") > Duration::from_secs(30));

        // Once it passes, requests flow again and the deadline is dropped.
        let passed = RateLimitGate::default();
        passed.arm(Duration::ZERO);
        assert_eq!(passed.wait(), None);
        assert!(passed.until.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_request_in_flight_is_cut_short_by_a_rebuild_poke() {
        let (tx, mut rx) = mpsc::channel::<Poke>(8);
        // Nothing waiting -> the request completes normally.
        assert_eq!(until_poke(&mut rx, async { 7 }).await, Some(7));

        // A poke abandons a request that would otherwise run to its timeout.
        tx.try_send(Poke).unwrap();
        let slow = tokio::time::sleep(Duration::from_secs(30));
        assert_eq!(until_poke(&mut rx, slow).await, None);

        // Websocket task gone: nothing left to interrupt, so the request is seen through.
        drop(tx);
        assert_eq!(until_poke(&mut rx, async { 7 }).await, Some(7));
    }

    #[test]
    fn recent_stats_cache_hits_only_on_matching_newest_match() {
        let mut cache = RecentStatsCache::default();
        // Miss when empty.
        assert_eq!(cache.get("p", "m1"), None);

        let stats = RecentStats { headshot_percent: Some(25), kd: Some(1.28) };
        cache.put("p", "m1", stats);
        // Hit only when the newest match id matches -> no re-download of match-details.
        // HS% and KD ride the same entry (they come from the same payloads).
        assert_eq!(cache.get("p", "m1"), Some(stats));
        // A newer match for the same player -> miss (must recompute).
        assert_eq!(cache.get("p", "m2"), None);
        // Persists across matches (not cleared with the per-match cache).
        assert_eq!(cache.get("p", "m1"), Some(stats));
    }

    #[test]
    fn recent_stats_cache_evicts_the_oldest_player_past_its_cap() {
        let mut cache = RecentStatsCache::default();
        let stats = RecentStats { headshot_percent: Some(25), kd: Some(1.28) };
        for i in 0..RECENT_STATS_CACHE_CAP + 2 {
            cache.put(&format!("p{i}"), "m", stats);
        }
        assert_eq!(cache.map.len(), RECENT_STATS_CACHE_CAP);
        assert_eq!(cache.order.len(), RECENT_STATS_CACHE_CAP);
        assert_eq!(cache.get("p0", "m"), None);
        assert_eq!(cache.get("p1", "m"), None);
        assert_eq!(cache.get("p2", "m"), Some(stats));

        // A returning player whose newest match changed must not queue a second slot.
        cache.put("p2", "m2", stats);
        assert_eq!(cache.order.len(), RECENT_STATS_CACHE_CAP);
        assert_eq!(cache.get("p2", "m2"), Some(stats));
    }

    #[test]
    fn the_first_progress_emit_is_immediate_and_a_burst_after_it_coalesces() {
        let window = Duration::from_millis(PROGRESS_COALESCE_MS);
        // The table's first paint is never held back.
        assert!(should_emit(None, false));
        // A burst inside the window collapses into the flush that follows it...
        assert!(!should_emit(Some(Duration::ZERO), false));
        assert!(!should_emit(Some(window - Duration::from_millis(1)), false));
        // ...and once it has passed, progress flows again.
        assert!(should_emit(Some(window), false));
        assert!(should_emit(Some(window * 10), false));
        // A phase boundary goes out whatever the window says.
        assert!(should_emit(None, true));
        assert!(should_emit(Some(Duration::ZERO), true));
    }

    #[test]
    fn the_gate_starts_its_window_only_on_an_emit_it_let_through() {
        let mut gate = ProgressGate::default();
        assert!(gate.take(false), "first paint");
        assert!(!gate.take(false), "inside the window");
        assert!(gate.take(true), "boundary flush");
        assert!(!gate.take(false), "the forced emit restarted the window");
    }

    #[test]
    fn an_enriched_snapshot_never_carries_a_pending_row() {
        // The contract's invariant, at its worst case: nothing has been fetched at all.
        let roster = roster_of("me");
        let parties = HashMap::new();
        let static_data = StaticData::default();
        let seasons: Vec<content::Season> = Vec::new();
        let parts = SnapshotParts {
            players: &roster.players,
            parties: &parties,
            own_team: roster.own_team.clone(),
            map: None,
            mode: None,
            status: AppStatus::Ingame,
            static_data: &static_data,
            own_puuid: "me",
            season_id: "s1",
            seasons: &seasons,
            ingame: true,
        };
        let cache = MatchCache::default();

        let settled = assemble_snapshot(&parts, &cache, false, true);
        assert!(settled.enriched);
        assert!(settled
            .players
            .iter()
            .all(|r| r.pending == crate::riot::types::PendingStats::default()));

        // The same cache mid-build says every group is still coming.
        let in_flight = assemble_snapshot(&parts, &cache, false, false);
        assert!(!in_flight.enriched);
        assert!(in_flight
            .players
            .iter()
            .all(|r| r.pending.rank && r.pending.skins));
    }

    #[test]
    fn giving_up_settles_a_table_that_was_still_loading() {
        let roster = roster_of("me");
        let parties = HashMap::new();
        let static_data = StaticData::default();
        let seasons: Vec<content::Season> = Vec::new();
        let parts = SnapshotParts {
            players: &roster.players,
            parties: &parties,
            own_team: roster.own_team.clone(),
            map: None,
            mode: None,
            status: AppStatus::Ingame,
            static_data: &static_data,
            own_puuid: "me",
            season_id: "s1",
            seasons: &seasons,
            ingame: true,
        };
        let cache = MatchCache::default();

        // What the retry schedule gives up on: the last progress snapshot of a failed build.
        let mut snap = assemble_snapshot(&parts, &cache, false, false);
        assert!(snap
            .players
            .iter()
            .any(|r| r.pending != PendingStats::default()));

        assert!(settle_pending(&mut snap));
        assert!(snap.enriched);
        assert!(snap
            .players
            .iter()
            .all(|r| r.pending == PendingStats::default()));
        // ...and it says exactly what the build would have published had it reached the end.
        assert!(same_content(
            &snap,
            &assemble_snapshot(&parts, &cache, false, true)
        ));

        // An already-settled table changes nothing, so no redundant emit goes out.
        assert!(!settle_pending(&mut snap));
    }

    #[test]
    fn dedup_ignores_only_the_timestamp() {
        let a = TrackerSnapshot::not_running(Some("Waiting for Valorant...".into()));
        let mut b = a.clone();
        b.last_updated = a.last_updated + 5_000;
        assert!(same_content(&a, &b), "a fresh build of the same state must not re-emit");

        b.message = Some("Loading...".into());
        assert!(!same_content(&a, &b));

        let mut c = a.clone();
        c.status = AppStatus::Menus;
        assert!(!same_content(&a, &c));

        let mut d = a.clone();
        d.enriched = !a.enriched;
        assert!(!same_content(&a, &d));
    }
}
