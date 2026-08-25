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
use crate::riot::stats::{self, HitCounts, RrHistory};
use crate::riot::types::{AppStatus, MapInfo, SessionLoopState, TrackerSnapshot};
use crate::riot::websocket::Poke;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

/// Dev-only snapshot capture. Compiled out of release builds entirely, and inert unless
/// `VLT_DEBUG_CAPTURE` names a directory — see `docs/testing.md`.
#[cfg(debug_assertions)]
mod debug_capture {
    use super::TrackerSnapshot;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Emission counter, so captured files sort in the order they were published.
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Write `snapshot` as pretty JSON to `$VLT_DEBUG_CAPTURE/snapshot-{n:04}-{status}.json`.
    /// Best-effort: every failure (unset var, unwritable dir, serialization) is ignored so
    /// capture can never affect the running app.
    pub fn write(snapshot: &TrackerSnapshot) {
        let Some(dir) = std::env::var_os("VLT_DEBUG_CAPTURE") else {
            return;
        };
        if dir.is_empty() {
            return;
        }
        let dir = std::path::PathBuf::from(dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("snapshot-{:04}-{:?}.json", n, snapshot.status));
        if let Ok(json) = serde_json::to_string_pretty(snapshot) {
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
    season_id: String,
    /// Content-service season list, kept for the peak-rank act label.
    seasons: Vec<content::Season>,
    /// Names/MMR/stats cached per match id (+ state) so an in-match presence update (score
    /// changes every round) does not refetch them (L1).
    cache: MatchCache,
    /// HS% cached across matches within the session, keyed by puuid + the player's newest
    /// competitive match id — so a returning player's ~500 KB match-details are not
    /// re-downloaded while their newest match is unchanged (phase 2 constraint).
    hs_cache: HsCache,
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
    names: HashMap<String, String>,
    mmr: HashMap<String, MmrResponse>,
    /// puuid -> ΔRR + last-5 pips (phase 2).
    updates: HashMap<String, RrHistory>,
    /// puuid -> HS% over recent matches (phase 2; inner None == "N/a").
    headshots: HashMap<String, Option<u32>>,
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
        if self.match_id.as_deref() != Some(match_id) {
            self.names.clear();
            self.mmr.clear();
            self.updates.clear();
            self.headshots.clear();
            self.skins.clear();
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

/// Session-lived HS% cache: puuid -> (newest competitive match id it was computed from,
/// the HS%). Persists across matches (NOT cleared on MENUS) so it self-invalidates only
/// when a player's newest competitive match changes.
#[derive(Default)]
struct HsCache {
    map: HashMap<String, (String, Option<u32>)>,
}

impl HsCache {
    /// Cached HS% for `puuid` iff it was computed from the same `newest_match_id`.
    fn get(&self, puuid: &str, newest_match_id: &str) -> Option<Option<u32>> {
        self.map
            .get(puuid)
            .filter(|(id, _)| id == newest_match_id)
            .map(|(_, hs)| *hs)
    }

    fn put(&mut self, puuid: &str, newest_match_id: &str, hs: Option<u32>) {
        self.map.insert(puuid.to_string(), (newest_match_id.to_string(), hs));
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
    let static_data = static_data::fetch(&public).await.unwrap_or_default();
    let client_version = static_data.version.clone();

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
        season_id,
        seasons,
        cache: MatchCache::default(),
        hs_cache: HsCache::default(),
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
            // Bail immediately if the client is gone (lockfile removed) rather than
            // burning the full backoff schedule on a dead client (C3).
            if lockfile::default_path().map(|p| !p.exists()).unwrap_or(true) {
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
    // never lost behind the burst (HIGH-2).
    loop {
        let interrupted = {
            let mut ctx = BuildCtx { state, emitter, rx: &mut rx };
            build_and_publish(session, &mut ctx).await
        };
        if interrupted {
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
async fn wait_for_rebuild_poke(rx: &mut mpsc::Receiver<Poke>, state: &TrackerState) -> bool {
    while let Some(first) = rx.recv().await {
        let poke = collapse(drain_pokes(rx), first);
        if poke_triggers_rebuild(poke, state.status()) {
            return true;
        }
    }
    false
}

/// Build a snapshot and publish it, routing BadClaims through a token refresh + one retry
/// (C7) and NotReady through a "Loading..." placeholder. Shared by the initial paint and
/// the event loop so both handle token expiry identically. Returns true when the build was
/// interrupted mid-enrichment and the caller should rebuild immediately (HIGH-2).
async fn build_and_publish(session: &mut Session, ctx: &mut BuildCtx<'_>) -> bool {
    match build_snapshot(session, ctx).await {
        Ok(interrupted) => interrupted,
        Err(Error::NotReady) => {
            publish(
                ctx.state,
                ctx.emitter,
                TrackerSnapshot::not_running(Some("Loading...".into())),
            );
            false
        }
        Err(Error::BadClaims) => {
            if refresh_tokens(session).await.is_ok() {
                build_snapshot(session, ctx).await.unwrap_or(false)
            } else {
                false
            }
        }
        Err(_) => false, // transient (404 race, rate limit) — wait for the next event
    }
}

/// Build (and publish) a full snapshot for the current state (Menus / Pregame / Ingame).
/// Returns true if enrichment was interrupted (see `build_and_publish`).
async fn build_snapshot(session: &mut Session, ctx: &mut BuildCtx<'_>) -> Result<bool> {
    let presences = session.local.presences().await?;
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
            Ok(false)
        }
    }
}

/// Fetch + assemble a pregame or coregame snapshot, using the two-phase emit: the first
/// snapshot carries names + ranks + RR + peak + WR (all free once names+MMR are in) with
/// the heavy fields (rrChange / recentResults / headshotPercent / skins) empty; a second,
/// enriched snapshot follows once those are fetched. Returns true if enrichment aborted on
/// a mid-burst poke (HIGH-2).
async fn build_match_snapshot(
    session: &mut Session,
    info: &PresenceInfo,
    parties: &HashMap<String, String>,
    ingame: bool,
    ctx: &mut BuildCtx<'_>,
) -> Result<bool> {
    let own = session.own_puuid.clone();

    // Match id. On the RESOURCE_NOT_FOUND transition race, retry once — after ~5s for
    // coregame, immediately for pregame (spec §10.5, C9).
    let id_json = if ingame {
        fetch_with_retry(Duration::from_secs(5), || session.remote.coregame_match_id(&own)).await?
    } else {
        fetch_with_retry(Duration::ZERO, || session.remote.pregame_match_id(&own)).await?
    };
    let match_id = match_state::extract_match_id(&id_json).ok_or(Error::ResourceNotFound)?;

    // Match data (owned Values consumed by the extractors — no deep clone, L4).
    let (players, own_team, map_id): (Vec<MatchPlayer>, Option<String>, Option<String>) = if ingame
    {
        let m = session.remote.coregame_match(&match_id).await?;
        let data = match_state::extract_coregame(m, &own);
        (data.players, data.own_team, data.map_id)
    } else {
        let m = session.remote.pregame_match(&match_id).await?;
        let data = match_state::extract_pregame(m, &own);
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
        return Ok(false);
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

    // === Phase 2: competitiveupdates + HS% + loadout skins. Interruptible. ===
    let interrupted = fetch_phase2(
        &session.remote,
        &mut session.cache,
        &mut session.hs_cache,
        &puuids,
        ingame,
        &match_id,
        ctx.rx,
    )
    .await?;
    if interrupted {
        // Phase-1 data stays cached (enriched == false) so the immediate rebuild reuses it
        // and only finishes the missing phase-2 work.
        return Ok(true);
    }

    session.cache.enriched = true;
    let snap2 = assemble_snapshot(session, &players, parties, own_team, map, mode, status, true);
    publish(ctx.state, ctx.emitter, snap2);
    Ok(false)
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
        headshots: &session.cache.headshots,
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

/// Run a remote fetch with one backed-off retry on a 429 (spec §10.6, C5) rather than
/// silently degrading. BadClaims and other errors pass straight through to the caller.
async fn with_rate_limit_retry<F, Fut>(f: F) -> Result<serde_json::Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    match f().await {
        Err(Error::RateLimited) => {
            tokio::time::sleep(Duration::from_secs(6)).await;
            f().await
        }
        other => other,
    }
}

/// Batch name resolution. Propagates BadClaims so the caller refreshes tokens + retries
/// once (C4); other transport errors degrade to an empty map (names render as placeholders
/// rather than failing the whole table).
async fn fetch_names(remote: &RemoteClient, puuids: &[String]) -> Result<HashMap<String, String>> {
    match with_rate_limit_retry(|| remote.names(puuids)).await {
        Ok(v) => match names::parse_name_response(&v) {
            Ok(map) => Ok(map),
            Err(Error::BadClaims) => Err(Error::BadClaims),
            Err(_) => Ok(HashMap::new()),
        },
        Err(Error::BadClaims) => Err(Error::BadClaims),
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
/// reuse). MMR is the WR source too, so this covers every "free" field. BadClaims
/// propagates for the shared token refresh (C4/C7); a single MMR failure degrades that row
/// to Unranked. The inter-request delay is applied only BETWEEN requests (LOW: no trailing
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
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            Err(_) => { /* private profile / hiccup -> Unranked for this row only */ }
        }
    }
    Ok(())
}

/// Phase 2 of the two-phase emit: competitiveupdates (ΔRR + last-5 + recent match ids),
/// HS% (throttled + session-cached), and — INGAME only — loadout skins. Only puuids not
/// already cached are fetched (HIGH-1 reuse of pregame data + resume-after-abort). Between
/// per-player requests it checks the poke channel and returns `Ok(true)` to abort promptly
/// when a new presence event arrives, so a dodge/transition is not blocked behind the
/// remaining fetches (HIGH-2). Partial results are left in the cache so the rebuild only
/// finishes the missing work. BadClaims propagates for the shared refresh; other per-player
/// failures degrade that field only. The inter-request delay spaces requests only BETWEEN
/// them (LOW: no trailing sleep, none after a skip/cache-hit).
async fn fetch_phase2(
    remote: &RemoteClient,
    cache: &mut MatchCache,
    hs_cache: &mut HsCache,
    puuids: &[String],
    ingame: bool,
    match_id: &str,
    rx: &mut mpsc::Receiver<Poke>,
) -> Result<bool> {
    // Tracks whether any request has been issued yet, so the 120 ms delay only ever sits
    // *between* two real requests across the whole phase.
    let mut sent_any = false;

    // competitiveupdates: one request per player missing history.
    let missing_updates: Vec<String> = puuids
        .iter()
        .filter(|p| !cache.updates.contains_key(*p))
        .cloned()
        .collect();
    for puuid in &missing_updates {
        if abort_pending(rx, ingame) {
            return Ok(true);
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
            Err(_) => { /* no history for this row */ }
        }
    }

    // HS%: up to RECENT_MATCHES_FOR_HS match-details per player missing it, session-cached.
    for puuid in puuids {
        if cache.headshots.contains_key(puuid) {
            continue;
        }
        if abort_pending(rx, ingame) {
            return Ok(true);
        }
        let Some(newest) = cache.updates.get(puuid).and_then(|h| h.newest_match_id()) else {
            // No recent competitive matches -> HS% is "N/a".
            cache.headshots.insert(puuid.clone(), None);
            continue;
        };
        // Session cache hit (same newest match) -> reuse, no match-details fetch.
        if let Some(hs) = hs_cache.get(puuid, newest) {
            cache.headshots.insert(puuid.clone(), hs);
            continue;
        }
        // Cache miss -> fetch up to N match-details and accumulate this player's hits.
        let newest = newest.to_string();
        let match_ids = cache
            .updates
            .get(puuid)
            .map(|h| h.recent_match_ids.clone())
            .unwrap_or_default();
        let mut acc = HitCounts::default();
        for mid in &match_ids {
            if abort_pending(rx, ingame) {
                return Ok(true);
            }
            if sent_any {
                tokio::time::sleep(inter_request_delay()).await;
            }
            sent_any = true;
            match with_rate_limit_retry(|| remote.match_details(mid)).await {
                Ok(v) => stats::accumulate_match_hits(&mut acc, &v, puuid),
                Err(Error::BadClaims) => return Err(Error::BadClaims),
                Err(_) => { /* skip this match, keep whatever we have */ }
            }
        }
        let hs = acc.headshot_percent();
        hs_cache.put(puuid, &newest, hs);
        cache.headshots.insert(puuid.clone(), hs);
    }

    // Loadout skins: one request per match, INGAME only.
    if ingame && cache.skins.is_empty() {
        if abort_pending(rx, ingame) {
            return Ok(true);
        }
        match with_rate_limit_retry(|| remote.coregame_loadouts(match_id)).await {
            Ok(v) => {
                cache.skins = loadout::parse_loadouts(&v);
            }
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            Err(_) => { /* no skins this match */ }
        }
    }

    Ok(false)
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
    fn hs_cache_hits_only_on_matching_newest_match() {
        let mut hs = HsCache::default();
        // Miss when empty.
        assert_eq!(hs.get("p", "m1"), None);

        hs.put("p", "m1", Some(25));
        // Hit only when the newest match id matches -> no re-download of match-details.
        assert_eq!(hs.get("p", "m1"), Some(Some(25)));
        // A newer match for the same player -> miss (must recompute).
        assert_eq!(hs.get("p", "m2"), None);
        // Persists across matches (not cleared with the per-match cache).
        assert_eq!(hs.get("p", "m1"), Some(Some(25)));
    }
}
