//! Tauri entry point: manages the tracker state, exposes the two commands, and emits the
//! `tracker-state` event. All Riot logic lives under `riot`; orchestration in `app_state`.

#[macro_use]
mod debug_log;

mod app_state;
mod riot;

use app_state::{tracker_main, Emitter, TrackerState};
use riot::types::TrackerSnapshot;
use std::sync::Arc;
use tauri::{AppHandle, Emitter as _, State};

/// The single event channel to the frontend.
const TRACKER_EVENT: &str = "tracker-state";

/// Emits snapshots to the frontend as the `tracker-state` event.
struct TauriEmitter {
    app: AppHandle,
}

impl Emitter for TauriEmitter {
    fn emit(&self, snapshot: &TrackerSnapshot) {
        let _ = self.app.emit(TRACKER_EVENT, snapshot);
    }
}

/// Return the current snapshot on demand.
#[tauri::command]
fn get_tracker_state(state: State<'_, Arc<TrackerState>>) -> TrackerSnapshot {
    state.snapshot()
}

/// Start the background tracker loop. Idempotent — repeated calls are no-ops.
#[tauri::command]
fn start_tracker(app: AppHandle, state: State<'_, Arc<TrackerState>>) {
    if state.begin() {
        let state = Arc::clone(&state);
        let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter { app });
        tauri::async_runtime::spawn(async move {
            tracker_main(state, emitter).await;
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(TrackerState::default()))
        .invoke_handler(tauri::generate_handler![get_tracker_state, start_tracker])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
