//! USB DFU 1.1 with DfuSe address/page commands; no USART/SPI interfaces.
use crate::protocol::APP;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_stm32::{
    Peri, bind_interrupts,
    flash::{Blocking, Flash},
    peripherals,
    usb::{Driver, InterruptHandler},
};
use embassy_usb::{
    Builder, Handler,
    control::{InResponse, OutResponse, Recipient, Request, RequestType},
    types::StringIndex,
};

const FLASH: u32 = 0x0800_0000;
const END: u32 = 0x0802_0000;
const TRANSFER: usize = 2048;
static RESTART: AtomicBool = AtomicBool::new(false);
bind_interrupts!(struct Irqs { USB_LP => InterruptHandler<peripherals::USB>; });

struct Dfu<'a> {
    flash: Flash<'a, Blocking>,
    string: StringIndex,
    address: u32,
    state: u8,
    status: u8,
}

impl Dfu<'_> {
    fn handles(req: &Request) -> bool {
        req.request_type == RequestType::Class
            && req.recipient == Recipient::Interface
            && req.index == 0
    }
    fn error(&mut self, status: u8) {
        self.status = status;
        self.state = 10;
    }
    fn address(&self, block: u16) -> Option<u32> {
        self.address
            .checked_add(u32::from(block.checked_sub(2)?) * TRANSFER as u32)
    }
    fn download(&mut self, block: u16, data: &[u8]) {
        if !matches!(self.state, 2 | 5) {
            self.error(15);
            return;
        }
        if data.is_empty() {
            self.state = 6;
            return;
        }
        self.state = 3;
        if block == 0 {
            if data.len() != 5 {
                self.error(15);
                return;
            }
            let address = u32::from_le_bytes(data[1..5].try_into().unwrap());
            match data[0] {
                0x21 if (FLASH..END).contains(&address) => self.address = address,
                0x41 if (APP..END).contains(&address) && address.is_multiple_of(2048) => {
                    if self
                        .flash
                        .blocking_erase(address - FLASH, address - FLASH + 2048)
                        .is_err()
                    {
                        self.error(4);
                    }
                }
                _ => self.error(8), // Includes mass erase and unprotect: unsupported.
            }
        } else if let Some(address) = self.address(block) {
            let length = (data.len() + 7) & !7;
            if address < APP
                || !address.is_multiple_of(8)
                || length > TRANSFER
                || address
                    .checked_add(length as u32)
                    .is_none_or(|end| end > END)
            {
                self.error(8);
                return;
            }
            let mut padded = [0xff; TRANSFER];
            padded[..data.len()].copy_from_slice(data);
            if self
                .flash
                .blocking_write(address - FLASH, &padded[..length])
                .is_err()
            {
                self.error(3);
            }
        } else {
            self.error(8);
        }
    }
}

impl Handler for Dfu<'_> {
    fn reset(&mut self) {
        self.state = 2;
        self.status = 0;
        self.address = FLASH;
    }
    fn get_string(&mut self, index: StringIndex, _: u16) -> Option<&str> {
        (index == self.string).then_some("@Internal Flash /0x08000000/16*02Ka,48*02Kg")
    }
    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        if !Self::handles(&req) {
            return None;
        }
        match req.request {
            1 if data.len() <= TRANSFER => self.download(req.value, data),
            4 if data.is_empty() && self.state == 10 => {
                self.state = 2;
                self.status = 0;
            }
            6 if data.is_empty() && matches!(self.state, 2 | 3 | 5 | 9) => {
                self.state = 2;
                self.status = 0;
            }
            _ => return Some(OutResponse::Rejected),
        }
        Some(OutResponse::Accepted)
    }
    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if !Self::handles(&req) {
            return None;
        }
        match req.request {
            3 if req.length == 6 => {
                self.state = match self.state {
                    3 => 4,
                    4 => 5,
                    6 => 7,
                    state => state,
                };
                buf[..6].copy_from_slice(&[self.status, 10, 0, 0, self.state, 0]);
                if self.state == 7 {
                    RESTART.store(true, Ordering::Relaxed);
                }
                Some(InResponse::Accepted(&buf[..6]))
            }
            5 if req.length == 1 => {
                buf[0] = self.state;
                Some(InResponse::Accepted(&buf[..1]))
            }
            2 if matches!(self.state, 2 | 9) && usize::from(req.length) <= TRANSFER => {
                if req.value == 0 {
                    buf[..3].copy_from_slice(&[0, 0x21, 0x41]);
                    return Some(InResponse::Accepted(&buf[..3]));
                }
                let Some(address) = self.address(req.value) else {
                    return Some(InResponse::Rejected);
                };
                if !(FLASH..END).contains(&address) {
                    self.error(8);
                    return Some(InResponse::Rejected);
                }
                let length = usize::from(req.length).min((END - address) as usize);
                if self
                    .flash
                    .blocking_read(address - FLASH, &mut buf[..length])
                    .is_err()
                {
                    self.error(1);
                    return Some(InResponse::Rejected);
                }
                self.state = if length < usize::from(req.length) {
                    2
                } else {
                    9
                };
                Some(InResponse::Accepted(&buf[..length]))
            }
            _ => Some(InResponse::Rejected),
        }
    }
}

#[embassy_executor::task]
pub async fn run(
    usb: Peri<'static, peripherals::USB>,
    dp: Peri<'static, peripherals::PA12>,
    dm: Peri<'static, peripherals::PA11>,
    flash: Peri<'static, peripherals::FLASH>,
) {
    let driver = Driver::new(usb, Irqs, dp, dm);
    // STM32 DFU identity for this local prototype; not a production allocation.
    let mut config = embassy_usb::Config::new(0x0483, 0xdf11);
    config.manufacturer = Some("keeb-v2 prototype");
    config.product = Some("keeb-v2 recovery");
    config.serial_number = Some(embassy_stm32::uid::uid_hex());
    config.max_power = 500;
    config.device_class = 0;
    config.device_sub_class = 0;
    config.device_protocol = 0;
    config.composite_with_iads = false;
    let mut descriptor = [0; 128];
    let mut bos = [0; 64];
    let mut control = [0; TRANSFER];
    let mut handler;
    let mut builder = Builder::new(
        driver,
        config,
        &mut descriptor,
        &mut bos,
        &mut [],
        &mut control,
    );
    let string = builder.string();
    handler = Dfu {
        flash: Flash::new_blocking(flash),
        string,
        address: FLASH,
        state: 2,
        status: 0,
    };
    {
        let mut function = builder.function(0xfe, 1, 2);
        let mut interface = function.interface();
        let mut alternate = interface.alt_setting(0xfe, 1, 2, Some(string));
        // bmAttributes, wDetachTimeOut, wTransferSize, bcdDFU (DfuSe 1.1a).
        alternate.descriptor(0x21, &[0x0b, 0xe8, 0x03, 0x00, 0x08, 0x1a, 0x01]);
    }
    builder.handler(&mut handler);
    let mut device = builder.build();
    let restart = async {
        loop {
            embassy_time::Timer::after_millis(10).await;
            if RESTART.load(Ordering::Relaxed) {
                embassy_time::Timer::after_millis(100).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    };
    embassy_futures::join::join(device.run(), restart).await;
}
