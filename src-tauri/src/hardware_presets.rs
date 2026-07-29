use serde::Deserialize;
use serde_json::{Map, Value};
use std::sync::OnceLock;

static HARDWARE_PRESETS: OnceLock<Result<HardwarePresetData, String>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct HardwarePresetData {
  #[serde(rename = "presetKeys")]
  preset_keys: Vec<String>,
  presets: Vec<HardwarePreset>,
}

#[derive(Debug, Deserialize)]
pub struct HardwarePreset {
  pub id: String,
  pub os: String,
  pub config: Map<String, Value>,
}

pub fn apply_hardware_preset(
  current_fingerprint: Option<&str>,
  preset_id: &str,
) -> Result<(String, String), String> {
  let data = HARDWARE_PRESETS
    .get_or_init(load_hardware_presets)
    .as_ref()
    .map_err(|e| e.clone())?;
  let preset = data
    .presets
    .iter()
    .find(|item| item.id == preset_id)
    .ok_or_else(|| format!("Unknown hardware preset: {preset_id}"))?;

  let mut root = match current_fingerprint {
    Some(value) if !value.trim().is_empty() => {
      serde_json::from_str::<Value>(value).map_err(|e| format!("Invalid fingerprint JSON: {e}"))?
    }
    _ => Value::Object(Map::new()),
  };

  let is_wrapped = root.get("fingerprint").is_some();
  let fingerprint = if is_wrapped {
    root
      .get_mut("fingerprint")
      .ok_or_else(|| "Wrapped fingerprint is missing".to_string())?
  } else {
    &mut root
  };
  if !fingerprint.is_object() {
    *fingerprint = Value::Object(Map::new());
  }

  let object = fingerprint
    .as_object_mut()
    .ok_or_else(|| "Fingerprint must be a JSON object".to_string())?;

  for key in &data.preset_keys {
    object.remove(key);
  }
  for (key, value) in &preset.config {
    object.insert(key.clone(), value.clone());
  }

  let fingerprint_json =
    serde_json::to_string(&root).map_err(|e| format!("Failed to serialize fingerprint: {e}"))?;

  Ok((fingerprint_json, preset.os.clone()))
}

fn load_hardware_presets() -> Result<HardwarePresetData, String> {
  serde_json::from_str(include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/hardware-presets-data.json"
  )))
  .map_err(|e| format!("Failed to load hardware presets: {e}"))
}
