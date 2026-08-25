//! Local Riot Client HTTPS API (127.0.0.1, self-signed cert, basic auth).
//! Provides entitlements/access tokens, presences, and region-locale. See spec §1-2, §4.

use crate::riot::error::{Error, Result};
use crate::riot::lockfile::Lockfile;
use crate::riot::presence::RawPresence;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

/// Entitlements + access token from the local client.
#[derive(Debug, Clone, Deserialize)]
pub struct EntitlementsToken {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    /// The entitlements JWT (`X-Riot-Entitlements-JWT`).
    pub token: String,
    /// The local player's own PUUID.
    pub subject: String,
}

/// `/riotclient/region-locale` response.
#[derive(Debug, Clone, Deserialize)]
pub struct RegionLocale {
    pub region: String,
}

/// A client scoped to the local Riot Client. Accepts the self-signed cert — this client
/// is only ever pointed at 127.0.0.1.
pub struct LocalClient {
    http: reqwest::Client,
    lockfile: Lockfile,
}

impl LocalClient {
    pub fn new(lockfile: Lockfile) -> Result<Self> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { http, lockfile })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.lockfile.local_base_url(), path)
    }

    /// GET a local endpoint with basic auth, mapping "not ready" bodies to `NotReady`.
    async fn get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(self.url(path))
            .header("Authorization", self.lockfile.basic_auth_header())
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if is_not_ready(&body) || status.as_u16() == 404 {
            return Err(Error::NotReady);
        }
        if !status.is_success() {
            return Err(Error::Http(format!("local {path} -> {status}")));
        }
        Ok(body)
    }

    /// GET with retry (3 attempts, 5s apart) for the client-still-starting case.
    async fn get_retry(&self, path: &str) -> Result<Value> {
        let mut last = Error::NotReady;
        for attempt in 0..3 {
            match self.get(path).await {
                Ok(v) => return Ok(v),
                Err(e) if is_starting_up(&e) => {
                    last = e;
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Fetch entitlements + access token (retries while the client is starting).
    pub async fn entitlements(&self) -> Result<EntitlementsToken> {
        let body = self.get_retry("/entitlements/v1/token").await?;
        serde_json::from_value(body).map_err(Error::from)
    }

    /// Fetch the client's region (for shard/host construction).
    pub async fn region_locale(&self) -> Result<RegionLocale> {
        let body = self.get_retry("/riotclient/region-locale").await?;
        serde_json::from_value(body).map_err(Error::from)
    }

    /// Fetch all current presences, optionally handing back the untouched response body
    /// alongside the parsed roster. The body is cloned only when `raw` is set, so the
    /// normal path (`raw == false`) pays nothing — it exists for the dev-only capture
    /// (`docs/testing.md`), where unknown/undecoded fields are what needs inspecting.
    pub async fn presences_with_raw(&self, raw: bool) -> Result<(Vec<RawPresence>, Option<Value>)> {
        let body = self.get("/chat/v4/presences").await?;
        let captured = if raw { Some(body.clone()) } else { None };
        // Deserialized by reference: the roster is read up to 60x/min in agent select, so the
        // array is never cloned out of the body first.
        let presences = body
            .get("presences")
            .and_then(|p| p.as_array())
            .map(|list| list.iter().filter_map(|v| RawPresence::deserialize(v).ok()).collect())
            .unwrap_or_default();
        Ok((presences, captured))
    }
}

/// Whether a failed local GET means the client is still starting, so the bounded retry should
/// take another attempt. Spec §1 counts ANY non-200 (not just 404) or an RPC_ERROR body as "not
/// ready yet" while the client boots, which is what `Error::Http` carries here. Everything else
/// (a malformed body, a transport failure) is a different problem and fails at once. Pure.
fn is_starting_up(err: &Error) -> bool {
    matches!(err, Error::NotReady | Error::Http(_))
}

/// True if a local response body is a "client still starting" placeholder.
fn is_not_ready(body: &Value) -> bool {
    if let Some(code) = body.get("errorCode").and_then(|v| v.as_str()) {
        if code == "RPC_ERROR" {
            return true;
        }
    }
    if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
        return msg.contains("not ready") || msg.contains("Invalid URI");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_rpc_error_not_ready() {
        assert!(is_not_ready(&json!({ "errorCode": "RPC_ERROR" })));
    }

    #[test]
    fn detects_message_not_ready() {
        assert!(is_not_ready(&json!({ "message": "Entitlements token is not ready yet" })));
        assert!(is_not_ready(&json!({ "message": "Invalid URI format" })));
    }

    #[test]
    fn ready_body_is_not_flagged() {
        assert!(!is_not_ready(&json!({ "accessToken": "x", "token": "y", "subject": "z" })));
    }

    #[test]
    fn any_non_200_is_retried_while_the_client_starts() {
        // Spec §1: during startup every non-200 (not only 404) reads as "not ready yet".
        assert!(is_starting_up(&Error::NotReady));
        assert!(is_starting_up(&Error::Http("local /x -> 503".into())));
        // A body we could not make sense of, or a dead socket, is a different failure.
        assert!(!is_starting_up(&Error::MalformedPayload("x".into())));
        assert!(!is_starting_up(&Error::ResourceNotFound));
    }

    #[test]
    fn parses_entitlements_shape() {
        let e: EntitlementsToken =
            serde_json::from_value(json!({ "accessToken": "a", "token": "t", "subject": "s", "issuer": "i", "entitlements": [] })).unwrap();
        assert_eq!(e.access_token, "a");
        assert_eq!(e.token, "t");
        assert_eq!(e.subject, "s");
    }
}
