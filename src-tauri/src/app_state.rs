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
use crate::riot::stats::{self, MatchTotals, RecentStats, RrHistory};
use crate::riot::types::{AppStatus, MapInfo, SessionLoopState, TrackerSnapshot};
use crate::riot::websocket::Poke;
use std::collections::HashMap;
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
/// dedup is what makes the two-phase emit safe: the phase-1 snapshot and the enriched
/// phase-2 snapshot differ (heavy fields fill in), so both fire, while a phase that adds
/// nothing (e.g. a pregame with no recent matches) is silently suppressed.
fn publish(state: &TrackerState, emitter: &Arc<dyn Emitter>, mut snap: TrackerSnapshot) {
    {
        let prev = state.snapshot.lock().unwrap();
        // Compare ignoring timestamp so identical content doesn't re-fire.
        let mut cmp = snap.clone();
        cmp.last_updated = prev.last_updated;
        if *prev == cmp {
            return;
        }
    }
    snap.last_updated = crate::riot::types::now_millis();
    state.store(snap.clone());
    #[cfg(debug_assertions)]
    debug_capture::write(&snap);
    emitter.emit(&snap);
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

/// Fold one more poke into the strongest seen so far (`Own` outranks `Other`). Pure — this is
/// the burst-collapse rule, so a mix of own + other events rebuilds once, as an own poke.
fn collapse(strongest: Option<Poke>, next: Poke) -> Poke {
    match strongest {
        Some(s) if s >= next => s,
        _ => next,
    }
}

/// A drained poke warrants a rebuild when it is our own presence event (any state — it can
/// carry a transition), or when it is another player's and we are in agent select, where
/// their presence is how a teammate's agent pick becomes visible. Pure.
fn poke_triggers_rebuild(poke: Poke, status: AppStatus) -> bool {
    poke == Poke::Own || status == AppStatus::Pregame
}

/// Agent-select poll cadence (see `poll_interval`).
const PREGAME_POLL_MS: u64 = 1000;

/// How long the loop may wait for a poke before rebuilding anyway, for a given status.
/// `Some` only in Pregame: Riot pushes no presence event when a NON-FRIEND lobby player picks
/// or locks an agent, and our own presence doesn't change either, so agent select would sit
/// still for its whole ~100 s after the entry events. Polling the local pregame endpoint once
/// a second keeps the roster live (vRY's main loop polls for the same reason). Every other
/// status stays purely event-driven — `None` means "wait indefinitely". Pure.
fn poll_interval(status: AppStatus) -> Option<Duration> {
    (status == AppStatus::Pregame).then(|| Duration::from_millis(PREGAME_POLL_MS))
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

/// Drain the poke channel, returning the strongest poke that was waiting (`None` if empty).
fn drain_pokes(rx: &mut mpsc::Receiver<Poke>) -> Option<Poke> {
    let mut strongest = None;
    while let Ok(poke) = rx.try_recv() {
        strongest = Some(collapse(strongest, poke));
    }
    strongest
}

/// Drain the poke channel mid-burst and report whether the enrichment should abort: a new
/// presence event (possibly a dodge / state transition) means the loop should rebuild for the
/// current state rather than block it behind the remaining ~500 KB match-details fetches
/// (HIGH-2). In-match, only our own presence qualifies — another player's is dropped here so
/// the ingame path never gains a rebuild from it.
fn abort_pending(rx: &mut mpsc::Receiver<Poke>, ingame: bool) -> bool {
    let status = if ingame { AppStatus::Ingame } else { AppStatus::Pregame };
    drain_pokes(rx).is_some_and(|poke| poke_triggers_rebuild(poke, status))
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

    /// Drop everything (transition to MENUS / not-running).
    fn invalidate(&mut self) {
        *self = Self::default();
    }
}

/// Session-lived match-details stat cache: puuid -> (newest competitive match id the stats
/// were computed from, the HS% + KD they yielded). Both figures come from the same payloads,
/// so they share one entry. Persists across matches (NOT cleared on MENUS) so it
/// self-invalidates only when a player's newest competitive match changes.
#[derive(Default)]
struct RecentStatsCache {
    map: HashMap<String, (String, RecentStats)>,
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
        self.map.insert(puuid.to_string(), (newest_match_id.to_string(), stats));
    }
}

/// Top-level loop: wait for the client, connect, run until the connection drops, repeat.
pub async fn tracker_main(state: Arc<TrackerState>, emitter: Arc<dyn Emitter>) {
    loop {
        // Phase 1: wait for the lockfile.
        let lockfile = loop {
            match lockfile::read() {
                Ok(lf) => break lf,
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
                run_session(&mut session, &state, &emitter).await;
            }
            Err(_) => {
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
    })
}

/// Re-fetch tokens after a BAD_CLAIMS and update the remote client. The puuid is the same
/// account for the life of the session, so it is not overwritten here.
async fn refresh_tokens(session: &mut Session) -> Result<()> {
    let entitlements = session.local.entitlements().await?;
    session
        .remote
        .set_tokens(entitlements.access_token, entitlements.token);
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
    // Poke channel: the websocket sends a `Poke` per Valorant presence event (own vs another
    // player's), and an `Own` poke after every reconnect so we re-poll presence for any
    // transition missed while the socket was down (C2). The task ends (dropping tx) only when
    // the client is gone.
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
                break;
            }
            // Re-poll after the drop so any transition during the outage is picked up (C2).
            // Always an `Own` poke: it must rebuild whatever state we are in.
            if tx.send(Poke::Own).await.is_err() {
                break; // receiver gone
            }
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
            // Schedule exhausted — fall back to the event-driven wait below.
        }
        // A success, an interruption, or an exhausted schedule all start the next failure
        // with a full retry budget.
        retry_attempt = 0;
        if outcome == BuildOutcome::Interrupted {
            drain_pokes(&mut rx);
            continue;
        }
        if !wait_for_rebuild_poke(&mut rx, state).await {
            break; // websocket task ended -> client gone
        }
    }

    ws_task.abort();
}

/// Wait for a poke that warrants a rebuild for the currently published status, collapsing
/// each burst into one (L1). Pokes that don't warrant one (another player's presence outside
/// agent select) are dropped without any refetch. Returns false when the websocket task ended.
///
/// In Pregame the wait is bounded by `poll_interval`, so an elapsed timeout triggers a rebuild
/// just like a poke would — that tick is what makes teammates' agent picks visible, since Riot
/// pushes no presence event for them. Identical rebuilds are suppressed by the snapshot dedup
/// in `publish`, and a tick can't stack with pokes: the rebuild drains the channel anyway.
async fn wait_for_rebuild_poke(rx: &mut mpsc::Receiver<Poke>, state: &TrackerState) -> bool {
    loop {
        let first = match poll_interval(state.status()) {
            Some(tick) => match tokio::time::timeout(tick, rx.recv()).await {
                Ok(Some(poke)) => poke,
                Ok(None) => return false, // websocket task ended -> client gone
                Err(_) => return true,    // poll tick -> rebuild agent select
            },
            None => match rx.recv().await {
                Some(poke) => poke,
                None => return false,
            },
        };
        let poke = collapse(drain_pokes(rx), first);
        if poke_triggers_rebuild(poke, state.status()) {
            return true;
        }
    }
}

/// Wait out a retry backoff. The wait is cut short by any poke — a real event is a
/// better reason to rebuild than the timer, and the rebuild drains the channel anyway, so no
/// poke is lost. Returns false when the websocket task ended (client gone).
async fn wait_before_retry(rx: &mut mpsc::Receiver<Poke>, delay: Duration) -> bool {
    match tokio::time::timeout(delay, rx.recv()).await {
        Ok(Some(_)) => true,
        Ok(None) => false, // websocket task ended -> client gone
        Err(_) => true,    // backoff elapsed -> retry the build
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

    let parties = presence::party_grouping(&presences);

    match info.session_state {
        Some(SessionLoopState::Pregame) => {
            build_match_snapshot(session, &info, &parties, false, ctx).await
        }
        Some(SessionLoopState::Ingame) => {
            build_match_snapshot(session, &info, &parties, true, ctx).await
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

/// Fetch + assemble a pregame or coregame snapshot, using the two-phase emit: the first
/// snapshot carries names + ranks + RR + peak + WR (all free once names+MMR are in) with
/// the heavy fields (rrChange / recentResults / headshotPercent / kd / skins) empty; a second,
/// enriched snapshot follows once those are fetched. Reports `Interrupted` when enrichment
/// aborted on a mid-burst poke, and `Retry` when phase 2 could not settle every player —
/// the snapshot is published either way, just not as `enriched`.
async fn build_match_snapshot(
    session: &mut Session,
    info: &PresenceInfo,
    parties: &HashMap<String, String>,
    ingame: bool,
    ctx: &mut BuildCtx<'_>,
) -> Result<BuildOutcome> {
    let own = session.own_puuid.clone();

    // Match id. On the RESOURCE_NOT_FOUND transition race, retry once — after ~5s for
    // coregame, immediately for pregame (spec §10.5, C9). The 429 retry sits INSIDE the
    // 404-race retry so a rate limit is backed off once per attempt rather than being
    // multiplied by it.
    let id_json = if ingame {
        fetch_with_retry(Duration::from_secs(5), || {
            with_rate_limit_retry(|| session.remote.coregame_match_id(&own))
        })
        .await?
    } else {
        fetch_with_retry(Duration::ZERO, || {
            with_rate_limit_retry(|| session.remote.pregame_match_id(&own))
        })
        .await?
    };
    let match_id = match_state::extract_match_id(&id_json).ok_or(Error::ResourceNotFound)?;

    // Match data (owned Values consumed by the extractors — no deep clone, L4).
    let (players, own_team, map_id): (Vec<MatchPlayer>, Option<String>, Option<String>) = if ingame
    {
        let m = with_rate_limit_retry(|| session.remote.coregame_match(&match_id)).await?;
        // A payload we can't read is an error, not an empty lobby: erroring here happens
        // BEFORE `begin_match`, so nothing is cached and the loop retries.
        let data = match_state::extract_coregame(m, &own)?;
        (data.players, data.own_team, data.map_id)
    } else {
        let m = with_rate_limit_retry(|| session.remote.pregame_match(&match_id)).await?;
        let data = match_state::extract_pregame(m, &own)?;
        (data.players, data.own_team, data.map_id)
    };

    let status = if ingame { AppStatus::Ingame } else { AppStatus::Pregame };
    let mode = if info.is_custom_game() {
        Some("Custom Game".to_string())
    } else {
        info.queue_id.as_deref().map(game_mode_name)
    };
    let map = session.static_data.map(map_id.as_deref());

    // Fully cached (enriched, correct state) -> a single snapshot, no fetch. This is the
    // common in-match path: the score changes every round but nothing here is refetched.
    if session.cache.is_fresh_for(&match_id, ingame) {
        let snap = assemble_snapshot(session, &players, parties, own_team, map, mode, status, true);
        publish(ctx.state, ctx.emitter, snap);
        return Ok(BuildOutcome::Done);
    }

    // Not fresh: prepare the cache for this match (keeping any same-match data to reuse on a
    // pregame→ingame upgrade), then proactively refresh the token before the burst so a
    // BadClaims can't strand us mid-burst and force a redo (MEDIUM-2). Best-effort: if the
    // refresh fails we proceed with the current token and the BadClaims arm still covers it.
    session.cache.begin_match(&match_id, ingame);
    let _ = refresh_tokens(session).await;

    let puuids: Vec<String> = players.iter().map(|p| p.puuid.clone()).collect();

    // === Phase 1: names + MMR (fast fields). Publish immediately. ===
    fetch_phase1(&session.remote, &mut session.cache, &puuids).await?;
    let snap1 = assemble_snapshot(
        session,
        &players,
        parties,
        own_team.clone(),
        map.clone(),
        mode.clone(),
        status,
        false,
    );
    publish(ctx.state, ctx.emitter, snap1);

    // === Phase 2: competitiveupdates + HS%/KD + loadout skins. Interruptible. ===
    let outcome = fetch_phase2(
        &session.remote,
        &mut session.cache,
        &mut session.recent_stats_cache,
        &puuids,
        ingame,
        &match_id,
        ctx.rx,
    )
    .await?;

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

    session.cache.enriched = is_final;
    let snap2 = assemble_snapshot(session, &players, parties, own_team, map, mode, status, is_final);
    publish(ctx.state, ctx.emitter, snap2);
    Ok(if is_final { BuildOutcome::Done } else { BuildOutcome::Retry })
}

/// Assemble a `TrackerSnapshot` from whatever the cache currently holds. Both phases and the
/// fully-cached path go through here; the heavy fields are simply empty until phase 2 has
/// populated the cache, so the phase-1 snapshot naturally carries null/empty stats.
#[allow(clippy::too_many_arguments)] // cohesive snapshot fields; grouping them would add an
// unrequested struct for no real gain.
fn assemble_snapshot(
    session: &Session,
    players: &[MatchPlayer],
    parties: &HashMap<String, String>,
    own_team: Option<String>,
    map: Option<MapInfo>,
    mode: Option<String>,
    status: AppStatus,
    enriched: bool,
) -> TrackerSnapshot {
    let rows = assemble_players(&AssembleInput {
        players,
        names: &session.cache.names,
        mmr: &session.cache.mmr,
        parties,
        updates: &session.cache.updates,
        recent_stats: &session.cache.recent_stats,
        skins: &session.cache.skins,
        static_data: &session.static_data,
        own_puuid: &session.own_puuid,
        own_team: own_team.as_deref(),
        current_season_id: &session.season_id,
        seasons: &session.seasons,
    });
    TrackerSnapshot {
        status,
        map,
        mode,
        own_team,
        players: rows,
        enriched,
        last_updated: crate::riot::types::now_millis(),
        message: None,
    }
}

/// Retry a fetch once on the RESOURCE_NOT_FOUND state-transition race (spec §10.5). `delay`
/// is the wait before the single retry (5s for coregame, zero for pregame — C9).
async fn fetch_with_retry<F, Fut>(delay: Duration, f: F) -> Result<serde_json::Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    match f().await {
        Err(Error::ResourceNotFound) => {
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
/// Run a remote fetch with one backed-off retry on a 429 (spec §10.6, C5) rather than
/// silently degrading. BadClaims and other errors pass straight through to the caller; so does
/// a 429 the retry could not clear, which `warrants_token_refresh` then routes into the token
/// refresh, since headers we keep sending are part of what the limiter counts.
/// Every pd AND glz call goes through here, including the match-id/match fetches the
/// 1 s agent-select poll drives: the wait happens INSIDE the build, so a rate-limited poll
/// tick stretches rather than stacking another attempt on top.
async fn with_rate_limit_retry<F, Fut>(f: F) -> Result<serde_json::Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    match f().await {
        Err(Error::RateLimited(retry_after)) => {
            tokio::time::sleep(rate_limit_backoff(retry_after)).await;
            f().await
        }
        other => other,
    }
}

/// Batch name resolution. Propagates the errors that want fresh tokens so the caller refreshes
/// tokens and retries once (C4) — an unspent rate limit included, since it is lobby-wide and
/// would otherwise settle the whole table on placeholder names. Other transport errors degrade
/// to an empty map (names render as placeholders rather than failing the whole table).
async fn fetch_names(remote: &RemoteClient, puuids: &[String]) -> Result<HashMap<String, String>> {
    match with_rate_limit_retry(|| remote.names(puuids)).await {
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

/// Phase 1 of the two-phase emit: resolve names (batch) + MMR (per player) into the cache,
/// fetching only the puuids not already cached from a same-match pregame build (HIGH-1
/// reuse). MMR is the WR source too, so this covers every "free" field. Stale claims and an
/// unspent rate limit propagate for the shared token refresh (C4/C7); any other single MMR
/// failure degrades that row to Unranked, which is the "ranks never error a row" contract for
/// a genuinely absent record — a lobby-wide rate limit is not that, and settling every row as
/// Unranked because of one would be wrong. The inter-request delay is applied only
/// BETWEEN requests (LOW: no trailing
/// sleep), and now covers the MMR fetches too (LOW: consistency with the spec's per-player
/// 120 ms).
async fn fetch_phase1(
    remote: &RemoteClient,
    cache: &mut MatchCache,
    puuids: &[String],
) -> Result<()> {
    // Names: one batch call for the puuids we don't already hold.
    let missing_names: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.names.contains_key(*p))
        .cloned()
        .collect();
    if !missing_names.is_empty() {
        let fetched = fetch_names(remote, &missing_names).await?;
        cache.names.extend(fetched);
    }

    // MMR: per player, only the ones missing, spaced between requests.
    let missing_mmr: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.mmr.contains_key(*p))
        .cloned()
        .collect();
    for (i, puuid) in missing_mmr.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(inter_request_delay()).await;
        }
        match with_rate_limit_retry(|| remote.mmr(puuid)).await {
            Ok(v) => {
                cache.mmr.insert(puuid.clone(), parse_mmr(v));
            }
            Err(e) if warrants_token_refresh(&e) => return Err(e),
            Err(_) => { /* private profile / hiccup -> Unranked for this row only */ }
        }
    }
    Ok(())
}

/// Phase 2 of the two-phase emit: competitiveupdates (ΔRR + last-5 + recent match ids),
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
async fn fetch_phase2(
    remote: &RemoteClient,
    cache: &mut MatchCache,
    recent_stats_cache: &mut RecentStatsCache,
    puuids: &[String],
    ingame: bool,
    match_id: &str,
    rx: &mut mpsc::Receiver<Poke>,
) -> Result<Phase2Outcome> {
    // Tracks whether any request has been issued yet, so the 120 ms delay only ever sits
    // *between* two real requests across the whole phase.
    let mut sent_any = false;
    // Set by any transient failure — the whole pass is then not final.
    let mut partial = false;

    // competitiveupdates: one request per player missing history.
    let missing_updates: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.updates.contains_key(*p))
        .cloned()
        .collect();
    for puuid in &missing_updates {
        if abort_pending(rx, ingame) {
            return Ok(Phase2Outcome::Interrupted);
        }
        if sent_any {
            tokio::time::sleep(inter_request_delay()).await;
        }
        sent_any = true;
        match with_rate_limit_retry(|| remote.competitive_updates(puuid)).await {
            Ok(v) => {
                cache.updates.insert(puuid.clone(), stats::parse_competitive_updates(v));
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
        if abort_pending(rx, ingame) {
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
            continue;
        };
        // Session cache hit (same newest match) -> reuse, no match-details fetch.
        if let Some(stats) = recent_stats_cache.get(puuid, &newest) {
            cache.recent_stats.insert(puuid.clone(), stats);
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
            if abort_pending(rx, ingame) {
                return Ok(Phase2Outcome::Interrupted);
            }
            if sent_any {
                tokio::time::sleep(inter_request_delay()).await;
            }
            sent_any = true;
            match with_rate_limit_retry(|| remote.match_details(mid)).await {
                Ok(v) => stats::accumulate_match_totals(&mut acc, &v, puuid),
                Err(Error::BadClaims) => return Err(Error::BadClaims),
                Err(_) => any_failed = true,
            }
        }
        if any_failed {
            // A window missing some of its matches yields the WRONG HS%/KD. Cache neither
            // per-match nor per-session (the session entry would otherwise stay wrong until
            // the player's next competitive match) and retry the whole window.
            partial = true;
            continue;
        }
        let recent = acc.recent_stats();
        recent_stats_cache.put(puuid, &newest, recent);
        cache.recent_stats.insert(puuid.clone(), recent);
    }

    // Loadout skins: one request per match, INGAME only.
    if ingame && cache.skins.is_empty() {
        if abort_pending(rx, ingame) {
            return Ok(Phase2Outcome::Interrupted);
        }
        match with_rate_limit_retry(|| remote.coregame_loadouts(match_id)).await {
            Ok(v) => {
                cache.skins = loadout::parse_loadouts(&v);
            }
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            Err(_) => partial = true,
        }
    }

    Ok(if partial {
        Phase2Outcome::Partial
    } else {
        Phase2Outcome::Complete
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_collapses_to_the_strongest_poke() {
        // Own outranks Other in any order, so a mixed burst rebuilds as one own poke.
        assert_eq!(collapse(None, Poke::Other), Poke::Other);
        assert_eq!(collapse(None, Poke::Own), Poke::Own);
        assert_eq!(collapse(Some(Poke::Other), Poke::Own), Poke::Own);
        assert_eq!(collapse(Some(Poke::Own), Poke::Other), Poke::Own);
        assert_eq!(collapse(Some(Poke::Other), Poke::Other), Poke::Other);
    }

    #[test]
    fn own_pokes_rebuild_in_every_state() {
        for status in [
            AppStatus::ValorantNotRunning,
            AppStatus::Menus,
            AppStatus::Pregame,
            AppStatus::Ingame,
        ] {
            assert!(poke_triggers_rebuild(Poke::Own, status));
        }
    }

    #[test]
    fn other_pokes_rebuild_only_in_pregame() {
        // Agent select is the only state where another player's presence changes our table.
        assert!(poke_triggers_rebuild(Poke::Other, AppStatus::Pregame));
        assert!(!poke_triggers_rebuild(Poke::Other, AppStatus::Ingame));
        assert!(!poke_triggers_rebuild(Poke::Other, AppStatus::Menus));
        assert!(!poke_triggers_rebuild(Poke::Other, AppStatus::ValorantNotRunning));
    }

    #[test]
    fn only_pregame_polls_on_a_timer() {
        // Agent select gets a bounded wait (no presence events for non-friends' picks); every
        // other state stays purely event-driven.
        assert_eq!(
            poll_interval(AppStatus::Pregame),
            Some(Duration::from_millis(PREGAME_POLL_MS))
        );
        assert_eq!(poll_interval(AppStatus::Ingame), None);
        assert_eq!(poll_interval(AppStatus::Menus), None);
        assert_eq!(poll_interval(AppStatus::ValorantNotRunning), None);
    }

    #[test]
    fn drain_collapses_and_empties_the_channel() {
        let (tx, mut rx) = mpsc::channel::<Poke>(8);
        assert_eq!(drain_pokes(&mut rx), None);
        tx.try_send(Poke::Other).unwrap();
        tx.try_send(Poke::Own).unwrap();
        tx.try_send(Poke::Other).unwrap();
        assert_eq!(drain_pokes(&mut rx), Some(Poke::Own));
        assert_eq!(drain_pokes(&mut rx), None);
    }

    #[test]
    fn enrichment_aborts_on_other_pokes_only_in_pregame() {
        let (tx, mut rx) = mpsc::channel::<Poke>(8);
        tx.try_send(Poke::Other).unwrap();
        assert!(!abort_pending(&mut rx, true), "ingame must ignore other players' pokes");
        tx.try_send(Poke::Other).unwrap();
        assert!(abort_pending(&mut rx, false), "pregame rebuilds on an agent lock");
        tx.try_send(Poke::Other).unwrap();
        tx.try_send(Poke::Own).unwrap();
        assert!(abort_pending(&mut rx, true), "own poke still aborts ingame");
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
        tx.try_send(Poke::Other).unwrap();
        assert!(wait_before_retry(&mut rx, Duration::from_secs(30)).await);
        // Websocket task gone -> end the session so the top level re-reads the lockfile.
        drop(tx);
        assert!(!wait_before_retry(&mut rx, Duration::from_secs(30)).await);
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
        c.invalidate();
        assert!(!c.is_fresh_for("m", true));
        assert!(c.names.is_empty());
        assert_eq!(c.match_id, None);
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
}
