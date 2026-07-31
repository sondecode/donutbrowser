//! Persistence for automation scenario templates.
//!
//! Stateless like `GroupManager` — every call reads and writes the
//! `automation/` directory under the app data dir, so there is no global
//! mutable state to initialize.
//!
//! One file per scenario (`automation/{id}.json`), mirroring how proxies and
//! groups are stored. The previous single `automation_scenarios.json` blob made
//! every edit re-upload the whole collection and gave two devices editing
//! different scenarios a last-write-wins fight over one object; per-scenario
//! files give each one its own `updated_at`, its own remote key and its own
//! tombstone. `migrate_legacy_file` moves the old layout across on first read.

use serde::{Deserialize, Serialize};
use std::fs;

use crate::automation::scenario::Scenario;
use crate::proxy_manager::now_secs;

/// The pre-split on-disk shape, still parsed so existing installs migrate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ScenariosData {
  #[serde(default)]
  pub(crate) scenarios: Vec<Scenario>,
  #[serde(default)]
  pub(crate) seeded: bool,
  #[serde(default)]
  pub(crate) updated_at: Option<u64>,
}

/// Directory-level bookkeeping that isn't a scenario.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreState {
  /// Whether the shipped default template has already been written out. Tracked
  /// separately so deleting it doesn't make it reappear on the next read.
  #[serde(default)]
  seeded: bool,
}

const STATE_FILE: &str = "state.json";

pub struct ScenarioStore;

impl ScenarioStore {
  pub fn new() -> Self {
    Self
  }

  pub(crate) fn dir(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join("automation")
  }

  fn scenario_path(&self, id: &str) -> std::path::PathBuf {
    self.dir().join(format!("{id}.json"))
  }

  fn state_path(&self) -> std::path::PathBuf {
    self.dir().join(STATE_FILE)
  }

  fn legacy_path(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join("automation_scenarios.json")
  }

  fn ensure_dir(&self) -> Result<std::path::PathBuf, String> {
    let dir = self.dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create automation dir: {e}"))?;
    Ok(dir)
  }

  fn load_state(&self) -> StoreState {
    fs::read_to_string(self.state_path())
      .ok()
      .and_then(|content| serde_json::from_str(&content).ok())
      .unwrap_or_default()
  }

  fn save_state(&self, state: &StoreState) -> Result<(), String> {
    self.ensure_dir()?;
    let json = serde_json::to_string_pretty(state)
      .map_err(|e| format!("Failed to serialize automation state: {e}"))?;
    fs::write(self.state_path(), json).map_err(|e| format!("Failed to write automation state: {e}"))
  }

  /// Split the pre-split blob into per-scenario files, once. The old file is
  /// renamed rather than deleted so a bad migration stays recoverable by hand.
  fn migrate_legacy_file(&self) -> Result<(), String> {
    let legacy = self.legacy_path();
    if !legacy.exists() {
      return Ok(());
    }

    let content =
      fs::read_to_string(&legacy).map_err(|e| format!("Failed to read scenarios file: {e}"))?;
    let data: ScenariosData =
      serde_json::from_str(&content).map_err(|e| format!("Failed to parse scenarios file: {e}"))?;

    self.ensure_dir()?;
    for scenario in &data.scenarios {
      // Never clobber a per-scenario file that already exists: it is newer than
      // the blob by definition.
      let path = self.scenario_path(&scenario.id);
      if !path.exists() {
        self.write_scenario(scenario)?;
      }
    }
    self.save_state(&StoreState {
      seeded: data.seeded,
    })?;

    let retired = legacy.with_extension("json.migrated");
    fs::rename(&legacy, &retired)
      .map_err(|e| format!("Failed to retire legacy scenarios file: {e}"))?;
    log::info!(
      "Migrated {} automation scenario(s) into {}",
      data.scenarios.len(),
      self.dir().display()
    );
    Ok(())
  }

  pub(crate) fn write_scenario(&self, scenario: &Scenario) -> Result<(), String> {
    self.ensure_dir()?;
    let json = serde_json::to_string_pretty(scenario)
      .map_err(|e| format!("Failed to serialize scenario: {e}"))?;
    fs::write(self.scenario_path(&scenario.id), json)
      .map_err(|e| format!("Failed to write scenario file: {e}"))
  }

  /// Every scenario on disk, unsorted, with no seeding side effect.
  fn read_all(&self) -> Result<Vec<Scenario>, String> {
    let dir = self.dir();
    if !dir.exists() {
      return Ok(Vec::new());
    }

    let mut scenarios = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("Failed to read automation dir: {e}"))? {
      let entry = entry.map_err(|e| format!("Failed to read automation dir entry: {e}"))?;
      let path = entry.path();
      if path.extension().is_none_or(|ext| ext != "json") {
        continue;
      }
      if path.file_name().is_some_and(|name| name == STATE_FILE) {
        continue;
      }
      match fs::read_to_string(&path).map(|c| serde_json::from_str::<Scenario>(&c)) {
        Ok(Ok(scenario)) => scenarios.push(scenario),
        Ok(Err(e)) => log::warn!("Skipping unparsable scenario file {}: {e}", path.display()),
        Err(e) => log::warn!("Skipping unreadable scenario file {}: {e}", path.display()),
      }
    }
    Ok(scenarios)
  }

  fn after_user_edit(&self, scenario_id: &str) {
    let _ = crate::events::emit("automation-scenarios-changed", ());
    crate::sync::queue_automation_scenario_sync_if_available(scenario_id.to_string());
  }

  /// Load every scenario, seeding the shipped default on first ever read.
  pub fn list(&self) -> Result<Vec<Scenario>, String> {
    self.migrate_legacy_file()?;

    let mut scenarios = self.read_all()?;
    let mut state = self.load_state();
    if !state.seeded {
      let mut builtin = Scenario::builtin_warmup();
      builtin.updated_at = Some(now_secs());
      self.write_scenario(&builtin)?;
      scenarios.insert(0, builtin);
      state.seeded = true;
      self.save_state(&state)?;
    }

    // Newest edit first, with a name tiebreak so the order is stable across
    // machines rather than dependent on directory iteration.
    scenarios.sort_by(|a, b| {
      b.updated_at
        .unwrap_or(0)
        .cmp(&a.updated_at.unwrap_or(0))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(scenarios)
  }

  /// Read one scenario straight off disk, skipping the seeding pass.
  pub(crate) fn read(&self, id: &str) -> Result<Option<Scenario>, String> {
    let path = self.scenario_path(id);
    if !path.exists() {
      return Ok(None);
    }
    let content =
      fs::read_to_string(&path).map_err(|e| format!("Failed to read scenario file: {e}"))?;
    serde_json::from_str(&content)
      .map(Some)
      .map_err(|e| format!("Failed to parse scenario file: {e}"))
  }

  pub(crate) fn remove_file(&self, id: &str) -> Result<(), String> {
    let path = self.scenario_path(id);
    if path.exists() {
      fs::remove_file(&path).map_err(|e| format!("Failed to delete scenario file: {e}"))?;
    }
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

    let existing = self.list()?;
    let name = scenario.name.trim().to_string();
    if existing.iter().any(|s| s.name == name) {
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
    scenario.last_sync = None;

    self.write_scenario(&scenario)?;
    self.after_user_edit(&scenario.id);
    Ok(scenario)
  }

  /// Replace a scenario's contents. `id` and `built_in` are preserved.
  pub fn update(&self, id: &str, mut scenario: Scenario) -> Result<Scenario, String> {
    scenario.validate()?;

    let existing = self.list()?;
    let current = existing.iter().find(|s| s.id == id).ok_or_else(|| {
      serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
        .to_string()
    })?;

    let name = scenario.name.trim().to_string();
    if existing.iter().any(|s| s.id != id && s.name == name) {
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
    scenario.built_in = current.built_in;
    scenario.updated_at = Some(now_secs());
    scenario.sync_enabled = current.sync_enabled;
    scenario.last_sync = current.last_sync;

    self.write_scenario(&scenario)?;
    self.after_user_edit(id);
    Ok(scenario)
  }

  pub fn set_sync_enabled(&self, id: &str, enabled: bool) -> Result<Scenario, String> {
    let mut scenario = self.get(id)?;
    scenario.sync_enabled = enabled;
    scenario.updated_at = Some(now_secs());

    self.write_scenario(&scenario)?;
    self.after_user_edit(id);
    Ok(scenario)
  }

  /// Record an upload or download. Deliberately does NOT touch `updated_at`:
  /// sync bookkeeping must never look like a user edit, or the next reconcile
  /// would see this side as newer and push it straight back.
  pub(crate) fn mark_synced(&self, id: &str, at: u64) -> Result<(), String> {
    let Some(mut scenario) = self.read(id)? else {
      return Ok(());
    };
    scenario.last_sync = Some(at);
    self.write_scenario(&scenario)
  }

  pub fn delete(&self, app_handle: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let existing = self.list()?;
    let scenario = existing.iter().find(|s| s.id == id).ok_or_else(|| {
      serde_json::json!({ "code": "AUTOMATION_SCENARIO_NOT_FOUND", "params": { "id": id } })
        .to_string()
    })?;
    let was_synced = scenario.sync_enabled;

    self.remove_file(id)?;
    let _ = crate::events::emit("automation-scenarios-changed", ());

    // Without a tombstone the next reconcile sees the scenario missing locally
    // but present remotely and downloads it straight back.
    if was_synced {
      let id = id.to_string();
      let app_handle = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        match crate::sync::SyncEngine::create_from_settings(&app_handle).await {
          Ok(engine) => {
            if let Err(e) = engine.delete_automation_scenario(&id).await {
              log::warn!("Failed to delete scenario {id} from sync: {e}");
            }
          }
          Err(e) => log::debug!("Sync not configured, skipping remote deletion: {e}"),
        }
      });
    }
    Ok(())
  }
}

impl Default for ScenarioStore {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::automation::scenario::{Scenario, Step};

  fn scenario(id: &str, name: &str) -> Scenario {
    Scenario {
      id: id.to_string(),
      name: name.to_string(),
      description: None,
      variables: Vec::new(),
      steps: vec![Step::CloseProfile],
      built_in: false,
      updated_at: Some(1_000),
      sync_enabled: true,
      last_sync: None,
    }
  }

  fn legacy_blob(store: &ScenarioStore, scenarios: Vec<Scenario>, seeded: bool) {
    fs::create_dir_all(crate::app_dirs::data_subdir()).unwrap();
    let data = ScenariosData {
      scenarios,
      seeded,
      updated_at: Some(2_000),
    };
    fs::write(
      store.legacy_path(),
      serde_json::to_string_pretty(&data).unwrap(),
    )
    .unwrap();
  }

  #[test]
  fn legacy_blob_becomes_one_file_per_scenario() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());
    let store = ScenarioStore::new();
    legacy_blob(
      &store,
      vec![scenario("a", "Alpha"), scenario("b", "Beta")],
      true,
    );

    let listed = store.list().unwrap();

    assert_eq!(listed.len(), 2);
    assert!(store.dir().join("a.json").exists());
    assert!(store.dir().join("b.json").exists());
    // The old blob is retired, not deleted, so a bad migration stays recoverable.
    assert!(!store.legacy_path().exists());
    assert!(crate::app_dirs::data_subdir()
      .join("automation_scenarios.json.migrated")
      .exists());
  }

  #[test]
  fn migration_preserves_a_deleted_builtin() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());
    let store = ScenarioStore::new();
    // seeded=true with no scenarios means the user deleted the shipped template.
    legacy_blob(&store, Vec::new(), true);

    assert!(store.list().unwrap().is_empty());
  }

  #[test]
  fn migration_never_clobbers_an_existing_per_scenario_file() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());
    let store = ScenarioStore::new();

    let mut newer = scenario("a", "Newer");
    newer.updated_at = Some(9_999);
    store.write_scenario(&newer).unwrap();
    legacy_blob(&store, vec![scenario("a", "Stale")], true);

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Newer");
  }

  #[test]
  fn mark_synced_records_the_backup_without_faking_an_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());
    let store = ScenarioStore::new();
    store.write_scenario(&scenario("a", "Alpha")).unwrap();

    store.mark_synced("a", 5_000).unwrap();

    let stored = store.read("a").unwrap().unwrap();
    assert_eq!(stored.last_sync, Some(5_000));
    // Bumping updated_at here would make the next reconcile think this side is
    // newer and push it straight back — the ping-pong bug.
    assert_eq!(stored.updated_at, Some(1_000));
  }

  #[test]
  fn state_file_is_not_mistaken_for_a_scenario() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());
    let store = ScenarioStore::new();

    // First read seeds the builtin and writes state.json alongside it.
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.dir().join(STATE_FILE).exists());
    assert_eq!(store.list().unwrap().len(), 1);
  }
}
