mod monitor;
mod services;
mod startup;

use monitor::{ProcessDetail, Sampler, Snapshot};
use services::{ServiceActionResult, ServiceView};
use startup::StartupEntry;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

pub struct AppState {
    snapshot: Mutex<Option<Snapshot>>,
}

#[tauri::command]
fn get_snapshot(state: tauri::State<AppState>) -> Option<Snapshot> {
    state.snapshot.lock().ok()?.clone()
}

#[tauri::command]
fn kill_process(pid: u32, force: bool) -> Result<bool, String> {
    monitor::kill_process(pid, force)
}

#[tauri::command]
fn kill_process_tree(pid: u32, force: bool) -> Result<usize, String> {
    monitor::kill_process_tree(pid, force)
}

#[tauri::command]
fn get_process_detail(pid: u32) -> Result<ProcessDetail, String> {
    monitor::process_detail(pid)
}

#[tauri::command]
fn get_startup() -> Vec<StartupEntry> {
    startup::list_startup()
}

#[tauri::command]
fn set_startup(id: String, enabled: bool) -> Result<StartupEntry, String> {
    startup::set_startup(&id, enabled)
}

#[tauri::command]
fn get_services(scope: String) -> Result<Vec<ServiceView>, String> {
    services::list_services(&scope)
}

#[tauri::command]
fn service_action(
    scope: String,
    unit: String,
    action: String,
) -> Result<ServiceActionResult, String> {
    services::service_action(&scope, &unit, &action)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState {
        snapshot: Mutex::new(None),
    };
    // Sampler thread: refresh every second, independent of the UI poll.
    // The frontend reads the latest cached snapshot, so a slow render
    // never blocks sampling and sampling never blocks a kill click.
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut sampler = Sampler::new();
                loop {
                    let snap = sampler.sample();
                    if let Some(state) = handle.try_state::<AppState>() {
                        if let Ok(mut guard) = state.snapshot.lock() {
                            *guard = Some(snap);
                        }
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            kill_process,
            kill_process_tree,
            get_process_detail,
            get_startup,
            set_startup,
            get_services,
            service_action
        ])
        .run(tauri::generate_context!())
        .expect("error while running TaskMon");
}
