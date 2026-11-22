//! What OMEN Gaming Hub does on this laptop, minus everything else.
//! Command map: see docs/PROTOCOL.md.

use windows::core::Result;

use crate::bios::Bios;

const OMEN: u32 = 0x20008;
const KB: u32 = 0x20009;
const READ: u32 = 1;
const WRITE: u32 = 2;

const KB_ON: u8 = 0xE4;
const KB_OFF: u8 = 0x64;
const KB_ZONE0: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Eco,
    Balanced,
    Performance,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Eco, Mode::Balanced, Mode::Performance];

    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Mode::Eco,
            2 => Mode::Performance,
            _ => Mode::Balanced,
        }
    }
    pub fn as_u32(self) -> u32 {
        self as u32
    }
    pub fn next(self) -> Self {
        match self {
            Mode::Balanced => Mode::Performance,
            Mode::Performance => Mode::Eco,
            Mode::Eco => Mode::Balanced,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Eco => "Eco",
            Mode::Balanced => "Balanced",
            Mode::Performance => "Performance",
        }
    }
    /// Sustained CPU power limit (W). OMEN's own values for this SKU are 55 (balanced) and
    /// 65 (performance); eco sits inside its 25–35 W lower band.
    fn cpu_watts(self) -> u8 {
        match self {
            Mode::Eco => 35,
            Mode::Balanced => 55,
            Mode::Performance => 65,
        }
    }
    /// (custom TGP, Dynamic Boost) for the NVIDIA GPU.
    fn gpu(self) -> (u8, u8) {
        match self {
            Mode::Eco => (0, 0),
            Mode::Balanced => (0, 1),
            Mode::Performance => (1, 1),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Gpu {
    Hybrid = 0,
    Discrete = 1,
}

pub struct Hw {
    bios: Bios,
    /// Thermal policy V1 uses 0x30/0x31 for the fan mode instead of 0/1.
    v1: bool,
    fans: usize,
    gpu_modes: u8,
}

impl Hw {
    pub fn new() -> Result<Self> {
        let bios = Bios::new()?;
        let design = bios.get(OMEN, 0x28, &[0; 4], 128)?;
        let fans = bios.get(OMEN, 0x10, &[0; 4], 4).ok().and_then(|d| d.first().copied()).unwrap_or(2);
        Ok(Self {
            v1: design.get(3) == Some(&1),
            gpu_modes: design.get(7).copied().unwrap_or(0),
            fans: fans.clamp(1, 3) as usize,
            bios,
        })
    }

    pub fn apply_mode(&self, mode: Mode) -> Result<()> {
        let fan = match (mode, self.v1) {
            (Mode::Performance, true) => 0x31,
            (Mode::Performance, false) => 0x01,
            (_, true) => 0x30,
            (_, false) => 0x00,
        };
        // [0xFF, fan mode, 1 = BIOS runs the fan curve (no software loop needed), 0]
        self.bios.set(OMEN, 0x1A, &[0xFF, fan, 0x01, 0x00])?;
        let (tgp, boost) = mode.gpu();
        self.bios.set(OMEN, 0x22, &[tgp, boost, 0x01, 0x57])?;
        let w = mode.cpu_watts();
        // [PL2, PL1, PL4 (0xFF = keep), concurrent TDP (0xFF = keep)]
        self.bios.set(OMEN, 0x29, &[w, w, 0xFF, 0xFF])
    }

    /// Fan speeds in RPM.
    pub fn fan_rpm(&self) -> Result<Vec<u32>> {
        let d = self.bios.get(OMEN, 0x2D, &[0; 4], 128)?;
        Ok(d.iter().take(self.fans).map(|&b| b as u32 * 100).collect())
    }

    pub fn set_max_fan(&self, on: bool) -> Result<()> {
        self.bios.set(OMEN, 0x27, &[on as u8])
    }

    /// Four zone colours as 0xRRGGBB.
    pub fn kb_colors(&self) -> Result<[u32; 4]> {
        let d = self.bios.get(KB, 2, &[0], 128)?;
        let mut out = [0u32; 4];
        for (i, c) in out.iter_mut().enumerate() {
            let o = KB_ZONE0 + i * 3;
            if let Some(px) = d.get(o..o + 3) {
                *c = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
            }
        }
        Ok(out)
    }

    pub fn set_kb_colors(&self, colors: [u32; 4]) -> Result<()> {
        let mut d = self.bios.get(KB, 2, &[0], 128)?;
        d.resize(128, 0);
        for (i, c) in colors.iter().enumerate() {
            let o = KB_ZONE0 + i * 3;
            d[o] = (c >> 16) as u8;
            d[o + 1] = (c >> 8) as u8;
            d[o + 2] = *c as u8;
        }
        self.bios.set(KB, 3, &d)
    }

    pub fn kb_on(&self) -> Result<bool> {
        Ok(self.bios.get(KB, 4, &[0], 128)?.first().is_some_and(|&b| b & 0x80 != 0))
    }

    pub fn set_kb_on(&self, on: bool) -> Result<()> {
        self.bios.set(KB, 5, &[if on { KB_ON } else { KB_OFF }, 0, 0, 0])
    }

    /// [custom TGP, Dynamic Boost, D-state, GPS] as the BIOS reports it.
    pub fn debug_gpu_status(&self) -> Result<Vec<u8>> {
        self.bios.get(OMEN, 0x21, &[0; 4], 4)
    }

    pub fn debug_power(&self) -> Result<Vec<u8>> {
        Ok(self.bios.get(OMEN, 0x2A, &[0; 4], 128)?.into_iter().take(8).collect())
    }

    pub fn gpu_switch_supported(&self) -> bool {
        // bit 1 = hybrid, bit 2 = discrete
        self.gpu_modes & 0b110 == 0b110
    }

    pub fn gpu(&self) -> Result<Gpu> {
        let d = self.bios.get(READ, 0x52, &[], 4)?;
        Ok(if d.first() == Some(&1) { Gpu::Discrete } else { Gpu::Hybrid })
    }

    /// Takes effect after a restart.
    pub fn set_gpu(&self, g: Gpu) -> Result<()> {
        self.bios.set(WRITE, 0x52, &[g as u8, 0, 0, 0])
    }
}
