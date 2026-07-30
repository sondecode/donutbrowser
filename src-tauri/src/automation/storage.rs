//! Persistence for automation scenario templates.
//!
//! Stateless like `GroupManager` — every call reads and writes
//! `automation_scenarios.json` under the app data dir, so there is no global
//! mutable state to initialize.

use serde::{Deserialize, Serialize};
use std::fs;

use crate::automation::scenario::Scenario;
use crate::proxy_manager::now_secs;

#[derive(Debug, Default, Serialize, Deserialize)]
struct ScenariosData {
  #[serde(default)]
  scenarios: Vec<Scenario>,
  /// Whether the shipped default template has already been written out. Tracked
  /// separately so deleting it doesn't make it reappear on the next read.
  #[serde(default)]
  seeded: bool,
}

pub struct ScenarioStore;

impl ScenarioStore {
  pub fn new() -> Self {
    Self
  }

  fn file_path(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join("automation_scenarios.json")
  }

  fn load(&self) -> Result<ScenariosData, String> {
    let path = self.file_path();
    if !path.exists() {
      return Ok(ScenariosData::default());
    }
    let content =
      fs::read_to_string(&path).map_err(|e| format!("Failed to read scenarios file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse scenarios file: {e}"))
  }

  fn save(&self, data: &ScenariosData) -> Result<(), String> {
    let path = self.file_path();
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("Failed to create data dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(data)
      .map_err(|e| format!("Failed to serialize scenarios: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write scenarios file: {e}"))
  }

  /// Load every scenario, seeding the shipped default on first ever read.
  pub fn list(&self) -> Result<Vec<Scenario>, String> {
    let mut data = self.load()?;
    if !data.seeded {
      let mut builtin = Scenario::builtin_warmup();
      builtin.updated_at = Some(now_secs());
      data.scenarios.insert(0, builtin);
      data.seeded = true;
      self.save(&data)?;
    }
    Ok(data.scenarios)
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

    let mut data = ScenariosData {
      scenarios: self.list()?,
      seeded: true,
    };

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

    data.scenarios.push(scenario.clone());
    self.save(&data)?;
    Ok(scenario)
  }

  /// Replace a scenario's contents. `id` and `built_in` are preserved.
  pub fn update(&self, id: &str, mut scenario: Scenario) -> Result<Scenario, String> {
    scenario.validate()?;

    let mut data = ScenariosData {
      scenarios: self.list()?,
      seeded: true,
    };

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

    data.scenarios[index] = scenario.clone();
    self.save(&data)?;
    Ok(scenario)
  }

  pub fn delete(&self, id: &str) -> Result<(), String> {
    let mut data = ScenariosData {
      scenarios: self.list()?,
      seeded: true,
    };

    let before = data.scenarios.len();
    data.scenarios.retain(|s| s.id != id);
    if data.scenarios.len() == before {
      return Err(
        serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
          .to_string(),
      );
    }

    self.save(&data)
  }
}

impl Default for ScenarioStore {
  fn default() -> Self {
    Self::new()
  }
}
