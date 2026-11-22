//! Fan curve editor: one small GDI window with six draggable points and live temperatures.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::fan::{Curve, DEFAULT_CURVE, MAX_LEVEL, MIN_LEVEL};

const T_MIN: f32 = 30.0;
const T_MAX: f32 = 100.0;
const L_TOP: f32 = 60.0;

struct Editor {
    hwnd: HWND,
    curve: Curve,
    drag: Option<usize>,
}

thread_local! {
    static ED: RefCell<Option<Editor>> = const { RefCell::new(None) };
}
static REGISTERED: AtomicBool = AtomicBool::new(false);

fn color(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

pub fn open(curve: Curve) {
    if let Some(h) = ED.with(|e| e.borrow().as_ref().map(|e| e.hwnd)) {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else { return };
        if !REGISTERED.swap(true, Ordering::Relaxed) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: w!("omenless-curve"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }
        let s = GetDpiForSystem() as i32;
        let (cw, ch) = (560 * s / 96, 360 * s / 96);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST,
            w!("omenless-curve"),
            w!("Fan curve"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            cw,
            ch,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            return;
        };
        let dark = BOOL::from(!crate::sys::light_apps());
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        ED.with(|e| *e.borrow_mut() = Some(Editor { hwnd, curve, drag: None }));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Repaint the live markers (called from the fan tick).
pub fn refresh() {
    if let Some(h) = ED.with(|e| e.borrow().as_ref().map(|e| e.hwnd)) {
        unsafe {
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

struct Plot {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    s: f32,
}

impl Plot {
    fn of(hwnd: HWND) -> Self {
        let mut rc = RECT::default();
        let s = unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
            GetDpiForWindow(hwnd) as f32 / 96.0
        };
        Plot { l: 52.0 * s, t: 34.0 * s, r: rc.right as f32 - 20.0 * s, b: rc.bottom as f32 - 52.0 * s, s }
    }
    fn px(&self, t: f32, lvl: f32) -> POINT {
        POINT {
            x: (self.l + (t - T_MIN) / (T_MAX - T_MIN) * (self.r - self.l)) as i32,
            y: (self.b - lvl / L_TOP * (self.b - self.t)) as i32,
        }
    }
    fn val(&self, x: i32, y: i32) -> (f32, f32) {
        let t = T_MIN + (x as f32 - self.l) / (self.r - self.l) * (T_MAX - T_MIN);
        let l = (self.b - y as f32) / (self.b - self.t) * L_TOP;
        (t, l)
    }
}

fn commit(curve: Curve) {
    crate::with_app(|a| {
        a.cfg.curve = curve;
        a.cfg.save();
        a.fan.force();
    });
}

unsafe fn text(dc: HDC, x: i32, y: i32, s: &str) {
    let w: Vec<u16> = s.encode_utf16().collect();
    let _ = TextOutW(dc, x, y, &w);
}

unsafe fn paint(hwnd: HWND, curve: &Curve, drag: Option<usize>) {
    let light = crate::sys::light_apps();
    let (bg, grid, ink, accent) = if light { (0xF5F5F7, 0xDDDDE2, 0x6E6E73, 0x0A84FF) } else { (0x1C1C1E, 0x3A3A3C, 0x98989D, 0x0A84FF) };
    let (cpu_c, gpu_c) = (0xFF9F0A, 0x30D158);
    let live = crate::with_app(|a| (a.fan.cpu, a.fan.gpu, a.fan.level.map(|l| (l, a.fan.gpu_level(l))))).unwrap_or_default();

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let dc = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
    let old_bmp = SelectObject(dc, bmp.into());
    let p = Plot::of(hwnd);
    let s = p.s;

    let brush = CreateSolidBrush(color(bg));
    FillRect(dc, &rc, brush);
    let _ = DeleteObject(brush.into());
    let font = CreateFontW((-13.0 * s) as i32, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"));
    let old_font = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, color(ink));

    // grid + labels
    let pen = CreatePen(PS_SOLID, 1, color(grid));
    let old_pen = SelectObject(dc, pen.into());
    for t in (30..=100).step_by(10) {
        let a = p.px(t as f32, 0.0);
        let b = p.px(t as f32, L_TOP);
        let _ = MoveToEx(dc, a.x, a.y, None);
        let _ = LineTo(dc, b.x, b.y);
        text(dc, a.x - (8.0 * s) as i32, a.y + (6.0 * s) as i32, &format!("{t}°"));
    }
    for l in (0..=60).step_by(10) {
        let a = p.px(T_MIN, l as f32);
        let b = p.px(T_MAX, l as f32);
        let _ = MoveToEx(dc, a.x, a.y, None);
        let _ = LineTo(dc, b.x, b.y);
        let label = if l == 0 { "off".to_string() } else { format!("{}k", l / 10) };
        text(dc, (10.0 * s) as i32, a.y - (8.0 * s) as i32, &label);
    }
    SelectObject(dc, old_pen);
    let _ = DeleteObject(pen.into());

    // live temperatures
    let mut header = Vec::new();
    for (temp, c, name) in [(live.0, cpu_c, "CPU"), (live.1, gpu_c, "GPU")] {
        let Some(t) = temp else { continue };
        let pen = CreatePen(PS_DOT, 1, color(c));
        let old = SelectObject(dc, pen.into());
        let a = p.px(t.clamp(T_MIN, T_MAX), 0.0);
        let b = p.px(t.clamp(T_MIN, T_MAX), L_TOP);
        let _ = MoveToEx(dc, a.x, a.y, None);
        let _ = LineTo(dc, b.x, b.y);
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
        header.push((c, format!("{name} {t:.0}°")));
    }
    let mut x = p.l as i32;
    for (c, h) in &header {
        SetTextColor(dc, color(*c));
        text(dc, x, (9.0 * s) as i32, h);
        x += (70.0 * s) as i32;
    }
    SetTextColor(dc, color(ink));
    if let Some((cpu, gpu)) = live.2 {
        let now = if cpu == 0 { "fans off".to_string() } else { format!("fans {} / {} rpm", cpu as u32 * 100, gpu as u32 * 100) };
        text(dc, x, (9.0 * s) as i32, &now);
    }

    // curve
    let mut pts = vec![p.px(T_MIN, curve[0].1 as f32)];
    pts.extend(curve.iter().map(|&(t, l)| p.px(t as f32, l as f32)));
    pts.push(p.px(T_MAX, curve[5].1 as f32));
    let pen = CreatePen(PS_SOLID, (2.5 * s) as i32, color(accent));
    let old = SelectObject(dc, pen.into());
    let _ = Polyline(dc, &pts);
    let brush = CreateSolidBrush(color(accent));
    let old_brush = SelectObject(dc, brush.into());
    for (i, &(t, l)) in curve.iter().enumerate() {
        let c = p.px(t as f32, l as f32);
        let r = if drag == Some(i) { 7.0 * s } else { 5.0 * s } as i32;
        let _ = Ellipse(dc, c.x - r, c.y - r, c.x + r, c.y + r);
        if drag == Some(i) {
            let lv = if l == 0 { "off".to_string() } else { format!("{} rpm", l as u32 * 100) };
            text(dc, c.x + (10.0 * s) as i32, c.y - (22.0 * s) as i32, &format!("{t}° · {lv}"));
        }
    }
    SelectObject(dc, old_brush);
    let _ = DeleteObject(brush.into());
    SelectObject(dc, old);
    let _ = DeleteObject(pen.into());

    text(dc, p.l as i32, rc.bottom - (24.0 * s) as i32, "Drag the points. Fans follow the hotter of CPU and GPU. Right-click to reset.");

    let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(dc), 0, 0, SRCCOPY);
    SelectObject(dc, old_font);
    let _ = DeleteObject(font.into());
    SelectObject(dc, old_bmp);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(dc);
    let _ = EndPaint(hwnd, &ps);
}

fn mouse(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let state = ED.with(|e| e.borrow().as_ref().map(|e| (e.curve, e.drag)));
                if let Some((c, d)) = state {
                    paint(hwnd, &c, d);
                } else {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = mouse(lp);
                let p = Plot::of(hwnd);
                let hit = ED.with(|e| {
                    let mut e = e.borrow_mut();
                    let e = e.as_mut()?;
                    let i = e.curve.iter().position(|&(t, l)| {
                        let c = p.px(t as f32, l as f32);
                        ((c.x - x).pow(2) + (c.y - y).pow(2)) as f32 <= (12.0 * p.s).powi(2)
                    })?;
                    e.drag = Some(i);
                    Some(())
                });
                if hit.is_some() {
                    SetCapture(hwnd);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = mouse(lp);
                let p = Plot::of(hwnd);
                let (t, l) = p.val(x, y);
                let moved = ED.with(|e| {
                    let mut e = e.borrow_mut();
                    let e = e.as_mut()?;
                    let i = e.drag?;
                    let c = &mut e.curve;
                    let lo_t = if i == 0 { T_MIN } else { c[i - 1].0 as f32 + 2.0 };
                    let hi_t = if i == 5 { T_MAX } else { c[i + 1].0 as f32 - 2.0 };
                    let lo_l = if i == 0 { 0 } else { c[i - 1].1 };
                    let hi_l = if i == 5 { MAX_LEVEL } else { c[i + 1].1 };
                    let lvl = if l < MIN_LEVEL as f32 * 0.6 { 0 } else { (l.round() as u8).clamp(MIN_LEVEL, MAX_LEVEL) };
                    c[i] = (t.clamp(lo_t, hi_t).round() as u8, lvl.clamp(lo_l, hi_l.max(lo_l)));
                    Some(())
                });
                if moved.is_some() {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let done = ED.with(|e| e.borrow_mut().as_mut().and_then(|e| e.drag.take().map(|_| e.curve)));
                if let Some(c) = done {
                    let _ = ReleaseCapture();
                    commit(c);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                ED.with(|e| {
                    if let Some(e) = e.borrow_mut().as_mut() {
                        e.curve = DEFAULT_CURVE;
                    }
                });
                commit(DEFAULT_CURVE);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_DESTROY => {
                ED.with(|e| *e.borrow_mut() = None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
