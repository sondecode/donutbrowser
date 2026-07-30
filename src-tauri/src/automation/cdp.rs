//! Shared Chrome DevTools Protocol client.
//!
//! Both the MCP server and the automation engine drive profiles over CDP, so the
//! transport lives here and each caller maps `String` errors into its own error
//! type. Only Chromium targets expose a CDP port — see
//! `crate::browser::is_chromium_target`.

use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::profile::BrowserProfile;
use crate::profile::ProfileManager;

/// Resolve the CDP port a running profile listens on.
///
/// Port info is written by the launcher, which can lag the process spawn, so
/// this retries for up to 10 seconds before giving up.
pub async fn cdp_port_for_profile(profile: &BrowserProfile) -> Result<u16, String> {
  if !crate::browser::is_chromium_target(&profile.browser) {
    return Err(format!(
      "Profile '{}' is not a Chromium profile, so it has no CDP endpoint",
      profile.name
    ));
  }

  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let profile_path = profile.get_profile_data_path(&profiles_dir);
  let profile_path_str = profile_path.to_string_lossy();

  for attempt in 0..10 {
    if attempt > 0 {
      tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if let Some(port) = crate::wayfern_manager::WayfernManager::instance()
      .get_cdp_port(&profile_path_str)
      .await
    {
      return Ok(port);
    }
  }

  Err(format!(
    "No CDP connection available for profile '{}'. Make sure the browser is running.",
    profile.name
  ))
}

/// Resolve the WebSocket debugger URL of the profile's first page target.
pub async fn ws_url_for_port(port: u16) -> Result<String, String> {
  let url = format!("http://127.0.0.1:{port}/json");
  let client = reqwest::Client::new();

  let max_attempts = 15;
  let mut last_err = String::new();
  for attempt in 0..max_attempts {
    if attempt > 0 {
      tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    match client
      .get(&url)
      .timeout(std::time::Duration::from_secs(3))
      .send()
      .await
    {
      Ok(resp) => match resp.json::<Vec<serde_json::Value>>().await {
        Ok(targets) => {
          if let Some(ws_url) = targets
            .iter()
            .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .and_then(|t| t.get("webSocketDebuggerUrl"))
            .and_then(|v| v.as_str())
          {
            return Ok(ws_url.to_string());
          }
          last_err = "No page target found in browser".to_string();
        }
        Err(e) => last_err = format!("Failed to parse CDP targets: {e}"),
      },
      Err(e) => last_err = format!("Failed to connect to browser CDP endpoint: {e}"),
    }
  }

  Err(last_err)
}

/// Convenience: resolve port then WebSocket URL for a running profile.
pub async fn ws_url_for_profile(profile: &BrowserProfile) -> Result<String, String> {
  let port = cdp_port_for_profile(profile).await?;
  ws_url_for_port(port).await
}

/// Send one CDP command and return its `result` object.
pub async fn send(
  ws_url: &str,
  method: &str,
  params: serde_json::Value,
) -> Result<serde_json::Value, String> {
  let (mut ws_stream, _) = connect_async(ws_url)
    .await
    .map_err(|e| format!("Failed to connect to CDP WebSocket: {e}"))?;

  let command = serde_json::json!({ "id": 1, "method": method, "params": params });

  ws_stream
    .send(Message::Text(command.to_string().into()))
    .await
    .map_err(|e| format!("Failed to send CDP command: {e}"))?;

  while let Some(msg) = ws_stream.next().await {
    let msg = msg.map_err(|e| format!("CDP WebSocket error: {e}"))?;
    if let Message::Text(text) = msg {
      let response: serde_json::Value = serde_json::from_str(text.as_str())
        .map_err(|e| format!("Failed to parse CDP response: {e}"))?;
      if response.get("id") == Some(&serde_json::json!(1)) {
        if let Some(error) = response.get("error") {
          return Err(format!("CDP error: {error}"));
        }
        return Ok(
          response
            .get("result")
            .cloned()
            .unwrap_or(serde_json::json!({})),
        );
      }
    }
  }

  Err("No response received from CDP".to_string())
}

/// Send a command and additionally wait for `Page.loadEventFired`.
///
/// Returns the command's own result. A load that never fires within
/// `timeout_secs` is not an error — the command result is returned anyway, since
/// pages with long-polling requests may never reach a quiet load event.
pub async fn send_and_wait_for_load(
  ws_url: &str,
  method: &str,
  params: serde_json::Value,
  timeout_secs: u64,
) -> Result<serde_json::Value, String> {
  let (mut ws_stream, _) = connect_async(ws_url)
    .await
    .map_err(|e| format!("Failed to connect to CDP WebSocket: {e}"))?;

  let enable_cmd = serde_json::json!({ "id": 1, "method": "Page.enable", "params": {} });
  ws_stream
    .send(Message::Text(enable_cmd.to_string().into()))
    .await
    .map_err(|e| format!("Failed to send Page.enable: {e}"))?;

  loop {
    let msg = ws_stream
      .next()
      .await
      .ok_or_else(|| "WebSocket closed waiting for Page.enable response".to_string())?
      .map_err(|e| format!("CDP WebSocket error: {e}"))?;
    if let Message::Text(text) = msg {
      let resp: serde_json::Value = serde_json::from_str(text.as_str()).unwrap_or_default();
      if resp.get("id") == Some(&serde_json::json!(1)) {
        break;
      }
    }
  }

  let command = serde_json::json!({ "id": 2, "method": method, "params": params });
  ws_stream
    .send(Message::Text(command.to_string().into()))
    .await
    .map_err(|e| format!("Failed to send CDP command: {e}"))?;

  let mut command_result = None;
  let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

  loop {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
      break;
    }

    let msg = match tokio::time::timeout(remaining, ws_stream.next()).await {
      Ok(Some(Ok(msg))) => msg,
      Ok(Some(Err(e))) => return Err(format!("CDP WebSocket error: {e}")),
      Ok(None) => break,
      Err(_) => break,
    };

    if let Message::Text(text) = msg {
      let response: serde_json::Value = serde_json::from_str(text.as_str()).unwrap_or_default();

      if response.get("id") == Some(&serde_json::json!(2)) {
        if let Some(error) = response.get("error") {
          return Err(format!("CDP error: {error}"));
        }
        command_result = Some(
          response
            .get("result")
            .cloned()
            .unwrap_or(serde_json::json!({})),
        );
      }

      if response.get("method") == Some(&serde_json::json!("Page.loadEventFired")) {
        break;
      }
    }
  }

  let disable_cmd = serde_json::json!({ "id": 3, "method": "Page.disable", "params": {} });
  let _ = ws_stream
    .send(Message::Text(disable_cmd.to_string().into()))
    .await;

  command_result.ok_or_else(|| "No response received from CDP".to_string())
}

/// One command scheduled at an offset from the sequence start.
#[derive(Debug, Clone)]
pub struct TimedCommand {
  /// Seconds after the sequence began.
  pub at: f64,
  pub method: String,
  pub params: serde_json::Value,
}

/// Stream a batch of commands over a **single** connection, honouring each
/// item's scheduled offset.
///
/// Input events exist to reproduce human timing, and `send` opens a fresh
/// WebSocket per call — a 40-event mouse path would spend far more time in
/// connection setup than in the intervals being simulated, destroying the very
/// cadence we generated. Offsets are scheduled against one absolute start
/// instant so send/drain latency can't accumulate into drift.
pub async fn send_timed_sequence(ws_url: &str, commands: &[TimedCommand]) -> Result<(), String> {
  if commands.is_empty() {
    return Ok(());
  }

  let (mut ws_stream, _) = connect_async(ws_url)
    .await
    .map_err(|e| format!("Failed to connect to CDP WebSocket: {e}"))?;

  let start = tokio::time::Instant::now();

  for (index, command) in commands.iter().enumerate() {
    tokio::time::sleep_until(start + std::time::Duration::from_secs_f64(command.at.max(0.0))).await;

    let payload = serde_json::json!({
      "id": index + 1,
      "method": command.method,
      "params": command.params,
    });

    ws_stream
      .send(Message::Text(payload.to_string().into()))
      .await
      .map_err(|e| format!("Failed to send {}: {e}", command.method))?;

    // Drain the acknowledgement so the socket doesn't back up. Guarded by a
    // timeout: a missing reply must not stall the whole sequence.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), ws_stream.next()).await;
  }

  Ok(())
}

/// Evaluate JS in the page and return the raw `Runtime.evaluate` result,
/// surfacing thrown exceptions as errors.
pub async fn evaluate(ws_url: &str, expression: &str) -> Result<serde_json::Value, String> {
  let result = send(
    ws_url,
    "Runtime.evaluate",
    serde_json::json!({
      "expression": expression,
      "returnByValue": true,
      "awaitPromise": true,
    }),
  )
  .await?;

  if let Some(exception) = result.get("exceptionDetails") {
    let msg = exception
      .get("exception")
      .and_then(|e| e.get("description"))
      .or_else(|| exception.get("text"))
      .and_then(|v| v.as_str())
      .unwrap_or("JavaScript evaluation failed");
    return Err(msg.to_string());
  }

  Ok(
    result
      .get("result")
      .and_then(|r| r.get("value"))
      .cloned()
      .unwrap_or(serde_json::json!(null)),
  )
}
