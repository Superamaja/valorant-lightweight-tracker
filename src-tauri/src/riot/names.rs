//! Batch name-service resolution parsing. See spec §6.

use crate::riot::error::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct NameEntry {
    #[serde(rename = "Subject", default)]
    subject: String,
    #[serde(rename = "GameName", default)]
    game_name: String,
    #[serde(rename = "TagLine", default)]
    tag_line: String,
}

/// Parse the name-service response into puuid -> "GameName#TagLine".
///
/// A player whose GameName and TagLine both come back blank is left OUT of the map
/// (unresolved — the caller renders a placeholder). An `errorCode` object instead of an
/// array signals token expiry: returns `BadClaims` so the caller refreshes + retries once.
pub fn parse_name_response(value: &Value) -> Result<HashMap<String, String>> {
    if value.get("errorCode").is_some() {
        return Err(Error::BadClaims);
    }
    let entries: Vec<NameEntry> = serde_json::from_value(value.clone())?;
    let mut out = HashMap::new();
    for e in entries {
        if e.subject.is_empty() {
            continue;
        }
        if e.game_name.is_empty() && e.tag_line.is_empty() {
            continue; // withheld / unresolved
        }
        out.insert(e.subject, format!("{}#{}", e.game_name, e.tag_line));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_names() {
        let resp = json!([
            { "Subject": "a", "GameName": "Foo", "TagLine": "1234", "PUUID": "a" },
            { "Subject": "b", "GameName": "Bar", "TagLine": "EU", "PUUID": "b" }
        ]);
        let map = parse_name_response(&resp).unwrap();
        assert_eq!(map.get("a").unwrap(), "Foo#1234");
        assert_eq!(map.get("b").unwrap(), "Bar#EU");
    }

    #[test]
    fn skips_blank_names() {
        let resp = json!([{ "Subject": "a", "GameName": "", "TagLine": "" }]);
        let map = parse_name_response(&resp).unwrap();
        assert!(!map.contains_key("a"));
    }

    #[test]
    fn error_code_signals_bad_claims() {
        let resp = json!({ "errorCode": "BAD_CLAIMS", "message": "..." });
        assert!(matches!(parse_name_response(&resp), Err(Error::BadClaims)));
    }
}
