use crate::{
    dfu::{Control, Dfu},
    image::{APP, FLASH, Image, META, hash},
};
use anyhow::{Result, ensure};

pub trait FlashTransport {
    fn read(&mut self, address: u32, length: usize) -> Result<Vec<u8>>;
    fn write(&mut self, address: u32, bytes: &[u8]) -> Result<()>;
    fn leave(&mut self) -> Result<()>;
}
impl<T: Control> FlashTransport for Dfu<T> {
    fn read(&mut self, a: u32, n: usize) -> Result<Vec<u8>> {
        Dfu::read(self, a, n)
    }
    fn write(&mut self, a: u32, b: &[u8]) -> Result<()> {
        Dfu::write(self, a, b)
    }
    fn leave(&mut self) -> Result<()> {
        Dfu::leave(self)
    }
}
pub fn install(dfu: &mut impl FlashTransport, image: &Image) -> Result<()> {
    println!("Checking resident bootloader...");
    let boot = dfu.read(FLASH, (APP - FLASH) as usize)?;
    ensure!(
        hash(&boot) == image.boot_hash,
        "bootloader differs from package; no flash changed. Use a matching package or install factory.bin with J-Link"
    );
    println!("Invalidating application commit...");
    dfu.write(META, &[0; 16])?;
    ensure!(
        dfu.read(META, 16)? == [0; 16],
        "commit invalidation failed; application was not erased"
    );
    println!("Writing {} bytes over USB...", image.app.len());
    dfu.write(APP, &image.app)?;
    println!("Verifying entire application and bootloader...");
    ensure!(
        dfu.read(APP, image.app.len())? == image.app,
        "application readback mismatch; retry in recovery"
    );
    ensure!(
        dfu.read(FLASH, boot.len())? == boot,
        "bootloader changed unexpectedly"
    );
    println!("Committing verified application...");
    dfu.write(META, &image.metadata())?;
    ensure!(
        dfu.read(META, 16)? == image.metadata(),
        "commit readback mismatch; retry in recovery"
    );
    println!("Restarting...");
    dfu.leave()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{END, META_MAGIC, sample};
    struct Fake {
        mem: Vec<u8>,
        count: usize,
        fail: usize,
        corrupt: bool,
        left: bool,
    }
    impl Fake {
        fn new(fail: usize) -> Self {
            let image = sample();
            let mut mem = vec![0xff; (END - FLASH) as usize];
            mem[..(APP - FLASH) as usize].fill(0xa5);
            mem[(APP - FLASH) as usize..(APP - FLASH) as usize + image.app.len()]
                .copy_from_slice(&image.app);
            mem[(META - FLASH) as usize..(META - FLASH) as usize + 16]
                .copy_from_slice(&image.metadata());
            Self {
                mem,
                count: 0,
                fail,
                corrupt: false,
                left: false,
            }
        }
        fn step(&mut self) -> Result<()> {
            self.count += 1;
            ensure!(self.count != self.fail, "disconnected");
            Ok(())
        }
        fn valid(&self) -> bool {
            let m = &self.mem[(META - FLASH) as usize..];
            let w = |i| u32::from_le_bytes(m[i..i + 4].try_into().unwrap());
            w(0) == META_MAGIC
                && (8..=META - APP).contains(&w(4))
                && w(12) == !w(8)
                && crc32fast::hash(&self.mem[(APP - FLASH) as usize..(APP - FLASH + w(4)) as usize])
                    == w(8)
        }
    }
    impl FlashTransport for Fake {
        fn read(&mut self, a: u32, n: usize) -> Result<Vec<u8>> {
            self.step()?;
            Ok(self.mem[(a - FLASH) as usize..(a - FLASH) as usize + n].to_vec())
        }
        fn write(&mut self, a: u32, b: &[u8]) -> Result<()> {
            self.step()?;
            let lo = (a - FLASH) as usize;
            let hi = lo + b.len();
            self.mem[lo..hi.div_ceil(2048) * 2048].fill(0xff);
            self.mem[lo..hi].copy_from_slice(b);
            if self.corrupt && a == APP {
                self.mem[lo + 100] ^= 1;
            }
            Ok(())
        }
        fn leave(&mut self) -> Result<()> {
            self.step()?;
            self.left = true;
            Ok(())
        }
    }
    #[test]
    fn every_interruption_can_retry() {
        for step in 1..=9 {
            let mut d = Fake::new(step);
            assert!(install(&mut d, &sample()).is_err());
            assert!(!d.left);
            if (3..=7).contains(&step) {
                assert!(!d.valid());
            }
            assert_eq!(
                &d.mem[..(APP - FLASH) as usize],
                &vec![0xa5; (APP - FLASH) as usize]
            );
            d.fail = 0;
            install(&mut d, &sample()).unwrap();
            assert!(d.valid() && d.left);
        }
    }
    #[test]
    fn wrong_boot_and_bad_readback_never_commit() {
        let mut d = Fake::new(0);
        d.mem[0] ^= 1;
        assert!(install(&mut d, &sample()).is_err());
        assert_eq!(d.count, 1);
        let mut d = Fake::new(0);
        d.corrupt = true;
        assert!(install(&mut d, &sample()).is_err());
        assert!(!d.valid() && !d.left);
    }
}
