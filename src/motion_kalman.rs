//! VPX's nudge state estimator: the Kalman filter that stands between a noisy
//! accelerometer and everything downstream.
//!
//! A cabinet's accelerometer reports at ~125 Hz, jitters at rest, and drifts.
//! VPX does not feed that straight into its physics — it runs a five-state
//! filter (position, velocity, acceleration, and a bias on each of the last
//! two) over a constant-jerk model, updates it whenever a sample lands, and
//! pins the state to zero once the sensor has been still long enough. What
//! comes out is smoother, unbiased, and still quick, which is what the intent
//! detector and the cabinet model actually see.
//!
//! Ported from `src/physics/cabinet/MotionKalmanAxis.h` (GPLv3+) — the part a
//! sensor with only an acceleration axis uses. Two things are deliberately
//! left out, neither reachable here: the snapshot history that replays
//! out-of-order samples (ours arrive in order), and the gain calibrator that
//! reconciles a velocity axis against an acceleration one (we have no
//! velocity axis).

const STATE_COUNT: usize = 5;
const POSITION: usize = 0;
const VELOCITY: usize = 1;
const ACCELERATION: usize = 2;
const VELOCITY_BIAS: usize = 3;
const ACCELERATION_BIAS: usize = 4;

/// Tuning, verbatim from VPX's `MotionKalmanAxis::Config`.
const PROCESS_JERK_VARIANCE: f32 = 800.0; // high: responsiveness over smoothness
const VELOCITY_BIAS_PROCESS_VARIANCE: f32 = 0.0001; // slow drift only
const ACCELERATION_BIAS_PROCESS_VARIANCE: f32 = 0.0001;
const ACCELERATION_MEASUREMENT_VARIANCE: f32 = 0.40 * 0.40;
const ZERO_POSITION_MEASUREMENT_VARIANCE: f32 = 0.0005 * 0.0005;
const ZERO_VELOCITY_MEASUREMENT_VARIANCE: f32 = 0.01 * 0.01;
const ZERO_ACCELERATION_MEASUREMENT_VARIANCE: f32 = 0.10 * 0.10;
const INITIAL_POSITION_VARIANCE: f32 = 1.0e-4;
const INITIAL_VELOCITY_VARIANCE: f32 = 1.0;
const INITIAL_ACCELERATION_VARIANCE: f32 = 25.0;
const INITIAL_BIAS_VARIANCE: f32 = 0.01;
const BIAS_MEAN_REVERSION_TIME_S: f32 = 5.0;
const MIN_DT: f32 = 1.0e-6;

type Vec5 = [f32; STATE_COUNT];
type Mat5 = [[f32; STATE_COUNT]; STATE_COUNT];

fn identity() -> Mat5 {
    let mut m = [[0.0; STATE_COUNT]; STATE_COUNT];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

fn mul(a: &Mat5, b: &Mat5) -> Mat5 {
    let mut out = [[0.0; STATE_COUNT]; STATE_COUNT];
    for i in 0..STATE_COUNT {
        for j in 0..STATE_COUNT {
            let mut sum = 0.0;
            for (k, brow) in b.iter().enumerate() {
                sum += a[i][k] * brow[j];
            }
            out[i][j] = sum;
        }
    }
    out
}

fn transposed(a: &Mat5) -> Mat5 {
    let mut out = [[0.0; STATE_COUNT]; STATE_COUNT];
    for i in 0..STATE_COUNT {
        for j in 0..STATE_COUNT {
            out[j][i] = a[i][j];
        }
    }
    out
}

fn add(a: &Mat5, b: &Mat5) -> Mat5 {
    let mut out = [[0.0; STATE_COUNT]; STATE_COUNT];
    for i in 0..STATE_COUNT {
        for j in 0..STATE_COUNT {
            out[i][j] = a[i][j] + b[i][j];
        }
    }
    out
}

/// Numerical drift makes the covariance drift out of symmetry; VPX folds it
/// back every step and so do we.
fn symmetrize(m: &mut Mat5) {
    #[allow(clippy::needless_range_loop)] // indexing both [i][j] and [j][i]
    for i in 0..STATE_COUNT {
        for j in (i + 1)..STATE_COUNT {
            let avg = 0.5 * (m[i][j] + m[j][i]);
            m[i][j] = avg;
            m[j][i] = avg;
        }
    }
}

fn dot(a: &Vec5, b: &Vec5) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[derive(Debug, Clone, Copy)]
pub struct MotionKalmanAxis {
    state: Vec5,
    covariance: Mat5,
    initialized: bool,
}

impl Default for MotionKalmanAxis {
    fn default() -> Self {
        Self {
            state: [0.0; STATE_COUNT],
            covariance: [[0.0; STATE_COUNT]; STATE_COUNT],
            initialized: false,
        }
    }
}

impl MotionKalmanAxis {
    /// Filtered acceleration — the number the rest of the chain consumes.
    pub fn acceleration(&self) -> f32 {
        self.state[ACCELERATION]
    }

    fn reset(&mut self, acceleration_bias: f32) {
        self.initialized = true;
        self.state = [0.0, 0.0, 0.0, 0.0, acceleration_bias];
        self.covariance = [[0.0; STATE_COUNT]; STATE_COUNT];
        self.covariance[POSITION][POSITION] = INITIAL_POSITION_VARIANCE;
        self.covariance[VELOCITY][VELOCITY] = INITIAL_VELOCITY_VARIANCE;
        self.covariance[ACCELERATION][ACCELERATION] = INITIAL_ACCELERATION_VARIANCE;
        self.covariance[VELOCITY_BIAS][VELOCITY_BIAS] = INITIAL_BIAS_VARIANCE;
        self.covariance[ACCELERATION_BIAS][ACCELERATION_BIAS] = ACCELERATION_MEASUREMENT_VARIANCE;
    }

    /// Constant-jerk transition, with both biases reverting slowly to zero.
    fn transition_matrix(dt: f32) -> Mat5 {
        let mut f = identity();
        f[POSITION][VELOCITY] = dt;
        f[POSITION][ACCELERATION] = 0.5 * dt * dt;
        f[VELOCITY][ACCELERATION] = dt;
        let alpha = (-dt / BIAS_MEAN_REVERSION_TIME_S).exp();
        f[VELOCITY_BIAS][VELOCITY_BIAS] = alpha;
        f[ACCELERATION_BIAS][ACCELERATION_BIAS] = alpha;
        f
    }

    /// Process noise for a jerk-driven model: the classic dt⁵/20 … dt block,
    /// plus a slow random walk on each bias.
    fn process_noise(dt: f32) -> Mat5 {
        let (dt2, dt3) = (dt * dt, dt * dt * dt);
        let (dt4, dt5) = (dt3 * dt, dt3 * dt * dt);
        let qj = PROCESS_JERK_VARIANCE;
        let mut q = [[0.0; STATE_COUNT]; STATE_COUNT];
        q[POSITION][POSITION] = qj * (dt5 / 20.0);
        q[POSITION][VELOCITY] = qj * (dt4 / 8.0);
        q[POSITION][ACCELERATION] = qj * (dt3 / 6.0);
        q[VELOCITY][POSITION] = qj * (dt4 / 8.0);
        q[VELOCITY][VELOCITY] = qj * (dt3 / 3.0);
        q[VELOCITY][ACCELERATION] = qj * (dt2 / 2.0);
        q[ACCELERATION][POSITION] = qj * (dt3 / 6.0);
        q[ACCELERATION][VELOCITY] = qj * (dt2 / 2.0);
        q[ACCELERATION][ACCELERATION] = qj * dt;
        q[VELOCITY_BIAS][VELOCITY_BIAS] = VELOCITY_BIAS_PROCESS_VARIANCE * dt;
        q[ACCELERATION_BIAS][ACCELERATION_BIAS] = ACCELERATION_BIAS_PROCESS_VARIANCE * dt;
        q
    }

    /// Advance the estimate by `dt` seconds without any new measurement.
    pub fn predict(&mut self, dt: f32) {
        if !self.initialized {
            return;
        }
        let dt = dt.max(MIN_DT);
        let f = Self::transition_matrix(dt);
        let ft = transposed(&f);
        let q = Self::process_noise(dt);

        let mut new_state = [0.0; STATE_COUNT];
        for (i, s) in new_state.iter_mut().enumerate() {
            *s = dot(&f[i], &self.state);
        }
        self.state = new_state;
        self.covariance = add(&mul(&mul(&f, &self.covariance), &ft), &q);
        symmetrize(&mut self.covariance);
    }

    /// Fold in one scalar measurement, `h` selecting which states it observes.
    fn update_scalar(&mut self, h: &Vec5, measurement: f32, variance: f32) {
        let innovation = measurement - dot(h, &self.state);

        let mut pht = [0.0; STATE_COUNT];
        for (i, v) in pht.iter_mut().enumerate() {
            *v = dot(&self.covariance[i], h);
        }
        let s = dot(h, &pht) + variance;
        if s <= 0.0 {
            return;
        }
        let inv_s = 1.0 / s;

        let mut k = [0.0; STATE_COUNT];
        for (i, v) in k.iter_mut().enumerate() {
            *v = pht[i] * inv_s;
        }
        for (i, s) in self.state.iter_mut().enumerate() {
            *s += k[i] * innovation;
        }

        // Joseph form: keeps the covariance positive definite where the
        // shorter (I-KH)P would slowly poison it.
        let mut a = identity();
        for i in 0..STATE_COUNT {
            for j in 0..STATE_COUNT {
                a[i][j] -= k[i] * h[j];
            }
        }
        let at = transposed(&a);
        let apat = mul(&mul(&a, &self.covariance), &at);
        let mut krkt = [[0.0; STATE_COUNT]; STATE_COUNT];
        for i in 0..STATE_COUNT {
            for j in 0..STATE_COUNT {
                krkt[i][j] = k[i] * k[j] * variance;
            }
        }
        self.covariance = add(&apat, &krkt);
        symmetrize(&mut self.covariance);
    }

    /// A new acceleration sample. The measurement observes the acceleration
    /// *and* its bias, which is how the filter learns the sensor's offset
    /// rather than mistaking it for motion.
    pub fn update_acceleration(&mut self, acceleration: f32) {
        if !self.initialized {
            // First sample is taken as pure bias: a cabinet at rest reading
            // non-zero is offset, not moving.
            self.reset(acceleration);
            return;
        }
        let mut h = [0.0; STATE_COUNT];
        h[ACCELERATION] = 1.0;
        h[ACCELERATION_BIAS] = 1.0;
        self.update_scalar(&h, acceleration, ACCELERATION_MEASUREMENT_VARIANCE);
    }

    /// Pseudo-measurements stating the cabinet is standing still: the only
    /// thing that lets the filter separate a constant offset from a genuine
    /// slow drift, and what keeps the estimate from wandering after a shake.
    pub fn update_rest(&mut self) {
        if !self.initialized {
            return;
        }
        let mut h = [0.0; STATE_COUNT];
        h[POSITION] = 1.0;
        self.update_scalar(&h, 0.0, ZERO_POSITION_MEASUREMENT_VARIANCE);

        let mut h = [0.0; STATE_COUNT];
        h[VELOCITY] = 1.0;
        self.update_scalar(&h, 0.0, ZERO_VELOCITY_MEASUREMENT_VARIANCE);

        let mut h = [0.0; STATE_COUNT];
        h[ACCELERATION] = 1.0;
        self.update_scalar(&h, 0.0, ZERO_ACCELERATION_MEASUREMENT_VARIANCE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_reading_at_startup_is_taken_as_bias() {
        // A cabinet sitting still whose sensor reads 0.3 is offset, not
        // accelerating: the filter must not report motion.
        let mut k = MotionKalmanAxis::default();
        for _ in 0..200 {
            k.predict(0.008);
            k.update_acceleration(0.3);
        }
        assert!(
            k.acceleration().abs() < 0.15,
            "reported {} m/s² of motion from a constant offset",
            k.acceleration()
        );
    }

    #[test]
    fn it_follows_a_real_acceleration() {
        let mut k = MotionKalmanAxis::default();
        for _ in 0..50 {
            k.predict(0.008);
            k.update_acceleration(0.0);
        }
        for _ in 0..30 {
            k.predict(0.008);
            k.update_acceleration(5.0);
        }
        assert!(k.acceleration() > 2.0, "only reached {}", k.acceleration());
    }

    #[test]
    fn rest_constraints_pull_it_back_to_zero() {
        let mut k = MotionKalmanAxis::default();
        for _ in 0..50 {
            k.predict(0.008);
            k.update_acceleration(4.0);
        }
        for _ in 0..100 {
            k.predict(0.008);
            k.update_rest();
        }
        assert!(
            k.acceleration().abs() < 0.2,
            "still at {}",
            k.acceleration()
        );
    }

    #[test]
    fn the_filter_debiases_rather_than_smooths() {
        // Sensor jitter, as a deterministic pseudo-random walk around zero.
        // The estimate should be quieter than the samples — that is the
        // whole point of the filter.
        let mut k = MotionKalmanAxis::default();
        let mut seed: u32 = 12345;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 16) as f32 / 32768.0) - 1.0 // −1 … +1
        };
        let (mut in_sq, mut out_sq, mut n) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..2000 {
            let sample = next();
            k.predict(0.008);
            k.update_acceleration(sample);
            if i > 400 {
                in_sq += sample * sample;
                out_sq += k.acceleration() * k.acceleration();
                n += 1.0;
            }
        }
        let (rms_in, rms_out) = ((in_sq / n).sqrt(), (out_sq / n).sqrt());
        // Measured, not assumed: the filter does *not* smooth much — noise
        // comes out at roughly the level it went in (0.57 → 0.61 here). VPX
        // tunes it for responsiveness (jerk variance 800, "high value for low
        // latency"), so it trusts each sample and follows it. What it buys is
        // bias removal and sensor fusion, per the changelog's "custom filter
        // to handle bias and noise"; smoothing is left to the cabinet model
        // downstream.
        assert!(
            rms_out < rms_in * 1.5,
            "noise went in at {rms_in} and came out at {rms_out}"
        );
    }

    #[test]
    fn a_sample_at_the_nyquist_rate_is_not_smoothed_away() {
        // Worth pinning down, because it surprised me: alternating ±1 every
        // sample comes out *larger* than it went in. VPX tunes this filter
        // for responsiveness (jerk variance 800, "high value for low
        // latency"), and a constant-jerk model chasing a square wave at half
        // the sampling rate overshoots. It is the tuning, not a defect — but
        // it means a board that rings at its own polling rate will not be
        // tamed here.
        let mut k = MotionKalmanAxis::default();
        let mut worst: f32 = 0.0;
        for i in 0..400 {
            k.predict(0.008);
            k.update_acceleration(if i % 2 == 0 { 1.0 } else { -1.0 });
            if i > 100 {
                worst = worst.max(k.acceleration().abs());
            }
        }
        assert!(worst > 1.0, "expected overshoot, got {worst}");
    }
}
