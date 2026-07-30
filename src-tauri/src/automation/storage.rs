//! Persistence for automation scenario templates.
//!
//! Stateless like `GroupManager` — every call reads and writes
//! `automation_scenarios.json` under the app data dir, so there is no global
//! mutable state to initialize.

use serde::{Deserialize, Serialize};
use std::fs;

use crate::automation::scenario::Scenario;
use crate::proxy_manager::now_secs;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ScenariosData {
  #[serde(default)]
  pub(crate) scenarios: Vec<Scenario>,
  /// Whether the shipped default template has already been written out. Tracked
  /// separately so deleting it doesn't make it reappear on the next read.
  #[serde(default)]
  pub(crate) seeded: bool,
  /// Unix seconds of the last meaningful collection edit. Scenario sync is a
  /// single JSON object, so this timestamp drives collection-level LWW.
  #[serde(default)]
  pub(crate) updated_at: Option<u64>,
}

impl ScenariosData {
  pub(crate) fn sync_payload(&self) -> Self {
    Self {
      scenarios: self
        .scenarios
        .iter()
        .filter(|scenario| scenario.sync_enabled)
        .cloned()
        .collect(),
      seeded: self.seeded,
      updated_at: self.updated_at,
    }
  }

  pub(crate) fn merge_remote(&mut self, mut remote: Self) {
    let disabled_ids: std::collections::HashSet<String> = self
      .scenarios
      .iter()
      .filter(|scenario| !scenario.sync_enabled)
      .map(|scenario| scenario.id.clone())
      .collect();

    for scenario in &mut remote.scenarios {
      scenario.sync_enabled = true;
    }

    self
      .scenarios
      .retain(|scenario| !scenario.sync_enabled || disabled_ids.contains(&scenario.id));
    self.scenarios.extend(
      remote
        .scenarios
        .into_iter()
        .filter(|scenario| !disabled_ids.contains(&scenario.id)),
    );
    self.seeded = self.seeded || remote.seeded;
    self.updated_at = remote.updated_at;
  }
}

pub struct ScenarioStore;

impl ScenarioStore {
  pub fn new() -> Self {
    Self
  }

  fn file_path(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join("automation_scenarios.json")
  }

  pub(crate) fn load_data(&self) -> Result<ScenariosData, String> {
    let path = self.file_path();
    if !path.exists() {
      return Ok(ScenariosData::default());
    }
    let content =
      fs::read_to_string(&path).map_err(|e| format!("Failed to read scenarios file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse scenarios file: {e}"))
  }

  pub(crate) fn save_data(&self, data: &ScenariosData) -> Result<(), String> {
    let path = self.file_path();
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("Failed to create data dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(data)
      .map_err(|e| format!("Failed to serialize scenarios: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write scenarios file: {e}"))
  }

  fn save_user_edit(&self, data: &mut ScenariosData) -> Result<(), String> {
    data.updated_at = Some(now_secs());
    self.save_data(data)?;
    let _ = crate::events::emit("automation-scenarios-changed", ());
    crate::sync::queue_automation_scenarios_sync_if_available();
    Ok(())
  }

  /// Load every scenario, seeding the shipped default on first ever read.
  pub fn list(&self) -> Result<Vec<Scenario>, String> {
    let mut data = self.load_data()?;
    if !data.seeded {
      let mut builtin = Scenario::builtin_warmup();
      builtin.updated_at = Some(now_secs());
      data.scenarios.insert(0, builtin);
      data.seeded = true;
      data.updated_at = Some(now_secs());
      self.save_data(&data)?;
    }
    Ok(data.scenarios)
  }

  pub(crate) fn sync_payload(&self) -> Result<ScenariosData, String> {
    let _ = self.list()?;
    Ok(self.load_data()?.sync_payload())
  }

  pub(crate) fn merge_from_sync(&self, remote: ScenariosData) -> Result<(), String> {
    let _ = self.list()?;
    let mut local = self.load_data()?;
    local.merge_remote(remote);
    self.save_data(&local)?;
    let _ = crate::events::emit("automation-scenarios-changed", ());
    Ok(())
  }

  pub fn get(&self, id: &str) -> Result<Scenario, String> {
    self
      .list()?
      .into_iter()
      .find(|s| s.id == id)
      .ok_or_else(|| {
        serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
          .to_string()
      })
  }

  /// Store a new scenario. The caller's `id` is ignored — a fresh UUID is always
  /// assigned so an MCP client can't collide with an existing template.
  pub fn create(&self, mut scenario: Scenario) -> Result<Scenario, String> {
    scenario.validate()?;

    let _ = self.list()?;
    let mut data = self.load_data()?;
    data.seeded = true;

    let name = scenario.name.trim().to_string();
    if data.scenarios.iter().any(|s| s.name == name) {
      return Err(
        serde_json::json!({
          "code": "AUTOMATION_SCENARIO_ALREADY_EXISTS",
          "params": { "name": name }
        })
        .to_string(),
      );
    }

    scenario.id = uuid::Uuid::new_v4().to_string();
    scenario.name = name;
    scenario.built_in = false;
    scenario.updated_at = Some(now_secs());
    scenario.sync_enabled = true;

    data.scenarios.push(scenario.clone());
    self.save_user_edit(&mut data)?;
    Ok(scenario)
  }

  /// Replace a scenario's contents. `id` and `built_in` are preserved.
  pub fn update(&self, id: &str, mut scenario: Scenario) -> Result<Scenario, String> {
    scenario.validate()?;

    let _ = self.list()?;
    let mut data = self.load_data()?;
    data.seeded = true;

    let index = data
      .scenarios
      .iter()
      .position(|s| s.id == id)
      .ok_or_else(|| {
        serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
          .to_string()
      })?;

    let name = scenario.name.trim().to_string();
    if data.scenarios.iter().any(|s| s.id != id && s.name == name) {
      return Err(
        serde_json::json!({
          "code": "AUTOMATION_SCENARIO_ALREADY_EXISTS",
          "params": { "name": name }
        })
        .to_string(),
      );
    }

    scenario.id = id.to_string();
    scenario.name = name;
    scenario.built_in = data.scenarios[index].built_in;
    scenario.updated_at = Some(now_secs());
    scenario.sync_enabled = data.scenarios[index].sync_enabled;

    data.scenarios[index] = scenario.clone();
    self.save_user_edit(&mut data)?;
    Ok(scenario)
  }

  pub fn set_sync_enabled(&self, id: &str, enabled: bool) -> Result<Scenario, String> {
    let _ = self.list()?;
    let mut data = self.load_data()?;
    data.seeded = true;

    let index = data
      .scenarios
      .iter()
      .position(|s| s.id == id)
      .ok_or_else(|| {
        serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
          .to_string()
      })?;

    data.scenarios[index].sync_enabled = enabled;
    data.scenarios[index].updated_at = Some(now_secs());
    let scenario = data.scenarios[index].clone();
    self.save_user_edit(&mut data)?;
    Ok(scenario)
  }

  pub fn delete(&self, id: &str) -> Result<(), String> {
    let _ = self.list()?;
    let mut data = self.load_data()?;
    data.seeded = true;

    let before = data.scenarios.len();
    data.scenarios.retain(|s| s.id != id);
    if data.scenarios.len() == before {
      return Err(
        serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
          .to_string(),
      );
    }

    self.save_user_edit(&mut data)
  }
}

impl Default for ScenarioStore {
  fn default() -> Self {
    Self::new()
  }
}
