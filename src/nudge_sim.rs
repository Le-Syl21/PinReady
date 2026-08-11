//! VPX's nudge chain, ported so the wizard can be shaken and believed.
//!
//! Between the accelerometer and the tilt plumb, VPX does rather more than
//! scale a number, and what it does depends on the sensor *type*:
//!
//! - **Cabinet Sensor** — the measured acceleration is smoothed and applied
//!   as-is, scaled by the strength factor and by the ratio of the real
//!   cabinet's weight to the simulated one.
//! - **Intent Sensor** — the acceleration is not passed on at all. Peaks are
//!   detected, oscillation echoes are discarded, and above 1 m/s² a calibrated
//!   25 ms half-cosine impulse is injected instead. This is what makes it
//!   forgiving of noisy USB sensors, and why a shake that never reaches the
//!   threshold does nothing whatsoever.
//!
//! Either way the result drives a damped harmonic model of the cabinet, whose
//! acceleration is what finally reaches the plumb. Simulating one and showing
//! the other would make the tilt page a decoration.
//!
//! Ported from `src/physics/cabinet/` (GPLv3+): `NudgeIntentHandler.cpp`,
//! `CabinetPhysics.cpp`, `DampedHarmonicOscillator.h`.

/// Cabinet mass used by the simulated model, and VPX's default for the real
/// one (`Mapping.NudgeN.CabWeight`).
const MODEL_MASS_KG: f32 = 113.0;
/// VPX's default for `Mapping.NudgeN.CabWeight`, used when the ini leaves it
/// empty — which it does unless someone weighed their cabinet.
pub const DEFAULT_CAB_WEIGHT_KG: f32 = 113.0;
/// Oscillator tuning, "calibrated on real cabinets (from CFTBL to King Kong)".
const OSC_X: (f32, f32) = (9.3, 0.052);
const OSC_Y: (f32, f32) = (5.8, 0.055);
const DT: f32 = 0.001;

/// One axis of the cabinet's springy response to a force.
#[derive(Debug, Clone, Copy)]
struct Oscillator {
    k: f32,
    damping: f32,
    velocity: f32,
    displacement: f32,
    acceleration: f32,
}

impl Oscillator {
    fn new(freq: f32, zeta: f32) -> Self {
        let omega0 = std::f32::consts::TAU * freq;
        Self {
            k: MODEL_MASS_KG * omega0 * omega0,
            damping: 2.0 * zeta * (MODEL_MASS_KG * omega0),
            velocity: 0.0,
            displacement: 0.0,
            acceleration: 0.0,
        }
    }

    fn step(&mut self, force: f32) {
        self.acceleration =
            (force - self.damping * self.velocity - self.k * self.displacement) / MODEL_MASS_KG;
        self.velocity += self.acceleration * DT;
        self.displacement += self.velocity * DT;
    }
}

/// Peak detector for the Intent Sensor: turns a messy acceleration trace into
/// the discrete "the player meant to nudge, this hard" events VPX acts on.
#[derive(Debug, Clone, Copy)]
struct IntentDetector {
    time: i32,
    segment_is_peak: bool,
    segment_strength: f32,
    segment_start: i32,
    segment_end: i32,
    segment_impulse_sent: bool,
    last_impulse_time: i32,
    last_impulse_strength: f32,
    impulse: (f32, f32),
    impulse_elapsed: i32,
    impulse_delay: i32,
}

/// Impulse length in ms — physical nudges are this short.
const IMPULSE_LENGTH: i32 = 25;
/// Below this the shake is not treated as an intent at all.
const IMPULSE_THRESHOLD: f32 = 1.0;
/// Same value, exposed so the UI can draw where it falls on a sensor gauge.
pub const INTENT_THRESHOLD_MS2: f32 = IMPULSE_THRESHOLD;
/// 0.5 g is a firm nudge; at that point no delay is added.
const NO_DELAY_STRENGTH: f32 = 5.0;
/// Sensor acquisition period assumed for the delay, in ms.
const SENSOR_POLL_MS: f32 = 8.0;

impl Default for IntentDetector {
    fn default() -> Self {
        Self {
            time: 0,
            segment_is_peak: false,
            segment_strength: 0.0,
            segment_start: 0,
            segment_end: 0,
            segment_impulse_sent: false,
            last_impulse_time: 0,
            last_impulse_strength: 0.0,
            impulse: (0.0, 0.0),
            impulse_elapsed: IMPULSE_LENGTH + 1,
            impulse_delay: 0,
        }
    }
}

impl IntentDetector {
    fn impulse_in_progress(&self) -> bool {
        self.impulse_elapsed <= self.impulse_length_with_delay()
    }

    fn impulse_length_with_delay(&self) -> i32 {
        IMPULSE_LENGTH + self.impulse_delay
    }

    /// Acceleration the impulse contributes right now: a half cosine, so it
    /// rises and falls the way a real knock does.
    fn impulse_acceleration(&self) -> (f32, f32) {
        if !self.impulse_in_progress() || self.impulse_elapsed < self.impulse_delay {
            return (0.0, 0.0);
        }
        let t = (self.impulse_elapsed - self.impulse_delay) as f32 / IMPULSE_LENGTH as f32;
        let shape = 0.5 * (1.0 - (std::f32::consts::TAU * t).cos());
        (self.impulse.0 * shape, self.impulse.1 * shape)
    }

    /// Send the impulse early, the more so the harder the nudge: a sensor
    /// polled every 8 ms would otherwise miss the apex of a 25 ms event.
    fn impulse_delay_for(strength: f32) -> i32 {
        let firmness = (strength / NO_DELAY_STRENGTH).clamp(0.0, 1.0);
        (3.0 * SENSOR_POLL_MS + (SENSOR_POLL_MS - 3.0 * SENSOR_POLL_MS) * firmness) as i32
    }

    fn evaluate_impulse(&mut self, nudge: (f32, f32)) {
        // Discard the smaller peaks that follow a strong one: that is the
        // cabinet ringing, not the player nudging again.
        if self.segment_strength <= self.last_impulse_strength
            && self.segment_end - self.last_impulse_time <= 300
        {
            return;
        }
        if self.segment_strength > IMPULSE_THRESHOLD {
            self.impulse = nudge;
            self.impulse_elapsed = 0;
            self.impulse_delay = Self::impulse_delay_for(self.segment_strength);
            self.segment_impulse_sent = true;
        }
    }

    fn step(&mut self, accel: (f32, f32)) {
        self.impulse_elapsed += 1;
        self.time += 1;

        // A front nudge is always a push, never a pull.
        let nudge = (accel.0, accel.1.min(0.0));
        let strength = (nudge.0 * nudge.0 + nudge.1 * nudge.1).sqrt();

        if self.segment_is_peak {
            if strength > self.segment_strength {
                self.segment_strength = strength;
                self.segment_end = self.time;
                if !self.segment_impulse_sent {
                    self.evaluate_impulse(nudge);
                } else if self.impulse_in_progress() {
                    // The nudge grew while the impulse was still building —
                    // follow it rather than under-report the shot.
                    let new_strength_sqr = nudge.0 * nudge.0 + nudge.1 * nudge.1;
                    let current_sqr =
                        self.impulse.0 * self.impulse.0 + self.impulse.1 * self.impulse.1;
                    if new_strength_sqr > current_sqr {
                        self.impulse = nudge;
                        if self.impulse_elapsed < self.impulse_delay {
                            self.impulse_delay = self
                                .impulse_elapsed
                                .max(Self::impulse_delay_for(new_strength_sqr.sqrt()));
                        }
                    }
                }
            } else if strength < self.segment_strength * 0.9 {
                // Peak over; remember it so its echoes can be ignored.
                self.last_impulse_time = self.segment_end;
                self.last_impulse_strength = self.segment_strength;
                self.segment_strength = strength;
                self.segment_start = self.time;
                self.segment_end = self.time;
                self.segment_is_peak = false;
            }
        } else if strength < self.segment_strength {
            self.segment_strength = strength;
            self.segment_end = self.time;
        } else if strength > 0.1_f32.max(self.segment_strength * 1.1) {
            self.segment_strength = strength;
            self.segment_start = self.time;
            self.segment_end = self.time;
            self.segment_is_peak = true;
            self.segment_impulse_sent = false;
            self.evaluate_impulse(nudge);
        }
    }
}

/// The whole chain: sensor acceleration in, cabinet acceleration out.
#[derive(Debug, Clone, Copy)]
pub struct NudgeSim {
    osc_x: Oscillator,
    osc_y: Oscillator,
    intent: IntentDetector,
    /// The state estimator VPX always applies, whatever the mode: it removes
    /// the sensor's bias so a board mounted a degree off level doesn't read
    /// as a permanent shove.
    kalman_x: crate::motion_kalman::MotionKalmanAxis,
    kalman_y: crate::motion_kalman::MotionKalmanAxis,
    /// Consecutive samples spent below the rest threshold, per axis.
    rest_count: (i32, i32),
    /// Exponential smoothing used by the Cabinet Sensor path.
    ema: (f32, f32),
    cabinet_acceleration: (f32, f32),
}

impl Default for NudgeSim {
    fn default() -> Self {
        Self {
            osc_x: Oscillator::new(OSC_X.0, OSC_X.1),
            osc_y: Oscillator::new(OSC_Y.0, OSC_Y.1),
            intent: IntentDetector::default(),
            kalman_x: crate::motion_kalman::MotionKalmanAxis::default(),
            kalman_y: crate::motion_kalman::MotionKalmanAxis::default(),
            rest_count: (0, 0),
            ema: (0.0, 0.0),
            cabinet_acceleration: (0.0, 0.0),
        }
    }
}

impl NudgeSim {
    /// Advance one millisecond.
    ///
    /// `accel` is the sensor reading already converted to m/s², `strength` the
    /// per-sensor factor, `cab_weight` the real cabinet's mass in kg, and
    /// `sensor_type` VPX's `Mapping.NudgeN.Type` (0 game controller,
    /// 1 intent, 2 cabinet).
    pub fn step(&mut self, accel: (f32, f32), strength: f32, cab_weight: f32, sensor_type: i32) {
        // The filter comes first and always, in both modes. Rest constraints
        // are applied once an axis has been quiet long enough — that is what
        // lets it tell a constant offset from a slow genuine drift.
        const REST_THRESHOLD_MS2: f32 = 0.1;
        const REST_SAMPLES: i32 = 275;
        self.kalman_x.predict(DT);
        self.kalman_y.predict(DT);
        self.kalman_x.update_acceleration(accel.0);
        self.kalman_y.update_acceleration(accel.1);
        self.rest_count.0 = if accel.0.abs() < REST_THRESHOLD_MS2 {
            self.rest_count.0 + 1
        } else {
            0
        };
        self.rest_count.1 = if accel.1.abs() < REST_THRESHOLD_MS2 {
            self.rest_count.1 + 1
        } else {
            0
        };
        if self.rest_count.0 > REST_SAMPLES {
            self.kalman_x.update_rest();
        }
        if self.rest_count.1 > REST_SAMPLES {
            self.kalman_y.update_rest();
        }
        let accel = (self.kalman_x.acceleration(), self.kalman_y.acceleration());

        match sensor_type {
            // Cabinet Sensor: the measurement drives the cabinet directly,
            // smoothed, and rescaled from the real cabinet's mass to the
            // simulated one.
            2 => {
                let alpha = DT / 0.004; // EMA time constant of 4 ms
                self.ema.0 += (accel.0 - self.ema.0) * alpha.min(1.0);
                self.ema.1 += (accel.1 - self.ema.1) * alpha.min(1.0);
                let mass_ratio = cab_weight / MODEL_MASS_KG;
                let a = (
                    self.ema.0 * strength * mass_ratio,
                    self.ema.1 * strength * mass_ratio,
                );
                self.osc_x.step(MODEL_MASS_KG * a.0);
                self.osc_y.step(MODEL_MASS_KG * a.1);
            }
            // Intent Sensor (and the gamepad path, which differs only in how
            // its raw value is obtained): peaks become impulses.
            _ => {
                // Front/side energy balance, empirical in VPX too.
                self.intent
                    .step((accel.0 * strength * (4.0 / 3.0), accel.1 * strength));
                let impulse = self.intent.impulse_acceleration();
                if self.intent.impulse_in_progress() {
                    self.osc_x.step(MODEL_MASS_KG * impulse.0);
                    self.osc_y.step(MODEL_MASS_KG * impulse.1);
                } else {
                    self.osc_x.step(0.0);
                    self.osc_y.step(0.0);
                }
            }
        }
        self.cabinet_acceleration = (self.osc_x.acceleration, self.osc_y.acceleration);
    }

    /// What the plumb — and the ball — actually receive.
    pub fn cabinet_acceleration(&self) -> (f32, f32) {
        self.cabinet_acceleration
    }

    /// Whether an intent impulse is being delivered right now, so the UI can
    /// show that a shake was *recognised* even before the plumb reacts —
    /// the difference between "too weak to count" and "counted, but the
    /// plumb hasn't swung yet".
    pub fn impulse_active(&self) -> bool {
        self.intent.impulse_in_progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a knock of `ms` milliseconds at `accel` m/s², then let it settle.
    fn knock(sim: &mut NudgeSim, accel: f32, ms: usize, sensor_type: i32) -> f32 {
        let mut peak: f32 = 0.0;
        for i in 0..600 {
            let a = if i < ms { (accel, 0.0) } else { (0.0, 0.0) };
            sim.step(a, 1.0, 113.0, sensor_type);
            peak = peak.max(sim.cabinet_acceleration().0.abs());
        }
        peak
    }

    #[test]
    fn intent_ignores_what_stays_under_the_threshold() {
        // 0.5 m/s² is half the intent threshold: nothing should come out.
        let mut sim = NudgeSim::default();
        let peak = knock(&mut sim, 0.5, 20, 1);
        assert!(peak < 0.01, "leaked {peak} m/s²");
    }

    #[test]
    fn intent_answers_a_firm_knock() {
        let mut sim = NudgeSim::default();
        let peak = knock(&mut sim, 6.0, 20, 1);
        assert!(peak > 0.5, "only got {peak} m/s²");
    }

    #[test]
    fn cabinet_mode_passes_small_shakes_through() {
        // The same sub-threshold shake that intent discards does reach the
        // cabinet here — the whole difference between the two modes.
        let mut sim = NudgeSim::default();
        let peak = knock(&mut sim, 0.5, 20, 2);
        assert!(peak > 0.01, "cabinet mode swallowed it: {peak}");
    }

    #[test]
    fn the_cabinet_rings_and_settles() {
        let mut sim = NudgeSim::default();
        knock(&mut sim, 6.0, 20, 2);
        for _ in 0..2000 {
            sim.step((0.0, 0.0), 1.0, 113.0, 2);
        }
        let resting = sim.cabinet_acceleration().0.abs();
        assert!(resting < 0.05, "still moving at {resting} m/s²");
    }
}

/// Widest angle the plumb reaches for a 100 ms shove of `sensor_accel`
/// (m/s², as read by the sensor), running the whole chain: dead zone already
/// applied by the caller, then Kalman, then the mode's own handling, then the
/// cabinet oscillator, then the pendulum.
///
/// This single number answers both questions the wizard asks — how hard a
/// shove a given tilt threshold needs, and how far a given sensor range can
/// still tilt — so neither needs a nested search.
pub fn peak_tilt_angle(
    sensor_accel: f32,
    sensor_type: i32,
    strength: f32,
    cab_weight: f32,
    damping: f32,
) -> f32 {
    // A threshold no shove reaches, so the plumb is never pinned and reports
    // the angle it would have swung to.
    const UNREACHABLE_DEG: f32 = 90.0;
    // The estimator takes the very first sample it sees as bias ("idle-start
    // assumption", MotionKalmanAxis::UpdateAcceleration), and only trusts an
    // axis as being at rest after 275 quiet samples. Shoving from the first
    // millisecond would therefore book the whole knock as a sensor offset and
    // report a cabinet that barely moved. A real sensor idles first, so idle
    // here too.
    const SETTLE_MS: usize = 400;
    // 1.5 s is well past the peak of a 100 ms shove on a 9 Hz cabinet.
    const DURATION_MS: usize = SETTLE_MS + 1500;
    const SHOVE_MS: usize = SETTLE_MS + 100;

    let mut sim = NudgeSim::default();
    let mut plumb = crate::plumb::Plumb::default();
    let mut peak = 0.0_f32;
    for i in 0..DURATION_MS {
        let accel = if (SETTLE_MS..SHOVE_MS).contains(&i) {
            (sensor_accel, 0.0)
        } else {
            (0.0, 0.0)
        };
        sim.step(accel, strength, cab_weight, sensor_type);
        plumb.step(sim.cabinet_acceleration(), damping, UNREACHABLE_DEG);
        let angle = plumb.angle_deg();
        if !angle.is_finite() {
            // A shove far past any real sensor's range destabilises the 1 kHz
            // integration — the same integration VPX runs, which simply never
            // meets such a value in play. Report the rod laid flat rather than
            // the angle reached just before it blew up: a shove that violent
            // passes every threshold, so answering "barely moved" would be
            // exactly backwards.
            return 180.0;
        }
        peak = peak.max(angle);
    }
    peak
}

/// Sensor acceleration, in m/s², a 100 ms shove must reach for the plumb to
/// pass `threshold_deg`; infinite when no shove within a sensor's reach does.
///
/// This is what places the tilt ring on the wizard's dial: the ring is drawn
/// where the shove has to land, so the dot crossing it means the table tilts.
pub fn sensor_accel_for_tilt(
    threshold_deg: f32,
    sensor_type: i32,
    strength: f32,
    cab_weight: f32,
    damping: f32,
) -> f32 {
    let reaches =
        |a: f32| peak_tilt_angle(a, sensor_type, strength, cab_weight, damping) >= threshold_deg;
    // Climb rather than bisect from the top: past a sensor's range the
    // integration destabilises, so probing the high bound first would answer
    // "never" for a threshold a much gentler shove reaches. 100 m/s² is 10 g,
    // beyond every cabinet sensor's full scale.
    let mut lo = 0.0_f32;
    let mut hi = f32::INFINITY;
    let mut probe = 0.05_f32;
    while probe < 100.0 {
        if reaches(probe) {
            hi = probe;
            break;
        }
        lo = probe;
        probe *= 1.2;
    }
    if !hi.is_finite() {
        return f32::INFINITY;
    }
    for _ in 0..14 {
        let mid = 0.5 * (lo + hi);
        if reaches(mid) {
            hi = mid
        } else {
            lo = mid
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod settle_tests {
    use super::*;

    /// The estimator books its very first sample as sensor bias, so a shove
    /// starting on the first millisecond is read as an offset. The reading
    /// does not merely shrink — it stops tracking the shove altogether, and
    /// the harder the knock the worse it gets. Every measurement here idles
    /// first; this test exists because getting it wrong produces numbers that
    /// look plausible and are off by two orders of magnitude.
    #[test]
    fn a_hard_shove_from_a_cold_start_never_reaches_the_cabinet() {
        let cold = {
            let mut sim = NudgeSim::default();
            let mut peak = 0.0_f32;
            for i in 0..1500 {
                let a = if i < 100 { (70.0, 0.0) } else { (0.0, 0.0) };
                sim.step(a, 1.0, DEFAULT_CAB_WEIGHT_KG, 1);
                peak = peak.max(sim.cabinet_acceleration().0.abs());
            }
            peak
        };
        assert!(
            cold < 5.0,
            "a 70 m/s² shove read as bias should barely move the cabinet, got {cold}"
        );
        // The same shove, once the sensor has idled, lays the plumb well past
        // any threshold VPX offers.
        let settled = peak_tilt_angle(70.0, 1, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0);
        assert!(settled > 4.0, "a settled shove should tilt, got {settled}");
    }

    /// Harder shoves tilt further, across every range a cabinet sensor offers.
    #[test]
    fn peak_angle_grows_with_the_shove() {
        for mode in [1, 2] {
            let mut previous = 0.0;
            for accel in [1.0_f32, 2.5, 5.0, 10.0, 20.0, 40.0, 78.5] {
                let angle = peak_tilt_angle(accel, mode, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0);
                assert!(
                    angle >= previous,
                    "mode {mode}: {accel} m/s² gave {angle}°, below the {previous}° of a softer shove"
                );
                previous = angle;
            }
        }
    }

    /// Strength divides the shove needed, which is what lets the wizard offer
    /// it as the fix when a tilt threshold sits out of the sensor's reach.
    #[test]
    fn strength_scales_the_shove_needed_inversely() {
        let at_one = sensor_accel_for_tilt(1.1125, 1, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0);
        let at_two = sensor_accel_for_tilt(1.1125, 1, 2.0, DEFAULT_CAB_WEIGHT_KG, 1.0);
        let ratio = at_one / at_two;
        assert!((ratio - 2.0).abs() < 0.15, "expected ~2x, got {ratio}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Not an assertion — a bench printing what each tilt threshold costs, so
    /// the numbers quoted in the UI can be re-derived after any port change.
    /// `cargo test -- --ignored --nocapture bench`
    #[test]
    #[ignore]
    fn tilt_thresholds_in_sensor_acceleration() {
        for (mode, name) in [(1, "intent"), (2, "cabinet")] {
            println!("\n=== {name} (strength 1.0, damping 1.0) ===");
            println!("  angle    m/s²      g    % of ±1g   % of ±2g");
            for deg in [0.15_f32, 1.0, 1.1125, 2.075, 4.0] {
                let a = sensor_accel_for_tilt(deg, mode, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0);
                println!(
                    "  {deg:5.2}° {a:7.2} {:6.2} {:9.0} % {:9.0} %",
                    a / 9.80665,
                    100.0 * a / 9.80665,
                    100.0 * a / (2.0 * 9.80665)
                );
            }
        }
        println!("\n=== intent, effect of strength on the 1.11° ring ===");
        for s in [0.5_f32, 1.0, 1.5, 2.0] {
            let a = sensor_accel_for_tilt(1.1125, 1, s, DEFAULT_CAB_WEIGHT_KG, 1.0);
            println!(
                "  strength {s:.1} → {a:6.2} m/s² = {:3.0} % of ±1g",
                100.0 * a / 9.80665
            );
        }
    }
    /// Reach of each sensor range: the widest tilt a full-scale shove still
    /// trips, and the narrowest one that the first shove clearing the filters
    /// already trips — the two ends of the usable tilt slider.
    #[test]
    #[ignore]
    fn reachable_tilt_range_per_sensor_range() {
        const G: f32 = 9.80665;
        for (mode, name) in [(1, "intent"), (2, "cabinet")] {
            println!("\n=== {name} ===");
            for range in [1.0_f32, 2.0, 4.0, 8.0] {
                let full = range * G;
                let dead = 0.03 * full;
                let floor = if mode == 1 {
                    dead.max(INTENT_THRESHOLD_MS2)
                } else {
                    dead
                };
                let ceiling =
                    peak_tilt_angle(full, mode, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0).clamp(0.15, 4.0);
                let bottom =
                    peak_tilt_angle(floor, mode, 1.0, DEFAULT_CAB_WEIGHT_KG, 1.0).clamp(0.15, 4.0);
                println!(
                    "  ±{range:.0} g  full scale {full:5.1} m/s²  min tilt {bottom:.2}°  max tilt {ceiling:.2}°"
                );
            }
        }
    }
}
