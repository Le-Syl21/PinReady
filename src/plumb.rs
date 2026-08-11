//! A faithful copy of VPX's tilt plumb, so the wizard can be shaken instead
//! of guessed at.
//!
//! VPX does not compare acceleration to a threshold: it simulates a pendulum
//! — a bob on a 10 cm rod, driven by gravity plus the cabinet's acceleration,
//! with non-linear angular damping, integrated every millisecond — and tilts
//! when the rod's angle passes `PlumbThresholdAngle`. A short violent knock
//! can leave it unmoved while three rhythmic taps trip it, which no
//! instantaneous threshold can express.
//!
//! Ported from `src/physics/cabinet/PlumbHandler.cpp` (GPLv3+), same
//! constants, same integration step.

/// Rod length in metres (`m_plumbPoleLength`).
const POLE_LENGTH: f32 = 0.10;
/// Damping coefficients, scaled by the user's `PlumbDamping`.
const DAMPING_COEF_0: f32 = 1.25;
const DAMPING_COEF_1: f32 = 0.75;
const GRAVITY: f32 = 9.806_65;
/// VPX integrates at 1 kHz.
const DT: f32 = 0.001;

#[derive(Debug, Clone, Copy)]
pub struct Plumb {
    /// Bob position in metres, always at `POLE_LENGTH` from the pivot.
    pos: [f32; 3],
    /// Angular velocity.
    omega: [f32; 3],
    /// Set on the frame the plumb crossed its threshold, so the UI can say so.
    pub tilted: bool,
}

impl Default for Plumb {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0, -POLE_LENGTH],
            omega: [0.0; 3],
            tilted: false,
        }
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

impl Plumb {
    /// Advance one millisecond under a cabinet acceleration in m/s².
    ///
    /// `damping` is the user's `PlumbDamping`, `threshold_deg` the tilt angle.
    pub fn step(&mut self, cab_accel: (f32, f32), damping: f32, threshold_deg: f32) {
        let threshold = threshold_deg.to_radians();
        if threshold <= 0.0 {
            return;
        }

        // Change of reference frame: the bob feels the cabinet's acceleration
        // reversed, plus gravity pulling down.
        let plumb_acc = [-cab_accel.0, -cab_accel.1, -GRAVITY];

        // Torque per unit mass about the pivot, then angular acceleration for
        // a point mass at distance L (the mass cancels out).
        let torque = cross(self.pos, plumb_acc);
        let mut alpha = [
            torque[0] / (POLE_LENGTH * POLE_LENGTH),
            torque[1] / (POLE_LENGTH * POLE_LENGTH),
            torque[2] / (POLE_LENGTH * POLE_LENGTH),
        ];

        // Non-linear damping: the linear term settles small oscillations, the
        // quadratic one keeps fast swings from running away.
        let d = DAMPING_COEF_0 * damping + DAMPING_COEF_1 * damping * length(self.omega);
        for (a, w) in alpha.iter_mut().zip(self.omega.iter_mut()) {
            *a -= *w * d;
            *w += *a * DT;
        }

        // Keep the angular velocity orthogonal to the rod: any component
        // along it is spin, physically irrelevant here and a source of drift.
        let mut pole_axis = [
            self.pos[0] / POLE_LENGTH,
            self.pos[1] / POLE_LENGTH,
            self.pos[2] / POLE_LENGTH,
        ];
        let spin = dot(self.omega, pole_axis);
        for (w, axis) in self.omega.iter_mut().zip(pole_axis.iter()) {
            *w -= axis * spin;
        }

        // Rigid-body kinematics, renormalised so the rod keeps its length.
        let velocity = cross(self.omega, self.pos);
        for (p, v) in self.pos.iter_mut().zip(velocity.iter()) {
            *p += v * DT;
        }
        let len = length(self.pos);
        if len > 1.0e-8 {
            for p in self.pos.iter_mut() {
                *p *= POLE_LENGTH / len;
            }
        } else {
            self.pos = [0.0, 0.0, -POLE_LENGTH];
        }

        pole_axis = [
            self.pos[0] / POLE_LENGTH,
            self.pos[1] / POLE_LENGTH,
            self.pos[2] / POLE_LENGTH,
        ];
        let spin = dot(self.omega, pole_axis);
        for (w, axis) in self.omega.iter_mut().zip(pole_axis.iter()) {
            *w -= axis * spin;
        }

        // Tilt when the rod passes the threshold, and pin the bob at the
        // limit so it cannot wander past it — as VPX does.
        let psi = (self.pos[0] * self.pos[0] + self.pos[1] * self.pos[1])
            .sqrt()
            .atan2(-self.pos[2]);
        self.tilted = psi > threshold;
        if self.tilted {
            let limit = threshold - 1.0e-3;
            self.pos[2] = -POLE_LENGTH * limit.cos();
            let xy = POLE_LENGTH * limit.sin();
            let theta = self.pos[0].atan2(self.pos[1]);
            self.pos[0] = xy * theta.sin();
            self.pos[1] = xy * theta.cos();
        }
    }

    /// Rod angle from vertical, in degrees — what the threshold is compared
    /// against.
    pub fn angle_deg(&self) -> f32 {
        (self.pos[0] * self.pos[0] + self.pos[1] * self.pos[1])
            .sqrt()
            .atan2(-self.pos[2])
            .to_degrees()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_rest_the_plumb_hangs_straight_down() {
        let mut plumb = Plumb::default();
        for _ in 0..1000 {
            plumb.step((0.0, 0.0), 1.0, 1.0);
        }
        assert!(plumb.angle_deg() < 0.001, "got {}", plumb.angle_deg());
        assert!(!plumb.tilted);
    }

    #[test]
    fn a_sustained_push_eventually_tilts() {
        // 5 m/s² held for half a second is a firm, prolonged shove.
        let mut plumb = Plumb::default();
        let mut tilted = false;
        for _ in 0..500 {
            plumb.step((5.0, 0.0), 1.0, 1.0);
            tilted |= plumb.tilted;
        }
        assert!(tilted, "angle reached {}", plumb.angle_deg());
    }

    #[test]
    fn a_brief_knock_tilts_at_one_degree_but_not_at_four() {
        // 5 m/s² for 5 ms. The bob leaves with ~0.25 rad/s, and a 10 cm
        // pendulum swings that into roughly 1.4° — so it trips VPX's 1°
        // default and not a 4° one. This is why a cabinet set at the default
        // seems to tilt on every firm shot.
        let swing = |threshold: f32| {
            let mut plumb = Plumb::default();
            let mut tilted = false;
            for i in 0..500 {
                let accel = if i < 5 { (5.0, 0.0) } else { (0.0, 0.0) };
                plumb.step(accel, 1.0, threshold);
                tilted |= plumb.tilted;
            }
            tilted
        };
        assert!(swing(1.0), "a firm knock should trip the 1° default");
        assert!(!swing(4.0), "the same knock should not trip 4°");
    }

    #[test]
    fn damping_brings_it_back_to_rest() {
        let mut plumb = Plumb::default();
        for _ in 0..50 {
            plumb.step((3.0, 0.0), 1.0, 4.0);
        }
        let swinging = plumb.angle_deg();
        for _ in 0..3000 {
            plumb.step((0.0, 0.0), 1.0, 4.0);
        }
        // Damping is gentle by design — a real plumb keeps swinging for a
        // while — so this checks decay, not a full stop.
        assert!(
            plumb.angle_deg() < swinging * 0.5,
            "still at {} after {}",
            plumb.angle_deg(),
            swinging
        );
    }
}
