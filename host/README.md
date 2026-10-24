# keeb host tools

All host USB interop is Rust: one `keeb` CLI provides device discovery, the
CDC event listener, firmware packaging, and a native USB DFU updater.
`nusb` performs DFU control transfers directly through the OS; `serialport`
uses the OS CDC driver. No Python, pyserial, dfu-util, libusb or libudev is
required by this tool. The build command invokes Cargo for the two embedded
Rust projects.

## Build and use

From the project root:

```sh
cargo build --manifest-path host/Cargo.toml --release --locked
host/target/release/keeb list
host/target/release/keeb listen
host/target/release/keeb listen --json --duration 30
host/target/release/keeb build
host/target/release/keeb update firmware/artifacts/update/keeb-v2.kbf
```

Optionally install the executable on PATH:

```sh
cargo install --path host --locked
keeb list
keeb listen
keeb build --project /path/to/keeb-v2
keeb update /path/to/keeb-v2/firmware/artifacts/update/keeb-v2.kbf
```

Close the listener before updating. Serial access is exclusive. `--serial`
selects the stable MCU UID in both application and recovery modes; `--port`
selects an application serial device. The tool rejects ambiguous selections
and never chooses J-Link's separate serial port.

To enter recovery without flashing, use `keeb bootloader`. If the application
is broken, hold the encoder while connecting USB, release it, then run:

```sh
keeb update --recovery firmware/artifacts/update/keeb-v2.kbf
```

Ctrl-C cancels an update at the next bounded transfer/poll boundary. The
commit record stays invalid until the full application is verified; retry
with `--recovery`. Cancellation after a completed commit may leave the valid
application running normally, in which case use a normal update.

The listener reconnects after device disconnect/reset, including reopening
after five seconds without valid events if the OS leaves a silent port open.
It prints state only
when changed unless `--json` is selected, and detects event sequence gaps.
It reports events; it does not inject keyboard shortcuts or control other
host applications.

## USB access

Application: `c0de:cafe`, product `keeb-v2 events`. Recovery: `0483:df11`,
product `keeb-v2 recovery`. Both report the
MCU's unique ID as their serial number. These are local prototype USB identities. Recovery discovery also
checks the DFU interface and functional descriptor, and the updater verifies
the resident bootloader's SHA256 before writing anything.

On this Linux workstation, `dialout` provides serial access and the installed
`firmware/tools/70-keeb-update.rules` provides recovery USB access. See the
[bootloader instructions](../bootloader/README.md) for setup and memory layout.
The Rust host tools have been hardware-tested on Linux; other OSes have not
been hardware-tested here.

## Event protocol v1

USB CDC carries newline-delimited JSON. Packet/read boundaries do not define
records. The listener asserts DTR and explicitly requests `hello` to avoid
losing the opening greeting to serial buffering. Reads and writes use bounded
timeouts; commands are confirmed by a reply or USB disconnect, not `tcdrain`.

- `hello`: device, protocol version, update capability and application CRC32.
- `key`: sequence, uptime, 1-based key, 0-based row/column, pressed boolean.
- `encoder`: sequence, uptime, signed delta, push state, display power and
  brightness. Includes push release and rotation at brightness limits.
- `state`: uptime, 16-bit held-key mask, wrapping encoder count, push state,
  display power/brightness, and dropped-event count. Sent once per second.

The typed decoder validates version, required fields and key coordinates,
limits lines to 4096 bytes, and resynchronizes after malformed input. Extra
fields are accepted for compatible protocol extensions. Sequence numbers
wrap at 32 bits; snapshots have no sequence. After a gap, reconcile held
inputs from the next state snapshot. Historical missed actions cannot be
reconstructed. The firmware queues 32 events and does not retain offline
history.

Host commands: `{"v":1,"cmd":"hello"}` and
`{"v":1,"cmd":"bootloader"}`, each followed by a newline. Firmware accepts
commands split across packets and ignores malformed/oversized commands.
USB HID reports are not implemented.

## Package and updater behavior

`keeb build` emits the existing layout-v2 `.kbf` format, raw application,
padded bootloader and factory image. Its ELF extractor validates load segments,
addresses, bounds and overlaps. The `.kbf` output was checked byte-for-byte
against the previous packager before the accompanying RTT firmware fix.

The updater checks package hashes/vectors, matches the keypad by UID across
re-enumeration, validates the bootloader, invalidates the commit record,
erases/programs application pages, reads the complete application and
bootloader back, then writes/verifies the commit record. It returns success
only after the expected application sends both hello and state records.

## Checks

```sh
cargo test --manifest-path host/Cargo.toml --locked
cargo fmt --manifest-path host/Cargo.toml --check
cargo clippy --manifest-path host/Cargo.toml --all-targets --locked -- -D warnings
```

Tests cover fragmented and malformed events, package corruption, bad vectors,
ELF bounds, DFU requests/state transitions, short/error replies, write guards,
readback corruption, and retry after interruption at every update stage.
Hardware logs are under the ignored `firmware/artifacts/rust-host/` directory.

The migration also enabled firmware `defmt-rtt/disable-blocking-mode`: a
previous probe-rs session could otherwise leave logging in blocking mode,
stalling USB once its unattended RTT buffer filled.
With the fix installed, a 180-second listener run after disconnecting the
RTT reader received 180 state snapshots, no dropped events, and a maximum
snapshot interval of 1061 ms.
The listener also resumed hello/state events automatically after a hardware
reset; serial disconnect errors trigger reconnection, while a closed stdout
pipe exits cleanly.
