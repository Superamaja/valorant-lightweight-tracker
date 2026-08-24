//! Local websocket listener for presence/state-change push. Message parsing is pure and
//! tested; the connect/reconnect loop is IO (spec §4-A).

use crate::riot::error::{Error, Result};
use crate::riot::lockfile::Lockfile;
use crate::riot::presence::RawPresence;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Subscribe frame for presence events.
const SUBSCRIBE: &str = "[5, \"OnJsonApiEvent_chat_v4_presences\"]";

/// Parse a raw websocket text frame, returning the local player's own presence entry if
/// this is a `/chat/v4/presences` event containing it. Pure — testable.
///
/// Message shape: `[opcode, eventName, { uri, data: { presences: [...] } }]`.
/// League presence entries are skipped.
pub fn parse_presence_event(text: &str, own_puuid: &str) -> Option<RawPresence> {
    let value: Value = serde_json::from_str(text).ok()?;
    let arr = value.as_array()?;
    let payload = arr.get(2)?;
    if payload.get("uri").and_then(|u| u.as_str()) != Some("/chat/v4/presences") {
        return None;
    }
    let presences = payload.get("data")?.get("presences")?.as_array()?;
    for p in presences {
        // Skip (don't abort) a single malformed entry so one bad presence in the batch
        // can't discard the whole event (C6). Deserialize by ref to avoid a deep clone.
        let Ok(raw) = RawPresence::deserialize(p) else { continue };
        if raw.puuid == own_puuid && raw.is_valorant() {
            return Some(raw);
        }
    }
    None
}

/// Connect to the local websocket, subscribe, and poke the caller once per own-presence
/// event until the connection drops. The channel carries a unit poke (`()`) — the payload
/// is unused because the session re-polls full presence over REST on every poke, so there's
/// no reason to ship the presence struct across the channel.
///
/// Returns `Ok(())` once a connection was established and later closed (so the caller can
/// reset its reconnect backoff — C8), and `Err` only when the connection could not be
/// established at all.
pub async fn run_listener(
    lockfile: &Lockfile,
    own_puuid: &str,
    tx: mpsc::Sender<()>,
) -> Result<()> {
    let mut request = lockfile
        .local_ws_url()
        .into_client_request()
        .map_err(|e| Error::WebSocket(e.to_string()))?;
    request
        .headers_mut()
        .insert("Authorization", lockfile.basic_auth_header().parse().unwrap());

    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| Error::WebSocket(e.to_string()))?;
    let connector = tokio_tungstenite::Connector::NativeTls(tls);

    let (ws_stream, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        false,
        Some(connector),
    )
    .await
    .map_err(|e| Error::WebSocket(e.to_string()))?;

    let (mut write, mut read) = ws_stream.split();
    write
        .send(Message::Text(SUBSCRIBE.into()))
        .await
        .map_err(|e| Error::WebSocket(e.to_string()))?;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if parse_presence_event(&text, own_puuid).is_some() && tx.send(()).await.is_err() {
                    break; // receiver gone -> stop
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            // Mid-stream error: the connection was established, so treat this like a
            // normal close (Ok) — the caller reconnects and resets its backoff (C8).
            Err(_) => break,
        }
    }
    // Reached only after a successful connect; a clean/aborted close returns Ok so the
    // caller knows the connection had come up (backoff reset).
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot::presence;
    use crate::riot::types::SessionLoopState;
    use base64::Engine;
    use serde_json::json;

    fn encode_private(state: &str) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&json!({ "sessionLoopState": state })).unwrap())
    }

    #[test]
    fn extracts_own_presence_from_event() {
        let text = serde_json::to_string(&json!([
            8,
            "OnJsonApiEvent_chat_v4_presences",
            { "uri": "/chat/v4/presences", "data": { "presences": [
                { "puuid": "other", "product": "valorant", "private": encode_private("MENUS") },
                { "puuid": "me", "product": "valorant", "private": encode_private("INGAME") }
            ]}}
        ]))
        .unwrap();
        let raw = parse_presence_event(&text, "me").unwrap();
        assert_eq!(raw.puuid, "me");
        let info = presence::info_for(&raw).unwrap();
        assert_eq!(info.session_state, Some(SessionLoopState::Ingame));
    }

    #[test]
    fn ignores_non_presence_uri() {
        let text = serde_json::to_string(&json!([
            8, "OnJsonApiEvent_chat_v6_messages",
            { "uri": "/chat/v6/messages", "data": {} }
        ]))
        .unwrap();
        assert!(parse_presence_event(&text, "me").is_none());
    }

    #[test]
    fn returns_none_when_own_puuid_absent() {
        let text = serde_json::to_string(&json!([
            8, "OnJsonApiEvent_chat_v4_presences",
            { "uri": "/chat/v4/presences", "data": { "presences": [
                { "puuid": "other", "product": "valorant", "private": encode_private("MENUS") }
            ]}}
        ]))
        .unwrap();
        assert!(parse_presence_event(&text, "me").is_none());
    }
}
