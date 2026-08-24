//! Riot data pipeline. All Riot-API-shape interpretation lives here; the frontend only
//! ever receives the display-ready `types::TrackerSnapshot`.

pub mod assemble;
pub mod constants;
pub mod content;
pub mod error;
pub mod lockfile;
pub mod local_api;
pub mod match_state;
pub mod names;
pub mod presence;
pub mod rank;
pub mod remote_api;
pub mod static_data;
pub mod types;
pub mod websocket;
