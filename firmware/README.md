# keeb-v2 firmware

Minimal Rust / Embassy bring-up for the STM32G431CBT6. Board wiring is
documented in [FIRMWARE_NOTES.md](../FIRMWARE_NOTES.md).

## Build and run

The application links at `0x08008000` behind the resident Rust USB bootloader.
USB updates and recovery have been tested on the board; see
[bootloader status and installation](../bootloader/README.md).
From the project root, build both images:

```sh
cargo build --manifest-path host/Cargo.toml --release --locked
host/target/release/keeb build
# After the one-time bootloader installation (see bootloader/README.md):
host/target/release/keeb update firmware/artifacts/update/keeb-v2.kbf
```

Rustup installs the pinned Rust toolchain and Cortex-M4F target. Install
`probe-rs` separately for initial installation and debugging. The previous
`cargo run` workflow does not maintain the boot commit record and its runner
has been removed. USB supplies power and exposes a CDC ACM event stream. No UART connection is
needed. See [host listener instructions](../host/README.md). RTT is always
nonblocking, even after probe-rs sets the channel to blocking mode, so
disconnecting the debugger cannot stall USB or the animation on a full log buffer.

```sh
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

## Current behavior

- Internal HSI clock, Embassy TIM4 timer driver, and a once-per-second RTT heartbeat.
- SPI1 at 4 MHz reads the W25Q128 JEDEC identity without modifying flash,
  then switches to 8 MHz for the display animation.
- Scans all 16 switch positions, with active-high columns, pull-down rows,
  10 µs settling, and 5 ms stable-state debounce. Logs presses and releases
  as `SW1` through `SW16`; the assembled top row is `SW1` through `SW4`.
- RGB LEDs start off. Each debounced press lights its corresponding LED dim
  white; release turns it off. Multiple held keys can light together. Brightness
  is scaled to a maximum of 16/255 per channel.
- Initializes the four connected ST7735S panels, then runs a stylized ocean
  sunset with rolling swells, golden reflections, and teal water across four spaced windows. Global backlight PWM uses
  20 kHz, starting at 10% duty and adjustable with the rotary encoder. Unconnected addresses 5–12 remain in reset-default sleep.
- The encoder changes global ScreenKey brightness; its push switch toggles
  display output. SWD pins remain available. USB serial events are available; USB HID is not implemented yet.

## RGB driver

`src/rgb.rs` owns PA8 as TIM1_CH1 and DMA1_CH1, with timer-update DMA requests.
`Leds::write(&[Rgb; 16]).await` accepts logical RGB values from 0–255 and
applies the fixed bring-up brightness scale. Pixels are in board raster order,
top left first. The matrix scanner owns the driver and transmits a new frame
only when debounced key states change. SPI1 remains available for screens and flash.

At the current 16 MHz timer clock, each 800 kHz bit uses 20 timer ticks.
Zero is 5 ticks high (312.5 ns); one is 10 ticks high (625 ns). Frames contain
GRB bytes, MSB first, with 160 µs of zero duty before and after the data.
The timer stays at zero duty between frames. These timings follow the
[OPSCO SK6812MINI-E datasheet](https://cdn-shop.adafruit.com/product-files/4960/4960_SK6812MINI-E_REV02_EN.pdf).
The driver asserts the expected timer period; revisit timing if clocks change.

The fixed ceiling limits the estimated RGB channel load to about 36 mA even
for all-white (excluding idle current). This is a conservative starting point,
not the future content-aware whole-board power budget.

Hardware validation: the earlier color/chase demo visibly controlled the twelve
unobscured LED positions. The four under the assembled ScreenKeys remain harder
to see. RTT captured presses and releases from SW1–SW4, including simultaneous
presses. Local captures and RTT logs are in ignored `artifacts/`.

## ScreenKey driver

`src/screens.rs` owns SPI1, decoder address/enable pins, common DC/reset, and
flash CS after the startup JEDEC check. Every transfer disables the decoder,
sets address and DC, then enables exactly one screen. It deselects the decoder
on success or SPI error. Flash CS stays high. Shared reset is pulsed once for
all panels, and display initialization runs in its own Embassy task.

The 128×128 IPS panel uses RGB565 and inversion. Register settings come from
[Waveshare's ScreenKey example](https://github.com/waveshareteam/0.85inch-ScreenKey-Module/tree/7d5f0c54384edeffe4ae17e35e8573eff8551e4b/Arduino-Uno/LCD_0inch85/LCD_0inch85),
with MADCTL `0xC8` and X/Y offsets 2/3 configured explicitly in `screens.rs`. See
[panel documentation](https://files.waveshare.com/wiki/0.85inch_ScreenKey_Module/0.85inch_ScreenKey_LCD.pdf)
and [THIRD_PARTY.md](THIRD_PARTY.md).

Initialization uses a 256-byte row buffer and yields after each row. Animation
uses DMA for pixel data and short blocking writes for command packets. DMA1_CH2
is SPI1 TX and DMA1_CH3 is reserved for RX; RGB continues using DMA1_CH1.
TIM2_CH3 on PA9 provides backlight PWM; TIM1/DMA1_CH1 are reserved for RGB,
TIM4 for Embassy time, and TIM3 decodes the rotary encoder.

The webcam sees the keypad upside down: logical key/display 1 is at the
camera's bottom-right, and 4 is at bottom-left. Screen labels identify decoder
addresses plus one, which correspond to J4–J15. The panels are write-only,
so an initialization log confirms transmitted commands, not panel presence.
The four connected top-row panels were verified with `HELLO 01` through
`HELLO 04` labels, upright from the keypad user's side. They now display the
wave. Addresses 5–12 were exercised in earlier bring-up but are not visually
verified and are excluded from this animation.

## Ocean experiment and physical screen spacing

`src/screens/wave.rs` treats the LCDs as four windows into one ocean scene.
The PCB/enclosure layout gives **23 mm horizontal key pitch**, while the LCD
active area is **15.2064 mm wide**. Thus each visible 128-pixel panel advances
193.60 scene pixels from the previous panel's origin. About 65.60 pixels of
scene are hidden between panels (7.7936 mm). The full span from the first LCD's
left edge to the fourth LCD's right edge is 84.2064 mm.

Mapping uses fixed-point coordinates with 1/256-pixel precision. All wave
layers and foam use these physical scene coordinates, so motion progresses
through the bezel/key gaps rather than joining the visible edges directly.
`PITCH` and `ACTIVE_WIDTH` are explicit calibration constants in `wave.rs`.
They use nominal PCB and panel dimensions, assuming centered, aligned caps;
individual assembly offsets have not been measured.

The sunset pairs a plum/coral/apricot sky with complementary deep teal water.
A large warm sun sits near the golden-ratio point of the physical panorama
(scene X=438), partially obscured by the swells. Broken gold reflections widen
toward the foreground, with warmer foam near the sun and muted cool foam
elsewhere. Three water layers move at different speeds and wavelengths; motion
is slower than the original ocean experiment for a calmer scene. After the
initial full scene draw, each 16-pixel-wide tile updates only rectangles around
the old/new layer boundaries. Static gradients permit this without stale pixels.
A 4 KiB scratch buffer and old/new column heights replace a full framebuffer.
Time controls animation phase independently of frame rate, capped at 30 fps.
The optimized boundary renderer avoids recomputing the whole scene per pixel;
its output is tested against the full reference renderer.

The pre-sunset ocean release build measured **15.4–15.6 fps**, about 64 ms per
four-screen update and 15.8 KiB of pixel data per update. The added sunset
reflection shading measures about **13.2–13.4 fps** on the board.
Consult current RTT statistics for subsequent changes. Release builds now
optimize for speed (`opt-level = 3`); the current application occupies about 72 KiB of flash.
The earlier single-curve experiment achieved about 26 fps. Current RTT logs
report actual ocean update rates every five seconds. The original full-screen
blocking path at 4 MHz took about 1.04 s for four screens, including rendering,
command overhead, and row yields.

A full RGB565 frame across four panels is 131,072 bytes. The ideal wire-only
limits are 3.81 fps at 4 MHz and 7.63 fps at 8 MHz. DMA removes CPU polling;
it does not increase wire bandwidth. Dirty rectangles make this particular
animation practical. Full-screen video would need a different bandwidth or
pixel-format tradeoff. The panel specifies a minimum 66 ns SPI write cycle
(about 15.15 MHz) in the
[ST7735S datasheet, serial interface table](https://files.waveshare.com/upload/e/e2/ST7735S_V1.1_20111121.pdf).
The current 8 MHz setting is below that limit and was visually tested.

All four panels are updated sequentially on the shared bus. There is no
simultaneous frame latch or tearing synchronization wired, so panel-to-panel
update skew remains. Key scanning runs while pixel DMA transfers are pending;
RGB still lights only for held keys.

RTT reports frame rate (`fps_x10`), mean update duration, and bytes per frame
every five seconds. Logs and webcam snapshots are in ignored `artifacts/`.
To run the host-side dirty-update regression test:

```sh
rustc --edition=2024 --test src/screens/wave.rs -o /tmp/keeb-wave-tests
/tmp/keeb-wave-tests
```

The tests check physical window spacing and exact pixel equality between partial
updates and a complete scene redraw, including large phase jumps and wraparound.

## Rotary encoder

The Alps EC11E15244G1 on PA6/PA7 uses TIM3 quadrature mode 1 (x2), matching
30 detents / 15 pulses. Both inputs have internal pull-ups. Hardware captures
rotation even while the CPU renders; software accepts the net counter change
after 3 ms of stable count, with wrapping signed deltas across 0/65535.
See [Alps part specifications](https://tech.alpsalpine.com/e/products/detail/EC11E15244G1/).

- Turn: change global backlight duty by two percentage points per count,
  clamped to 2–50%, starting at 10%.
- Press PC13: toggle all four displays, with 20 ms debounce and no repeat
  while held. A button already held at boot must be released before toggling.
- Off: backlight duty goes to zero, ST7735S display-off commands are sent,
  and animation SPI updates pause. Logic remains powered.
- On: display-on commands restore the remembered brightness and animation
  resumes. Turning while off changes the remembered brightness without waking.
- Settings persist across toggles, not across power cycles. Key-controlled
  RGB LEDs remain independent of display power.

The encoder publishes the latest complete setting atomically. The display
owner applies it between animation frames (roughly one frame of latency),
keeping all SPI/decoder access in a single task. On initial startup the
backlights stay dark until initialization and the first scene draw complete.
RTT logs both encoder changes and applied display settings. Hardware testing
captured rotation in both directions and display off/on presses, including
restoring 50% brightness after a toggle. Clockwise polarity has not been
independently correlated with the physical motion in the camera view.

```sh
rustc --edition=2024 --test src/encoder/control.rs -o /tmp/keeb-control-tests
/tmp/keeb-control-tests
```

The control test covers contact bounce, counter wrap, brightness clamps,
press/hold behavior, and brightness memory while off.

## USB host connection

`src/usb.rs` exposes CDC ACM on PA12/PA11 using the internal HSI48 oscillator
with CRS synchronized to USB SOF. It streams newline-delimited JSON v1; no
custom kernel driver is needed. The host listener discovers the keypad by
VID/PID, product string, and optional MCU serial number (the STM32's unique
ID; `keeb list` shows it).

Key/encoder tasks publish debounced events without blocking. Periodic snapshots
provide current held-key, encoder, and display state; a bounded 32-event queue
and a dropped-event counter make slow-host behavior explicit. Encoder events
include rotation at brightness limits and push release, independent of whether
the local display setting changes. Existing display/encoder/RGB behavior is
preserved. See [host/README.md](../host/README.md) for protocol and usage.

```sh
# From the project root, after building the Rust host CLI:
host/target/release/keeb listen
host/target/release/keeb listen --json
```

A charge-only cable can power the board but cannot carry this event stream.

## Host debug access

If `probe-rs list` sees J-Link but opening it reports permission denied,
install the included rule once:

```sh
sudo install -m 644 tools/70-keeb-jlink.rules /etc/udev/rules.d/70-keeb-jlink.rules
sudo udevadm control --reload-rules
```

Then unplug and reconnect J-Link. The rule is restricted to J-Link serial
`000600109060`. The development user belongs to `plugdev`.
The first bring-up used a temporary ACL on the current USB device node;
that ACL does not survive reconnection.

## Bring-up observations (2026-09-12)

- J-Link enumerated as `1366:0105`, serial `000600109060`.
- Read DBGMCU IDCODE `0x20036468`; flash-size register reported 128 KiB.
- The initial eight flash words were erased (`0xffffffff`). The core reported
  lockup before programming. `probe-rs info` also reported `DeviceEn bit not
  set`, although chip-specific memory reads and programming succeeded.
- Flashed the debug firmware successfully and received the hello message,
  recurring heartbeat, and SPI JEDEC identity `[ef, 40, 18]`.
- Logitech C920 capture works on `/dev/video0`. The first ScreenKey visibly
  displayed the hello label and color strip during isolated bring-up.

Next steps: map host events to application actions, then add USB HID if needed.
The current backlight and RGB ceilings are conservative fixed limits; a
whole-board power estimator is still pending.

API references: [Embassy STM32](https://docs.embassy.dev/embassy-stm32/),
[Embassy executor 0.10](https://docs.embassy.dev/embassy-executor/0.10.0/cortex-m/index.html).
