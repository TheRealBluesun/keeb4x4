# keeb4x4 v2 — firmware notes (Rust + embassy)

Hardware facts established while designing the board. Everything here was read
off `keeb4x4.kicad_pcb` / `keeb4x4.kicad_sch`, not from memory.

MCU: **STM32G431CBT6**, LQFP-48, crystal-less USB (HSI48 + CRS).
32 of 48 pins used for signals.

## Pin map

| Function | Pin | Notes |
|---|---|---|
| COL0–3 | PA0, PA1, PA2, PA3 | matrix columns, **drive HIGH** |
| ROW0–3 | PB12, PB13, PB14, PB15 | matrix rows, **input pull-down** |
| ENC_A / ENC_B | PA6 / PA7 | TIM3_CH1/CH2, hardware quadrature |
| ENC_SW | PC13 | encoder push, active low |
| LED_DATA_MCU | PA8 | → U4 level shifter → SK6812 chain |
| SPI1 SCK / MOSI / MISO | PA5 / PB5 / PB4 | shared: 12 ScreenKeys + flash |
| LCD_DC / LCD_RST | PB0 / PB1 | **common to all 12 screens** |
| LCD_BL_PWM | PA9 | Q1 gate — dims *all* backlights together |
| SCS_A0–A3 | PA4, PB2, PB10, PB11 | 74HC154 address (A0 = LSB) |
| SCS_EN | PA10 | 74HC154 /E1, **active low** |
| FLASH_CS | PB6 | W25Q128, active low |
| I²C1 SCL / SDA | PA15 / PB7 | SSD1306 OLED header (J2) |
| USB DM / DP | PA11 / PA12 | |
| SWDIO / SWCLK | PA13 / PA14 | J3 |
| BOOT0 | PB8 | |

## Key matrix — COL2ROW, active high

`COLn → switch → K<row><col> → diode anode → cathode → ROWn`, one 1N4148W per
key (D1–D16) for N-key rollover.

The diode anodes face the **columns**, so current only flows column → row.
That means:

- Columns are outputs, driven **HIGH** one at a time.
- Rows are inputs with **internal pull-downs**. A pressed key reads HIGH.
- The usual drive-low / pull-up arrangement **will not work** — the diodes
  block it.

There are no external matrix resistors on the board; you rely on the STM32's
internal pull-downs. Scan all four columns in turn (~1 kHz is plenty) and
debounce ~5 ms in software.

Key numbering: `SW1..SW4` = row 0, `SW5..SW8` = row 1, etc. `SW13–SW16` (bottom
row) are plain keys with no ScreenKey connector.

## Rotary encoder — half-step

Alps **EC11E15244G1**: 30 detents, 15 pulses/rev, so one quadrature cycle spans
**two detents**.

Configure TIM3 encoder mode to count on **one channel only (×2)** → exactly one
count per detent. Counting both edges (×4) gives two counts per click; counting
full cycles gives one count per two clicks.

`ENC_SW` on PC13 is a plain GPIO, active low, needs a pull-up (internal is fine)
and debouncing.

## RGB — SK6812MINI-E ×16

PA8 → U4 (74AHCT1G125, /OE tied low) shifts 3.3 V → 5 V → `LED_DATA` → LED1.
Chain order is simple raster: **LED1 → LED2 → … → LED16**, left-to-right along
each row, top row first. Same index order as the switches.

Reverse-mount, emitting up through board cutouts. 800 kbit/s, GRB byte order,
24 bits/LED (see the OPSCO SK6812MINI-E datasheet).

Pad mapping was a bug once — see `fab/README.md`. It is now correct:
pad1 = GND, pad2 = DIN, pad3 = VDD, pad4 = DOUT.

## ScreenKeys — 12 × ST7735S on one SPI bus

Connectors J4–J15 (FH12 12-pin 0.5 mm FPC), pinout:
`1 GND, 2 LEDK, 3 NC, 4 NC, 5 LEDA, 6 VDD, 7 D/C, 8 CS, 9 SCLK, 10 MOSI, 11 RST, 12 GND`

**Chip select** goes through U5 (74HC154, 4→16 decoder, active-low outputs):

- `SCS_A3..A0` selects the screen; `SCS_1` = Y0 = **address 0** (J4), through
  to `SCS_12` = Y11 = **address 11** (J15). Addresses 12–15 are unused.
- `SCS_EN` (PA10) is `/E1`; `/E0` is hard-tied to GND. So **SCS_EN low =
  decoder enabled**, and the addressed output pulls low. Drive **SCS_EN high to
  deselect every screen** — do this before touching the flash.
- Set the address bits *before* asserting SCS_EN, or you'll glitch other CS
  lines as the address settles.

**Shared bus caveats:**

- `LCD_DC` and `LCD_RST` are common to all twelve. You cannot hold different
  screens in different DC states — set DC, then select, then clock.
- The W25Q128 flash sits on the same SPI1 bus with its own CS (PB6). The
  ST7735S tops out around 15 MHz while the flash will do 133 MHz, so either
  run the bus slow throughout or reconfigure the prescaler when switching
  between screen writes and asset reads.
- MISO (PB4) is only used by the flash — the screens are write-only.

**Backlights** are one global dimmer, not per-key: each key's LEDA goes through
its own 100 Ω (R6–R17) to VBUS, and all cathodes tie to `BL_K`, switched low-side
by Q1 (AO3400A, gate = PA9). PWM on PA9 dims all twelve together. ~20 mA each.

## ⚠️ Power budget — the real design constraint

Bus-powered from a desktop port with **no PD negotiation** (just the 5.1 kΩ CC
pulldowns), so the host never tells us what it can spare.

| Load | Worst case |
|---|---|
| 16 × SK6812 full white (36 mA) | ~576 mA |
| 12 × ScreenKey backlights (20 mA) | ~240 mA |
| 12 × ST7735S logic | ~96 mA |
| STM32 + flash | ~50 mA |
| **Total** | **~960 mA** |

USB-A 2.0 = 500 mA, USB 3.0 = 900 mA. **Full brightness on everything exceeds
both.** Nothing in hardware limits this — it has to be enforced in firmware.

Both knobs exist: global brightness scaling on the WS2812 data, and PWM duty on
PA9 for the backlights.

Worth doing properly rather than just clamping:

- **Content-aware estimator.** Before latching a frame, sum all 48 channel
  values, convert to predicted mA, and scale the whole frame if it exceeds
  budget. This is what FastLED's `set_max_power_in_volts_and_milliamps` does —
  you get full brightness whenever the frame is sparse, and graceful global
  dimming only when it isn't.
- Budget the backlights separately from the RGB, since backlight duty is a
  slow-moving UI decision and LED frames change fast.
- Dim or blank ScreenKeys that aren't displaying anything meaningful — twelve
  screens at 20 mA is a big fixed draw for content nobody's looking at.
- Assume 500 mA unless proven otherwise. Pick a conservative default and make
  the cap a runtime setting.

## Not wired / hand-solder

`SW1–SW16` (MX switches), `SW17` (EC11E15244G1), `J2` (OLED header), `J3` (SWD
header), `H1–H8` (M3 mounting) are all through-hole and excluded from the JLC
BOM/CPL. See `fab/README.md`.
