#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
mod dfu;
mod protocol;
use protocol::*;

const RCC: u32 = 0x4002_1000;
const GPIOA: u32 = 0x4800_0000;
const GPIOB: u32 = 0x4800_0400;
const GPIOC: u32 = 0x4800_0800;

fn read(address: u32) -> u32 {
    unsafe { read_volatile(address as *const u32) }
}
fn write(address: u32, value: u32) {
    unsafe { write_volatile(address as *mut u32, value) }
}
fn modify(address: u32, mask: u32, value: u32) {
    write(address, (read(address) & !mask) | value);
}

fn pin(port: u32, number: u32, mode: u32, pull: u32, high: bool) {
    write(port + 0x18, 1 << (number + if high { 0 } else { 16 }));
    modify(port, 3 << (number * 2), mode << (number * 2));
    modify(port + 0x0c, 3 << (number * 2), pull << (number * 2));
}

fn valid_app() -> bool {
    let length = read(META + 4);
    let checksum = read(META + 8);
    if read(META) != META_MAGIC
        || read(META + 12) != !checksum
        || !valid_vectors(read(APP), read(APP + 4), length)
    {
        return false;
    }
    let image = unsafe { core::slice::from_raw_parts(APP as *const u8, length as usize) };
    crc32(image) == checksum
}

unsafe fn jump(address: u32) -> ! {
    // We only enter here from reset, before any interrupts or DMA are enabled.
    write(0xe000_ed08, address); // SCB.VTOR
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
    unsafe { cortex_m::asm::bootload(address as *const u32) }
}

#[entry]
fn main() -> ! {
    // Require reset clock state if entered directly by a debugger or loader.
    // G4 encodes HSI as SWS=01 (not 00).
    if read(RCC) & (1 << 24) != 0 || read(RCC + 8) & 0x0c != 0x04 {
        cortex_m::peripheral::SCB::sys_reset();
    }
    modify(RCC + 0x4c, 0, 7); // GPIO A/B/C clocks
    let _ = read(RCC + 0x4c);
    if read(GPIOA + 0x1c) & 0x10000 != 0 {
        cortex_m::peripheral::SCB::sys_reset();
    }
    pin(GPIOC, 13, 0, 1, false); // encoder switch input, pull-up
    cortex_m::asm::delay(320_000); // 20 ms at reset's 16 MHz HSI
    let held = read(GPIOC + 0x10) & (1 << 13) == 0;
    cortex_m::asm::delay(320_000);
    let held = held && read(GPIOC + 0x10) & (1 << 13) == 0;
    let requested = read(MAILBOX) == REQUEST && read(MAILBOX + 4) == !REQUEST;
    write(MAILBOX, 0);
    write(MAILBOX + 4, 0);
    if !held && !requested && valid_app() {
        unsafe { jump(APP) }
    }

    let mut config = embassy_stm32::Config::default();
    config.rcc.hsi48 = Some(embassy_stm32::rcc::Hsi48Config {
        sync_from_usb: true,
    });
    config.rcc.mux.clk48sel = embassy_stm32::rcc::mux::Clk48sel::HSI48;
    let p = embassy_stm32::init(config);
    // USB-only recovery: never enter ST ROM, which resets GPIO and drives
    // pins wired to encoder contacts. Keep displays and external flash idle.
    for n in 0..=10 {
        match n {
            6 | 7 => pin(GPIOA, n, 0, 1, false), // encoder A/B
            10 => pin(GPIOA, n, 1, 0, true),     // screen decoder disabled
            _ => pin(GPIOA, n, 1, 0, false),
        }
    }
    pin(GPIOB, 1, 1, 0, false); // LCD reset asserted
    pin(GPIOB, 6, 1, 0, true); // external flash deselected
    for n in 12..=15 {
        pin(GPIOB, n, 0, 2, false);
    } // matrix rows
    static EXECUTOR: static_cell::StaticCell<embassy_executor::Executor> =
        static_cell::StaticCell::new();
    EXECUTOR
        .init(embassy_executor::Executor::new())
        .run(|spawner| {
            spawner.spawn(dfu::run(p.USB, p.PA12, p.PA11, p.FLASH).unwrap());
        })
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    recover_after_fault()
}

fn recover_after_fault() -> ! {
    // A torn flash doubleword can fault during validation. On the next reset,
    // skip application/metadata reads and enter USB recovery directly.
    write(MAILBOX, REQUEST);
    write(MAILBOX + 4, !REQUEST);
    cortex_m::asm::dsb();
    cortex_m::peripheral::SCB::sys_reset()
}

#[exception]
unsafe fn NonMaskableInt() -> ! {
    recover_after_fault()
}

#[exception]
unsafe fn HardFault(_: &cortex_m_rt::ExceptionFrame) -> ! {
    recover_after_fault()
}
