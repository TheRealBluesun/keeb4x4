# keeb-v2 USB recovery, layout v2

The Rust / Embassy USB-only bootloader is installed and tested on the board.
The host listener, packager and updater are Rust. The `keeb` CLI performs
native USB DFU transfers using `nusb`; no Python or dfu-util is required.

## Normal development

From the project root:

```sh
cargo build --manifest-path host/Cargo.toml --release --locked
host/target/release/keeb build
host/target/release/keeb update firmware/artifacts/update/keeb-v2.kbf
```

Close `keeb listen` before updating; the tools use exclusive serial access.
The updater asks the application for a hello, requests recovery, follows the
USB device, checks the resident bootloader, invalidates the application commit
record, writes the application and reads every byte back. It then writes and
verifies the commit record, restarts the board, and requires both a hello with
the expected image checksum and a state snapshot before reporting success.
No J-Link is needed for this workflow.

## Recovery

Hold the encoder button while reconnecting USB. Release it once the device
appears as `keeb-v2 recovery`, then run:

```sh
host/target/release/keeb update --recovery firmware/artifacts/update/keeb-v2.kbf
```

Missing, incomplete or corrupt applications enter recovery automatically.
A valid application that hangs can still be bypassed using the encoder
button: the bootloader samples it before running any application code.
An interrupted update requires retrying the update; there is no saved-image
rollback. The bootloader remains available throughout application updates.

## Host setup

Build the CLI with Cargo as above, or install it with
`cargo install --path host --locked`. See [host tools](../host/README.md).

Linux permissions:

```sh
sudo install -m 644 firmware/tools/70-keeb-update.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=usb
```

Add your user to the `plugdev` and `dialout` groups. Application CDC uses
`c0de:cafe`; recovery uses STM32 DFU identity `0483:df11` for this local
prototype. These are not production VID/PID allocations. Both modes now use
the same MCU UID serial (shown by `keeb list`). Recovery's product string
is `keeb-v2 recovery`. The updater follows the same UID across USB modes.
Use `--serial` (either mode) or `--port` (application mode) to select a
device; ambiguous selection fails before writing.

## Memory and image format

| Address | Size | Purpose |
| --- | ---: | --- |
| `0x08000000` | 32 KiB | Resident Rust USB-only bootloader |
| `0x08008000` | 94 KiB | Application, including vectors |
| `0x0801f800` | 2 KiB | Commit record, separate erase page |
| `0x20007ff0` | 16 bytes | Reset-surviving mailbox, excluded from both stacks |

The bootloader currently uses about 15.5 KiB of flash; the application uses
about 72 KiB. Commit record: four little-endian u32 words, `0x3242454b`,
application length, CRC32/ISO-HDLC, and bitwise complement of CRC32. Startup
checks length, aligned SRAM stack, Thumb reset vector and full application
CRC. A validation fault requests USB recovery on the next reset.

The `.kbf` package contains layout/address/length, application CRC32 and
SHA256, and SHA256 of the padded 32 KiB bootloader. The host rejects corrupt,
oversized, wrongly linked and mismatched-bootloader packages before changing
flash. CRC/SHA checks provide integrity; packages are not signed.

The **device itself** rejects erasing/programming the bootloader region,
mass erase, option-byte access and out-of-range addresses. Its DFU descriptor
also advertises the boot region as read-only. The host further limits writes
to the application and its 16-byte commit record. No option bytes or readout
protection settings were changed; J-Link can still reflash the bootloader.

`keeb build` generates these files under `firmware/artifacts/update/`:

- `keeb-v2.kbf`: normal USB update package.
- `application.bin`: relocated application, not a standalone factory image.
- `bootloader.bin`: bootloader padded to 32 KiB.
- `factory.bin`: complete 128 KiB initial image, including commit record.

The ELF extractor checks every load segment. Both builds use `--nmagic` to
avoid a synthetic ELF-header load segment outside the application region.
Do not use the old raw `cargo run` flashing workflow: it does not update the
commit record. Its Cargo runner has been removed.

## Initial installation or bootloader replacement

The original pre-bootloader flash was backed up to the ignored
`firmware/artifacts/update/original-pre-bootloader.bin` before installation.
A new board or a deliberate bootloader replacement needs an SWD probe
(ST-Link V2 clone or J-Link) once. See [docs/PROGRAMMER.md](../docs/PROGRAMMER.md)
for wiring and setup:

```sh
host/target/release/keeb build
probe-rs download --chip STM32G431CBTx --protocol swd --speed 100 \
  --binary-format bin --base-address 0x08000000 --verify \
  firmware/artifacts/update/factory.bin
probe-rs reset --chip STM32G431CBTx --protocol swd --speed 100
```

For RTT without replacing the application:

```sh
probe-rs attach --chip STM32G431CBTx --protocol swd --speed 100 \
  firmware/target/thumbv7em-none-eabihf/release/keeb-v2
```

## Hardware verification, 2026-09-14

- Normal startup, USB DFU programming, complete readback, commit and return
  to the expected application passed without an active debugger.
- Two consecutive updates of the final package passed without an active
  debugger. Initial application CRC32: `b955331e`. USB state snapshots reported
  zero dropped events; a subsequent brief RTT check showed the animation
  running at 12.5–12.9 fps. The webcam no longer faced the keyboard, so no
  final visual display check was possible.
- Recovery GPIO reads confirmed PA6/PA7 remain inputs, PA9 backlight low,
  PA10 decoder disabled, PB6 flash deselected and PB1 LCD reset low.
- Terminated an actual dfu-util download mid-transfer, reset the MCU, checked
  that the partial application was rejected and recovered over USB.
- Erased metadata, a torn commit, invalid vectors and a bad application CRC
  each entered recovery after reset; the full bootloader remained unchanged.
- Installed a valid-checksum application that loops forever without USB.
  At a breakpoint immediately before the bootloader's first PC13 sample,
  changed PC13's pull-up to a pull-down and read its actual input low. Resuming
  entered USB recovery, independent of the hung application. This electrically
  simulates a held button; a human press-and-replug check is still outstanding.
- Sent raw USB requests bypassing dfu-util's address checks: erase/program
  of the boot region, mass erase, option-byte and overflow addresses were
  rejected by the device. Readback confirmed the bootloader was unchanged.

Fault-trial logs, button simulation and raw-request checks are in
`firmware/artifacts/update/`. Actual removal of power during a flash write
has not been tested; interruption tests used a terminated USB transfer and
MCU reset.

ST's ROM DFU was tested first. It **clears GPIO locks during initialization**
and reconfigures encoder/backlight pins for other interfaces, so this board
uses its own USB-only bootloader. It keeps displays off and external flash
idle in recovery and performs a full system reset to exit DFU.

## Software checks

The host is now a Rust CLI; its tests cover event framing, package/ELF
validation, native DFU control transfers, guarded writes and failure/retry
at every update stage. Embedded boot-contract tests remain separate.

```sh
cargo test --manifest-path host/Cargo.toml --locked
rustc --edition=2024 --test bootloader/src/protocol.rs -o /tmp/keeb-boot-tests
/tmp/keeb-boot-tests
(cd bootloader && cargo fmt --check && cargo clippy --release --locked -- -D warnings)
(cd firmware && cargo fmt --check && cargo clippy --release --locked -- -D warnings)
```

References: ST [AN2606, section 52](https://www.st.com/resource/en/application_note/an2606-introduction-to-system-memory-boot-mode-on-stm32-mcus-stmicroelectronics.pdf),
[AN3156, USB DFU protocol](https://www.st.com/resource/en/application_note/an3156-usb-dfu-protocol-used-in-the-stm32-bootloader-stmicroelectronics.pdf),
and the [dfu-util manual](https://dfu-util.sourceforge.net/dfu-util.1.html).

## Native Rust host migration, 2026-09-14

The Rust packager matched the prior `.kbf` byte-for-byte. Native USB updates
and Ctrl-C cancellation after commit invalidation were exercised on the
board; `keeb update --recovery` restored and verified the firmware without
J-Link or dfu-util. Logs are in `firmware/artifacts/rust-host/`.

The application now enables `defmt-rtt/disable-blocking-mode`: probe-rs
otherwise leaves RTT in blocking mode, which can stall USB after its reader
disconnects and the log buffer fills. The Rust updater installed this fix;
the new application CRC32 is `ed1cc84d`. The bootloader is unchanged.
