#![windows_subsystem = "windows"]
//! omenless: the useful part of OMEN Gaming Hub as one tray icon.
//! Performance mode, max fans, keyboard light, GPU mux, OMEN key. No telemetry, no services.

mod bios;
mod hw;
mod sys;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{w, BSTR, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterPowerSettingNotification;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::SystemServices::GUID_ACDC_POWER_SOURCE;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::Wmi::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use hw::{Gpu, Hw, Mode};
use sys::{wide, Config};

const WM_TRAY: u32 = WM_APP + 1;
const WM_OMEN_KEY: u32 = WM_APP + 2;

const TIMER_IDLE: usize = 1;
const TIMER_REAPPLY: usize = 2;
const IDLE_AFTER_MS: u32 = 120_000;

const ID_MODE: u32 = 100; // + Mode as u32
const ID_MAX_FAN: u32 = 110;
const ID_KB_ON: u32 = 120;
const ID_KB_IDLE: u32 = 121;
const ID_KB_CUSTOM: u32 = 122;
const ID_KB_PRESET: u32 = 130; // + index
const ID_GPU_HYBRID: u32 = 150;
const ID_GPU_DISCRETE: u32 = 151;
const ID_AUTOSTART: u32 = 160;
const ID_QUIT: u32 = 199;

const PRESETS: [(&str, [u32; 4]); 9] = [
    ("White", [0xFFFFFF; 4]),
    ("Red", [0xFF0000; 4]),
    ("Orange", [0xFF5000; 4]),
    ("Green", [0x00FF00; 4]),
    ("Cyan", [0x00FFFF; 4]),
    ("Blue", [0x0000FF; 4]),
    ("Purple", [0x8000FF; 4]),
    ("Pink", [0xFF0080; 4]),
    ("Spectrum", [0xFF0000, 0xFFC000, 0x00FF40, 0x0040FF]),
];

struct App {
    hw: Hw,
    cfg: Config,
    hwnd: HWND,
    icon: HICON,
    dimmed: bool,
    autostart: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn alert(text: &str) {
    let t = wide(text);
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), w!("omenless"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST);
    }
}

fn report<T>(what: &str, r: windows::core::Result<T>) -> Option<T> {
    r.map_err(|e| alert(&format!("{what} failed: {}", e.message()))).ok()
}

fn main() {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\omenless"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = CoInitializeSecurity(None, -1, None, None, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE, None);

        let hw = match Hw::new() {
            Ok(h) => h,
            Err(e) => {
                alert(&format!("Can't reach the HP BIOS interface ({}).\nRun as administrator on an HP OMEN/Victus laptop.", e.message()));
                return;
            }
        };
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("--test") {
            let report = self_test(&hw, &Config::load());
            let _ = std::fs::write(args.get(2).map(String::as_str).unwrap_or("omenless-test.txt"), report);
            return;
        }

        let Ok(hinst) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: w!("omenless"), ..Default::default() };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("omenless"), w!("omenless"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
            return;
        };
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let _ = RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);

        let cfg = Config::load();
        let app = App { icon: sys::mode_icon(cfg.mode), hw, cfg, hwnd, dimmed: false, autostart: sys::autostart_enabled() };
        apply_all(&app);
        tray(&app, NIM_ADD);
        if app.cfg.idle_off {
            SetTimer(Some(hwnd), TIMER_IDLE, 5000, None);
        }
        APP.with(|a| *a.borrow_mut() = Some(app));

        spawn_omen_key_listener(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Exercises every write against the BIOS and reads it back. Leaves the saved mode applied
/// and the keyboard as it was.
fn self_test(hw: &Hw, cfg: &Config) -> String {
    use std::fmt::Write;
    let wait = |ms| std::thread::sleep(std::time::Duration::from_millis(ms));
    let mut out = String::new();
    let mut line = |s: String| {
        let _ = writeln!(out, "{s}");
    };
    line(format!("fans {:?} rpm, gpu {:?}", hw.fan_rpm(), hw.gpu().map(|g| g as u8)));
    for mode in [Mode::Performance, Mode::Eco, Mode::Balanced] {
        let r = hw.apply_mode(mode);
        wait(800);
        line(format!("{:<11} apply {:?} | gpu status {:?} | power {:?}", mode.name(), r.map_err(|e| e.message()), hw.debug_gpu_status(), hw.debug_power()));
    }
    let before = hw.kb_colors();
    line(format!("kb on {:?}, colours {:06X?}", hw.kb_on(), before));
    let spectrum = PRESETS[8].1;
    line(format!("kb set spectrum {:?} -> {:06X?}", hw.set_kb_colors(spectrum).map_err(|e| e.message()), hw.kb_colors()));
    wait(1500);
    if let Ok(c) = before {
        line(format!("kb restore {:?}", hw.set_kb_colors(c).map_err(|e| e.message())));
    }
    line(format!("max fan on {:?}", hw.set_max_fan(true).map_err(|e| e.message())));
    wait(6000);
    line(format!("  -> fans {:?}", hw.fan_rpm()));
    line(format!("max fan off {:?}", hw.set_max_fan(false).map_err(|e| e.message())));
    wait(1000);
    line(format!("  -> fans {:?}", hw.fan_rpm()));
    line(format!("final apply {} {:?}", cfg.mode.name(), hw.apply_mode(cfg.mode).map_err(|e| e.message())));
    sys::set_power_overlay(cfg.mode);
    out
}

fn apply_all(app: &App) {
    let _ = app.hw.apply_mode(app.cfg.mode);
    let _ = app.hw.set_max_fan(app.cfg.max_fan);
    sys::set_power_overlay(app.cfg.mode);
}

fn tray(app: &App, op: NOTIFY_ICON_MESSAGE) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: app.icon,
        ..Default::default()
    };
    let tip = format!("{}{}", app.cfg.mode.name(), if app.cfg.max_fan { " · max fans" } else { "" });
    for (d, s) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *d = s;
    }
    unsafe {
        let _ = Shell_NotifyIconW(op, &nid);
    }
}

fn set_mode(app: &mut App, mode: Mode) {
    app.cfg.mode = mode;
    app.cfg.save();
    report("Setting mode", app.hw.apply_mode(mode));
    sys::set_power_overlay(mode);
    let old = app.icon;
    app.icon = sys::mode_icon(mode);
    tray(app, NIM_MODIFY);
    unsafe {
        let _ = DestroyIcon(old);
    }
}

// ---------- menu ----------

struct MenuState {
    mode: Mode,
    rpm: Vec<u32>,
    max_fan: bool,
    kb_on: bool,
    kb: [u32; 4],
    idle_off: bool,
    gpu: Option<Gpu>,
    autostart: bool,
}

fn add(menu: HMENU, id: u32, text: &str, checked: bool, enabled: bool) {
    let t = wide(text);
    let mut flags = MF_STRING;
    if checked {
        flags |= MF_CHECKED;
    }
    if !enabled {
        flags |= MF_GRAYED;
    }
    unsafe {
        let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(t.as_ptr()));
    }
}

fn sep(menu: HMENU) {
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

fn submenu(menu: HMENU, sub: HMENU, text: &str) {
    let t = wide(text);
    unsafe {
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(t.as_ptr()));
    }
}

fn build_menu(s: &MenuState) -> windows::core::Result<HMENU> {
    unsafe {
        let m = CreatePopupMenu()?;
        let fans = if s.rpm.is_empty() { "Fans —".to_string() } else { format!("Fans  {} rpm", s.rpm.iter().map(u32::to_string).collect::<Vec<_>>().join(" · ")) };
        add(m, 0, &fans, false, false);
        sep(m);
        for mode in Mode::ALL {
            add(m, ID_MODE + mode.as_u32(), mode.name(), false, true);
        }
        let _ = CheckMenuRadioItem(m, ID_MODE, ID_MODE + 2, ID_MODE + s.mode.as_u32(), MF_BYCOMMAND.0);
        sep(m);
        add(m, ID_MAX_FAN, "Max fans", s.max_fan, true);

        let kb = CreatePopupMenu()?;
        add(kb, ID_KB_ON, "Backlight", s.kb_on, true);
        add(kb, ID_KB_IDLE, "Turn off when idle", s.idle_off, true);
        sep(kb);
        for (i, (name, colors)) in PRESETS.iter().enumerate() {
            add(kb, ID_KB_PRESET + i as u32, name, *colors == s.kb, true);
        }
        add(kb, ID_KB_CUSTOM, "Custom colour…", false, true);
        submenu(m, kb, "Keyboard");

        if let Some(gpu) = s.gpu {
            let g = CreatePopupMenu()?;
            add(g, ID_GPU_HYBRID, "Hybrid (saves battery)", false, true);
            add(g, ID_GPU_DISCRETE, "Discrete only", false, true);
            let cur = if gpu == Gpu::Discrete { ID_GPU_DISCRETE } else { ID_GPU_HYBRID };
            let _ = CheckMenuRadioItem(g, ID_GPU_HYBRID, ID_GPU_DISCRETE, cur, MF_BYCOMMAND.0);
            submenu(m, g, "Graphics");
        }
        sep(m);
        add(m, ID_AUTOSTART, "Start with Windows", s.autostart, true);
        add(m, ID_QUIT, "Quit", false, true);
        Ok(m)
    }
}

fn show_menu(hwnd: HWND) {
    let Some(state) = with_app(|a| MenuState {
        mode: a.cfg.mode,
        rpm: a.hw.fan_rpm().unwrap_or_default(),
        // The BIOS max-fan getter reads 0 even while it is on, so trust our own setting.
        max_fan: a.cfg.max_fan,
        kb_on: a.hw.kb_on().unwrap_or(false),
        kb: a.hw.kb_colors().unwrap_or_default(),
        idle_off: a.cfg.idle_off,
        gpu: if a.hw.gpu_switch_supported() { a.hw.gpu().ok() } else { None },
        autostart: a.autostart,
    }) else {
        return;
    };
    let Ok(menu) = build_menu(&state) else { return };
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if cmd != 0 {
            command(hwnd, cmd, &state);
        }
    }
}

fn command(hwnd: HWND, id: u32, s: &MenuState) {
    match id {
        _ if (ID_MODE..ID_MODE + 3).contains(&id) => {
            with_app(|a| set_mode(a, Mode::from_u32(id - ID_MODE)));
        }
        ID_MAX_FAN => {
            with_app(|a| {
                a.cfg.max_fan = !a.cfg.max_fan;
                a.cfg.save();
                report("Max fans", a.hw.set_max_fan(a.cfg.max_fan));
                tray(a, NIM_MODIFY);
            });
        }
        ID_KB_ON => {
            with_app(|a| report("Keyboard backlight", a.hw.set_kb_on(!s.kb_on)));
        }
        ID_KB_IDLE => {
            with_app(|a| {
                a.cfg.idle_off = !a.cfg.idle_off;
                a.cfg.save();
                unsafe {
                    if a.cfg.idle_off {
                        SetTimer(Some(hwnd), TIMER_IDLE, 5000, None);
                    } else {
                        let _ = KillTimer(Some(hwnd), TIMER_IDLE);
                        a.dimmed = false;
                    }
                }
            });
        }
        _ if (ID_KB_PRESET..ID_KB_PRESET + PRESETS.len() as u32).contains(&id) => {
            let colors = PRESETS[(id - ID_KB_PRESET) as usize].1;
            with_app(|a| {
                if report("Keyboard colour", a.hw.set_kb_colors(colors)).is_some() && !s.kb_on {
                    let _ = a.hw.set_kb_on(true);
                }
            });
        }
        ID_KB_CUSTOM => {
            if let Some(rgb) = pick_color(hwnd, s.kb[0]) {
                with_app(|a| {
                    if report("Keyboard colour", a.hw.set_kb_colors([rgb; 4])).is_some() && !s.kb_on {
                        let _ = a.hw.set_kb_on(true);
                    }
                });
            }
        }
        ID_GPU_HYBRID | ID_GPU_DISCRETE => {
            let want = if id == ID_GPU_DISCRETE { Gpu::Discrete } else { Gpu::Hybrid };
            if Some(want) != s.gpu {
                switch_gpu(want);
            }
        }
        ID_AUTOSTART => {
            let on = !s.autostart;
            if sys::set_autostart(on) {
                with_app(|a| a.autostart = on);
            } else {
                alert("Couldn't update the logon task.");
            }
        }
        ID_QUIT => unsafe {
            with_app(|a| tray(a, NIM_DELETE));
            PostQuitMessage(0);
        },
        _ => {}
    }
}

fn switch_gpu(want: Gpu) {
    let name = if want == Gpu::Discrete { "Discrete only" } else { "Hybrid" };
    let text = wide(&format!("Switch graphics to {name}?\n\nThe change needs a restart. Restart now?"));
    let r = unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), w!("omenless"), MB_YESNOCANCEL | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST) };
    if r == IDCANCEL {
        return;
    }
    if with_app(|a| report("Graphics switch", a.hw.set_gpu(want))).flatten().is_none() {
        return;
    }
    if r == IDYES {
        let _ = std::process::Command::new("shutdown").args(["/r", "/t", "0"]).spawn();
    }
}

fn pick_color(hwnd: HWND, current: u32) -> Option<u32> {
    static mut CUSTOM: [COLORREF; 16] = [COLORREF(0xFFFFFF); 16];
    let bgr = |c: u32| ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF);
    unsafe {
        let mut cc = CHOOSECOLORW {
            lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: hwnd,
            rgbResult: COLORREF(bgr(current)),
            lpCustColors: &raw mut CUSTOM as *mut COLORREF,
            Flags: CC_RGBINIT | CC_FULLOPEN,
            ..Default::default()
        };
        let _ = SetForegroundWindow(hwnd);
        ChooseColorW(&mut cc).as_bool().then(|| bgr(cc.rgbResult.0))
    }
}

// ---------- idle backlight ----------

fn idle_tick(app: &mut App) {
    let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut lii).as_bool() {
            return;
        }
        let idle = GetTickCount().wrapping_sub(lii.dwTime);
        if !app.dimmed {
            // Only dim a backlight that is on; respect Fn+F4 turning it off.
            if idle >= IDLE_AFTER_MS && app.hw.kb_on().unwrap_or(false) && app.hw.set_kb_on(false).is_ok() {
                app.dimmed = true;
                SetTimer(Some(app.hwnd), TIMER_IDLE, 250, None);
            }
        } else if idle < 1000 {
            let _ = app.hw.set_kb_on(true);
            app.dimmed = false;
            SetTimer(Some(app.hwnd), TIMER_IDLE, 5000, None);
        }
    }
}

// ---------- OMEN key (BIOS WMI event 29 / 0x21A5) ----------

fn spawn_omen_key_listener(hwnd: HWND) {
    let target = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(svc) = bios::connect("root\\wmi") else { return };
        let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
        let Ok(events) = svc.ExecNotificationQuery(&BSTR::from("WQL"), &BSTR::from("SELECT * FROM hpqBEvnt"), flags, None) else { return };
        loop {
            let mut objs = [None];
            let mut n = 0u32;
            if events.Next(-1, &mut objs, &mut n).is_err() {
                return;
            }
            let Some(ev) = objs[0].take() else { continue };
            let read = |name: PCWSTR| {
                let mut v = windows::Win32::System::Variant::VARIANT::default();
                ev.Get(name, 0, &mut v, None, None).ok().and_then(|_| i32::try_from(&v).ok())
            };
            if read(w!("EventID")) == Some(29) && read(w!("EventData")) == Some(0x21A5) {
                let _ = PostMessageW(Some(HWND(target as _)), WM_OMEN_KEY, WPARAM(0), LPARAM(0));
            }
        }
    });
}

// ---------- window proc ----------

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                let ev = (lp.0 & 0xFFFF) as u32;
                if ev == WM_LBUTTONUP || ev == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_OMEN_KEY => {
                with_app(|a| set_mode(a, a.cfg.mode.next()));
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == TIMER_REAPPLY {
                    let _ = KillTimer(Some(hwnd), TIMER_REAPPLY);
                    with_app(|a| apply_all(a));
                } else if wp.0 == TIMER_IDLE {
                    with_app(idle_tick);
                }
                LRESULT(0)
            }
            WM_POWERBROADCAST => {
                // Resume or AC/battery switch: BIOS may reset limits, re-apply shortly after.
                if wp.0 == PBT_APMRESUMEAUTOMATIC as usize || wp.0 == PBT_POWERSETTINGCHANGE as usize {
                    SetTimer(Some(hwnd), TIMER_REAPPLY, 2000, None);
                }
                LRESULT(1)
            }
            _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                with_app(|a| tray(a, NIM_ADD));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
