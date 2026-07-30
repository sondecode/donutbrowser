//! Batch executor for automation scenarios.
//!
//! One run covers many profiles. Each profile gets its own task, gated by a
//! semaphore so at most `concurrency` browsers are alive at once, and staggered
//! by a random jitter so launches don't land in lockstep. Run state lives in a
//! process-global map that the UI polls/subscribes to and MCP reads.

use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::automation::cdp;
use crate::automation::scenario::{substitute, Scenario, ScrollDirection, Step, MAX_CONCURRENCY};
use crate::automation::storage::ScenarioStore;
use crate::human_mouse;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::now_secs;

pub const RUN_UPDATED_EVENT: &str = "automation-run-updated";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
  Running,
  Completed,
  Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRunStatus {
  Pending,
  Running,
  Completed,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRun {
  pub profile_id: String,
  pub profile_name: String,
  pub status: ProfileRunStatus,
  /// Zero-based index of the step currently executing.
  pub current_step_index: Option<usize>,
  pub current_step_kind: Option<String>,
  pub total_steps: usize,
  pub error: Option<String>,
  pub started_at: Option<u64>,
  pub finished_at: Option<u64>,
  /// Absolute paths of screenshots captured so far.
  pub screenshots: Vec<String>,
  /// Unix seconds at which the active dwell ends, so the UI can count down.
  pub waiting_until: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRun {
  pub id: String,
  pub scenario_id: String,
  pub scenario_name: String,
  pub status: RunStatus,
  pub created_at: u64,
  pub finished_at: Option<u64>,
  pub concurrency: u32,
  pub profiles: Vec<ProfileRun>,
}

impl AutomationRun {
  fn recompute_status(&mut self) {
    let all_done = self.profiles.iter().all(|p| {
      matches!(
        p.status,
        ProfileRunStatus::Completed | ProfileRunStatus::Failed | ProfileRunStatus::Cancelled
      )
    });
    if !all_done {
      return;
    }
    let any_cancelled = self
      .profiles
      .iter()
      .any(|p| p.status == ProfileRunStatus::Cancelled);
    self.status = if any_cancelled {
      RunStatus::Cancelled
    } else {
      RunStatus::Completed
    };
    self.finished_at = Some(now_secs());
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
  pub scenario_id: String,
  /// Profiles named explicitly. Every one must be runnable — an ineligible
  /// entry fails the whole request, because the caller asked for it by name.
  #[serde(default)]
  pub profile_ids: Vec<String>,
  /// Run every runnable profile in this group. Unlike `profile_ids`, members
  /// that can't be automated (not Chromium, already running) are skipped rather
  /// than failing the request — the caller asked for "the group", not for those
  /// specific profiles. Combined with `profile_ids` as a union.
  #[serde(default)]
  pub group_id: Option<String>,
  #[serde(default = "default_concurrency")]
  pub concurrency: u32,
  #[serde(default)]
  pub jitter_min_secs: u64,
  #[serde(default)]
  pub jitter_max_secs: u64,
  #[serde(default)]
  pub variables: HashMap<String, String>,
  #[serde(default)]
  pub headless: bool,
}

fn default_concurrency() -> u32 {
  1
}

lazy_static::lazy_static! {
  static ref RUNS: Mutex<HashMap<String, AutomationRun>> = Mutex::new(HashMap::new());
  static ref CANCEL_FLAGS: Mutex<HashMap<String, Arc<AtomicBool>>> = Mutex::new(HashMap::new());
}

/// Mutate a run in place, then emit the updated snapshot. The lock is always
/// released before the emit so an event handler can never deadlock us.
fn with_run<F: FnOnce(&mut AutomationRun)>(run_id: &str, mutate: F) {
  let snapshot = {
    let mut runs = match RUNS.lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    let Some(run) = runs.get_mut(run_id) else {
      return;
    };
    mutate(run);
    run.recompute_status();
    run.clone()
  };
  let _ = crate::events::emit(RUN_UPDATED_EVENT, &snapshot);
}

fn with_profile<F: FnOnce(&mut ProfileRun)>(run_id: &str, profile_index: usize, mutate: F) {
  with_run(run_id, |run| {
    if let Some(profile) = run.profiles.get_mut(profile_index) {
      mutate(profile);
    }
  });
}

fn cancel_flag(run_id: &str) -> Arc<AtomicBool> {
  let mut flags = match CANCEL_FLAGS.lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };
  flags
    .entry(run_id.to_string())
    .or_insert_with(|| Arc::new(AtomicBool::new(false)))
    .clone()
}

pub fn get_run(run_id: &str) -> Option<AutomationRun> {
  let runs = match RUNS.lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };
  runs.get(run_id).cloned()
}

pub fn list_runs() -> Vec<AutomationRun> {
  let runs = match RUNS.lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
  };
  let mut all: Vec<AutomationRun> = runs.values().cloned().collect();
  all.sort_by_key(|run| std::cmp::Reverse(run.created_at));
  all
}

/// Ask a run to stop. Profiles mid-dwell wake within a second; each one is
/// killed as its task unwinds.
pub fn cancel_run(run_id: &str) -> Result<(), String> {
  if get_run(run_id).is_none() {
    return Err(
      serde_json::json!({ "code": "AUTOMATION_RUN_NOT_FOUND", "params": { "id": run_id } })
        .to_string(),
    );
  }
  cancel_flag(run_id).store(true, Ordering::SeqCst);
  with_run(run_id, |run| {
    for profile in &mut run.profiles {
      if profile.status == ProfileRunStatus::Pending {
        profile.status = ProfileRunStatus::Cancelled;
        profile.finished_at = Some(now_secs());
      }
    }
  });
  Ok(())
}

/// Guards that depend only on the request itself — no profile or scenario
/// lookups — so they can be checked (and tested) without an `AppHandle`.
fn validate_request(request: &RunRequest) -> Result<(), String> {
  if request.profile_ids.is_empty() && request.group_id.is_none() {
    return Err(serde_json::json!({ "code": "AUTOMATION_NO_PROFILES_SELECTED" }).to_string());
  }
  if request.concurrency == 0 || request.concurrency > MAX_CONCURRENCY {
    return Err(
      serde_json::json!({
        "code": "AUTOMATION_CONCURRENCY_INVALID",
        "params": { "max": MAX_CONCURRENCY.to_string() }
      })
      .to_string(),
    );
  }
  if request.jitter_min_secs > request.jitter_max_secs {
    return Err(serde_json::json!({ "code": "AUTOMATION_JITTER_RANGE_INVALID" }).to_string());
  }
  Ok(())
}

/// Validate the request, register the run, and spawn the workers.
pub async fn start_run(
  app_handle: tauri::AppHandle,
  request: RunRequest,
) -> Result<AutomationRun, String> {
  let scenario = ScenarioStore::new().get(&request.scenario_id)?;
  scenario.validate()?;
  validate_request(&request)?;

  let all_profiles = ProfileManager::instance()
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;

  let mut selected: Vec<BrowserProfile> = Vec::new();
  for id in &request.profile_ids {
    let profile = all_profiles
      .iter()
      .find(|p| p.id.to_string() == *id)
      .ok_or_else(|| {
        serde_json::json!({ "code": "PROFILE_NOT_FOUND", "params": { "id": id } }).to_string()
      })?;

    if !crate::browser::is_chromium_target(&profile.browser) {
      return Err(
        serde_json::json!({
          "code": "AUTOMATION_PROFILE_NOT_CHROMIUM",
          "params": { "name": profile.name.clone() }
        })
        .to_string(),
      );
    }
    if profile.process_id.is_some() {
      return Err(
        serde_json::json!({
          "code": "AUTOMATION_PROFILE_ALREADY_RUNNING",
          "params": { "name": profile.name.clone() }
        })
        .to_string(),
      );
    }
    selected.push(profile.clone());
  }

  if let Some(group_id) = &request.group_id {
    if !crate::group_manager::GroupManager::new()
      .get_all_groups()
      .map_err(|e| format!("Failed to list profile groups: {e}"))?
      .iter()
      .any(|group| group.id == *group_id)
    {
      return Err(
        serde_json::json!({ "code": "GROUP_NOT_FOUND", "params": { "id": group_id } }).to_string(),
      );
    }

    for profile in &all_profiles {
      if profile.group_id.as_deref() != Some(group_id.as_str()) {
        continue;
      }
      // Skip rather than fail: the caller targeted the group, not this member.
      if !crate::browser::is_chromium_target(&profile.browser) || profile.process_id.is_some() {
        log::info!(
          "[automation] skipping '{}' from group {group_id} (not automatable right now)",
          profile.name
        );
        continue;
      }
      if selected.iter().any(|p| p.id == profile.id) {
        continue;
      }
      selected.push(profile.clone());
    }
  }

  if selected.is_empty() {
    return Err(serde_json::json!({ "code": "AUTOMATION_NO_ELIGIBLE_PROFILES" }).to_string());
  }

  let run_id = uuid::Uuid::new_v4().to_string();
  let total_steps = scenario.steps.len();
  let run = AutomationRun {
    id: run_id.clone(),
    scenario_id: scenario.id.clone(),
    scenario_name: scenario.name.clone(),
    status: RunStatus::Running,
    created_at: now_secs(),
    finished_at: None,
    concurrency: request.concurrency,
    profiles: selected
      .iter()
      .map(|p| ProfileRun {
        profile_id: p.id.to_string(),
        profile_name: p.name.clone(),
        status: ProfileRunStatus::Pending,
        current_step_index: None,
        current_step_kind: None,
        total_steps,
        error: None,
        started_at: None,
        finished_at: None,
        screenshots: Vec::new(),
        waiting_until: None,
      })
      .collect(),
  };

  {
    let mut runs = match RUNS.lock() {
      Ok(guard) => guard,
      Err(poisoned) => poisoned.into_inner(),
    };
    runs.insert(run_id.clone(), run.clone());
  }
  let cancel = cancel_flag(&run_id);
  let _ = crate::events::emit(RUN_UPDATED_EVENT, &run);

  let variables = scenario.resolve_variables(&request.variables);
  let semaphore = Arc::new(tokio::sync::Semaphore::new(request.concurrency as usize));
  let scenario = Arc::new(scenario);

  for (index, profile) in selected.into_iter().enumerate() {
    let app_handle = app_handle.clone();
    let run_id = run_id.clone();
    let cancel = cancel.clone();
    let semaphore = semaphore.clone();
    let scenario = scenario.clone();
    let variables = variables.clone();
    let headless = request.headless;
    let jitter = (request.jitter_min_secs, request.jitter_max_secs);

    tokio::spawn(async move {
      let permit = match semaphore.acquire().await {
        Ok(permit) => permit,
        Err(_) => return,
      };

      if cancel.load(Ordering::SeqCst) {
        with_profile(&run_id, index, |p| {
          p.status = ProfileRunStatus::Cancelled;
          p.finished_at = Some(now_secs());
        });
        return;
      }

      // Stagger real launches so a batch doesn't produce a burst of identical
      // session start times.
      let jitter_secs = random_in_range(jitter.0, jitter.1);
      if jitter_secs > 0
        && !sleep_cancellable(std::time::Duration::from_secs(jitter_secs), &cancel).await
      {
        with_profile(&run_id, index, |p| {
          p.status = ProfileRunStatus::Cancelled;
          p.finished_at = Some(now_secs());
        });
        return;
      }

      let outcome = run_one_profile(
        &app_handle,
        &run_id,
        index,
        &profile,
        &scenario,
        &variables,
        headless,
        &cancel,
      )
      .await;

      // Always try to leave no browser behind, whatever the outcome.
      let _ = crate::browser_runner::BrowserRunner::instance()
        .kill_browser_process(app_handle.clone(), &profile)
        .await;

      with_profile(&run_id, index, |p| {
        p.waiting_until = None;
        p.finished_at = Some(now_secs());
        match &outcome {
          Ok(()) => p.status = ProfileRunStatus::Completed,
          Err(err) if cancel.load(Ordering::SeqCst) => {
            p.status = ProfileRunStatus::Cancelled;
            log::info!("[automation] profile '{}' cancelled: {err}", p.profile_name);
          }
          Err(err) => {
            p.status = ProfileRunStatus::Failed;
            p.error = Some(err.clone());
            log::warn!("[automation] profile '{}' failed: {err}", p.profile_name);
          }
        }
      });

      drop(permit);
    });
  }

  Ok(run)
}

#[allow(clippy::too_many_arguments)]
async fn run_one_profile(
  app_handle: &tauri::AppHandle,
  run_id: &str,
  index: usize,
  profile: &BrowserProfile,
  scenario: &Scenario,
  variables: &HashMap<String, String>,
  headless: bool,
  cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
  with_profile(run_id, index, |p| {
    p.status = ProfileRunStatus::Running;
    p.started_at = Some(now_secs());
  });

  crate::browser_runner::launch_browser_profile_impl(
    app_handle.clone(),
    profile.clone(),
    None,
    None,
    headless,
    true,
  )
  .await?;

  let ws_url = cdp::ws_url_for_profile(profile).await?;

  // One cursor for the whole run, so each movement continues from where the
  // pointer already was.
  let (viewport_width, viewport_height) = viewport_size(&ws_url).await?;
  let mut cursor = PageCursor::new(viewport_width, viewport_height);

  let screenshot_dir = crate::app_dirs::data_subdir()
    .join("automation_screenshots")
    .join(run_id);

  for (step_index, step) in scenario.steps.iter().enumerate() {
    if cancel.load(Ordering::SeqCst) {
      return Err("cancelled".to_string());
    }

    with_profile(run_id, index, |p| {
      p.current_step_index = Some(step_index);
      p.current_step_kind = Some(step.kind().to_string());
      p.waiting_until = None;
    });

    log::info!(
      "[automation] run {run_id} profile '{}' step {}/{}: {}",
      profile.name,
      step_index + 1,
      scenario.steps.len(),
      step.kind()
    );

    match step {
      Step::CloseProfile => {
        crate::browser_runner::BrowserRunner::instance()
          .kill_browser_process(app_handle.clone(), profile)
          .await
          .map_err(|e| format!("Failed to close profile: {e}"))?;
        return Ok(());
      }
      Step::Screenshot { full_page } => {
        let path = capture_screenshot(
          &ws_url,
          &screenshot_dir,
          &profile.name,
          step_index,
          *full_page,
        )
        .await?;
        let path_str = path.to_string_lossy().to_string();
        with_profile(run_id, index, |p| p.screenshots.push(path_str.clone()));
      }
      Step::Dwell { min_secs, max_secs } => {
        let secs = random_in_range(*min_secs, *max_secs);
        with_profile(run_id, index, |p| {
          p.waiting_until = Some(now_secs() + secs);
        });
        if !sleep_cancellable(std::time::Duration::from_secs(secs), cancel).await {
          return Err("cancelled".to_string());
        }
        with_profile(run_id, index, |p| p.waiting_until = None);
      }
      other => execute_step(&ws_url, &mut cursor, other, variables, cancel).await?,
    }
  }

  Ok(())
}

async fn execute_step(
  ws_url: &str,
  cursor: &mut PageCursor,
  step: &Step,
  variables: &HashMap<String, String>,
  cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
  match step {
    Step::Navigate {
      url,
      wait_for_load,
      timeout_secs,
    } => {
      let resolved = substitute(url, variables);
      let params = serde_json::json!({ "url": resolved });
      if *wait_for_load {
        cdp::send_and_wait_for_load(ws_url, "Page.navigate", params, *timeout_secs).await?;
        wait_for_ready_state(ws_url, *timeout_secs, cancel).await?;
      } else {
        cdp::send(ws_url, "Page.navigate", params).await?;
      }
      Ok(())
    }
    Step::WaitForLoad { timeout_secs } => wait_for_ready_state(ws_url, *timeout_secs, cancel).await,
    Step::Scroll {
      direction,
      min_steps,
      max_steps,
    } => scroll_page(ws_url, cursor, *direction, *min_steps, *max_steps, cancel).await,
    Step::ClickRandomLink {
      same_domain_only,
      exclude_patterns,
      wait_for_load,
      timeout_secs,
    } => {
      click_random_link(
        ws_url,
        cursor,
        *same_domain_only,
        exclude_patterns,
        *wait_for_load,
        *timeout_secs,
        cancel,
      )
      .await
    }
    // Handled by the caller, which needs run state the step executor lacks.
    Step::Dwell { .. } | Step::Screenshot { .. } | Step::CloseProfile => Ok(()),
  }
}

/// Poll `document.readyState` until the page reports `complete`.
async fn wait_for_ready_state(
  ws_url: &str,
  timeout_secs: u64,
  cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
  let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
  loop {
    if cancel.load(Ordering::SeqCst) {
      return Err("cancelled".to_string());
    }
    let state = cdp::evaluate(ws_url, "document.readyState").await?;
    if state.as_str() == Some("complete") {
      return Ok(());
    }
    if tokio::time::Instant::now() >= deadline {
      // A page that never goes quiet is normal (long polling, streaming media);
      // continuing is better than failing the whole scenario.
      log::info!("[automation] readyState still {state} after {timeout_secs}s, continuing");
      return Ok(());
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  }
}

/// The pointer, as the page sees it.
///
/// Kept for the whole run so successive movements start from wherever the cursor
/// actually is. A fresh teleport before every action would look like a new hand
/// materialising each time.
struct PageCursor {
  position: human_mouse::Point,
}

impl PageCursor {
  /// Start somewhere plausible in the upper-middle of the viewport, as if the
  /// window had just been focused with the pointer already resting on it.
  fn new(viewport_width: f64, viewport_height: f64) -> Self {
    let (x, y) = {
      let mut rng = rand::rng();
      (
        rng.random_range(viewport_width * 0.25..viewport_width * 0.75),
        rng.random_range(viewport_height * 0.2..viewport_height * 0.6),
      )
    };
    Self {
      position: human_mouse::Point::new(x, y),
    }
  }

  /// Walk to `target` along a generated path, one socket for the whole motion.
  async fn move_to(&mut self, ws_url: &str, target: human_mouse::Point) -> Result<(), String> {
    let samples = human_mouse::generate_move(self.position, target);
    let commands: Vec<cdp::TimedCommand> = samples
      .iter()
      .map(|sample| cdp::TimedCommand {
        at: sample.time,
        method: "Input.dispatchMouseEvent".to_string(),
        params: serde_json::json!({
          "type": "mouseMoved",
          "x": sample.x,
          "y": sample.y,
          "buttons": 0,
          "pointerType": "mouse",
        }),
      })
      .collect();

    cdp::send_timed_sequence(ws_url, &commands).await?;
    self.position = target;
    Ok(())
  }

  /// Move to `target`, settle, then press and release.
  ///
  /// The approach, the pause and the button events go out as one scheduled
  /// sequence so the gap between arriving and pressing is the generated human
  /// delay rather than however long a new WebSocket handshake happened to take.
  async fn click_at(&mut self, ws_url: &str, target: human_mouse::Point) -> Result<(), String> {
    let samples = human_mouse::generate_move(self.position, target);
    let arrival = samples.last().map(|s| s.time).unwrap_or(0.0);
    let press_at = arrival + human_mouse::settle_before_press_secs();
    let release_at = press_at + human_mouse::press_hold_secs();

    let mut commands: Vec<cdp::TimedCommand> = samples
      .iter()
      .map(|sample| cdp::TimedCommand {
        at: sample.time,
        method: "Input.dispatchMouseEvent".to_string(),
        params: serde_json::json!({
          "type": "mouseMoved",
          "x": sample.x,
          "y": sample.y,
          "buttons": 0,
          "pointerType": "mouse",
        }),
      })
      .collect();

    commands.push(cdp::TimedCommand {
      at: press_at,
      method: "Input.dispatchMouseEvent".to_string(),
      params: serde_json::json!({
        "type": "mousePressed",
        "x": target.x,
        "y": target.y,
        "button": "left",
        "buttons": 1,
        "clickCount": 1,
        "pointerType": "mouse",
      }),
    });
    commands.push(cdp::TimedCommand {
      at: release_at,
      method: "Input.dispatchMouseEvent".to_string(),
      params: serde_json::json!({
        "type": "mouseReleased",
        "x": target.x,
        "y": target.y,
        "button": "left",
        "buttons": 0,
        "clickCount": 1,
        "pointerType": "mouse",
      }),
    });

    cdp::send_timed_sequence(ws_url, &commands).await?;
    self.position = target;
    Ok(())
  }

  /// Emit wheel ticks at the cursor, drifting slightly between them — a resting
  /// hand still moves, and wheel events arriving from a pixel-perfect fixed
  /// point are their own tell.
  async fn wheel(
    &mut self,
    ws_url: &str,
    ticks: &[f64],
    viewport_width: f64,
    viewport_height: f64,
  ) -> Result<(), String> {
    let mut commands = Vec::new();
    let mut clock = 0.0;
    let mut position = self.position;

    for delta in ticks {
      let (drift_x, drift_y, gap) = {
        let mut rng = rand::rng();
        (
          rng.random_range(-3.0..3.0),
          rng.random_range(-3.0..3.0),
          rng.random_range(0.35..1.5),
        )
      };
      position = human_mouse::Point::new(
        (position.x + drift_x).clamp(1.0, viewport_width - 2.0),
        (position.y + drift_y).clamp(1.0, viewport_height - 2.0),
      );

      commands.push(cdp::TimedCommand {
        at: clock,
        method: "Input.dispatchMouseEvent".to_string(),
        params: serde_json::json!({
          "type": "mouseMoved",
          "x": position.x,
          "y": position.y,
          "buttons": 0,
          "pointerType": "mouse",
        }),
      });
      commands.push(cdp::TimedCommand {
        at: clock + 0.02,
        method: "Input.dispatchMouseEvent".to_string(),
        params: serde_json::json!({
          "type": "mouseWheel",
          "x": position.x,
          "y": position.y,
          "deltaX": 0,
          "deltaY": delta,
          "pointerType": "mouse",
        }),
      });

      clock += gap;
    }

    cdp::send_timed_sequence(ws_url, &commands).await?;
    self.position = position;
    Ok(())
  }
}

/// Viewport size in CSS pixels.
async fn viewport_size(ws_url: &str) -> Result<(f64, f64), String> {
  let raw = cdp::evaluate(
    ws_url,
    "JSON.stringify([window.innerWidth, window.innerHeight])",
  )
  .await?;
  let dims: Vec<f64> = raw
    .as_str()
    .and_then(|s| serde_json::from_str(s).ok())
    .unwrap_or_else(|| vec![1280.0, 800.0]);
  Ok((
    dims.first().copied().unwrap_or(1280.0).max(1.0),
    dims.get(1).copied().unwrap_or(800.0).max(1.0),
  ))
}

/// Randomised wheel magnitudes for a scroll burst.
fn wheel_ticks(direction: ScrollDirection, count: u64, viewport_height: f64) -> Vec<f64> {
  let mut rng = rand::rng();
  (0..count)
    .map(|_| {
      let down = match direction {
        ScrollDirection::Down => true,
        ScrollDirection::Up => false,
        ScrollDirection::Random => rng.random::<bool>(),
      };
      let magnitude = rng.random_range(120.0..(viewport_height * 0.8).max(200.0));
      if down {
        magnitude
      } else {
        -magnitude
      }
    })
    .collect()
}

async fn scroll_page(
  ws_url: &str,
  cursor: &mut PageCursor,
  direction: ScrollDirection,
  min_steps: u32,
  max_steps: u32,
  cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
  if cancel.load(Ordering::SeqCst) {
    return Err("cancelled".to_string());
  }

  let (width, height) = viewport_size(ws_url).await?;

  // Move somewhere over the content before scrolling, the way a hand settles on
  // the page first.
  let resting = {
    let mut rng = rand::rng();
    human_mouse::Point::new(
      rng.random_range(width * 0.25..width * 0.75),
      rng.random_range(height * 0.25..height * 0.7),
    )
  };
  cursor.move_to(ws_url, resting).await?;

  let count = random_in_range(min_steps as u64, max_steps as u64);
  let ticks = wheel_ticks(direction, count, height);
  cursor.wheel(ws_url, &ticks, width, height).await
}

/// Pick one eligible anchor at random and tag it on `window.__donut_link` so its
/// geometry can be re-read later without re-running the selection.
const PICK_LINK_JS: &str = r#"
(() => {
  const sameDomainOnly = __SAME_DOMAIN__;
  const excludes = __EXCLUDES__;
  const tried = __TRIED__;
  const here = location.hostname;

  const candidates = Array.from(document.querySelectorAll('a[href]')).filter((a) => {
    const href = a.href;
    if (!href.startsWith('http://') && !href.startsWith('https://')) return false;
    if (href.replace(/#.*$/, '') === location.href.replace(/#.*$/, '')) return false;
    if (tried.includes(href)) return false;
    let host;
    try { host = new URL(href).hostname; } catch { return false; }
    if (sameDomainOnly && host !== here) return false;
    const lower = href.toLowerCase();
    if (excludes.some((p) => lower.includes(p))) return false;
    const rect = a.getBoundingClientRect();
    if (rect.width < 4 || rect.height < 4) return false;
    const style = getComputedStyle(a);
    if (style.visibility === 'hidden' || style.display === 'none' || style.pointerEvents === 'none') return false;
    return true;
  });

  if (candidates.length === 0) return JSON.stringify({ ok: false });

  const link = candidates[Math.floor(Math.random() * candidates.length)];
  // Keep the journey in one tab; a popup would leave the automated tab behind.
  link.removeAttribute('target');
  window.__donut_link = link;
  return JSON.stringify({ ok: true, href: link.href });
})()
"#;

/// Re-read the tagged link's live geometry and check that a click at its centre
/// would actually reach it.
///
/// Measuring at click time is the point: pages move under us constantly (lazy
/// images, sticky headers, injected ads), so coordinates captured even a few
/// hundred milliseconds earlier can address a different element entirely.
const MEASURE_LINK_JS: &str = r#"
(() => {
  const link = window.__donut_link;
  if (!link || !link.isConnected) return JSON.stringify({ ok: false });

  const rect = link.getBoundingClientRect();
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  const inViewport =
    rect.top >= 0 && rect.left >= 0 &&
    rect.bottom <= window.innerHeight && rect.right <= window.innerWidth &&
    rect.width >= 4 && rect.height >= 4;

  let hit = false;
  if (inViewport) {
    const at = document.elementFromPoint(cx, cy);
    hit = at !== null && (at === link || link.contains(at) || at.contains(link));
  }

  return JSON.stringify({
    ok: true,
    href: link.href,
    inViewport,
    hit,
    rect: { x: rect.left, y: rect.top, width: rect.width, height: rect.height },
    viewport: { width: window.innerWidth, height: window.innerHeight },
    // How far the link centre sits from the viewport centre, so the caller can
    // wheel towards it instead of jumping the scroll position programmatically.
    offsetFromCenter: cy - window.innerHeight / 2,
  });
})()
"#;

/// How many different links to try before giving up on the step.
const MAX_LINK_ATTEMPTS: usize = 3;
/// Cap on wheel bursts spent bringing one link into view.
const MAX_SCROLL_INTO_VIEW_BURSTS: usize = 12;

#[derive(Debug, Clone)]
struct LinkMeasurement {
  in_viewport: bool,
  hit: bool,
  rect: human_mouse::Rect,
  viewport: (f64, f64),
  offset_from_center: f64,
}

async fn measure_tagged_link(ws_url: &str) -> Result<Option<LinkMeasurement>, String> {
  let raw = cdp::evaluate(ws_url, MEASURE_LINK_JS).await?;
  let parsed: serde_json::Value = raw
    .as_str()
    .and_then(|s| serde_json::from_str(s).ok())
    .unwrap_or(serde_json::json!({ "ok": false }));

  if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
    return Ok(None);
  }

  let number = |value: Option<&serde_json::Value>| value.and_then(|v| v.as_f64()).unwrap_or(0.0);
  let rect = parsed.get("rect");

  Ok(Some(LinkMeasurement {
    in_viewport: parsed
      .get("inViewport")
      .and_then(|v| v.as_bool())
      .unwrap_or(false),
    hit: parsed.get("hit").and_then(|v| v.as_bool()).unwrap_or(false),
    rect: human_mouse::Rect {
      x: number(rect.and_then(|r| r.get("x"))),
      y: number(rect.and_then(|r| r.get("y"))),
      width: number(rect.and_then(|r| r.get("width"))),
      height: number(rect.and_then(|r| r.get("height"))),
    },
    viewport: (
      number(parsed.get("viewport").and_then(|v| v.get("width"))).max(1.0),
      number(parsed.get("viewport").and_then(|v| v.get("height"))).max(1.0),
    ),
    offset_from_center: number(parsed.get("offsetFromCenter")),
  }))
}

/// Wheel the tagged link into view. Deliberately not `scrollIntoView()`: that
/// jumps the scroll position with no input events behind it, which is both
/// unlike a person and inconsistent with every other motion we emit.
async fn wheel_link_into_view(
  ws_url: &str,
  cursor: &mut PageCursor,
  cancel: &Arc<AtomicBool>,
) -> Result<Option<LinkMeasurement>, String> {
  for burst in 0..MAX_SCROLL_INTO_VIEW_BURSTS {
    if cancel.load(Ordering::SeqCst) {
      return Err("cancelled".to_string());
    }

    let Some(measurement) = measure_tagged_link(ws_url).await? else {
      return Ok(None);
    };
    if measurement.in_viewport {
      return Ok(Some(measurement));
    }

    let (width, height) = measurement.viewport;
    if burst == 0 {
      let resting = {
        let mut rng = rand::rng();
        human_mouse::Point::new(
          rng.random_range(width * 0.3..width * 0.7),
          rng.random_range(height * 0.3..height * 0.6),
        )
      };
      cursor.move_to(ws_url, resting).await?;
    }

    // Cover at most ~70% of the remaining gap per burst so we ease in rather
    // than slamming the target to the edge of the viewport.
    let remaining = measurement.offset_from_center;
    let ticks = {
      let mut rng = rand::rng();
      let budget = (remaining.abs() * 0.7).clamp(80.0, height * 2.0);
      let mut spent = 0.0;
      let mut ticks = Vec::new();
      while spent < budget && ticks.len() < 8 {
        let magnitude = rng.random_range(90.0..(height * 0.55).max(140.0));
        spent += magnitude;
        ticks.push(if remaining > 0.0 {
          magnitude
        } else {
          -magnitude
        });
      }
      ticks
    };

    cursor.wheel(ws_url, &ticks, width, height).await?;
  }

  measure_tagged_link(ws_url).await
}

async fn click_random_link(
  ws_url: &str,
  cursor: &mut PageCursor,
  same_domain_only: bool,
  exclude_patterns: &[String],
  wait_for_load: bool,
  timeout_secs: u64,
  cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
  let excludes: Vec<String> = exclude_patterns.iter().map(|p| p.to_lowercase()).collect();
  let mut tried: Vec<String> = Vec::new();

  for attempt in 1..=MAX_LINK_ATTEMPTS {
    if cancel.load(Ordering::SeqCst) {
      return Err("cancelled".to_string());
    }

    // `document.readyState` is a useless "did we navigate?" signal on its own:
    // right after a click it still reads `complete` for the *old* page. The URL
    // is what actually proves the page moved.
    let url_before = cdp::evaluate(ws_url, "location.href")
      .await?
      .as_str()
      .unwrap_or_default()
      .to_string();

    let js = PICK_LINK_JS
      .replace(
        "__SAME_DOMAIN__",
        if same_domain_only { "true" } else { "false" },
      )
      .replace(
        "__EXCLUDES__",
        &serde_json::to_string(&excludes).unwrap_or_else(|_| "[]".to_string()),
      )
      .replace(
        "__TRIED__",
        &serde_json::to_string(&tried).unwrap_or_else(|_| "[]".to_string()),
      );

    let picked = cdp::evaluate(ws_url, &js).await?;
    let picked: serde_json::Value = picked
      .as_str()
      .and_then(|s| serde_json::from_str(s).ok())
      .unwrap_or(serde_json::json!({ "ok": false }));

    if picked.get("ok").and_then(|v| v.as_bool()) != Some(true) {
      return Err(serde_json::json!({ "code": "AUTOMATION_NO_LINK_FOUND" }).to_string());
    }

    let href = picked
      .get("href")
      .and_then(|v| v.as_str())
      .unwrap_or_default()
      .to_string();
    tried.push(href.clone());

    let Some(measurement) = wheel_link_into_view(ws_url, cursor, cancel).await? else {
      log::info!("[automation] link {href} vanished before it could be clicked");
      continue;
    };

    if !measurement.in_viewport || !measurement.hit {
      // Something is covering it, or it moved out from under us. Another link is
      // a better bet than clicking coordinates we know are wrong.
      log::info!(
        "[automation] link {href} not clickable (in_viewport={}, hit={}), trying another",
        measurement.in_viewport,
        measurement.hit
      );
      continue;
    }

    let target = human_mouse::sample_click_point(measurement.rect);
    cursor.click_at(ws_url, target).await?;

    if !wait_for_load {
      return Ok(());
    }

    if wait_for_url_change(ws_url, &url_before, timeout_secs, cancel).await? {
      return wait_for_ready_state(ws_url, timeout_secs, cancel).await;
    }

    log::info!(
      "[automation] click on {href} did not navigate (attempt {attempt}/{MAX_LINK_ATTEMPTS})"
    );
  }

  Err(serde_json::json!({ "code": "AUTOMATION_LINK_CLICK_FAILED" }).to_string())
}

/// Poll `location.href` until it differs from `previous`. Returns whether the
/// page actually moved before the timeout.
async fn wait_for_url_change(
  ws_url: &str,
  previous: &str,
  timeout_secs: u64,
  cancel: &Arc<AtomicBool>,
) -> Result<bool, String> {
  let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
  loop {
    if cancel.load(Ordering::SeqCst) {
      return Err("cancelled".to_string());
    }
    let current = cdp::evaluate(ws_url, "location.href").await?;
    if current.as_str().unwrap_or_default() != previous {
      return Ok(true);
    }
    if tokio::time::Instant::now() >= deadline {
      return Ok(false);
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  }
}

async fn capture_screenshot(
  ws_url: &str,
  dir: &std::path::Path,
  profile_name: &str,
  step_index: usize,
  full_page: bool,
) -> Result<std::path::PathBuf, String> {
  use base64::Engine;

  let mut params = serde_json::json!({ "format": "png" });
  if full_page {
    let layout = cdp::send(ws_url, "Page.getLayoutMetrics", serde_json::json!({})).await?;
    if let Some(content_size) = layout.get("contentSize") {
      params["clip"] = serde_json::json!({
        "x": 0,
        "y": 0,
        "width": content_size.get("width").and_then(|v| v.as_f64()).unwrap_or(1920.0),
        "height": content_size.get("height").and_then(|v| v.as_f64()).unwrap_or(1080.0),
        "scale": 1,
      });
      params["captureBeyondViewport"] = serde_json::json!(true);
    }
  }

  let result = cdp::send(ws_url, "Page.captureScreenshot", params).await?;
  let data = result
    .get("data")
    .and_then(|v| v.as_str())
    .ok_or_else(|| "Screenshot returned no data".to_string())?;
  let bytes = base64::engine::general_purpose::STANDARD
    .decode(data)
    .map_err(|e| format!("Failed to decode screenshot: {e}"))?;

  std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create screenshot dir: {e}"))?;
  let path = dir.join(format!(
    "{}-step{}-{}.png",
    sanitize_filename(profile_name),
    step_index + 1,
    now_secs()
  ));
  std::fs::write(&path, bytes).map_err(|e| format!("Failed to write screenshot: {e}"))?;

  Ok(path)
}

fn sanitize_filename(name: &str) -> String {
  name
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
        c
      } else {
        '_'
      }
    })
    .collect()
}

/// Inclusive random value in `[min, max]`.
fn random_in_range(min: u64, max: u64) -> u64 {
  if max <= min {
    return min;
  }
  let mut rng = rand::rng();
  rng.random_range(min..=max)
}

/// Sleep in short ticks so a cancel lands within a second even during a long
/// dwell. Returns `false` if the run was cancelled.
async fn sleep_cancellable(total: std::time::Duration, cancel: &Arc<AtomicBool>) -> bool {
  let tick = std::time::Duration::from_millis(500);
  let deadline = tokio::time::Instant::now() + total;
  loop {
    if cancel.load(Ordering::SeqCst) {
      return false;
    }
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
      return true;
    }
    tokio::time::sleep(remaining.min(tick)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn random_in_range_is_inclusive_and_handles_degenerate_ranges() {
    assert_eq!(random_in_range(5, 5), 5);
    assert_eq!(random_in_range(9, 3), 9);
    for _ in 0..200 {
      let value = random_in_range(2, 4);
      assert!((2..=4).contains(&value), "{value} out of range");
    }
  }

  #[test]
  fn sanitize_filename_strips_path_separators() {
    assert_eq!(sanitize_filename("../my profile/1"), "___my_profile_1");
    assert_eq!(sanitize_filename("Keep-me_9"), "Keep-me_9");
  }

  #[test]
  fn a_fresh_cursor_starts_inside_the_viewport() {
    for _ in 0..100 {
      let cursor = PageCursor::new(1280.0, 800.0);
      assert!(
        cursor.position.x > 0.0 && cursor.position.x < 1280.0,
        "x {} outside viewport",
        cursor.position.x
      );
      assert!(
        cursor.position.y > 0.0 && cursor.position.y < 800.0,
        "y {} outside viewport",
        cursor.position.y
      );
    }
  }

  #[test]
  fn wheel_ticks_all_point_the_requested_way() {
    for _ in 0..20 {
      assert!(
        wheel_ticks(ScrollDirection::Down, 6, 800.0)
          .iter()
          .all(|d| *d > 0.0),
        "down must scroll down"
      );
      assert!(
        wheel_ticks(ScrollDirection::Up, 6, 800.0)
          .iter()
          .all(|d| *d < 0.0),
        "up must scroll up"
      );
    }
  }

  #[test]
  fn random_direction_eventually_produces_both_ways() {
    let ticks = wheel_ticks(ScrollDirection::Random, 200, 800.0);
    assert!(ticks.iter().any(|d| *d > 0.0), "no downward tick");
    assert!(ticks.iter().any(|d| *d < 0.0), "no upward tick");
  }

  #[test]
  fn wheel_tick_count_matches_the_request() {
    assert_eq!(wheel_ticks(ScrollDirection::Down, 0, 800.0).len(), 0);
    assert_eq!(wheel_ticks(ScrollDirection::Down, 7, 800.0).len(), 7);
  }

  #[test]
  fn the_link_picker_js_receives_every_placeholder() {
    // A missed placeholder would reach the page as a literal `__TRIED__` and
    // throw at evaluation time rather than failing here.
    let filled = PICK_LINK_JS
      .replace("__SAME_DOMAIN__", "true")
      .replace("__EXCLUDES__", "[]")
      .replace("__TRIED__", "[]");
    for placeholder in ["__SAME_DOMAIN__", "__EXCLUDES__", "__TRIED__"] {
      assert!(
        !filled.contains(placeholder),
        "{placeholder} was left unsubstituted in the link picker JS"
      );
    }
  }

  #[test]
  fn the_link_scripts_never_use_scroll_into_view() {
    // Scrolling must go through wheel events; scrollIntoView jumps the page with
    // no input behind it, which is exactly the tell we removed.
    for js in [PICK_LINK_JS, MEASURE_LINK_JS] {
      assert!(
        !js.contains("scrollIntoView"),
        "link scripts must not jump the scroll position"
      );
    }
  }

  #[test]
  fn the_measure_script_hit_tests_before_we_click() {
    assert!(
      MEASURE_LINK_JS.contains("elementFromPoint"),
      "the click target must be hit-tested so overlays are caught"
    );
  }

  fn run_request_json(value: serde_json::Value) -> Result<RunRequest, serde_json::Error> {
    serde_json::from_value(value)
  }

  #[test]
  fn a_group_only_request_deserializes_without_profile_ids() {
    let request = run_request_json(serde_json::json!({
      "scenario_id": "s",
      "group_id": "g"
    }))
    .expect("group_id alone must be a valid target");
    assert!(request.profile_ids.is_empty());
    assert_eq!(request.group_id.as_deref(), Some("g"));
    assert_eq!(request.concurrency, 1);
  }

  #[test]
  fn an_individual_request_deserializes_without_a_group() {
    let request = run_request_json(serde_json::json!({
      "scenario_id": "s",
      "profile_ids": ["a", "b"]
    }))
    .expect("profile_ids alone must be a valid target");
    assert_eq!(request.profile_ids, vec!["a".to_string(), "b".to_string()]);
    assert!(request.group_id.is_none());
  }

  #[test]
  fn profile_ids_and_group_id_can_be_combined() {
    let request = run_request_json(serde_json::json!({
      "scenario_id": "s",
      "profile_ids": ["a"],
      "group_id": "g"
    }))
    .expect("both targets together must be accepted");
    assert_eq!(request.profile_ids, vec!["a".to_string()]);
    assert_eq!(request.group_id.as_deref(), Some("g"));
  }

  #[test]
  fn a_request_naming_neither_profiles_nor_a_group_is_rejected() {
    let request = run_request_json(serde_json::json!({ "scenario_id": "s" })).unwrap();
    let err = validate_request(&request).unwrap_err();
    assert!(err.contains("AUTOMATION_NO_PROFILES_SELECTED"));
  }

  #[test]
  fn either_target_alone_passes_validation() {
    for target in [
      serde_json::json!({ "scenario_id": "s", "profile_ids": ["a"] }),
      serde_json::json!({ "scenario_id": "s", "group_id": "g" }),
    ] {
      let request = run_request_json(target).unwrap();
      validate_request(&request).expect("a single target kind is enough");
    }
  }

  #[test]
  fn concurrency_outside_the_allowed_range_is_rejected() {
    for concurrency in [0, MAX_CONCURRENCY + 1] {
      let request = run_request_json(serde_json::json!({
        "scenario_id": "s",
        "group_id": "g",
        "concurrency": concurrency
      }))
      .unwrap();
      let err = validate_request(&request).unwrap_err();
      assert!(
        err.contains("AUTOMATION_CONCURRENCY_INVALID"),
        "concurrency {concurrency} should be rejected"
      );
    }
  }

  #[test]
  fn an_inverted_jitter_range_is_rejected() {
    let request = run_request_json(serde_json::json!({
      "scenario_id": "s",
      "group_id": "g",
      "jitter_min_secs": 90,
      "jitter_max_secs": 30
    }))
    .unwrap();
    let err = validate_request(&request).unwrap_err();
    assert!(err.contains("AUTOMATION_JITTER_RANGE_INVALID"));
  }

  #[tokio::test]
  async fn sleep_cancellable_returns_false_once_cancelled() {
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(!sleep_cancellable(std::time::Duration::from_secs(60), &cancel).await);
  }

  #[tokio::test]
  async fn sleep_cancellable_completes_short_waits() {
    let cancel = Arc::new(AtomicBool::new(false));
    assert!(sleep_cancellable(std::time::Duration::from_millis(20), &cancel).await);
  }

  #[tokio::test]
  async fn sleep_cancellable_wakes_when_flag_flips_mid_sleep() {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(100)).await;
      flag.store(true, Ordering::SeqCst);
    });
    let started = std::time::Instant::now();
    assert!(!sleep_cancellable(std::time::Duration::from_secs(30), &cancel).await);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
  }

  fn run_with(statuses: &[ProfileRunStatus]) -> AutomationRun {
    AutomationRun {
      id: "run".to_string(),
      scenario_id: "s".to_string(),
      scenario_name: "S".to_string(),
      status: RunStatus::Running,
      created_at: 0,
      finished_at: None,
      concurrency: 1,
      profiles: statuses
        .iter()
        .enumerate()
        .map(|(i, status)| ProfileRun {
          profile_id: i.to_string(),
          profile_name: format!("p{i}"),
          status: *status,
          current_step_index: None,
          current_step_kind: None,
          total_steps: 3,
          error: None,
          started_at: None,
          finished_at: None,
          screenshots: Vec::new(),
          waiting_until: None,
        })
        .collect(),
    }
  }

  #[test]
  fn run_stays_running_until_every_profile_settles() {
    let mut run = run_with(&[ProfileRunStatus::Completed, ProfileRunStatus::Running]);
    run.recompute_status();
    assert_eq!(run.status, RunStatus::Running);
    assert!(run.finished_at.is_none());
  }

  #[test]
  fn run_completes_when_all_profiles_finish() {
    let mut run = run_with(&[ProfileRunStatus::Completed, ProfileRunStatus::Failed]);
    run.recompute_status();
    assert_eq!(run.status, RunStatus::Completed);
    assert!(run.finished_at.is_some());
  }

  #[test]
  fn any_cancelled_profile_marks_the_run_cancelled() {
    let mut run = run_with(&[ProfileRunStatus::Completed, ProfileRunStatus::Cancelled]);
    run.recompute_status();
    assert_eq!(run.status, RunStatus::Cancelled);
  }

  #[test]
  fn cancelling_an_unknown_run_reports_not_found() {
    let err = cancel_run("does-not-exist").unwrap_err();
    assert!(err.contains("AUTOMATION_RUN_NOT_FOUND"));
  }
}
