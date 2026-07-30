//! Human-like mouse movement, the pointer counterpart to [`crate::human_typing`].
//!
//! Injected input already carries `isTrusted: true` because CDP dispatches it in
//! the browser process — what gives automation away is the *kinematics*. A
//! teleport straight to an element's geometric centre has infinite velocity, no
//! approach path, and lands on a pixel no hand ever hits. This module produces
//! the missing motion:
//!
//! - a cubic Bézier arc rather than a straight line, bowed by a random amount
//! - a bell-shaped speed profile (minimum-jerk), so the cursor accelerates,
//!   peaks, then decelerates into the target
//! - a duration that grows with distance, in the spirit of Fitts' law
//! - an occasional overshoot followed by a short corrective sub-movement
//! - sub-pixel tremor on the way, because hands are never perfectly steady
//! - a landing point scattered inside the target instead of its exact centre
//!
//! Generation is synchronous and returns a plain `Vec`: `ThreadRng` is not
//! `Send`, so nothing here may be held across an `.await`. Callers dispatch the
//! result with `cdp::send_timed_sequence`.

use rand::RngExt;

use crate::human_typing::normal_sample;

/// Chance of overshooting a far target and correcting back.
const PROB_OVERSHOOT: f64 = 0.35;
/// Below this travel distance people don't overshoot in any measurable way.
const OVERSHOOT_MIN_DISTANCE: f64 = 120.0;
const OVERSHOOT_FRACTION_MIN: f64 = 0.02;
const OVERSHOOT_FRACTION_MAX: f64 = 0.09;
/// Gap between arriving past the target and starting the correction.
const CORRECTION_REACTION_MEAN: f64 = 0.09;
const CORRECTION_REACTION_STD: f64 = 0.03;
/// Corrective sub-movements are quicker than the initial ballistic one.
const CORRECTION_SPEED_FACTOR: f64 = 0.45;

const MOVE_BASE_SECS: f64 = 0.09;
const MOVE_DISTANCE_COEFF: f64 = 0.16;
/// Reference distance for the log term; roughly a comfortable target width.
const MOVE_DISTANCE_SCALE: f64 = 40.0;
const MOVE_TIME_STD_FRACTION: f64 = 0.14;
const MOVE_MIN_SECS: f64 = 0.10;
const MOVE_MAX_SECS: f64 = 1.60;

/// Real mice report at roughly 60–160 Hz.
const REPORT_INTERVAL_MS_MIN: f64 = 6.0;
const REPORT_INTERVAL_MS_MAX: f64 = 14.0;
const MAX_STEPS: usize = 400;

const TREMOR_STD_PX: f64 = 0.45;
const BOW_FRACTION_MIN: f64 = 0.04;
const BOW_FRACTION_MAX: f64 = 0.20;

const PRESS_HOLD_MEAN: f64 = 0.085;
const PRESS_HOLD_STD: f64 = 0.025;
/// People don't press the instant the cursor arrives.
const SETTLE_MEAN: f64 = 0.10;
const SETTLE_STD: f64 = 0.04;

/// Landing-point spread as a fraction of the target's half-extent.
const CLICK_POINT_SPREAD: f64 = 0.28;
/// Landing points stay within this inset of the target, so they never graze the
/// border where a stray pixel would miss.
const CLICK_POINT_INSET: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
  pub x: f64,
  pub y: f64,
}

impl Point {
  pub fn new(x: f64, y: f64) -> Self {
    Self { x, y }
  }

  pub fn distance_to(&self, other: Point) -> f64 {
    ((other.x - self.x).powi(2) + (other.y - self.y).powi(2)).sqrt()
  }
}

/// A cursor sample: where the pointer is, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseSample {
  /// Seconds from the start of the movement.
  pub time: f64,
  pub x: f64,
  pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

impl Rect {
  pub fn center(&self) -> Point {
    Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
  }
}

/// Pick where inside `rect` the click should land: Gaussian around the centre,
/// clamped to an inset so it can't graze the edge. Never the exact centre —
/// pixel-perfect centre hits are a giveaway.
pub fn sample_click_point(rect: Rect) -> Point {
  let mut rng = rand::rng();
  let center = rect.center();

  let sample = |rng: &mut rand::rngs::ThreadRng, center: f64, extent: f64| {
    if extent <= 0.0 {
      return center;
    }
    let half = extent / 2.0;
    let value = normal_sample(rng, center, half * CLICK_POINT_SPREAD);
    let low = center - half * (1.0 - 2.0 * CLICK_POINT_INSET);
    let high = center + half * (1.0 - 2.0 * CLICK_POINT_INSET);
    value.clamp(low.min(high), high.max(low))
  };

  Point::new(
    sample(&mut rng, center.x, rect.width),
    sample(&mut rng, center.y, rect.height),
  )
}

/// Build the cursor samples for a movement from `from` to `to`.
///
/// The final sample is exactly `to` so the caller knows where the pointer ended
/// up; tremor is applied only to intermediate samples.
pub fn generate_move(from: Point, to: Point) -> Vec<MouseSample> {
  let mut rng = rand::rng();
  let distance = from.distance_to(to);

  if distance < 1.0 {
    // Already there. Still emit the destination so the pointer position the
    // caller records matches what the page saw.
    return vec![MouseSample {
      time: 0.0,
      x: to.x,
      y: to.y,
    }];
  }

  let mut samples = Vec::new();
  let mut clock = 0.0;

  let overshoots = distance > OVERSHOOT_MIN_DISTANCE && rng.random::<f64>() < PROB_OVERSHOOT;

  if overshoots {
    let past = overshoot_point(&mut rng, from, to, distance);
    append_leg(&mut samples, &mut clock, &mut rng, from, past, false);
    clock += normal_sample(&mut rng, CORRECTION_REACTION_MEAN, CORRECTION_REACTION_STD).max(0.02);
    append_leg(&mut samples, &mut clock, &mut rng, past, to, true);
  } else {
    append_leg(&mut samples, &mut clock, &mut rng, from, to, false);
  }

  samples
}

/// Seconds to wait after arriving before pressing the button.
pub fn settle_before_press_secs() -> f64 {
  let mut rng = rand::rng();
  normal_sample(&mut rng, SETTLE_MEAN, SETTLE_STD).clamp(0.03, 0.30)
}

/// How long the button stays down.
pub fn press_hold_secs() -> f64 {
  let mut rng = rand::rng();
  normal_sample(&mut rng, PRESS_HOLD_MEAN, PRESS_HOLD_STD).clamp(0.03, 0.20)
}

/// A point a little past `to`, along the direction of travel — where a hand
/// carried by momentum actually stops before correcting back.
fn overshoot_point(
  rng: &mut rand::rngs::ThreadRng,
  from: Point,
  to: Point,
  distance: f64,
) -> Point {
  let fraction = rng.random_range(OVERSHOOT_FRACTION_MIN..OVERSHOOT_FRACTION_MAX);
  let dx = (to.x - from.x) / distance;
  let dy = (to.y - from.y) / distance;
  let extra = distance * fraction;
  // Overshoots aren't purely along the line; add a little lateral error.
  let lateral = normal_sample(rng, 0.0, extra * 0.4);
  Point::new(
    to.x + dx * extra - dy * lateral,
    to.y + dy * extra + dx * lateral,
  )
}

/// Append one ballistic sub-movement. `is_correction` shortens it, matching how
/// corrective motions are faster and tighter than the initial throw.
fn append_leg(
  samples: &mut Vec<MouseSample>,
  clock: &mut f64,
  rng: &mut rand::rngs::ThreadRng,
  from: Point,
  to: Point,
  is_correction: bool,
) {
  let distance = from.distance_to(to);
  if distance <= 0.0 {
    return;
  }

  let mut duration =
    MOVE_BASE_SECS + MOVE_DISTANCE_COEFF * (1.0 + distance / MOVE_DISTANCE_SCALE).log2();
  if is_correction {
    duration *= CORRECTION_SPEED_FACTOR;
  }
  duration = normal_sample(rng, duration, duration * MOVE_TIME_STD_FRACTION)
    .clamp(MOVE_MIN_SECS, MOVE_MAX_SECS);

  let interval_ms = rng.random_range(REPORT_INTERVAL_MS_MIN..REPORT_INTERVAL_MS_MAX);
  let steps = ((duration * 1000.0 / interval_ms).round() as usize).clamp(2, MAX_STEPS);

  // Control points: pushed along the line and bowed to one side, so the path
  // curves the way an arm swings instead of tracking a ruler.
  let dx = (to.x - from.x) / distance;
  let dy = (to.y - from.y) / distance;
  let bow_sign = if rng.random::<bool>() { 1.0 } else { -1.0 };
  let bow = distance * rng.random_range(BOW_FRACTION_MIN..BOW_FRACTION_MAX) * bow_sign;

  let control = |along: f64, side: f64| {
    Point::new(
      from.x + dx * distance * along - dy * bow * side,
      from.y + dy * distance * along + dx * bow * side,
    )
  };
  let c1 = control(rng.random_range(0.15..0.40), rng.random_range(0.6..1.0));
  let c2 = control(rng.random_range(0.60..0.88), rng.random_range(0.6..1.0));

  for step in 1..=steps {
    let tau = step as f64 / steps as f64;
    let eased = minimum_jerk(tau);
    let mut point = cubic_bezier(from, c1, c2, to, eased);

    let is_last = step == steps;
    if !is_last {
      point.x += normal_sample(rng, 0.0, TREMOR_STD_PX);
      point.y += normal_sample(rng, 0.0, TREMOR_STD_PX);
    } else {
      // Land exactly on the intended pixel.
      point = to;
    }

    let jitter = rng.random_range(0.85..1.15);
    *clock += (duration / steps as f64) * jitter;

    samples.push(MouseSample {
      time: *clock,
      x: point.x,
      y: point.y,
    });
  }
}

/// Minimum-jerk position profile. Its derivative is a bell curve, which is what
/// makes the motion accelerate and decelerate like a real reach.
fn minimum_jerk(tau: f64) -> f64 {
  let t = tau.clamp(0.0, 1.0);
  10.0 * t.powi(3) - 15.0 * t.powi(4) + 6.0 * t.powi(5)
}

fn cubic_bezier(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
  let u = 1.0 - t;
  let w0 = u * u * u;
  let w1 = 3.0 * u * u * t;
  let w2 = 3.0 * u * t * t;
  let w3 = t * t * t;
  Point::new(
    w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
    w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn step_distances(samples: &[MouseSample]) -> Vec<f64> {
    samples
      .windows(2)
      .map(|w| {
        let dx = w[1].x - w[0].x;
        let dy = w[1].y - w[0].y;
        (dx * dx + dy * dy).sqrt()
      })
      .collect()
  }

  fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
  }

  #[test]
  fn a_move_ends_exactly_on_the_target() {
    for _ in 0..50 {
      let from = Point::new(10.0, 20.0);
      let to = Point::new(640.0, 480.0);
      let samples = generate_move(from, to);
      let last = samples.last().unwrap();
      assert_eq!((last.x, last.y), (to.x, to.y));
    }
  }

  #[test]
  fn timestamps_are_monotonic_and_finite() {
    for _ in 0..50 {
      let samples = generate_move(Point::new(0.0, 0.0), Point::new(900.0, 300.0));
      let mut previous = -1.0;
      for sample in &samples {
        assert!(sample.time.is_finite(), "non-finite time");
        assert!(
          sample.x.is_finite() && sample.y.is_finite(),
          "non-finite position"
        );
        assert!(
          sample.time > previous,
          "time went backwards: {} after {previous}",
          sample.time
        );
        previous = sample.time;
      }
    }
  }

  #[test]
  fn a_zero_distance_move_emits_only_the_destination() {
    let target = Point::new(100.0, 100.0);
    let samples = generate_move(target, Point::new(100.2, 100.3));
    assert_eq!(samples.len(), 1);
    assert_eq!(
      (samples[0].x, samples[0].y),
      (target.x + 0.2, target.y + 0.3)
    );
  }

  #[test]
  fn a_move_is_never_a_single_teleport() {
    // The bug this module exists to fix: one mouseMoved straight to the target.
    for _ in 0..50 {
      let samples = generate_move(Point::new(0.0, 0.0), Point::new(400.0, 400.0));
      assert!(
        samples.len() >= 5,
        "expected a path, got {} sample(s)",
        samples.len()
      );
    }
  }

  #[test]
  fn farther_targets_take_longer_to_reach() {
    let near: Vec<f64> = (0..40)
      .map(|_| {
        generate_move(Point::new(0.0, 0.0), Point::new(50.0, 0.0))
          .last()
          .unwrap()
          .time
      })
      .collect();
    let far: Vec<f64> = (0..40)
      .map(|_| {
        generate_move(Point::new(0.0, 0.0), Point::new(1200.0, 0.0))
          .last()
          .unwrap()
          .time
      })
      .collect();
    assert!(
      median(far) > median(near),
      "distance should increase duration"
    );
  }

  #[test]
  fn the_path_bows_instead_of_tracking_the_straight_line() {
    let from = Point::new(0.0, 0.0);
    let to = Point::new(600.0, 0.0);
    // A straight horizontal move: any real deviation shows up as |y|.
    let deviations: Vec<f64> = (0..30)
      .map(|_| {
        generate_move(from, to)
          .iter()
          .map(|s| s.y.abs())
          .fold(0.0_f64, f64::max)
      })
      .collect();
    assert!(
      median(deviations) > 2.0,
      "path should curve away from the straight line"
    );
  }

  #[test]
  fn speed_peaks_mid_flight_rather_than_being_constant() {
    // Exercise a single leg so an overshoot's second leg can't blur the profile.
    let mut samples = Vec::new();
    let mut clock = 0.0;
    let mut rng = rand::rng();
    append_leg(
      &mut samples,
      &mut clock,
      &mut rng,
      Point::new(0.0, 0.0),
      Point::new(800.0, 0.0),
      false,
    );

    let steps = step_distances(&samples);
    let sixth = steps.len() / 6;
    assert!(sixth >= 2, "need enough samples to compare phases");

    let opening: f64 = steps[..sixth].iter().sum::<f64>() / sixth as f64;
    let middle_start = steps.len() / 2 - sixth / 2;
    let middle: f64 = steps[middle_start..middle_start + sixth]
      .iter()
      .sum::<f64>()
      / sixth as f64;

    assert!(
      middle > opening * 2.0,
      "expected a bell-shaped speed profile, opening={opening:.2} middle={middle:.2}"
    );
  }

  #[test]
  fn click_points_stay_inside_the_target() {
    let rect = Rect {
      x: 100.0,
      y: 200.0,
      width: 80.0,
      height: 24.0,
    };
    for _ in 0..500 {
      let point = sample_click_point(rect);
      assert!(
        point.x >= rect.x && point.x <= rect.x + rect.width,
        "x {} outside rect",
        point.x
      );
      assert!(
        point.y >= rect.y && point.y <= rect.y + rect.height,
        "y {} outside rect",
        point.y
      );
    }
  }

  #[test]
  fn click_points_are_not_always_the_exact_center() {
    let rect = Rect {
      x: 0.0,
      y: 0.0,
      width: 120.0,
      height: 40.0,
    };
    let center = rect.center();
    let scattered = (0..200)
      .map(|_| sample_click_point(rect))
      .filter(|p| (p.x - center.x).abs() > 0.5 || (p.y - center.y).abs() > 0.5)
      .count();
    assert!(
      scattered > 150,
      "landing points should scatter, only {scattered}/200 differed from centre"
    );
  }

  #[test]
  fn a_zero_sized_target_falls_back_to_its_center() {
    let rect = Rect {
      x: 42.0,
      y: 7.0,
      width: 0.0,
      height: 0.0,
    };
    let point = sample_click_point(rect);
    assert_eq!((point.x, point.y), (42.0, 7.0));
  }

  #[test]
  fn minimum_jerk_spans_zero_to_one_and_is_symmetric() {
    assert!((minimum_jerk(0.0) - 0.0).abs() < 1e-9);
    assert!((minimum_jerk(1.0) - 1.0).abs() < 1e-9);
    assert!((minimum_jerk(0.5) - 0.5).abs() < 1e-9);
    // Monotonic: position never goes backwards.
    let mut previous = -1.0;
    for step in 0..=100 {
      let value = minimum_jerk(step as f64 / 100.0);
      assert!(value >= previous, "profile is not monotonic");
      previous = value;
    }
  }

  #[test]
  fn press_timings_stay_in_a_human_range() {
    for _ in 0..200 {
      let hold = press_hold_secs();
      assert!((0.03..=0.20).contains(&hold), "hold {hold} out of range");
      let settle = settle_before_press_secs();
      assert!(
        (0.03..=0.30).contains(&settle),
        "settle {settle} out of range"
      );
    }
  }
}
