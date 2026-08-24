//! Orchestration: the background state machine that connects to Valorant when it appears,
//! reconnects on loss, and emits a `TrackerSnapshot` on every change. Never crashes when
//! the game isn't running — that is a normal `ValorantNotRunning` snapshot.

use crate::riot::assemble::{assemble_players, AssembleInput};
use crate::riot::constants::game_mode_name;
use crate::riot::content;
use crate::riot::error::{Error, Result};
use crate::riot::lockfile::{self, Lockfile};
use crate::riot::local_api::LocalClient;
use crate::riot::match_state::{self, MatchPlayer};
use crate::riot::names;
use crate::riot::presence::{self, PresenceInfo};
use crate::riot::rank::{parse_mmr, MmrResponse};
use crate::riot::remote_api::{build_hosts, Auth, RemoteClient};
use crate::riot::static_data::{self, StaticData};
use crate::riot::types::{AppStatus, SessionLoopState, TrackerSnapshot};
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

    fn store(&self, snap: TrackerSnapshot) {
        *self.snapshot.lock().unwrap() = snap;
    }

    /// Mark the tracker as started. Returns true exactly once (the first call), so the
    /// caller spawns the loop only once — makes `start_tracker` idempotent.
    pub fn begin(&self) -> bool {
        !self.started.swap(true, Ordering::SeqCst)
    }
}

/// Emit + store a snapshot only if it differs from the last one (avoids UI churn).
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
    emitter.emit(&snap);
}

/// Everything needed once connected to a running client.
struct Session {
    lockfile: Lockfile,
    local: LocalClient,
    remote: RemoteClient,
    own_puuid: String,
    static_data: StaticData,
    season_id: String,
    /// Names/MMR cached per match id so an in-match presence update (score changes every
    /// round) does not refetch them (L1).
    cache: MatchCache,
}

/// Per-match cache of the expensive lookups (names + MMR). Keyed by match id; a new match
/// id or a transition to MENUS invalidates it. This is the core lightweightness guarantee:
/// only the first snapshot of a given match pays for name/MMR fetches.
#[derive(Default)]
struct MatchCache {
    match_id: Option<String>,
    names: HashMap<String, String>,
    mmr: HashMap<String, MmrResponse>,
}

impl MatchCache {
    /// True when the cache already holds names/MMR for `match_id` (skip the refetch).
    fn is_fresh_for(&self, match_id: &str) -> bool {
        self.match_id.as_deref() == Some(match_id)
    }

    /// Replace the cache with freshly-fetched data for a match.
    fn store(
        &mut self,
        match_id: String,
        names: HashMap<String, String>,
        mmr: HashMap<String, MmrResponse>,
    ) {
        self.match_id = Some(match_id);
        self.names = names;
        self.mmr = mmr;
    }

    /// Drop everything (transition to MENUS / not-running).
    fn invalidate(&mut self) {
        *self = Self::default();
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

    // Season id from content service.
    let season_id = match remote.content().await {
        Ok(content_json) => {
            let seasons = content::parse_seasons(&content_json);
            content::current_season_id(&seasons).unwrap_or_default()
        }
        Err(_) => String::new(),
    };

    Ok(Session {
        lockfile,
        local,
        remote,
        own_puuid: entitlements.subject,
        static_data,
        season_id,
        cache: MatchCache::default(),
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
    // Poke channel: the websocket sends `()` on each own-presence event, and once after
    // every reconnect so we re-poll presence for any transition missed while the socket
    // was down (C2). The task ends (dropping tx) only when the client is gone.
    let (tx, mut rx) = mpsc::channel::<()>(32);
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
            if tx.send(()).await.is_err() {
                break; // receiver gone
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(30);
        }
    });

    // Initial snapshot from REST (BadClaims routed through the refresh arm — C7).
    build_and_publish(session, state, emitter).await;

    // React to state-change pushes.
    while rx.recv().await.is_some() {
        // Collapse a burst of pokes (many presence updates arriving at once, or a poke
        // right behind a real event) into a single rebuild (L1).
        while rx.try_recv().is_ok() {}
        build_and_publish(session, state, emitter).await;
    }

    ws_task.abort();
}

/// Build a snapshot and publish it, routing BadClaims through a token refresh + one retry
/// (C7) and NotReady through a "Loading..." placeholder. Shared by the initial paint and
/// the event loop so both handle token expiry identically.
async fn build_and_publish(
    session: &mut Session,
    state: &Arc<TrackerState>,
    emitter: &Arc<dyn Emitter>,
) {
    match build_snapshot(session).await {
        Ok(snap) => publish(state, emitter, snap),
        Err(Error::NotReady) => publish(
            state,
            emitter,
            TrackerSnapshot::not_running(Some("Loading...".into())),
        ),
        Err(Error::BadClaims) => {
            if refresh_tokens(session).await.is_ok() {
                if let Ok(snap) = build_snapshot(session).await {
                    publish(state, emitter, snap);
                }
            }
        }
        Err(_) => { /* transient (404 race, rate limit) — wait for the next event */ }
    }
}

/// Build a full snapshot for the current state (Menus / Pregame / Ingame).
async fn build_snapshot(session: &mut Session) -> Result<TrackerSnapshot> {
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
            build_match_snapshot(session, &info, &parties, false).await
        }
        Some(SessionLoopState::Ingame) => {
            build_match_snapshot(session, &info, &parties, true).await
        }
        // MENUS or unknown -> menus snapshot. A new match always starts from menus, so this
        // is where per-match name/MMR cache is invalidated (L1 + pitfall §12).
        _ => {
            session.cache.invalidate();
            Ok(TrackerSnapshot {
                status: AppStatus::Menus,
                map: None,
                mode: None,
                own_team: None,
                players: Vec::new(),
                last_updated: crate::riot::types::now_millis(),
                message: None,
            })
        }
    }
}

/// Fetch + assemble a pregame or coregame snapshot.
async fn build_match_snapshot(
    session: &mut Session,
    info: &PresenceInfo,
    parties: &HashMap<String, String>,
    ingame: bool,
) -> Result<TrackerSnapshot> {
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

    // Names + MMR: fetched once per match id, then served from cache for every subsequent
    // in-match presence update (score changes each round must NOT refetch — L1).
    if !session.cache.is_fresh_for(&match_id) {
        let puuids: Vec<String> = players.iter().map(|p| p.puuid.clone()).collect();
        let names = fetch_names(&session.remote, &puuids).await?;
        let mmr = fetch_all_mmr(&session.remote, &puuids).await?;
        session.cache.store(match_id, names, mmr);
    }

    let status = if ingame { AppStatus::Ingame } else { AppStatus::Pregame };
    let mode = if info.is_custom_game() {
        Some("Custom Game".to_string())
    } else {
        info.queue_id.as_deref().map(game_mode_name)
    };
    let map = session.static_data.map(map_id.as_deref());

    let rows = assemble_players(&AssembleInput {
        players: &players,
        names: &session.cache.names,
        mmr: &session.cache.mmr,
        parties,
        static_data: &session.static_data,
        own_puuid: &own,
        own_team: own_team.as_deref(),
        current_season_id: &session.season_id,
    });

    Ok(TrackerSnapshot {
        status,
        map,
        mode,
        own_team,
        players: rows,
        last_updated: crate::riot::types::now_millis(),
        message: None,
    })
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

/// Fetch MMR for every player. A single player's failure degrades that row to Unranked,
/// never the whole table (spec §10.14), but a BadClaims means our token is stale for all
/// calls, so it propagates for the caller to refresh + retry once (C4).
async fn fetch_all_mmr(
    remote: &RemoteClient,
    puuids: &[String],
) -> Result<HashMap<String, MmrResponse>> {
    let mut out = HashMap::new();
    for puuid in puuids {
        match with_rate_limit_retry(|| remote.mmr(puuid)).await {
            Ok(v) => {
                out.insert(puuid.clone(), parse_mmr(v));
            }
            Err(Error::BadClaims) => return Err(Error::BadClaims),
            Err(_) => { /* private profile / hiccup -> Unranked for this row only */ }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(id: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(id.to_string(), format!("{id}#1"));
        m
    }

    #[test]
    fn cache_serves_same_match_and_refetches_on_new_match() {
        let mut cache = MatchCache::default();
        // Empty cache is never fresh (first snapshot of any match must fetch).
        assert!(!cache.is_fresh_for("match-a"));

        cache.store("match-a".to_string(), names("p"), HashMap::new());
        // Same match id -> reuse (an in-match presence update must NOT refetch).
        assert!(cache.is_fresh_for("match-a"));
        assert_eq!(cache.names.get("p").map(String::as_str), Some("p#1"));

        // A different match id -> stale, forcing a refetch.
        assert!(!cache.is_fresh_for("match-b"));
    }

    #[test]
    fn invalidate_forces_refetch() {
        let mut cache = MatchCache::default();
        cache.store("match-a".to_string(), names("p"), HashMap::new());
        assert!(cache.is_fresh_for("match-a"));

        // Transition to MENUS clears the cache (pitfall §12 / L1).
        cache.invalidate();
        assert!(!cache.is_fresh_for("match-a"));
        assert!(cache.names.is_empty());
        assert!(cache.mmr.is_empty());
    }
}
