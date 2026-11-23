# omenless

The useful part of OMEN Gaming Hub as one tray icon. ~250 KB Rust exe, ~15 MB RAM (OMEN's
background process alone used 323 MB), no services, no telemetry. The only background work is a
fan tick every 2 s.

## Features

- **Modes:** Eco, Balanced, Performance. Each sets the BIOS fan mode, NVIDIA cTGP + Dynamic
  Boost, the CPU power limit (35 / 55 / 65 W; the BIOS caps Performance at 60 W here) and the
  Windows power slider.
- **Fans:** Auto (HP's own curves per mode), Custom curve (drag-the-points editor with live
  CPU/GPU temps), Max. Max takes 10–15 s to spin up.
- **Keyboard:** 4-zone colour presets, custom colour, backlight on/off, off after 2 min idle.
- **Graphics:** Hybrid / Discrete only (needs a restart).
- **OMEN key** cycles Balanced → Performance → Eco.
- Re-applies everything after sleep and on AC/battery change.
- Start with Windows via an elevated logon task (no UAC prompt).

Tray icon: green ring = Eco, ring + dot = Balanced, red disc = Performance. The menu header shows
CPU / GPU temperature and both fan speeds.

Built for the Victus 16-s0 (board 8BD4). Other HP OMEN/Victus models use the same BIOS interface
but may differ; see [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Fans: read this

This laptop has **no fan curve in the BIOS**. The fans hold whatever speed was set last; OMEN
Gaming Hub set one every second. With OMEN stopped and nothing else running, the fans stayed at
0 rpm under 100 % CPU load. omenless runs that loop itself:

- Inputs are driverless: the ACPI CPU zone (`\_TZ.TSZ0`) and the GPU temperature Task Manager
  uses (D3DKMT). Reading it never wakes a sleeping NVIDIA GPU.
- CPU ≥ 92 °C or GPU ≥ 87 °C: full speed regardless of mode or curve.
- No temperature readable: fixed 3000 rpm.
- Quit from the menu: fans are left at 3000 rpm, never stopped.
- If omenless isn't running at all (not started, crashed), nothing drives the fans. Keep
  **Start with Windows** on.

## Install

```powershell
./scripts/install.ps1
```

Builds, copies to `C:\Program Files\omenless` (admin-only folder, since it runs elevated) and
starts it. Then tick **Start with Windows** in the menu.

`./scripts/install.ps1 -Test report.txt` (or `omenless.exe --test report.txt`) runs a hardware
self-test first: cycles modes, flashes the keyboard, max fans for 6 s, runs the fan loop, and
writes the read-backs to the file. `cargo test` covers the curve math.

## Replacing OMEN Gaming Hub

Don't run both: they fight over the fans. What was done on the reference machine:

- Uninstalled OMEN Gaming Hub: `Get-AppxPackage AD2F1837.OMENCommandCenter | Remove-AppxPackage`
- Disabled HP's telemetry service: `Set-Service HpTouchpointAnalyticsService -StartupType Disabled`
  (elevated)
- Kept HP's driver services (`HPOmenCap` and the other `*Cap` HSA services); other HP drivers may
  rely on them.

Not removed: `C:\Program Files\HP\OmenInstallMonitor` keeps running, and it isn't started by any
scheduled task, service or Run key. If OMEN reappears after a reboot, this is the likely cause.

## Status

Verified on hardware with read-back: all three modes, power limits, keyboard colours, Max fans,
and the Auto and Custom fan loop.

Not tested yet: the Graphics switch (needs a restart), the OMEN key event, idle backlight.
