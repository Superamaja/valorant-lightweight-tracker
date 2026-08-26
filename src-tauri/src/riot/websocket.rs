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

/// A presence event the session loop must react to: our own Valorant presence changed, so a
/// state transition is possible. Carries nothing — the session re-polls presence over REST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poke;

/// Classify a raw websocket text frame: `Some(Poke)` when a `/chat/v4/presences` event carries
/// the local player's Valorant presence, `None` otherwise. Pure — testable.
///
/// Another player's presence is deliberately NOT a poke. It cannot be filtered down to our
/// lobby here (the roster isn't known), so every online friend's event would otherwise drive a
/// pregame rebuild — unbounded work on top of the 1 s agent-select poll, which already detects
/// everything such an event could signal. Outside agent select they never mattered at all.
///
/// Message shape: `[opcode, eventName, { uri, data: { presences: [...] } }]`.
/// League presence entries are skipped.
pub fn parse_presence_event(text: &str, own_puuid: &str) -> Option<Poke> {
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
        if raw.is_valorant() && raw.puuid == own_puuid {
            return Some(Poke);
        }
    }
    None
}

/// Connect to the local websocket, subscribe, and poke the caller once per own-presence event
/// until the connection drops. The channel carries a bare `Poke` — the session re-polls full
/// presence over REST on every poke, so there's no reason to ship the presence struct across
/// the channel.
///
/// Returns `Ok(())` once a connection was established and later closed (so the caller can
/// reset its reconnect backoff — C8), and `Err` only when the connection could not be
/// established at all.
pub async fn run_listener(
    lockfile: &Lockfile,
    own_puuid: &str,
    tx: mpsc::Sender<Poke>,
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
    vlt_log!("ws", "connected + subscribed");

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Some(poke) = parse_presence_event(&text, own_puuid) {
                    if tx.send(poke).await.is_err() {
                        break; // receiver gone -> stop
                    }
                }
            }
            Ok(Message::Close(_)) => {
                vlt_log!("ws", "closed (server close)");
                break;
            }
            Ok(_) => {}
            // Mid-stream error: the connection was established, so treat this like a
            // normal close (Ok) — the caller reconnects and resets its backoff (C8).
            Err(_) => {
                vlt_log!("ws", "closed (stream error)");
                break;
            }
        }
    }
    // Reached only after a successful connect; a clean/aborted close returns Ok so the
    // caller knows the connection had come up (backoff reset).
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn encode_private(state: &str) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&json!({ "sessionLoopState": state })).unwrap())
    }

    /// Wrap presence entries in the websocket frame shape.
    fn frame(presences: Value) -> String {
        serde_json::to_string(&json!([
            8,
            "OnJsonApiEvent_chat_v4_presences",
            { "uri": "/chat/v4/presences", "data": { "presences": presences }}
        ]))
        .unwrap()
    }

    #[test]
    fn own_presence_in_event_is_a_poke() {
        let text = frame(json!([
            { "puuid": "other", "product": "valorant", "private": encode_private("MENUS") },
            { "puuid": "me", "product": "valorant", "private": encode_private("INGAME") }
        ]));
        assert_eq!(parse_presence_event(&text, "me"), Some(Poke));
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
    fn other_players_presence_is_ignored() {
        // A friend anywhere in the client, or a teammate in agent select: neither can be told
        // apart from the other here, and the 1 s pregame poll already covers the picks.
        for state in ["MENUS", "PREGAME", "INGAME"] {
            let text = frame(json!([
                { "puuid": "other", "product": "valorant", "private": encode_private(state) }
            ]));
            assert_eq!(parse_presence_event(&text, "me"), None);
        }
    }

    #[test]
    fn ignores_non_valorant_products() {
        // Our own League presence must not be mistaken for a Valorant state change.
        let text = frame(json!([
            { "puuid": "other", "product": "league_of_legends", "championId": 157 },
            { "puuid": "me", "product": "league_of_legends", "championId": 12 }
        ]));
        assert!(parse_presence_event(&text, "me").is_none());
    }

    #[test]
    fn malformed_entry_does_not_hide_a_valid_one() {
        // A single unparseable entry is skipped, not fatal (C6).
        let text = frame(json!([
            { "product": "valorant" }, // no puuid -> undeserializable
            { "puuid": "me", "product": "valorant", "private": encode_private("PREGAME") }
        ]));
        assert_eq!(parse_presence_event(&text, "me"), Some(Poke));
    }

    #[test]
    fn malformed_frame_is_none() {
        assert!(parse_presence_event("not json", "me").is_none());
        assert!(parse_presence_event("{}", "me").is_none());
        assert!(parse_presence_event("[8]", "me").is_none());
    }
}
