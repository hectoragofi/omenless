//! Software fan loop. This Victus has no BIOS auto curve: the fans hold whatever level was set
//! last (OMEN Gaming Hub set one every second). We set one every 2 s from driverless sensors.

use crate::hw::{Hw, Mode};
use crate::sensors::Sensors;

/// Fan levels are in 100 rpm. Below MIN the fan stops.
pub const MIN_LEVEL: u8 = 18;
pub const MAX_LEVEL: u8 = 58;
/// Used when no sensor can be read, and left behind when omenless quits.
pub const SAFE_LEVEL: u8 = 30;
const CPU_PANIC: f32 = 92.0;
const GPU_PANIC: f32 = 87.0;
const KEEPALIVE_TICKS: u32 = 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FanMode {
    Auto = 0,
    Custom = 1,
    Max = 2,
}

impl FanMode {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => FanMode::Custom,
            2 => FanMode::Max,
            _ => FanMode::Auto,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            FanMode::Auto => "Auto",
            FanMode::Custom => "Custom curve",
            FanMode::Max => "Max",
        }
    }
}

/// Six (°C, level) points; level 0 = off. Applied to the hotter of CPU and GPU.
pub type Curve = [(u8, u8); 6];
pub const DEFAULT_CURVE: Curve = [(40, 0), (50, 18), (60, 25), (70, 34), (80, 46), (90, 58)];

/// HP's own step tables for this SKU (Bigred_PHX_N21X2X4X6.json): (temps, levels).
type Table = (&'static [u8], &'static [u8]);
const HP_CPU: Table = (&[42, 53, 55, 57, 59, 61, 63, 65, 67], &[0, 18, 22, 23, 25, 28, 32, 34, 39]);
const HP_GPU: Table = (&[50, 60, 62, 64, 66, 68, 70, 72, 74], &[0, 18, 22, 23, 25, 28, 32, 34, 39]);
const HP_CPU_PERF: Table = (&[53, 55, 57, 59, 61, 63, 64, 65, 67, 70, 73, 77], &[0, 18, 23, 25, 28, 32, 34, 39, 41, 46, 50, 58]);
const HP_GPU_PERF: Table = (&[50, 54, 57, 60, 63, 66, 69, 71, 73, 75, 77], &[0, 22, 25, 28, 32, 34, 39, 41, 46, 50, 58]);

fn step((temps, levels): Table, t: f32) -> u8 {
    temps.iter().zip(levels).filter(|(&tt, _)| t >= tt as f32).map(|(_, &l)| l).last().unwrap_or(levels[0])
}

pub fn eval(curve: &Curve, t: f32) -> u8 {
    let (first, last) = (curve[0], curve[curve.len() - 1]);
    let raw = if t <= first.0 as f32 {
        first.1 as f32
    } else if t >= last.0 as f32 {
        last.1 as f32
    } else {
        let i = curve.windows(2).position(|w| t < w[1].0 as f32).unwrap_or(0);
        let ((t0, l0), (t1, l1)) = (curve[i], curve[i + 1]);
        let span = (t1 as f32 - t0 as f32).max(1.0);
        l0 as f32 + (l1 as f32 - l0 as f32) * (t - t0 as f32) / span
    };
    let l = raw.round() as u8;
    if l < MIN_LEVEL { 0 } else { l.min(MAX_LEVEL) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_step_table() {
        assert_eq!(step(HP_CPU, 30.0), 0);
        assert_eq!(step(HP_CPU, 53.0), 18);
        assert_eq!(step(HP_CPU, 64.9), 32);
        assert_eq!(step(HP_CPU, 99.0), 39);
        assert_eq!(step(HP_GPU_PERF, 77.0), 58);
    }

    #[test]
    fn custom_curve_interpolates_and_snaps() {
        let c = DEFAULT_CURVE;
        assert_eq!(eval(&c, 20.0), 0);
        assert_eq!(eval(&c, 45.0), 0); // 9 would stall the fan: off
        assert_eq!(eval(&c, 50.0), 18);
        assert_eq!(eval(&c, 65.0), 30);
        assert_eq!(eval(&c, 95.0), 58);
    }
}

pub struct FanLoop {
    sensors: Sensors,
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
    pub level: Option<u8>,
    since_send: u32,
    /// BIOS table: CPU fan level -> matching GPU fan level.
    pairs: Vec<(u8, u8)>,
}

fn smooth(prev: Option<f32>, now: Option<f32>) -> Option<f32> {
    match (prev, now) {
        (Some(p), Some(n)) => Some(p + (n - p) * if n > p { 0.35 } else { 0.15 }),
        (_, n) => n,
    }
}

impl FanLoop {
    pub fn new(hw: &Hw) -> Self {
        Self { sensors: Sensors::new(), cpu: None, gpu: None, level: None, since_send: 0, pairs: hw.fan_pairs().unwrap_or_default() }
    }

    /// Resend on the next tick (after resume, or when leaving Max).
    pub fn force(&mut self) {
        self.level = None;
    }

    pub fn gpu_level(&self, cpu: u8) -> u8 {
        if cpu == 0 {
            return 0;
        }
        self.pairs.iter().find(|p| p.0 == cpu).map_or(cpu, |p| p.1)
    }

    pub fn tick(&mut self, hw: &Hw, mode: Mode, fans: FanMode, curve: &Curve) {
        let r = self.sensors.read();
        self.cpu = smooth(self.cpu, r.cpu);
        self.gpu = smooth(self.gpu, r.gpu);
        if fans == FanMode::Max {
            return;
        }
        let target = match (self.cpu, self.gpu) {
            (None, None) => SAFE_LEVEL,
            (c, g) => {
                let (c, g) = (c.unwrap_or(0.0), g.unwrap_or(0.0));
                if c >= CPU_PANIC || g >= GPU_PANIC {
                    MAX_LEVEL
                } else if fans == FanMode::Custom {
                    eval(curve, c.max(g))
                } else if mode == Mode::Performance {
                    step(HP_CPU_PERF, c).max(step(HP_GPU_PERF, g))
                } else {
                    step(HP_CPU, c).max(step(HP_GPU, g))
                }
            }
        };
        // Spin up at once, wind down two steps per tick.
        let next = match self.level {
            Some(cur) if target < cur => {
                let down = cur.saturating_sub(2);
                if down < MIN_LEVEL { target } else { target.max(down) }
            }
            _ => target,
        };
        if Some(next) != self.level || self.since_send >= KEEPALIVE_TICKS {
            if hw.set_fan_levels(next, self.gpu_level(next)).is_ok() {
                self.level = Some(next);
            }
            self.since_send = 0;
        } else {
            self.since_send += 1;
        }
    }
}
