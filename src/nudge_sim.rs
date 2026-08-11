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
