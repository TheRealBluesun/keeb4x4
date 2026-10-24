# Parts to buy

JLC assembles all the surface-mount parts (MCU, LEDs, connectors, buttons,
passives). Everything below is **not on the assembled board**. You buy it
separately and either solder it in or plug it on.

## Shopping list

| Qty | Part | Notes |
| --- | --- | --- |
| 4 | **Waveshare 0.85inch ScreenKey, LCD keycap version** | See [ScreenKeys](#screenkeys) below. Buy the *LCD* (keycap-only) version, **not** the *Module* version. |
| 16 | MX-compatible mechanical switches | 5-pin (PCB-mount) recommended. See [Switches](#switches). |
| 12 | 1u MX keycaps | For the 12 keys without a screen. Any standard MX-stem cap. |
| 1 | **Alps Alpine EC11E15244G1** rotary encoder | Exact part matters. See [Encoder](#encoder). |
| 1 | Knob for a 6 mm D-shaft | Any knob for a 6 mm "D" (flatted) shaft. |
| 1 | 1×5 pin header, 2.54 mm | For J3 (SWD), used once to flash the bootloader. |
| 1 | ST-Link V2 clone + jumper wires | For the one-time bootloader flash. See [PROGRAMMER.md](PROGRAMMER.md). |
| 1 | USB-C cable | Must be a **data** cable. Charge-only cables are common and will power the board without it showing up on the computer. |
| 8 | M3 × 8 mm pan- or button-head screws | Only if you print the base. See [enclosure/README.md](../enclosure/README.md). |

Optional:

| Qty | Part | Notes |
| --- | --- | --- |
| 1 | 1×4 pin header + SSD1306 I²C OLED (128×32 or 128×64) | J2. Wired on the board but **not used by the current firmware**. |
| up to 8 | More ScreenKey LCD keycaps | The board has connectors for 12 (J4–J15, top three rows), but the current firmware only drives the top four. |

## ScreenKeys

**Product page:** <https://www.waveshare.com/0.85inch-screenkey.htm>
(documentation: <https://docs.waveshare.com/0.85inch-ScreenKey-Module-W>).
They cost about $6 each from Waveshare directly; they're sometimes on Amazon
too.

Waveshare sells four variants on that page. You want one of the two
**LCD** versions:

| Variant | Waveshare SKU | Use it? |
| --- | --- | --- |
| 0.85inch ScreenKey **LCD (W)**, white | 34086 | **Yes** |
| 0.85inch ScreenKey **LCD (B)**, black | 34087 | **Yes** |
| 0.85inch ScreenKey **Module (W)** | 34167 | No |
| 0.85inch ScreenKey **Module (B)** | 34168 | No |

The LCD version is just the keycap with a built-in 128×128 IPS screen and a
12-pin flat ribbon cable. The ribbon plugs straight into the board's FPC
connectors. The Module version adds Waveshare's own switch and breakout PCB,
which this board replaces. You'd be paying for parts you throw away.

**Where they go:** the shipped firmware drives four screens on the **top row**
(the keys above connectors J4, J5, J6, J7 — the row nearest the USB-C
connector's edge of the board). The ocean animation spans all four as one
scene.

**Connecting the ribbon:** each J4–J15 connector is a 0.5 mm-pitch FPC
connector with a hinged locking flap. Flip the flap up, slide the ribbon in
fully with its contacts facing the board, and press the flap down. Ribbons are
fragile: don't crease them sharply, and keep them out of the switch's travel.

## Switches

The board uses standard Cherry MX footprints, soldered, with **no switch
plate**. Without a plate, 3-pin ("plate-mount") switches can twist or tilt.
**5-pin ("PCB-mount") switches** have two extra plastic pegs that lock them
square to the board, so get those. Any brand of MX-compatible switch works
electrically: Cherry, Gateron, Kailh Box, Akko, etc.

Hot-swap sockets aren't supported; the switches are soldered directly.

Each switch sits above a reverse-mounted RGB LED that shines up through a hole
in the board. Switches with a clear or light-diffusing housing show the LEDs
best.

## Encoder

**Alps Alpine EC11E15244G1**, from DigiKey or Mouser:
<https://www.digikey.com/en/products/result?keywords=EC11E15244G1>

It needs to be exactly this part (15 pulses / 30 detents, built-in push
switch, 20 mm vertical shaft). Lookalikes don't fit the footprint or are
missing the push switch, which the firmware uses to toggle the screens. In
particular, **Bourns PEC11R / PEC12R don't fit**. See the encoder notes in
[fab/README.md](../fab/README.md#sw17-rotary-encoder--alps-ec11e15244g1)
for details.

## Soldering order

1. **J3 header first** (it's the lowest part), then flash the bootloader per
   [PROGRAMMER.md](PROGRAMMER.md) so you know the board works before
   committing switches to it.
2. Encoder.
3. Switches. Make sure each one sits flat before soldering all its pins.
   Tack one pin, check, then do the rest.
4. Plug in the ScreenKey ribbons, then press the ScreenKey caps onto the
   top-row switches and regular caps onto the rest.
