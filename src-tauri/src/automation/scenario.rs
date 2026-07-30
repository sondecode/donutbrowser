//! Declarative automation scenario templates.
//!
//! A scenario is a plain list of steps plus a list of declared variables. It is
//! stored as JSON and is the only contract the executor understands, so an MCP
//! client can author or rewrite a whole workflow by sending JSON — no code
//! changes needed. `step_schema()` returns a machine-readable description of
//! every step kind for exactly that purpose.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Hard cap on a single dwell so a malformed template can't pin a profile open
/// for days.
pub const MAX_DWELL_SECS: u64 = 6 * 60 * 60;
/// Hard cap on how many profiles may run at once.
pub const MAX_CONCURRENCY: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ScrollDirection {
  #[default]
  Down,
  Up,
  Random,
}

/// One unit of work. Untagged fields are optional so older stored scenarios keep
/// deserializing when a new knob is added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
  /// Point the current tab at a URL. Supports `{{variable}}` placeholders.
  Navigate {
    url: String,
    #[serde(default = "default_true")]
    wait_for_load: bool,
    #[serde(default = "default_load_timeout")]
    timeout_secs: u64,
  },
  /// Block until `document.readyState === "complete"`, or the timeout elapses.
  WaitForLoad {
    #[serde(default = "default_load_timeout")]
    timeout_secs: u64,
  },
  /// Scroll the page in randomized increments with randomized pauses, so the
  /// wheel event cadence doesn't look mechanical.
  Scroll {
    #[serde(default)]
    direction: ScrollDirection,
    #[serde(default = "default_scroll_min")]
    min_steps: u32,
    #[serde(default = "default_scroll_max")]
    max_steps: u32,
  },
  /// Pick a random in-page link and click it.
  ClickRandomLink {
    /// Restrict candidates to links whose hostname matches the current page's.
    #[serde(default = "default_true")]
    same_domain_only: bool,
    /// Substrings that disqualify a candidate href (login, logout, cart, …).
    #[serde(default)]
    exclude_patterns: Vec<String>,
    #[serde(default = "default_true")]
    wait_for_load: bool,
    #[serde(default = "default_load_timeout")]
    timeout_secs: u64,
  },
  /// Stay on the page for a randomized duration in `[min_secs, max_secs]`.
  Dwell { min_secs: u64, max_secs: u64 },
  /// Capture the viewport (or the full page) to a PNG under the run's folder.
  Screenshot {
    #[serde(default)]
    full_page: bool,
  },
  /// Terminate the browser. Implicit at the end of every run, so this step only
  /// matters when a scenario needs to close early.
  CloseProfile,
}

fn default_true() -> bool {
  true
}
fn default_load_timeout() -> u64 {
  30
}
fn default_scroll_min() -> u32 {
  3
}
fn default_scroll_max() -> u32 {
  8
}

impl Step {
  /// Stable identifier used for progress reporting and translation lookup.
  pub fn kind(&self) -> &'static str {
    match self {
      Step::Navigate { .. } => "navigate",
      Step::WaitForLoad { .. } => "wait_for_load",
      Step::Scroll { .. } => "scroll",
      Step::ClickRandomLink { .. } => "click_random_link",
      Step::Dwell { .. } => "dwell",
      Step::Screenshot { .. } => "screenshot",
      Step::CloseProfile => "close_profile",
    }
  }
}

/// A variable the scenario expects at run time. The UI renders one input per
/// entry; MCP clients read this to know what `variables` to pass to a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDef {
  pub name: String,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub variables: Vec<VariableDef>,
  pub steps: Vec<Step>,
  /// True for the template the app seeds on first run. Provenance only — it is
  /// as editable and deletable as any other scenario; the flag just lets the UI
  /// label it.
  #[serde(default)]
  pub built_in: bool,
  /// Unix seconds of the last meaningful user edit. Source of truth for sync
  /// conflict resolution (last-write-wins); bumped on edits only.
  #[serde(default)]
  pub updated_at: Option<u64>,
}

impl Scenario {
  /// The workflow this feature shipped with: open a site, read it like a person
  /// would, wander two links deep, then capture and close.
  pub fn builtin_warmup() -> Self {
    Self {
      id: "builtin-warmup".to_string(),
      name: "Website warmup".to_string(),
      description: Some(
        "Open a site, scroll, follow two random links with long dwells, screenshot, then close."
          .to_string(),
      ),
      variables: vec![VariableDef {
        name: "start_url".to_string(),
        description: Some("The first page to open".to_string()),
        default: Some("https://www.wikipedia.org".to_string()),
      }],
      steps: vec![
        Step::Navigate {
          url: "{{start_url}}".to_string(),
          wait_for_load: true,
          timeout_secs: 30,
        },
        Step::Scroll {
          direction: ScrollDirection::Down,
          min_steps: 3,
          max_steps: 8,
        },
        Step::ClickRandomLink {
          same_domain_only: true,
          exclude_patterns: default_exclude_patterns(),
          wait_for_load: true,
          timeout_secs: 30,
        },
        Step::Dwell {
          min_secs: 120,
          max_secs: 120,
        },
        Step::Scroll {
          direction: ScrollDirection::Down,
          min_steps: 2,
          max_steps: 6,
        },
        Step::ClickRandomLink {
          same_domain_only: true,
          exclude_patterns: default_exclude_patterns(),
          wait_for_load: true,
          timeout_secs: 30,
        },
        Step::Dwell {
          min_secs: 300,
          max_secs: 300,
        },
        Step::Screenshot { full_page: false },
        Step::CloseProfile,
      ],
      built_in: true,
      updated_at: None,
    }
  }

  /// Reject templates the executor can't run, before anything is launched.
  pub fn validate(&self) -> Result<(), String> {
    if self.name.trim().is_empty() {
      return Err(serde_json::json!({ "code": "NAME_CANNOT_BE_EMPTY" }).to_string());
    }
    if self.steps.is_empty() {
      return Err(serde_json::json!({ "code": "AUTOMATION_SCENARIO_EMPTY" }).to_string());
    }

    for (index, step) in self.steps.iter().enumerate() {
      let position = (index + 1).to_string();
      match step {
        Step::Navigate { url, .. } => {
          if url.trim().is_empty() {
            return Err(
              serde_json::json!({
                "code": "AUTOMATION_STEP_URL_REQUIRED",
                "params": { "step": position }
              })
              .to_string(),
            );
          }
        }
        Step::Scroll {
          min_steps,
          max_steps,
          ..
        } => {
          if min_steps > max_steps {
            return Err(
              serde_json::json!({
                "code": "AUTOMATION_STEP_RANGE_INVALID",
                "params": { "step": position }
              })
              .to_string(),
            );
          }
        }
        Step::Dwell { min_secs, max_secs } => {
          if min_secs > max_secs {
            return Err(
              serde_json::json!({
                "code": "AUTOMATION_STEP_RANGE_INVALID",
                "params": { "step": position }
              })
              .to_string(),
            );
          }
          if *max_secs > MAX_DWELL_SECS {
            return Err(
              serde_json::json!({
                "code": "AUTOMATION_STEP_DWELL_TOO_LONG",
                "params": { "step": position, "max": (MAX_DWELL_SECS / 3600).to_string() }
              })
              .to_string(),
            );
          }
        }
        Step::WaitForLoad { .. } | Step::ClickRandomLink { .. } => {}
        Step::Screenshot { .. } | Step::CloseProfile => {}
      }
    }

    Ok(())
  }

  /// Variable values to use for a run: declared defaults overridden by the
  /// caller's map.
  pub fn resolve_variables(&self, overrides: &HashMap<String, String>) -> HashMap<String, String> {
    let mut resolved: HashMap<String, String> = self
      .variables
      .iter()
      .filter_map(|v| v.default.clone().map(|d| (v.name.clone(), d)))
      .collect();
    for (key, value) in overrides {
      resolved.insert(key.clone(), value.clone());
    }
    resolved
  }
}

fn default_exclude_patterns() -> Vec<String> {
  [
    "login",
    "signin",
    "sign-in",
    "signup",
    "sign-up",
    "logout",
    "signout",
    "register",
    "checkout",
    "cart",
    "account",
    "subscribe",
    "donate",
    "mailto:",
    "tel:",
  ]
  .iter()
  .map(|s| s.to_string())
  .collect()
}

/// Replace every `{{name}}` occurrence with its value. Unknown placeholders are
/// left untouched so a typo surfaces as a visibly broken URL rather than a
/// silently empty one.
pub fn substitute(input: &str, variables: &HashMap<String, String>) -> String {
  let mut output = input.to_string();
  for (name, value) in variables {
    output = output.replace(&format!("{{{{{name}}}}}"), value);
  }
  output
}

/// Machine-readable description of every step kind, returned by the
/// `get_automation_step_schema` MCP tool so a client can generate valid
/// scenarios without guessing field names.
pub fn step_schema() -> serde_json::Value {
  serde_json::json!({
    "scenario": {
      "name": "string (required)",
      "description": "string (optional)",
      "variables": "array of { name, description?, default? } — referenced in step strings as {{name}}",
      "steps": "array of step objects, executed in order",
    },
    "notes": [
      "Every step object needs a \"type\" field naming its kind.",
      "The browser is launched before the first step and killed after the last one, so no explicit launch step exists.",
      "Automation only works on Chromium/Wayfern profiles — Firefox-based profiles have no CDP endpoint.",
      format!("dwell max_secs is capped at {MAX_DWELL_SECS} seconds."),
    ],
    "steps": [
      {
        "type": "navigate",
        "fields": {
          "url": "string (required) — supports {{variable}} placeholders",
          "wait_for_load": "bool (default true)",
          "timeout_secs": "number (default 30)",
        },
      },
      {
        "type": "wait_for_load",
        "fields": { "timeout_secs": "number (default 30)" },
      },
      {
        "type": "scroll",
        "fields": {
          "direction": "\"down\" | \"up\" | \"random\" (default \"down\")",
          "min_steps": "number (default 3)",
          "max_steps": "number (default 8) — actual count is random in [min, max]",
        },
      },
      {
        "type": "click_random_link",
        "fields": {
          "same_domain_only": "bool (default true)",
          "exclude_patterns": "array of substrings that disqualify an href",
          "wait_for_load": "bool (default true)",
          "timeout_secs": "number (default 30)",
        },
      },
      {
        "type": "dwell",
        "fields": {
          "min_secs": "number (required)",
          "max_secs": "number (required) — actual wait is random in [min, max]; use equal values for a fixed wait",
        },
      },
      {
        "type": "screenshot",
        "fields": { "full_page": "bool (default false)" },
      },
      {
        "type": "close_profile",
        "fields": {},
      },
    ],
    "example": Scenario::builtin_warmup(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn builtin_warmup_is_valid() {
    Scenario::builtin_warmup().validate().unwrap();
  }

  #[test]
  fn builtin_warmup_matches_the_documented_flow() {
    let kinds: Vec<&str> = Scenario::builtin_warmup()
      .steps
      .iter()
      .map(|s| s.kind())
      .collect();
    assert_eq!(
      kinds,
      vec![
        "navigate",
        "scroll",
        "click_random_link",
        "dwell",
        "scroll",
        "click_random_link",
        "dwell",
        "screenshot",
        "close_profile",
      ]
    );
  }

  #[test]
  fn builtin_warmup_dwells_two_then_five_minutes() {
    let dwells: Vec<u64> = Scenario::builtin_warmup()
      .steps
      .iter()
      .filter_map(|s| match s {
        Step::Dwell { min_secs, .. } => Some(*min_secs),
        _ => None,
      })
      .collect();
    assert_eq!(dwells, vec![120, 300]);
  }

  #[test]
  fn empty_steps_rejected() {
    let mut scenario = Scenario::builtin_warmup();
    scenario.steps.clear();
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("AUTOMATION_SCENARIO_EMPTY"));
  }

  #[test]
  fn blank_name_rejected() {
    let mut scenario = Scenario::builtin_warmup();
    scenario.name = "   ".to_string();
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("NAME_CANNOT_BE_EMPTY"));
  }

  #[test]
  fn inverted_dwell_range_rejected() {
    let mut scenario = Scenario::builtin_warmup();
    scenario.steps = vec![Step::Dwell {
      min_secs: 60,
      max_secs: 10,
    }];
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("AUTOMATION_STEP_RANGE_INVALID"));
  }

  #[test]
  fn overlong_dwell_rejected() {
    let mut scenario = Scenario::builtin_warmup();
    scenario.steps = vec![Step::Dwell {
      min_secs: 0,
      max_secs: MAX_DWELL_SECS + 1,
    }];
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("AUTOMATION_STEP_DWELL_TOO_LONG"));
  }

  #[test]
  fn empty_navigate_url_rejected() {
    let mut scenario = Scenario::builtin_warmup();
    scenario.steps = vec![Step::Navigate {
      url: "  ".to_string(),
      wait_for_load: true,
      timeout_secs: 30,
    }];
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("AUTOMATION_STEP_URL_REQUIRED"));
  }

  #[test]
  fn variables_substitute_and_override_defaults() {
    let scenario = Scenario::builtin_warmup();
    let resolved = scenario.resolve_variables(&HashMap::new());
    assert_eq!(
      resolved.get("start_url").map(String::as_str),
      Some("https://www.wikipedia.org")
    );

    let overrides = HashMap::from([("start_url".to_string(), "https://example.com".to_string())]);
    let resolved = scenario.resolve_variables(&overrides);
    assert_eq!(
      substitute("{{start_url}}/path", &resolved),
      "https://example.com/path"
    );
  }

  #[test]
  fn unknown_placeholder_is_left_intact() {
    let vars = HashMap::from([("a".to_string(), "1".to_string())]);
    assert_eq!(substitute("{{a}}/{{b}}", &vars), "1/{{b}}");
  }

  #[test]
  fn steps_round_trip_through_json() {
    let scenario = Scenario::builtin_warmup();
    let json = serde_json::to_string(&scenario).unwrap();
    let parsed: Scenario = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, scenario);
  }

  #[test]
  fn step_defaults_fill_in_from_minimal_json() {
    let step: Step = serde_json::from_str(r#"{"type":"navigate","url":"https://a.test"}"#).unwrap();
    assert_eq!(
      step,
      Step::Navigate {
        url: "https://a.test".to_string(),
        wait_for_load: true,
        timeout_secs: 30,
      }
    );

    let step: Step = serde_json::from_str(r#"{"type":"scroll"}"#).unwrap();
    assert_eq!(
      step,
      Step::Scroll {
        direction: ScrollDirection::Down,
        min_steps: 3,
        max_steps: 8,
      }
    );
  }

  #[test]
  fn step_schema_documents_every_kind() {
    let schema = step_schema();
    let documented: Vec<&str> = schema["steps"]
      .as_array()
      .unwrap()
      .iter()
      .map(|s| s["type"].as_str().unwrap())
      .collect();
    for kind in [
      "navigate",
      "wait_for_load",
      "scroll",
      "click_random_link",
      "dwell",
      "screenshot",
      "close_profile",
    ] {
      assert!(
        documented.contains(&kind),
        "{kind} missing from step schema"
      );
    }
  }
}
