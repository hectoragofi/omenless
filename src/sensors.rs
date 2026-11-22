//! Driverless temperatures.
//! - ACPI zone TSZ0: the EC's CPU-area sensor (same one HP's fan tables were tuned against).
//! - D3DKMT adapter perf data: what Task Manager shows per GPU. dxgkrnl answers it without
//!   waking a GPU that is in runtime D3, so polling never costs battery.

use windows::core::w;
use windows::Wdk::Graphics::Direct3D::*;
use windows::Win32::System::Performance::*;

pub struct Sensors {
    query: PDH_HQUERY,
    zone: Option<PDH_HCOUNTER>,
    adapters: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    /// ACPI CPU zone, °C.
    pub cpu: Option<f32>,
    /// Hottest GPU that reports a temperature, °C. None when every GPU is idle/asleep.
    pub gpu: Option<f32>,
}

impl Sensors {
    pub fn new() -> Self {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let mut counter = PDH_HCOUNTER::default();
            let zone = (PdhOpenQueryW(None, 0, &mut query) == 0
                && PdhAddEnglishCounterW(query, w!("\\Thermal Zone Information(\\_TZ.TSZ0)\\High Precision Temperature"), 0, &mut counter) == 0)
                .then_some(counter);

            let mut e = D3DKMT_ENUMADAPTERS2::default();
            let mut adapters = Vec::new();
            if D3DKMTEnumAdapters2(&mut e).is_ok() && e.NumAdapters > 0 {
                let mut infos = vec![std::mem::zeroed::<D3DKMT_ADAPTERINFO>(); e.NumAdapters as usize];
                e.pAdapters = infos.as_mut_ptr();
                if D3DKMTEnumAdapters2(&mut e).is_ok() {
                    adapters = infos[..e.NumAdapters as usize].iter().map(|a| a.hAdapter).collect();
                }
            }
            Self { query, zone, adapters }
        }
    }

    pub fn cpu(&mut self) -> Option<f32> {
        let counter = self.zone?;
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut v) != 0 {
                return None;
            }
            let c = (v.Anonymous.doubleValue / 10.0 - 273.15) as f32;
            (1.0..130.0).contains(&c).then_some(c)
        }
    }

    /// (adapter handle, °C) for every adapter that reports a temperature right now.
    pub fn gpus(&self) -> Vec<(u32, f32)> {
        self.adapters
            .iter()
            .filter_map(|&h| unsafe {
                let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
                let mut q = D3DKMT_QUERYADAPTERINFO {
                    hAdapter: h,
                    Type: KMTQAITYPE_ADAPTERPERFDATA,
                    pPrivateDriverData: &mut perf as *mut _ as *mut _,
                    PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
                };
                if D3DKMTQueryAdapterInfo(&mut q).is_err() || perf.Temperature == 0 {
                    return None;
                }
                let c = perf.Temperature as f32 / 10.0;
                (c < 130.0).then_some((h, c))
            })
            .collect()
    }

    pub fn read(&mut self) -> Reading {
        Reading { cpu: self.cpu(), gpu: self.gpus().into_iter().map(|(_, t)| t).reduce(f32::max) }
    }
}

impl Drop for Sensors {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
            for &h in &self.adapters {
                let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: h });
            }
        }
    }
}
