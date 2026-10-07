//! Spring animations for the settings window.
//!
//! Springs follow the SwiftUI model: `response` is the period of the
//! undamped oscillation in seconds and `damping` the damping ratio (1 is
//! critical, below 1 overshoots). Every step uses the closed-form solution,
//! so the result does not depend on the frame rate, and retargeting keeps
//! the current velocity, so an interrupted animation turns smoothly.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::hash::Hash;

/// Distance to the target below which a spring is considered settled.
const REST_DISTANCE: f32 = 0.0008;
/// Settled velocity, relative to the spring's angular frequency.
const REST_VELOCITY: f32 = 0.0008;
/// Safety net: no spring runs longer than this many periods.
const MAX_PERIODS: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub response: f32,
    pub damping: f32,
}

impl Params {
    pub const fn new(response: f32, damping: f32) -> Self {
        Self { response, damping }
    }

    fn omega(self) -> f32 {
        TAU / self.response.max(0.001)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
    params: Params,
    /// Time since the last retarget, for the safety net.
    elapsed: f32,
}

impl Spring {
    pub fn new(value: f32, params: Params) -> Self {
        Self { value, velocity: 0.0, target: value, params, elapsed: 0.0 }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_resting(&self) -> bool {
        self.value == self.target && self.velocity == 0.0
    }

    /// Moves the target; the current value and velocity are kept.
    pub fn retarget(&mut self, target: f32, params: Params) {
        if self.target != target || self.params != params {
            self.target = target;
            self.params = params;
            self.elapsed = 0.0;
        }
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
        self.elapsed = 0.0;
    }

    pub fn step(&mut self, dt: f32) {
        if self.is_resting() || dt <= 0.0 {
            return;
        }
        self.elapsed += dt;
        let omega = self.params.omega();
        let zeta = self.params.damping.max(0.0);
        let (x0, v0) = (self.value - self.target, self.velocity);

        let (x, v) = if zeta < 1.0 {
            let wd = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * dt).exp();
            let b = (v0 + zeta * omega * x0) / wd;
            let (sin, cos) = (wd * dt).sin_cos();
            let x = decay * (x0 * cos + b * sin);
            let v = decay * ((b * wd - zeta * omega * x0) * cos - (x0 * wd + zeta * omega * b) * sin);
            (x, v)
        } else if zeta == 1.0 {
            let decay = (-omega * dt).exp();
            let b = v0 + omega * x0;
            (decay * (x0 + b * dt), decay * (b - omega * (x0 + b * dt)))
        } else {
            let root = (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-omega * (zeta - root), -omega * (zeta + root));
            let c2 = (v0 - r1 * x0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        };

        let settled = x.abs() <= REST_DISTANCE && v.abs() <= REST_VELOCITY * omega;
        if settled || !x.is_finite() || self.elapsed > MAX_PERIODS * self.params.response {
            self.snap(self.target);
        } else {
            self.value = self.target + x;
            self.velocity = v;
        }
    }
}

/// A set of named springs that tick together.
pub struct Motion<K> {
    springs: HashMap<K, Spring>,
    /// `false` when the system asks for no animations: values jump.
    pub enabled: bool,
}

impl<K: Copy + Eq + Hash> Motion<K> {
    pub fn new(enabled: bool) -> Self {
        Self { springs: HashMap::new(), enabled }
    }

    /// Current value; unknown keys read as zero.
    pub fn get(&self, key: K) -> f32 {
        self.springs.get(&key).map_or(0.0, Spring::value)
    }

    /// Animates towards `target`. A new key starts from zero.
    pub fn to(&mut self, key: K, target: f32, params: Params) {
        let enabled = self.enabled;
        let spring = self.springs.entry(key).or_insert_with(|| Spring::new(0.0, params));
        if enabled {
            spring.retarget(target, params);
        } else {
            spring.snap(target);
        }
    }

    /// Jumps to `value` without animating.
    pub fn snap(&mut self, key: K, value: f32, params: Params) {
        self.springs.entry(key).or_insert_with(|| Spring::new(value, params)).snap(value);
    }

    /// Restarts an animation from `from` (for crossfades).
    pub fn restart(&mut self, key: K, from: f32, to: f32, params: Params) {
        self.snap(key, from, params);
        self.to(key, to, params);
    }

    pub fn is_moving(&self) -> bool {
        self.springs.values().any(|s| !s.is_resting())
    }

    /// Settles every spring at its target.
    pub fn finish(&mut self) {
        for spring in self.springs.values_mut() {
            spring.snap(spring.target());
        }
    }

    /// Advances all springs; returns whether any is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for spring in self.springs.values_mut() {
            spring.step(dt);
            moving |= !spring.is_resting();
        }
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(spring: &mut Spring, seconds: f32, dt: f32) -> f32 {
        let mut peak = spring.value();
        let mut t = 0.0;
        while t < seconds {
            spring.step(dt);
            peak = peak.max(spring.value());
            t += dt;
        }
        peak
    }

    #[test]
    fn settles_exactly_on_target() {
        for damping in [0.5, 0.72, 1.0, 1.4] {
            let mut spring = Spring::new(0.0, Params::new(0.3, damping));
            spring.retarget(1.0, Params::new(0.3, damping));
            run(&mut spring, 10.0, 1.0 / 60.0);
            assert!(spring.is_resting(), "damping {damping}");
            assert_eq!(spring.value(), 1.0);
        }
    }

    #[test]
    fn underdamped_overshoots_critical_does_not() {
        let mut bouncy = Spring::new(0.0, Params::new(0.3, 0.5));
        bouncy.retarget(1.0, Params::new(0.3, 0.5));
        assert!(run(&mut bouncy, 2.0, 0.001) > 1.05);

        let mut critical = Spring::new(0.0, Params::new(0.3, 1.0));
        critical.retarget(1.0, Params::new(0.3, 1.0));
        assert!(run(&mut critical, 2.0, 0.001) <= 1.0);
    }

    #[test]
    fn frame_rate_independent() {
        let params = Params::new(0.25, 0.8);
        let mut fast = Spring::new(0.0, params);
        let mut slow = Spring::new(0.0, params);
        fast.retarget(1.0, params);
        slow.retarget(1.0, params);
        for _ in 0..12 {
            fast.step(0.005);
        }
        for _ in 0..2 {
            slow.step(0.03);
        }
        assert!((fast.value() - slow.value()).abs() < 1e-4);
    }

    #[test]
    fn retarget_keeps_velocity() {
        let params = Params::new(0.3, 1.0);
        let mut spring = Spring::new(0.0, params);
        spring.retarget(1.0, params);
        spring.step(0.05);
        let (value, velocity) = (spring.value(), spring.velocity);
        assert!(velocity > 0.0);
        spring.retarget(0.0, params);
        assert_eq!(spring.value(), value);
        // Still heading up for a moment before turning around.
        spring.step(0.001);
        assert!(spring.value() > value);
    }

    #[test]
    fn disabled_motion_jumps() {
        let params = Params::new(0.3, 1.0);
        let mut motion = Motion::new(false);
        motion.to(1, 1.0, params);
        assert_eq!(motion.get(1), 1.0);
        assert!(!motion.is_moving());

        motion.enabled = true;
        motion.to(1, 0.0, params);
        assert!(motion.is_moving());
        assert!(motion.step(0.01));
        motion.finish();
        assert_eq!(motion.get(1), 0.0);
        assert_eq!(motion.get(2), 0.0);
    }
}
