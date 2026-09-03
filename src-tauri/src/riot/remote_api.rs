//! Remote Riot endpoints (pd / glz / shared). Header assembly, region->shard host
//! construction, and error mapping (BAD_CLAIMS, 404 races, 429). See spec §2-3, §5-7.

use crate::riot::constants::{
    normalize_region, region_to_shard, CLIENT_PLATFORM, USER_AGENT,
};
use crate::riot::error::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// pd / glz / shared base URLs for a region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hosts {
    pub pd: String,
    pub glz: String,
    pub shared: String,
    /// Normalized region (pbe -> na) used for glz + shared.
    pub region: String,
    pub shard: String,
}

/// Build hosts from a raw region string. Pure — testable.
pub fn build_hosts(raw_region: &str) -> Hosts {
    let region = normalize_region(raw_region);
    let shard = region_to_shard(raw_region);
    Hosts {
        pd: format!("https://pd.{shard}.a.pvp.net"),
        glz: format!("https://glz-{region}-1.{shard}.a.pvp.net"),
        shared: format!("https://shared.{shard}.a.pvp.net"),
        region,
        shard,
    }
}

/// Mutable auth context; refreshed on BAD_CLAIMS / 429.
#[derive(Debug, Clone)]
pub struct Auth {
    pub access_token: String,
    pub entitlements_token: String,
    pub client_version: String,
}

/// Client for the remote endpoints.
pub struct RemoteClient {
    http: reqwest::Client,
    pub hosts: Hosts,
    pub auth: Auth,
}

impl RemoteClient {
    pub fn new(hosts: Hosts, auth: Auth) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { http, hosts, auth })
    }

    /// Update tokens after a refresh (keeps client version).
    pub fn set_tokens(&mut self, access_token: String, entitlements_token: String) {
        self.auth.access_token = access_token;
        self.auth.entitlements_token = entitlements_token;
    }

    /// Override the client version (used once connected: the version read from own
    /// presence is authoritative, with valorant-api.com as bootstrap/fallback — the
    /// public version can lag the real client on patch days). See spec Live verification.
    pub fn set_client_version(&mut self, client_version: String) {
        self.auth.client_version = client_version;
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("Authorization", format!("Bearer {}", self.auth.access_token))
            .header("X-Riot-Entitlements-JWT", &self.auth.entitlements_token)
            .header("X-Riot-ClientPlatform", CLIENT_PLATFORM)
            .header("X-Riot-ClientVersion", &self.auth.client_version)
            .header("User-Agent", USER_AGENT)
    }

    /// GET a remote URL and map the standard error shapes.
    pub async fn get(&self, url: &str) -> Result<Value> {
        #[cfg(debug_assertions)]
        let (seq, started) = (crate::debug_log::next_request_seq(), std::time::Instant::now());
        let resp = self.apply_headers(self.http.get(url)).send().await?;
        #[cfg(debug_assertions)]
        let status = resp.status().as_u16();
        let result = Self::handle(resp).await;
        #[cfg(debug_assertions)]
        log_response(seq, "GET", url, status, started, &result);
        result
    }

    /// PUT a JSON body (name-service batch).
    pub async fn put_json(&self, url: &str, body: &Value) -> Result<Value> {
        #[cfg(debug_assertions)]
        let (seq, started) = (crate::debug_log::next_request_seq(), std::time::Instant::now());
        let resp = self.apply_headers(self.http.put(url)).json(body).send().await?;
        #[cfg(debug_assertions)]
        let status = resp.status().as_u16();
        let result = Self::handle(resp).await;
        #[cfg(debug_assertions)]
        log_response(seq, "PUT", url, status, started, &result);
        result
    }

    async fn handle(resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        if status.as_u16() == 429 {
            // Carry the server's own backoff up to the retry wrapper.
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            return Err(Error::RateLimited(retry_after));
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        map_body_error(status.as_u16(), body)
    }

    // --- typed endpoint helpers ---

    pub async fn coregame_match_id(&self, puuid: &str) -> Result<Value> {
        self.get(&format!("{}/core-game/v1/players/{}", self.hosts.glz, puuid)).await
    }

    pub async fn coregame_match(&self, match_id: &str) -> Result<Value> {
        self.get(&format!("{}/core-game/v1/matches/{}", self.hosts.glz, match_id)).await
    }

    pub async fn pregame_match_id(&self, puuid: &str) -> Result<Value> {
        self.get(&format!("{}/pregame/v1/players/{}", self.hosts.glz, puuid)).await
    }

    pub async fn pregame_match(&self, match_id: &str) -> Result<Value> {
        self.get(&format!("{}/pregame/v1/matches/{}", self.hosts.glz, match_id)).await
    }

    pub async fn mmr(&self, puuid: &str) -> Result<Value> {
        self.get(&format!("{}/mmr/v1/players/{}", self.hosts.pd, puuid)).await
    }

    /// competitiveupdates: ΔRR + last-N W/L + recent match ids (phase 2). Competitive queue,
    /// small window (spec Live verification + probe capture).
    pub async fn competitive_updates(&self, puuid: &str) -> Result<Value> {
        let end = crate::riot::constants::COMPETITIVE_UPDATES_END_INDEX;
        self.get(&format!(
            "{}/mmr/v1/players/{}/competitiveupdates?startIndex=0&endIndex={}&queue=competitive",
            self.hosts.pd, puuid, end
        ))
        .await
    }

    /// match-details for one match (~500 KB) — the HS% source (phase 2).
    pub async fn match_details(&self, match_id: &str) -> Result<Value> {
        self.get(&format!("{}/match-details/v1/matches/{}", self.hosts.pd, match_id)).await
    }

    /// coregame loadouts for a match (~79 KB) — Vandal/Phantom skins (phase 2, INGAME only).
    pub async fn coregame_loadouts(&self, match_id: &str) -> Result<Value> {
        self.get(&format!("{}/core-game/v1/matches/{}/loadouts", self.hosts.glz, match_id)).await
    }

    pub async fn names(&self, puuids: &[String]) -> Result<Value> {
        let body = serde_json::to_value(puuids)?;
        self.put_json(&format!("{}/name-service/v2/players", self.hosts.pd), &body).await
    }

    pub async fn content(&self) -> Result<Value> {
        self.get(&format!("{}/content-service/v3/content", self.hosts.shared)).await
    }
}

/// One console line per remote request: serial, verb, path, HTTP status, round trip, and the
/// mapped error when the response did not carry usable data.
#[cfg(debug_assertions)]
fn log_response(
    seq: u32,
    verb: &str,
    url: &str,
    status: u16,
    started: std::time::Instant,
    result: &Result<Value>,
) {
    let ms = started.elapsed().as_millis();
    match result {
        Ok(_) => vlt_log!("net", "#{seq} {verb} {} -> {status} ({ms}ms)", url_path(url)),
        Err(err) => {
            vlt_log!("net", "#{seq} {verb} {} -> {status} {err:?} ({ms}ms)", url_path(url))
        }
    }
}

/// The path of an absolute URL, so a log line is not two thirds host name, with every id-like
/// segment cut to its first 8 characters and the query dropped — puuids and match ids never
/// reach the console whole. A segment counts as an id when it is long and carries a digit,
/// which leaves the endpoint words (`core-game`, `competitiveupdates`, `v1`) readable.
#[cfg(debug_assertions)]
fn url_path(url: &str) -> String {
    let after_scheme = url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or(url);
    let path = after_scheme.split(['?', '#']).next().unwrap_or(after_scheme);
    path.split('/').map(redact_segment).collect::<Vec<_>>().join("/")
}

#[cfg(debug_assertions)]
fn redact_segment(segment: &str) -> &str {
    if segment.len() > 8 && segment.bytes().any(|b| b.is_ascii_digit()) {
        crate::debug_log::short(segment)
    } else {
        segment
    }
}

/// Longest `Retry-After` we honor. A server (or proxy) asking for minutes would otherwise
/// stall a rebuild well past the point where the match state has moved on.
pub const MAX_RETRY_AFTER_SECS: u64 = 30;

/// Parse a `Retry-After` header in its delay-seconds form, capped at `MAX_RETRY_AFTER_SECS`.
/// The HTTP-date form is not emitted by Riot's edge and is ignored (the caller then falls back
/// to its own backoff). Pure — testable.
pub fn parse_retry_after(value: &str) -> Option<u64> {
    let secs: u64 = value.trim().parse().ok()?;
    Some(secs.min(MAX_RETRY_AFTER_SECS))
}

/// Map an HTTP status + body into our error taxonomy (or pass the body through on success).
pub fn map_body_error(status: u16, body: Value) -> Result<Value> {
    if let Some(code) = body.get("errorCode").and_then(|v| v.as_str()) {
        match code {
            "BAD_CLAIMS" => return Err(Error::BadClaims),
            "RESOURCE_NOT_FOUND" => return Err(Error::ResourceNotFound),
            _ => {}
        }
    }
    match status {
        404 => Err(Error::ResourceNotFound),
        429 => Err(Error::RateLimited(None)),
        // Spec §10.3: a stale bearer/entitlements pair is what 401/403 actually mean, so they
        // become the token-refresh signal instead of a generic transport error — the caller's
        // single refresh-and-retry arm then covers them.
        401 | 403 => Err(Error::BadClaims),
        s if (200..300).contains(&s) => Ok(body),
        s => Err(Error::Http(format!("remote status {s}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_standard_region_hosts() {
        let h = build_hosts("eu");
        assert_eq!(h.pd, "https://pd.eu.a.pvp.net");
        assert_eq!(h.glz, "https://glz-eu-1.eu.a.pvp.net");
        assert_eq!(h.shared, "https://shared.eu.a.pvp.net");
    }

    #[test]
    fn br_maps_to_na_shard() {
        let h = build_hosts("br");
        assert_eq!(h.pd, "https://pd.na.a.pvp.net");
        assert_eq!(h.glz, "https://glz-br-1.na.a.pvp.net");
        // shared follows the shard, not the region (C1): br -> na.
        assert_eq!(h.shared, "https://shared.na.a.pvp.net");
        assert_eq!(h.shard, "na");
        assert_eq!(h.region, "br");
    }

    #[test]
    fn latam_maps_to_na_shard() {
        let h = build_hosts("latam");
        assert_eq!(h.pd, "https://pd.na.a.pvp.net");
        assert_eq!(h.glz, "https://glz-latam-1.na.a.pvp.net");
    }

    #[test]
    fn pbe_normalizes_to_na() {
        let h = build_hosts("pbe");
        assert_eq!(h.region, "na");
        assert_eq!(h.shard, "na");
        assert_eq!(h.glz, "https://glz-na-1.na.a.pvp.net");
    }

    #[test]
    fn maps_bad_claims() {
        assert!(matches!(
            map_body_error(200, json!({ "errorCode": "BAD_CLAIMS" })),
            Err(Error::BadClaims)
        ));
    }

    #[test]
    fn maps_resource_not_found() {
        assert!(matches!(
            map_body_error(200, json!({ "errorCode": "RESOURCE_NOT_FOUND" })),
            Err(Error::ResourceNotFound)
        ));
        assert!(matches!(map_body_error(404, json!({})), Err(Error::ResourceNotFound)));
    }

    #[test]
    fn maps_rate_limit() {
        // The body-only path knows no header, so the caller falls back to its own backoff.
        assert!(matches!(map_body_error(429, json!({})), Err(Error::RateLimited(None))));
    }

    #[test]
    fn maps_auth_statuses_to_the_refresh_signal() {
        // 401/403 must reach the token-refresh arm, not degrade to `Http`.
        assert!(matches!(map_body_error(401, json!({})), Err(Error::BadClaims)));
        assert!(matches!(map_body_error(403, json!({})), Err(Error::BadClaims)));
        // Everything else non-OK stays a generic transport error.
        assert!(matches!(map_body_error(500, json!({})), Err(Error::Http(_))));
    }

    #[test]
    fn parses_and_caps_retry_after() {
        assert_eq!(parse_retry_after("5"), Some(5));
        assert_eq!(parse_retry_after("  12 "), Some(12));
        assert_eq!(parse_retry_after("0"), Some(0));
        // Capped so a huge (or hostile) value can't stall a rebuild.
        assert_eq!(parse_retry_after("600"), Some(MAX_RETRY_AFTER_SECS));
        // HTTP-date form and junk are ignored -> caller uses its own backoff.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("-3"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn passes_success_body_through() {
        let ok = map_body_error(200, json!({ "MatchID": "x" })).unwrap();
        assert_eq!(ok.get("MatchID").unwrap(), "x");
    }
}

#[cfg(all(test, debug_assertions))]
mod log_path_tests {
    use super::url_path;

    #[test]
    fn truncates_id_segments_and_keeps_endpoint_words() {
        assert_eq!(
            url_path("https://pd.eu.a.pvp.net/mmr/v1/players/8f4c1d2e-3a5b-4c6d-8e9f-0a1b2c3d4e5f"),
            "/mmr/v1/players/8f4c1d2e"
        );
        assert_eq!(
            url_path(
                "https://glz-eu-1.eu.a.pvp.net/core-game/v1/matches/8f4c1d2e-3a5b-4c6d-8e9f-0a1b2c3d4e5f/loadouts"
            ),
            "/core-game/v1/matches/8f4c1d2e/loadouts"
        );
    }

    #[test]
    fn drops_the_query_string() {
        assert_eq!(
            url_path(
                "https://pd.eu.a.pvp.net/mmr/v1/players/8f4c1d2e-3a5b-4c6d-8e9f-0a1b2c3d4e5f/competitiveupdates?startIndex=0&endIndex=5&queue=competitive"
            ),
            "/mmr/v1/players/8f4c1d2e/competitiveupdates"
        );
    }

    #[test]
    fn a_url_without_a_path_survives() {
        assert_eq!(url_path("https://shared.eu.a.pvp.net"), "https://shared.eu.a.pvp.net");
    }
}
