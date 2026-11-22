# HP BIOS WMI protocol (Victus 16-s0, board 8BD4)

Found by decompiling OMEN Gaming Hub 1101.2611.2.0 (`OmenHsaClient`, `PerformanceControlHelper`,
`PowerControlBase`, `AdaptivePowerControlV1`, `FourZoneHelper`, `GraphicsSwitcherHelper`) and
checked on hardware with read-back.

## Transport

`root\wmi` → `hpqBIntM` instance → `hpqBIOSInt{0,4,128,1024,4096}(InData)`; the number is the
reply size. `InData` is an `hpqBDataIn`:

| field | value |
|---|---|
| Sign | `53 45 43 55` ("SECU") |
| Command | `1` read, `2` write, `0x20008` OMEN, `0x20009` keyboard light |
| CommandType | see below |
| Size / hpqBData | payload |

Reply `OutData`: `rwReturnCode` (0 = ok), `Data`. Needs admin.

## 0x20008 (OMEN)

| type | dir | payload / reply |
|---|---|---|
| 0x10 | get | `[fan count]` |
| 0x1A | set | `[0xFF, mode, biosFanControl, 0]`; mode V0: 0 default, 1 performance; V1: 0x30 default, 0x31 performance, 0x50 cool |
| 0x21 | get | `[cTGP, DynamicBoost(PPAB), D-state, GPS]` |
| 0x22 | set | `[cTGP, PPAB, D-state(1), 0x57]` |
| 0x23 | get | `[index]` → board sensor °C (0 IR, 1 ambient, 2 PCH, 3 VR) |
| 0x26/0x27 | get/set | max fan; set payload is 1 byte. Getter reads 0 even when on. |
| 0x28 | get | system design data: `[3]` thermal policy version, `[4]&1` software fan control, `[7]` graphics modes (bit1 hybrid, bit2 discrete) |
| 0x29 | set | `[PL2, PL1, PL4, concurrentTDP]`, 0xFF = keep (watts) |
| 0x2D | get | fan speeds, ×100 rpm |
| 0x2E | set | software fan level per fan (×100 rpm; +128 = reverse/clean) |
| 0x2F | get | BIOS fan table |
| 0x31 | set | idle on/off hint for the keyboard light |

## 0x20009 (keyboard)

| type | dir | payload |
|---|---|---|
| 2/3 | get/set | 128-byte block; zone *i* RGB at `25 + 3i` (4 zones) |
| 4/5 | get/set | brightness: `0xE4` on, `0x64` off (bit 7 = on) |

## Command 1/2, type 0x52

Graphics mux: `0` hybrid, `1` discrete. Write `[mode,0,0,0]`, applies after restart.

## Events

`hpqBEvnt` with `EventID 29`, `EventData 0x21A5` = OMEN key.

## What OMEN does per mode on this SKU

| mode | fan | cTGP | Dynamic Boost | PL1 |
|---|---|---|---|---|
| Eco | 0x30 | off | off | dynamic (25–35 W band) |
| Balanced | 0x30 | off | on | 55 W |
| Performance | 0x31 | on | on | 65 W |

In "auto" fans it runs its own software fan curve every second (`SwFanControl`). omenless instead
passes `biosFanControl = 1`, the path HP uses itself on battery/eco, so the BIOS fan table runs
and no software loop is needed.
