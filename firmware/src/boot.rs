//! Contract shared with the resident stub; see bootloader/src/protocol.rs.
#[path = "../../bootloader/src/protocol.rs"]
pub mod protocol;

pub fn request_update() -> ! {
    // SRAM survives SYSRESETREQ. Both words must match to reject random RAM.
    cortex_m::interrupt::disable();
    unsafe {
        core::ptr::write_volatile(protocol::MAILBOX as *mut u32, protocol::REQUEST);
        core::ptr::write_volatile((protocol::MAILBOX + 4) as *mut u32, !protocol::REQUEST);
    }
    cortex_m::asm::dsb();
    cortex_m::peripheral::SCB::sys_reset()
}
