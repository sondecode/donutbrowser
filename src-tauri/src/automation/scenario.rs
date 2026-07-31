//! Declarative automation scenario templates.
//!
//! A scenario is a plain list of steps plus a list of declared variables. It is
//! stored as JSON and is the only contract the executor understands, so an MCP
//! client can author or rewrite a whole workflow by sending JSON — no code
//! changes needed. `step_schema()` returns a machine-readable description of
//! every step kind for exactly that purpose.

use rand::RngExt;
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
    /// Sent as the request's `Referer`. A deep page arrived at with no referrer
    /// at all is its own signal, so set this whenever a scenario jumps straight
    /// to a URL instead of clicking its way there. Supports placeholders.
    #[serde(default)]
    referrer: Option<String>,
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
    /// Substrings a candidate href must contain — any one match qualifies it.
    /// Empty means every href qualifies. Set it to keep a click on the kind of
    /// page the scenario is actually about (`/shop/product/`, `/dp/`, …), so
    /// browsing your way to a target beats deep-linking to it.
    #[serde(default)]
    include_patterns: Vec<String>,
    #[serde(default = "default_true")]
    wait_for_load: bool,
    #[serde(default = "default_load_timeout")]
    timeout_secs: u64,
  },
  /// Close a modal that is covering the page — a survey invite, a newsletter
  /// prompt, an app interstitial — by clicking its close control, falling back to
  /// Escape. A no-op when nothing is in the way, so it is safe to place anywhere
  /// and never fails a run on its own.
  ///
  /// `click_random_link` already does this before each attempt; this step exists
  /// for the other reason a popup hurts: it lands in screenshots.
  DismissPopup,
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
      Step::DismissPopup => "dismiss_popup",
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
  /// Pool of values dealt out across the batch, one per profile, instead of
  /// every profile receiving the same string. This is what keeps a batch from
  /// sending N profiles to one identical URL — the tell that makes a batch
  /// legible as a batch.
  #[serde(default)]
  pub choices: Vec<String>,
  /// Refuse to start when the pool can't cover the batch, rather than wrapping
  /// around and handing the same value to more than one profile.
  #[serde(default)]
  pub unique: bool,
}

/// One shuffled pool per pool-backed variable, held for the length of a run.
pub type VariableDecks = HashMap<String, Vec<String>>;

/// Whether the caller pinned this variable for the whole run.
///
/// Blank counts as absent: the run dialog sends one entry per declared variable,
/// so an empty field has to mean "not set" rather than "set to nothing" — else
/// every pool-backed variable would be overridden into emptiness before it was
/// ever dealt.
fn has_override(overrides: &HashMap<String, String>, name: &str) -> bool {
  overrides
    .get(name)
    .is_some_and(|value| !value.trim().is_empty())
}

/// In-place Fisher–Yates. Hand-rolled so the deck depends only on
/// `random_range`, the same primitive every other randomised knob here uses.
fn shuffle<T>(items: &mut [T]) {
  let mut rng = rand::rng();
  for i in (1..items.len()).rev() {
    items.swap(i, rng.random_range(0..=i));
  }
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
  /// Whether this scenario participates in cloud/self-hosted sync. Defaults to
  /// true so existing scenarios keep syncing after the field is introduced.
  #[serde(default = "default_true")]
  pub sync_enabled: bool,
  /// Unix seconds of the last upload or download. Display only — `updated_at`
  /// decides sync direction — but it is what lets the UI answer "has this been
  /// backed up yet?" per scenario.
  #[serde(default)]
  pub last_sync: Option<u64>,
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
        choices: Vec::new(),
        unique: false,
      }],
      steps: vec![
        Step::Navigate {
          url: "{{start_url}}".to_string(),
          wait_for_load: true,
          timeout_secs: 30,
          referrer: None,
        },
        Step::Scroll {
          direction: ScrollDirection::Down,
          min_steps: 3,
          max_steps: 8,
        },
        Step::ClickRandomLink {
          same_domain_only: true,
          exclude_patterns: default_exclude_patterns(),
          include_patterns: Vec::new(),
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
          include_patterns: Vec::new(),
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
      sync_enabled: true,
      last_sync: None,
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
        Step::Screenshot { .. } | Step::CloseProfile | Step::DismissPopup => {}
      }
    }

    Ok(())
  }

  /// Shuffle every pool-backed variable once for the whole run, so handing card
  /// `index` to profile `index` gives each profile a different value for as long
  /// as the pool lasts. Variables the caller pinned get no deck — an explicit
  /// value is a deliberate request for all profiles to share it.
  pub fn shuffle_decks(&self, overrides: &HashMap<String, String>) -> VariableDecks {
    self
      .variables
      .iter()
      .filter(|v| !v.choices.is_empty() && !has_override(overrides, &v.name))
      .map(|v| {
        let mut deck = v.choices.clone();
        shuffle(&mut deck);
        (v.name.clone(), deck)
      })
      .collect()
  }

  /// Variable values for the profile at `index` in a batch.
  ///
  /// Precedence: a pinned override wins, else the profile's card from the
  /// shuffled deck, else the declared default. Undeclared overrides still
  /// substitute, so an MCP client can parameterise a step without editing the
  /// template first.
  pub fn resolve_variables_for(
    &self,
    overrides: &HashMap<String, String>,
    index: usize,
    decks: &VariableDecks,
  ) -> HashMap<String, String> {
    let mut resolved: HashMap<String, String> = HashMap::new();

    for variable in &self.variables {
      let value = if has_override(overrides, &variable.name) {
        overrides.get(&variable.name).cloned()
      } else {
        decks
          .get(&variable.name)
          .filter(|deck| !deck.is_empty())
          .map(|deck| deck[index % deck.len()].clone())
          .or_else(|| variable.default.clone())
      };
      if let Some(value) = value {
        resolved.insert(variable.name.clone(), value);
      }
    }

    for (name, value) in overrides {
      if !value.trim().is_empty() {
        resolved
          .entry(name.clone())
          .or_insert_with(|| value.clone());
      }
    }

    resolved
  }

  /// The first `unique` pool that can't cover `profile_count` profiles, as
  /// `(name, pool size)`.
  ///
  /// Lives here rather than in `validate()` because it depends on the size of
  /// the batch, not on the template — a scenario with a three-value pool is
  /// perfectly valid until someone points it at four profiles.
  pub fn undersized_unique_pool(
    &self,
    overrides: &HashMap<String, String>,
    profile_count: usize,
  ) -> Option<(&str, usize)> {
    self.variables.iter().find_map(|variable| {
      if !variable.unique
        || variable.choices.is_empty()
        || has_override(overrides, &variable.name)
        || variable.choices.len() >= profile_count
      {
        return None;
      }
      Some((variable.name.as_str(), variable.choices.len()))
    })
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
      "variables": "array of { name, description?, default?, choices?, unique? } — referenced in step strings as {{name}}",
      "steps": "array of step objects, executed in order",
    },
    "notes": [
      "Every step object needs a \"type\" field naming its kind.",
      "The browser is launched before the first step and killed after the last one, so no explicit launch step exists.",
      "Automation only works on Chromium/Wayfern profiles — Firefox-based profiles have no CDP endpoint.",
      format!("dwell max_secs is capped at {MAX_DWELL_SECS} seconds."),
      "A variable with a \"choices\" pool is dealt out one value per profile from a per-run shuffle, so a batch does not send every profile to the same URL. Set \"unique\": true to refuse a run whose batch is larger than the pool. Passing a value for the variable at run time pins it for every profile instead.",
      "Prefer reaching a target page with click_random_link + include_patterns over navigating straight to it: the click carries a referrer and a real user gesture, and each profile ends up on a different page.",
    ],
    "steps": [
      {
        "type": "navigate",
        "fields": {
          "url": "string (required) — supports {{variable}} placeholders",
          "wait_for_load": "bool (default true)",
          "timeout_secs": "number (default 30)",
          "referrer": "string (optional) — sent as the Referer header; supports placeholders",
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
          "include_patterns": "array of substrings an href must contain (any one match); empty means no restriction",
          "wait_for_load": "bool (default true)",
          "timeout_secs": "number (default 30)",
        },
      },
      {
        "type": "dismiss_popup",
        "fields": {},
        "note": "Closes a modal covering the page (survey invite, newsletter prompt) by clicking its close control, falling back to Escape. Does nothing when no popup is up, and never fails the run. click_random_link already does this automatically before each attempt — place this step before a screenshot so a popup does not end up in the capture.",
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
      referrer: None,
    }];
    let err = scenario.validate().unwrap_err();
    assert!(err.contains("AUTOMATION_STEP_URL_REQUIRED"));
  }

  /// A scenario carrying one pool-backed variable, the shape that replaces
  /// "duplicate the template once per URL".
  fn pooled(values: &[&str], unique: bool) -> Scenario {
    let mut scenario = Scenario::builtin_warmup();
    scenario.variables = vec![VariableDef {
      name: "product_url".to_string(),
      description: None,
      default: Some("https://fallback.test".to_string()),
      choices: values.iter().map(|v| v.to_string()).collect(),
      unique,
    }];
    scenario
  }

  fn deal(scenario: &Scenario, overrides: &HashMap<String, String>, count: usize) -> Vec<String> {
    let decks = scenario.shuffle_decks(overrides);
    (0..count)
      .map(|index| {
        scenario
          .resolve_variables_for(overrides, index, &decks)
          .get("product_url")
          .cloned()
          .unwrap_or_default()
      })
      .collect()
  }

  #[test]
  fn variables_substitute_and_override_defaults() {
    let scenario = Scenario::builtin_warmup();
    let decks = scenario.shuffle_decks(&HashMap::new());
    let resolved = scenario.resolve_variables_for(&HashMap::new(), 0, &decks);
    assert_eq!(
      resolved.get("start_url").map(String::as_str),
      Some("https://www.wikipedia.org")
    );

    let overrides = HashMap::from([("start_url".to_string(), "https://example.com".to_string())]);
    let resolved = scenario.resolve_variables_for(&overrides, 0, &decks);
    assert_eq!(
      substitute("{{start_url}}/path", &resolved),
      "https://example.com/path"
    );
  }

  #[test]
  fn a_pool_deals_a_different_value_to_every_profile() {
    // The bug this exists to catch: a batch pointing every profile at one URL.
    let scenario = pooled(&["a", "b", "c", "d"], false);
    for _ in 0..50 {
      let dealt = deal(&scenario, &HashMap::new(), 4);
      let distinct: std::collections::HashSet<&String> = dealt.iter().collect();
      assert_eq!(distinct.len(), 4, "profiles shared a value: {dealt:?}");
    }
  }

  #[test]
  fn a_pool_smaller_than_the_batch_wraps_instead_of_failing() {
    let scenario = pooled(&["a", "b"], false);
    let dealt = deal(&scenario, &HashMap::new(), 4);
    assert_eq!(dealt[0], dealt[2]);
    assert_eq!(dealt[1], dealt[3]);
    assert_ne!(dealt[0], dealt[1]);
  }

  #[test]
  fn a_unique_pool_that_cannot_cover_the_batch_is_reported() {
    let scenario = pooled(&["a", "b"], true);
    assert_eq!(
      scenario.undersized_unique_pool(&HashMap::new(), 3),
      Some(("product_url", 2))
    );
    assert_eq!(scenario.undersized_unique_pool(&HashMap::new(), 2), None);
    assert_eq!(scenario.undersized_unique_pool(&HashMap::new(), 1), None);
  }

  #[test]
  fn a_pool_without_unique_is_never_reported_as_undersized() {
    let scenario = pooled(&["a"], false);
    assert_eq!(scenario.undersized_unique_pool(&HashMap::new(), 9), None);
  }

  #[test]
  fn pinning_a_pooled_variable_gives_every_profile_the_same_value() {
    let scenario = pooled(&["a", "b", "c"], true);
    let overrides = HashMap::from([("product_url".to_string(), "pinned".to_string())]);
    assert_eq!(deal(&scenario, &overrides, 3), vec!["pinned"; 3]);
    // A deliberate pin is not an undersized pool.
    assert_eq!(scenario.undersized_unique_pool(&overrides, 99), None);
  }

  #[test]
  fn a_blank_override_does_not_shadow_the_pool() {
    // The run dialog sends one entry per declared variable, blank when the field
    // was left alone — treating that as a real value would empty every URL.
    let scenario = pooled(&["a", "b", "c"], false);
    let overrides = HashMap::from([("product_url".to_string(), "   ".to_string())]);
    let dealt = deal(&scenario, &overrides, 3);
    assert!(
      dealt
        .iter()
        .all(|value| ["a", "b", "c"].contains(&&**value)),
      "blank override leaked through: {dealt:?}"
    );
  }

  #[test]
  fn a_variable_with_no_pool_falls_back_to_its_default() {
    let mut scenario = pooled(&[], false);
    scenario.variables[0].choices.clear();
    assert_eq!(
      deal(&scenario, &HashMap::new(), 2),
      vec!["https://fallback.test"; 2]
    );
  }

  #[test]
  fn undeclared_overrides_still_substitute() {
    let scenario = Scenario::builtin_warmup();
    let overrides = HashMap::from([("extra".to_string(), "value".to_string())]);
    let resolved = scenario.resolve_variables_for(&overrides, 0, &VariableDecks::new());
    assert_eq!(resolved.get("extra").map(String::as_str), Some("value"));
  }

  #[test]
  fn shuffle_keeps_every_card() {
    let mut deck: Vec<u32> = (0..64).collect();
    shuffle(&mut deck);
    deck.sort_unstable();
    assert_eq!(deck, (0..64).collect::<Vec<u32>>());
  }

  #[test]
  fn shuffle_handles_degenerate_lengths() {
    shuffle::<u32>(&mut []);
    let mut one = [7];
    shuffle(&mut one);
    assert_eq!(one, [7]);
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
        referrer: None,
      }
    );

    // Older stored scenarios predate both fields and must keep deserializing.
    let step: Step = serde_json::from_str(r#"{"type":"click_random_link"}"#).unwrap();
    assert_eq!(
      step,
      Step::ClickRandomLink {
        same_domain_only: true,
        exclude_patterns: Vec::new(),
        include_patterns: Vec::new(),
        wait_for_load: true,
        timeout_secs: 30,
      }
    );

    let variable: VariableDef = serde_json::from_str(r#"{"name":"a"}"#).unwrap();
    assert!(variable.choices.is_empty());
    assert!(!variable.unique);

    // A unit variant needs no fields, and must round-trip under its snake_case tag.
    let step: Step = serde_json::from_str(r#"{"type":"dismiss_popup"}"#).unwrap();
    assert_eq!(step, Step::DismissPopup);
    assert_eq!(step.kind(), "dismiss_popup");
    assert_eq!(
      serde_json::to_value(&step).unwrap(),
      serde_json::json!({ "type": "dismiss_popup" })
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
      "dismiss_popup",
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
