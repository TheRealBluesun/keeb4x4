# keeb4x4 v2 — fabrication package

Generated 2026-07-24 from `keeb4x4.kicad_pcb` (DRC: 0 errors, 0 unconnected, 0 dangling).

## Files

| File | Upload to |
|---|---|
| `keeb4x4_gerbers.zip` | JLCPCB "Add gerber file" |
| `keeb4x4_bom.csv` | JLCPCB assembly — BOM |
| `keeb4x4_cpl.csv` | JLCPCB assembly — CPL / centroid |
| `gerbers/` | loose copies of everything in the zip |

## PCB order settings

| Setting | Value |
|---|---|
| Dimensions | 100 × 142.5 mm |
| Layers | **4** (F.Cu, In1.Cu = GND plane, In2.Cu = VBUS plane, B.Cu) |
| Thickness | 1.6 mm |
| Min track / clearance | 0.20 mm / 0.20 mm (well inside JLC's 0.0889 mm capability) |
| Vias | 0.60 mm pad / 0.30 mm drill |
| Copper weight | 1 oz |
| Surface finish | HASL is fine; ENIG if you want flatter pads for the 0.5 mm FPC connectors |
| Impedance control | not required |

Drill totals: 366 PTH, 58 NPTH (16 × 4.0 mm MX centre posts, 32 × 1.7 mm MX
alignment pegs, 8 × 3.2 mm M3 mounting holes, 2 × 0.65 mm EC11 posts).

## Assembly

100 placements, **double-sided**:

- **Bottom (85)** — all passives, STM32, flash, decoder, regulator, LEDs.
  The SK6812MINI-E are reverse-mount: they sit on B.Cu and emit upward
  through board cutouts.
- **Top (15)** — J1 USB-C, J4–J15 ScreenKey FPC connectors, SW18/SW19
  (BOOT/RESET).

Double-sided SMT costs more than single-sided at JLC. The top-side parts are
all there for physical reasons (ribbons come down from the keycaps, USB and
buttons must be reachable), so it isn't reducible.

### Not in the BOM — hand-solder these yourself

Through-hole, and JLC either won't place them or doesn't stock them:

- `SW1`–`SW16` — Cherry MX / Kailh Box switches
- `SW17` — **Alps Alpine EC11E15244G1** (chosen 2026-07-27, Digikey).
  See the part-selection notes below.
- `J2` — 1×4 header, SSD1306 OLED
- `J3` — 1×5 header, SWD
- `H1`–`H8` — M3 mounting holes (no part)

### LED pinout — fixed 2026-07-27

An earlier revision of this board had the SK6812 nets on the **wrong pads**.
The schematic used symbol `LED:SK6812MINI` (pins 1=DOUT, 2=VSS, 3=DIN, 4=VDD)
against footprint `LED_SK6812MINI-E_..._ReverseMount`, whose correct symbol is
`LED:SK6812MINI-E` (1=GND, 2=DIN, 3=VDD, 4=DOUT — confirmed against the
OPSCO SK6812MINI-E-012 datasheet, §5). Every LED had VDD on a data line.

Now corrected and verified: pad1=GND, pad2=DIN, pad3=VDD, pad4=DOUT on all 16,
with the chain running `LED_DATA → LED1 → … → LED16`. **Aligning JLC's pin-1
marker to pad 1 is now the right thing to do for the LEDs.**

### ⚠️ Check part rotations in JLC's preview

KiCad's footprint orientation and JLC's reel orientation disagree for a lot
of parts, and the CPL here uses KiCad's angles unmodified. JLC's online
preview renders each part on the board — step through it before paying.
Highest-risk parts on this board:

- `U2`/`U3`/`U4` (SOT-23-5/-6) and `Q1` (SOT-23) — SOT packages are the most
  common offenders, usually off by 180°
- `J1` USB-C
- `LED1`–`LED16` — reverse-mount, so a rotation error puts DIN/DOUT backwards
  and breaks the whole WS2812 chain
- `D1`–`D16` (SOD-123) — polarity
- `U6` SOIC-8 flash and `U1` LQFP-48 — check pin-1 dot placement

## SW17 rotary encoder — Alps EC11E15244G1

Chosen 2026-07-27. Footprint on the board is
`Rotary_Encoder:RotaryEncoder_Alps_EC11E-Switch_Vertical_H20mm`.

| Spec | Value |
|---|---|
| Detents / pulses per rev | **30 / 15** (half-step: 2 detents per quadrature cycle) |
| Push switch | Yes — 0.5 mm travel, 6 (+2.5/−2) N |
| Shaft | vertical, flat, 20 mm, ⌀6 mm |
| Detent torque | 10 ±7 mN·m |
| Land pattern | A/C/B 1.0 mm ⌀ at 2.5 mm pitch; S1/S2 14.5 mm away, 5.0 mm apart; 2 mounting slots 2.8 × 1.5 mm, 11.2 mm apart at 7.5 mm offset |

**Alps EC11E part-number decode** (worked out from the Alps product pages):
`EC11E` + `[pulses, 2 digits]` + `[20 = no switch / 24 = with switch]` + `4`
+ `[variant]`. So `EC11E**15**24**4G1**` = 15 pulses, with switch. This is the
only reliable way to read the family — Digikey's descriptions don't mention
the switch at all, and half the `EC11E…` catalogue has no switch fitted.

**Parts that do NOT fit — already checked, don't re-tread:**

- **Bourns PEC11R** (any) — despite the name it's a 12 mm body with a much
  tighter pin cluster: switch pins only 2.5 mm from A/C/B (we need 14.5 mm),
  posts 10.8 mm apart (we need 11.2 mm).
- **Bourns PEC12R** (`-3xxx` *and* `-4xxx`) — all five pin positions match
  ours exactly, but the mounting legs are **2.0 mm** thick and our slots are
  only **1.5 mm**. The `-3`/`-4` digit is the M9×0.75 bushing, not the legs,
  so switching variants doesn't help.
- Any `EC11E…20…` — no push switch, so the knob-press on PC13 would be dead.
- `EC11M…`, `EC11J…` (SMD), `EC1110…` — different sub-series, land pattern
  not verified.

**Firmware note (TIM3, ENC_A=PA6 / ENC_B=PA7):** detents are 2× pulses, so
one quadrature cycle spans two clicks. Configure the timer's encoder mode to
count on **one channel only (×2)** for exactly one count per detent. Counting
both edges (×4) gives two counts per click; counting full cycles gives one
count per two clicks.

## Sourcing watch items

- **`U1` STM32G431CBT6 (C529355)** — the tightest line. ~1,670 in stock and
  down sharply from April. Re-check before ordering; consider buying the MCUs
  separately as insurance.
- **`U5` 74HC154 (C2832236)** — ~1,292 in stock, aging part.
- **`J4`–`J15` Hirose FH12 (C88360)** — ~3,595 in stock, $0.31 ea. JUSHUO
  `AFC01-S12FCA-00` (C262661) is the same 12P / 0.5 mm / bottom-contact /
  hinged-lid spec at $0.079 with 118 k stock — worth swapping at volume, but
  verify the land pattern first.

Roughly $12/board in JLC-assembled parts at qty 5, plus ~$27 one-time in
Extended-part fees across 9 Extended lines.
