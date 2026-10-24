//! Native DfuSe control transfers; no dfu-util or libusb subprocess/library.
use crate::{
    Cancel,
    image::{APP, END, FLASH, META},
};
use anyhow::{Context, Result, bail, ensure};
use nusb::{
    MaybeFuture,
    transfer::{ControlIn, ControlOut, ControlType, Recipient},
};
use std::time::{Duration, Instant};
const PAGE: usize = 2048;
const TIMEOUT: Duration = Duration::from_secs(5);

pub trait Control {
    fn out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()>;
    fn input(&mut self, request: u8, value: u16, length: u16) -> Result<Vec<u8>>;
}
pub struct Usb {
    interface: nusb::Interface,
}
impl Control for Usb {
    fn out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    data,
                },
                TIMEOUT,
            )
            .wait()
            .context("DFU control OUT failed")
    }
    fn input(&mut self, request: u8, value: u16, length: u16) -> Result<Vec<u8>> {
        self.interface
            .control_in(
                ControlIn {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    length,
                },
                TIMEOUT,
            )
            .wait()
            .context("DFU control IN failed")
    }
}

pub fn devices(serial: Option<&str>) -> Result<Vec<nusb::DeviceInfo>> {
    Ok(nusb::list_devices()
        .wait()?
        .filter(|d| {
            d.vendor_id() == 0x0483
                && d.product_id() == 0xdf11
                && d.product_string() == Some("keeb-v2 recovery")
                && serial.is_none_or(|s| d.serial_number() == Some(s))
        })
        .collect())
}

pub struct Dfu<T> {
    io: T,
    cancel: Cancel,
}
impl Dfu<Usb> {
    pub fn wait(serial: Option<&str>, cancel: Cancel) -> Result<(Self, String)> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            cancel.check()?;
            let devices = devices(serial)?;
            ensure!(
                devices.len() <= 1,
                "multiple recovery devices; select --serial"
            );
            if let Some(info) = devices.first() {
                let serial = info
                    .serial_number()
                    .context("recovery device has no serial number")?
                    .to_owned();
                let device = info
                    .open()
                    .wait()
                    .context("cannot open recovery USB device; check udev permissions")?;
                let config = device.active_configuration()?;
                let alt = config
                    .interfaces()
                    .find(|i| i.interface_number() == 0)
                    .and_then(|i| i.alt_settings().find(|a| a.alternate_setting() == 0))
                    .context("DFU interface missing")?;
                ensure!(
                    (alt.class(), alt.subclass(), alt.protocol()) == (0xfe, 1, 2),
                    "unexpected DFU interface"
                );
                let functional = alt
                    .descriptors()
                    .find(|d| d.descriptor_type() == 0x21)
                    .context("DFU functional descriptor missing")?;
                let bytes = &*functional;
                ensure!(
                    bytes.len() == 9
                        && bytes[2] & 3 == 3
                        && bytes[5..7] == [0, 8]
                        && bytes[7..9] == [0x1a, 1],
                    "unsupported DFU transfer size or protocol"
                );
                let interface = device.claim_interface(0).wait().context(
                    "cannot claim DFU interface (another updater or missing permissions)",
                )?;
                interface.set_alt_setting(0).wait()?;
                let mut dfu = Self {
                    io: Usb { interface },
                    cancel,
                };
                dfu.idle()?;
                return Ok((dfu, serial));
            }
            ensure!(
                Instant::now() < deadline,
                "recovery did not appear; check USB data cable or hold encoder at power-on"
            );
            cancel.sleep(Duration::from_millis(100))?;
        }
    }
}

impl<T: Control> Dfu<T> {
    fn status(&mut self) -> Result<(u8, u8, Duration)> {
        self.cancel.check()?;
        let data = self.io.input(3, 0, 6)?;
        ensure!(data.len() == 6, "short DFU status reply");
        let ms = u32::from(data[1]) | u32::from(data[2]) << 8 | u32::from(data[3]) << 16;
        ensure!(ms <= 5000, "unreasonable DFU poll timeout: {ms} ms");
        Ok((data[0], data[4], Duration::from_millis(ms as u64)))
    }
    fn idle(&mut self) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (status, state, wait) = self.status()?;
            if state == 10 {
                self.io.out(4, 0, &[])?;
            } else {
                ensure!(status == 0, "DFU error status {status}, state {state}");
                match state {
                    2 => return Ok(()),
                    3 | 4 => self.cancel.sleep(wait.max(Duration::from_millis(1)))?,
                    5 | 9 => self.io.out(6, 0, &[])?,
                    _ => bail!("unexpected DFU state {state}"),
                }
            }
            ensure!(Instant::now() < deadline, "DFU did not become idle");
        }
    }
    fn download(&mut self, block: u16, data: &[u8], manifest: bool) -> Result<()> {
        self.cancel.check()?;
        self.io.out(1, block, data)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (status, state, wait) = self.status()?;
            ensure!(
                status == 0,
                "DFU rejected transfer: status {status}, state {state}"
            );
            if (!manifest && state == 5) || (manifest && matches!(state, 7 | 8)) {
                return Ok(());
            }
            ensure!(
                matches!(state, 3 | 4 | 6),
                "unexpected state after download: {state}"
            );
            ensure!(Instant::now() < deadline, "DFU download stalled");
            self.cancel.sleep(wait.max(Duration::from_millis(1)))?;
        }
    }
    fn command(&mut self, opcode: u8, address: u32) -> Result<()> {
        let mut bytes = vec![opcode];
        bytes.extend(address.to_le_bytes());
        self.download(0, &bytes, false)
    }
    pub fn read(&mut self, address: u32, length: usize) -> Result<Vec<u8>> {
        ensure!(
            address >= FLASH
                && length > 0
                && u32::try_from(length)
                    .ok()
                    .and_then(|n| address.checked_add(n))
                    .is_some_and(|n| n <= END),
            "read outside internal flash"
        );
        self.idle()?;
        self.command(0x21, address)?;
        self.idle()?;
        let mut bytes = Vec::with_capacity(length);
        for (i, chunk) in (0..length).step_by(PAGE).enumerate() {
            self.cancel.check()?;
            let size = (length - chunk).min(PAGE);
            let reply = self.io.input(2, u16::try_from(i + 2)?, size as u16)?;
            ensure!(
                reply.len() == size,
                "short DFU readback: expected {size}, got {}",
                reply.len()
            );
            bytes.extend(reply);
        }
        self.idle()?;
        Ok(bytes)
    }
    pub fn write(&mut self, address: u32, bytes: &[u8]) -> Result<()> {
        validate_write(address, bytes.len())?;
        self.idle()?;
        // Each operation owns complete erase pages. The last application page
        // is padded with FF by the bootloader; it never overlaps metadata.
        let end = address + bytes.len() as u32;
        for page in (address..end).step_by(PAGE) {
            self.command(0x41, page)?;
        }
        self.command(0x21, address)?;
        for (i, data) in bytes.chunks(PAGE).enumerate() {
            self.download(u16::try_from(i + 2)?, data, false)?;
        }
        self.idle()
    }
    pub fn leave(&mut self) -> Result<()> {
        self.idle()?;
        self.command(0x21, FLASH)?;
        self.download(2, &[], true)
    }
}

pub fn validate_write(address: u32, length: usize) -> Result<()> {
    let end = u32::try_from(length)
        .ok()
        .and_then(|n| address.checked_add(n));
    ensure!(
        length != 0
            && address.is_multiple_of(PAGE as u32)
            && ((APP..META).contains(&address) && end.is_some_and(|n| n <= META)
                || address == META && length == 16),
        "refusing write outside application/commit pages"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        replies: VecDeque<Vec<u8>>,
        sent: Vec<(u8, u16, Vec<u8>)>,
    }
    impl Control for Fake {
        fn out(&mut self, r: u8, v: u16, d: &[u8]) -> Result<()> {
            self.sent.push((r, v, d.to_vec()));
            Ok(())
        }
        fn input(&mut self, _: u8, _: u16, _: u16) -> Result<Vec<u8>> {
            self.replies.pop_front().context("unexpected request")
        }
    }
    fn status(state: u8) -> Vec<u8> {
        vec![0, 0, 0, 0, state, 0]
    }
    fn fake(replies: Vec<Vec<u8>>) -> Dfu<Fake> {
        Dfu {
            io: Fake {
                replies: replies.into(),
                sent: vec![],
            },
            cancel: Cancel::default(),
        }
    }
    #[test]
    fn guards_never_touch_usb() {
        let mut d = fake(vec![]);
        for (addr, len) in [
            (FLASH, 8),
            (APP - 1, 8),
            (META - 8, 16),
            (META, 17),
            (END, 16),
            (APP, 0),
            (APP, usize::MAX),
        ] {
            assert!(d.write(addr, &vec![0; len.min(1000)]).is_err());
        }
        assert!(d.io.sent.is_empty());
        assert!(validate_write(APP, usize::MAX).is_err());
    }
    #[test]
    fn transfer_orders_erase_address_write_and_abort() {
        let mut d = fake(vec![
            status(2),
            status(4),
            status(5),
            status(4),
            status(5),
            status(4),
            status(5),
            status(5),
            status(2),
        ]);
        d.write(META, &[0; 16]).unwrap();
        assert_eq!(
            d.io.sent.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(),
            vec![(1, 0), (1, 0), (1, 2), (6, 0)]
        );
        assert_eq!(d.io.sent[0].2[0], 0x41);
        assert_eq!(d.io.sent[1].2[0], 0x21);
    }
    #[test]
    fn short_and_error_responses_stop() {
        let mut short = fake(vec![vec![0; 5]]);
        assert!(short.idle().is_err());
        let mut failed = fake(vec![vec![8, 0, 0, 0, 10, 0]]);
        assert!(failed.download(2, &[1; 8], false).is_err());
        let mut timeout = fake(vec![vec![0, 255, 255, 255, 4, 0]]);
        assert!(timeout.idle().is_err());
        let mut read = fake(vec![status(2), status(5), status(5), status(2), vec![0; 7]]);
        assert!(read.read(APP, 8).is_err());
    }
}
