//! Small Windows helpers: settings in the registry, logon task, power overlay, tray icon.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::{w, GUID, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::hw::Mode;

const KEY: PCWSTR = w!("Software\\omenless");
const NO_WINDOW: u32 = 0x0800_0000;
const TASK: &str = "omenless";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------- settings ----------

pub struct Config {
    pub mode: Mode,
    pub max_fan: bool,
    pub idle_off: bool,
}

fn reg_get(name: &str) -> Option<u32> {
    let mut v = 0u32;
    let mut size = 4u32;
    let name = wide(name);
    let err = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, KEY, PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut size))
    };
    (err == ERROR_SUCCESS).then_some(v)
}

fn reg_set(name: &str, v: u32) {
    let name = wide(name);
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, KEY, PCWSTR(name.as_ptr()), REG_DWORD.0, Some(&v as *const u32 as _), 4);
    }
}

impl Config {
    pub fn load() -> Self {
        Self {
            mode: Mode::from_u32(reg_get("Mode").unwrap_or(1)),
            max_fan: reg_get("MaxFan") == Some(1),
            idle_off: reg_get("IdleOff") == Some(1),
        }
    }
    pub fn save(&self) {
        reg_set("Mode", self.mode.as_u32());
        reg_set("MaxFan", self.max_fan as u32);
        reg_set("IdleOff", self.idle_off as u32);
    }
}

pub fn light_taskbar() -> bool {
    let mut v = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as _),
            Some(&mut size),
        )
    };
    err == ERROR_SUCCESS && v == 1
}

// ---------- start with Windows (elevated logon task, no UAC prompt) ----------

pub fn autostart_enabled() -> bool {
    Command::new("schtasks")
        .args(["/Query", "/TN", TASK])
        .creation_flags(NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn set_autostart(on: bool) -> bool {
    if !on {
        return Command::new("schtasks")
            .args(["/Delete", "/TN", TASK, "/F"])
            .creation_flags(NO_WINDOW)
            .status()
            .is_ok_and(|s| s.success());
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command></Exec></Actions>
</Task>"#,
        user = esc(&user),
        exe = esc(&exe.to_string_lossy()),
    );
    let path = std::env::temp_dir().join("omenless-task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    if std::fs::write(&path, bytes).is_err() {
        return false;
    }
    let ok = Command::new("schtasks")
        .args(["/Create", "/TN", TASK, "/F", "/XML"])
        .arg(&path)
        .creation_flags(NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&path);
    ok
}

// ---------- Windows power mode slider (efficiency / balanced / performance) ----------

#[link(name = "powrprof", kind = "raw-dylib")]
extern "system" {
    fn PowerSetActiveOverlayScheme(overlay: *const GUID) -> u32;
}

pub fn set_power_overlay(mode: Mode) {
    let g = match mode {
        Mode::Eco => GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a),
        Mode::Balanced => GUID::zeroed(),
        Mode::Performance => GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238),
    };
    unsafe {
        PowerSetActiveOverlayScheme(&g);
    }
}

// ---------- tray icon, drawn at runtime ----------

/// Eco = ring, Balanced = ring + dot, Performance = filled disc.
pub fn mode_icon(mode: Mode) -> HICON {
    let light = light_taskbar();
    let rgb: u32 = match mode {
        Mode::Eco => 0x30D158,
        Mode::Balanced if light => 0x1C1C1E,
        Mode::Balanced => 0xF2F2F7,
        Mode::Performance => 0xFF453A,
    };
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let s = size as f32;
    let c = s / 2.0;
    let (outer, inner, dot) = match mode {
        Mode::Eco => (s * 0.44, s * 0.30, 0.0),
        Mode::Balanced => (s * 0.44, s * 0.30, s * 0.15),
        Mode::Performance => (s * 0.44, 0.0, 0.0),
    };
    let cover = |v: f32| v.clamp(0.0, 1.0);
    let mut px = vec![0u32; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let ring = cover(outer - d + 0.5) * if inner > 0.0 { cover(d - inner + 0.5) } else { 1.0 };
            let a = ring.max(cover(dot - d + 0.5));
            px[(y * size + x) as usize] = ((a * 255.0) as u32) << 24 | rgb;
        }
    }
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return HICON::default() };
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
