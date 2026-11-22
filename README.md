# omenless

The useful part of OMEN Gaming Hub as one tray icon. ~250 KB Rust exe, no services, no
telemetry, no polling while idle.

- Modes: Eco, Balanced, Performance (fan mode, NVIDIA cTGP + Dynamic Boost, CPU power limit,
  Windows power slider)
- Fans: Auto (HP's own curves per mode), Custom curve (drag-the-points editor with live CPU/GPU
  temps), Max. This laptop has no BIOS fan curve, so omenless runs the loop itself every 2 s
  from driverless sensors.
- Keyboard: 4-zone colour presets, custom colour, backlight on/off, off when idle
- Graphics: Hybrid / Discrete only (restart)
- OMEN key cycles Balanced → Performance → Eco
- Re-applies after sleep and on AC/battery change
- Start with Windows via an elevated logon task (no UAC prompt)

Tray icon: ring = Eco, ring + dot = Balanced, red disc = Performance.

Built for the Victus 16-s0 (board 8BD4). Other HP OMEN/Victus models use the same BIOS
interface but may differ; see [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Install

```powershell
./scripts/install.ps1
```

Builds, copies to `C:\Program Files\omenless` (admin-only folder, since it runs elevated) and
starts it. Then tick **Start with Windows** in the menu.

`./scripts/install.ps1 -Test report.txt` (or `omenless.exe --test report.txt`) runs a hardware
self-test first: cycles modes, flashes the keyboard, max fans for 6 s, runs the fan loop, and
writes the read-backs to the file. `cargo test` covers the curve math.

Don't run OMEN Gaming Hub at the same time: both drive the fans.
