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
    let region = normalize_region(raw_region).to_string();
    let shard = region_to_shard(raw_region).to_string();
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
        let resp = self.apply_headers(self.http.get(url)).send().await?;
        Self::handle(resp).await
    }

    /// PUT a JSON body (name-service batch).
    pub async fn put_json(&self, url: &str, body: &Value) -> Result<Value> {
        let resp = self.apply_headers(self.http.put(url)).json(body).send().await?;
        Self::handle(resp).await
    }

    async fn handle(resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(Error::RateLimited);
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

    pub async fn names(&self, puuids: &[String]) -> Result<Value> {
        let body = serde_json::to_value(puuids)?;
        self.put_json(&format!("{}/name-service/v2/players", self.hosts.pd), &body).await
    }

    pub async fn content(&self) -> Result<Value> {
        self.get(&format!("{}/content-service/v3/content", self.hosts.shared)).await
    }
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
        429 => Err(Error::RateLimited),
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
        assert!(matches!(map_body_error(429, json!({})), Err(Error::RateLimited)));
    }

    #[test]
    fn passes_success_body_through() {
        let ok = map_body_error(200, json!({ "MatchID": "x" })).unwrap();
        assert_eq!(ok.get("MatchID").unwrap(), "x");
    }
}
