//! Profile automation: declarative scenario templates executed against
//! Chromium/Wayfern profiles over CDP.
//!
//! - `scenario` — the template schema (the contract MCP clients author against)
//! - `storage` — scenario persistence
//! - `engine` — batch execution, concurrency, run state
//! - `cdp` — the CDP transport, shared with the MCP server

pub mod cdp;
pub mod engine;
pub mod scenario;
pub mod storage;

use engine::{AutomationRun, RunRequest};
use scenario::Scenario;
use storage::ScenarioStore;

#[tauri::command]
pub async fn list_automation_scenarios() -> Result<Vec<Scenario>, String> {
  ScenarioStore::new().list()
}

#[tauri::command]
pub async fn get_automation_scenario(scenario_id: String) -> Result<Scenario, String> {
  ScenarioStore::new().get(&scenario_id)
}

#[tauri::command]
pub async fn create_automation_scenario(scenario: Scenario) -> Result<Scenario, String> {
  ScenarioStore::new().create(scenario)
}

#[tauri::command]
pub async fn update_automation_scenario(
  scenario_id: String,
  scenario: Scenario,
) -> Result<Scenario, String> {
  ScenarioStore::new().update(&scenario_id, scenario)
}

#[tauri::command]
pub async fn delete_automation_scenario(scenario_id: String) -> Result<(), String> {
  ScenarioStore::new().delete(&scenario_id)
}

/// Machine-readable step reference, so the UI and MCP clients describe the same
/// template format without either hardcoding a second copy of it.
#[tauri::command]
pub async fn get_automation_step_schema() -> Result<serde_json::Value, String> {
  Ok(scenario::step_schema())
}

#[tauri::command]
pub async fn start_automation_run(
  app_handle: tauri::AppHandle,
  request: RunRequest,
) -> Result<AutomationRun, String> {
  engine::start_run(app_handle, request).await
}

/// Runs are also delivered live via the `automation-run-updated` event; this is
/// how a freshly mounted UI catches up on runs already in flight.
#[tauri::command]
pub async fn list_automation_runs() -> Result<Vec<AutomationRun>, String> {
  Ok(engine::list_runs())
}

#[tauri::command]
pub async fn cancel_automation_run(run_id: String) -> Result<(), String> {
  engine::cancel_run(&run_id)
}
