# Flashing the bootloader (one time, needs a programmer)

A freshly assembled board has an empty microcontroller. Before it can do
anything — including accept firmware over USB — the bootloader has to be
written once through the **SWD header (J3)** using a hardware programmer.
After that, every future firmware update goes over the normal USB-C cable
with `keeb update`, and you can put the programmer back in a drawer.

## What you need

| Item | Notes |
| --- | --- |
| **ST-Link V2 clone** | The ~$8–15 USB-stick ones on Amazon/AliExpress ("ST-Link V2 mini", usually an aluminium shell with a 10-pin header). They work fine for this. A genuine ST-Link, J-Link or any other probe-rs-supported probe also works. |
| **4–5 female-to-female jumper wires** | Usually included with the ST-Link. |
| **1×5 2.54 mm pin header, soldered into J3** | J3 is not populated by JLC. Solder a straight header in (you can skip this and hold the wires against the holes, but don't). |
| **USB-C cable** | Powers the board during flashing. |
| **probe-rs** | The flashing tool. Install below. |
| **Rust (rustup)** | To build the firmware images. See the top-level README. |

## J3 pinout

J3 is the 5-pin header labelled `J3` on the board.
**Pin 1 is the square pad.**

| J3 pin | Signal | Connect to ST-Link pin labelled |
| --- | --- | --- |
| 1 | 3V3 | **leave unconnected** (see below) |
| 2 | SWDIO | `SWDIO` |
| 3 | SWCLK | `SWCLK` (sometimes `SWCLK`/`TCK`) |
| 4 | GND | `GND` |
| 5 | NRST | `RST` (optional) |

The pin order on clone ST-Links varies between batches. **Go by the labels
printed on the stick's case, not by pin position.** If your case has no
labels, look up the listing's pinout photo before wiring.

**Power the board from USB-C, and don't connect the ST-Link's 3.3V pin.** The
board has its own 3.3 V regulator. Clone ST-Links push 3.3 V out of that pin
instead of sensing it, so connecting it puts two supplies fighting on the
same rail. GND must be connected; it's the reference for the data lines.

NRST is optional. The firmware leaves the SWD pins alone, so probe-rs can
attach to a running chip without it. On some older clones the `RST` pin
isn't actually wired to anything useful. Wire it if you have a spare jumper.

If you are using a J-Link with a 20-pin cable instead, see
[`jlinkpins.png`](jlinkpins.png) for its pinout (VTref → J3 pin 1 is
correct for a J-Link, since a J-Link only senses that pin).

## Install probe-rs

Linux/macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.ps1 | iex
```

Or with Cargo on any platform: `cargo install probe-rs-tools --locked`.

**Linux only:** install probe-rs's udev rules so you don't need `sudo`:

```sh
curl -fsSL https://probe.rs/files/69-probe-rs.rules | sudo tee /etc/udev/rules.d/69-probe-rs.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then unplug and replug the ST-Link. Also install the board's own USB rules
from [bootloader/README.md](../bootloader/README.md#host-setup) while you're
at it.

Full instructions, including Windows driver notes: <https://probe.rs/docs/getting-started/installation/>.

## Flash it

From the repo root:

```sh
# 1. Build the host tool and both firmware images.
#    This produces firmware/artifacts/update/factory.bin
#    (bootloader + application + commit record in one image).
cargo build --manifest-path host/Cargo.toml --release --locked
host/target/release/keeb build

# 2. Check the probe is visible.
probe-rs list

# 3. With the board plugged into USB-C and wired to the ST-Link,
#    check that probe-rs can see the chip.
probe-rs info --protocol swd --speed 100

# 4. Write the factory image (takes ~10–30 s at this speed).
probe-rs download --chip STM32G431CBTx --protocol swd --speed 100 \
  --binary-format bin --base-address 0x08000000 --verify \
  firmware/artifacts/update/factory.bin
probe-rs reset --chip STM32G431CBTx --protocol swd --speed 100
```

Then disconnect the ST-Link, unplug and replug USB-C, and check:

```sh
host/target/release/keeb list     # should show "keeb-v2 events"
host/target/release/keeb listen   # press keys, turn the encoder
```

Done. From now on, update firmware over USB:

```sh
host/target/release/keeb build
host/target/release/keeb update firmware/artifacts/update/keeb-v2.kbf
```

`--speed 100` (100 kHz) is deliberately slow and reliable with long jumper
wires. You can try `--speed 1000` once it works.

## Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `probe-rs list` shows nothing | Linux: udev rules not installed, or ST-Link not replugged after installing them. Windows: see the probe-rs driver notes. Try another USB port; some clones are flaky on hubs. |
| probe-rs complains the ST-Link **firmware is too old** | Update it with ST's free [STSW-LINK007](https://www.st.com/en/development-tools/stsw-link007.html) upgrade tool. This works on most clones. |
| Probe found, but `info` / `download` can't connect to the chip | Check the wiring against the case labels. SWDIO and SWCLK swapped is the usual mistake. Make sure GND is connected and the board is powered over USB-C. Keep the jumpers short. |
| Still can't connect | Wire NRST, then add `--connect-under-reset` to the probe-rs commands. If your clone's RST pin isn't wired, hold the board's **RESET** button, start the command, and release the button about half a second later. |
| `keeb list` shows nothing after flashing | Unplug and replug USB-C. On Linux, check the board's udev rules ([bootloader/README.md](../bootloader/README.md#host-setup)). |
| Board shows up as `keeb-v2 recovery` instead | The bootloader is fine but the application isn't. Run `keeb update --recovery firmware/artifacts/update/keeb-v2.kbf`. |

Once the bootloader is on the chip you shouldn't need the programmer again.
Even a bad firmware update can be recovered over USB (hold the encoder button
while plugging in). The programmer is only needed if you want to replace the
bootloader itself, or for live debugging (`probe-rs attach`, see
[bootloader/README.md](../bootloader/README.md)).
