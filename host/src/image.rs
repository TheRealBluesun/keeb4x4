//! Byte-compatible with the installed layout-v2 bootloader and .kbf packages.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub const FLASH: u32 = 0x0800_0000;
pub const APP: u32 = 0x0800_8000;
pub const META: u32 = 0x0801_f800;
pub const END: u32 = 0x0802_0000;
pub const MAILBOX: u32 = 0x2000_7ff0;
pub const META_MAGIC: u32 = 0x3242_454b;
const MAGIC: &[u8; 8] = b"KEEBV2\0\x01";
const HEADER: usize = 88;

pub fn hash(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}
fn word(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset + 4)
            .context("truncated image")?
            .try_into()?,
    ))
}
fn half(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        data.get(offset..offset + 2)
            .context("truncated ELF")?
            .try_into()?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub app: Vec<u8>,
    pub boot_hash: [u8; 32],
}
impl Image {
    pub fn new(app: Vec<u8>, boot_hash: [u8; 32]) -> Result<Self> {
        ensure!(
            (8..=(META - APP) as usize).contains(&app.len()),
            "application must fit the 94 KiB region"
        );
        let sp = word(&app, 0)?;
        let reset = word(&app, 4)?;
        ensure!(
            sp > 0x2000_0000 && sp <= MAILBOX && sp.is_multiple_of(8),
            "invalid application stack pointer"
        );
        ensure!(
            reset & 1 == 1 && reset & !1 >= APP && (reset & !1) < APP + app.len() as u32,
            "application must be linked at 0x08008000 with a valid Thumb reset vector"
        );
        Ok(Self { app, boot_hash })
    }
    pub fn crc(&self) -> u32 {
        crc32fast::hash(&self.app)
    }
    pub fn metadata(&self) -> Vec<u8> {
        [META_MAGIC, self.app.len() as u32, self.crc(), !self.crc()]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect()
    }
    pub fn pack(&self) -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        for value in [2, APP, self.app.len() as u32, self.crc()] {
            data.extend(value.to_le_bytes());
        }
        data.extend(self.boot_hash);
        data.extend(hash(&self.app));
        data.extend(&self.app);
        data
    }
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() >= HEADER && data.get(..8) == Some(MAGIC),
            "not a keeb-v2 package"
        );
        ensure!(
            word(data, 8)? == 2 && word(data, 12)? == APP,
            "unsupported firmware layout"
        );
        let app = &data[HEADER..];
        ensure!(
            app.len() == word(data, 16)? as usize
                && crc32fast::hash(app) == word(data, 20)?
                && hash(app).as_slice() == &data[56..88],
            "package length/checksum mismatch"
        );
        Self::new(app.to_vec(), data[24..56].try_into()?)
    }
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            fs::metadata(path)?.len() <= (HEADER + (META - APP) as usize) as u64,
            "oversized package"
        );
        Self::parse(&fs::read(path)?).with_context(|| format!("invalid package {}", path.display()))
    }
}

pub fn elf_flash(data: &[u8], start: u32, end: u32) -> Result<Vec<u8>> {
    ensure!(
        FLASH <= start && start < end && end <= END,
        "invalid flash range"
    );
    ensure!(
        data.len() >= 52 && data.get(..7) == Some(b"\x7fELF\x01\x01\x01"),
        "expected little-endian ELF32"
    );
    ensure!(
        half(data, 16)? == 2 && half(data, 18)? == 40,
        "expected ARM executable ELF"
    );
    let phoff = word(data, 28)? as usize;
    let phsize = half(data, 42)? as usize;
    let phnum = half(data, 44)? as usize;
    ensure!(
        phsize == 32
            && phoff
                .checked_add(phsize * phnum)
                .is_some_and(|n| n <= data.len()),
        "invalid ELF program headers"
    );
    let mut result = vec![0xff; (end - start) as usize];
    let mut occupied = vec![false; result.len()];
    let mut high = 0;
    for index in 0..phnum {
        let p = phoff + index * phsize;
        let size = word(data, p + 16)? as usize;
        if word(data, p)? != 1 || size == 0 {
            continue;
        }
        let address = word(data, p + 12)?;
        let offset = word(data, p + 4)? as usize;
        ensure!(
            address >= start && address.checked_add(size as u32).is_some_and(|n| n <= end),
            "ELF segment outside intended region: {address:#x}"
        );
        let source = offset
            .checked_add(size)
            .and_then(|n| data.get(offset..n))
            .context("truncated ELF segment")?;
        let low = (address - start) as usize;
        let upper = low + size;
        ensure!(
            !occupied[low..upper].iter().any(|v| *v),
            "overlapping ELF segments"
        );
        occupied[low..upper].fill(true);
        result[low..upper].copy_from_slice(source);
        high = high.max(upper);
    }
    ensure!(high != 0, "ELF contains no flash data");
    result.truncate(high);
    Ok(result)
}

#[cfg(test)]
pub fn sample() -> Image {
    let mut app = vec![0x5a; 4096];
    app[..4].copy_from_slice(&MAILBOX.to_le_bytes());
    app[4..8].copy_from_slice(&(APP + 9).to_le_bytes());
    Image::new(app, hash(&vec![0xa5; (APP - FLASH) as usize])).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_round_trip_and_corruption() {
        let image = sample();
        let packed = image.pack();
        assert_eq!(Image::parse(&packed).unwrap(), image);
        for pos in [0, 8, 12, 16, 20, 56, HEADER, packed.len() - 1] {
            let mut broken = packed.clone();
            broken[pos] ^= 1;
            assert!(Image::parse(&broken).is_err(), "offset {pos}");
        }
        for size in [0, 7, 87, packed.len() - 1] {
            assert!(Image::parse(&packed[..size]).is_err());
        }
    }
    #[test]
    fn invalid_vectors_and_size() {
        for (sp, rv) in [
            (MAILBOX + 8, APP + 9),
            (MAILBOX, FLASH + 9),
            (MAILBOX, APP + 8),
            (MAILBOX, APP + 4097),
        ] {
            let mut image = sample();
            image.app[..4].copy_from_slice(&sp.to_le_bytes());
            image.app[4..8].copy_from_slice(&rv.to_le_bytes());
            assert!(Image::new(image.app, image.boot_hash).is_err());
        }
        let mut image = sample();
        image.app.resize((META - APP + 1) as usize, 0);
        assert!(Image::new(image.app, image.boot_hash).is_err());
    }
    #[test]
    fn truncated_elf_and_bad_offsets() {
        for size in 0..100 {
            assert!(elf_flash(&vec![0; size], APP, META).is_err());
        }
        let mut elf = vec![0; 84];
        elf[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&40u16.to_le_bytes());
        elf[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        elf[42..44].copy_from_slice(&32u16.to_le_bytes());
        elf[44..46].copy_from_slice(&1u16.to_le_bytes());
        assert!(elf_flash(&elf, APP, META).is_err());
    }

    #[test]
    fn elf_load_segment_is_extracted_and_region_checked() {
        let mut elf = vec![0; 100];
        elf[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&40u16.to_le_bytes());
        elf[28..32].copy_from_slice(&52u32.to_le_bytes());
        elf[42..44].copy_from_slice(&32u16.to_le_bytes());
        elf[44..46].copy_from_slice(&1u16.to_le_bytes());
        for (index, value) in [1, 84, APP, APP, 16, 16, 5, 4].into_iter().enumerate() {
            elf[52 + index * 4..56 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        elf[84..].copy_from_slice(&sample().app[..16]);
        assert_eq!(elf_flash(&elf, APP, META).unwrap(), sample().app[..16]);
        elf[64..68].copy_from_slice(&FLASH.to_le_bytes());
        assert!(elf_flash(&elf, APP, META).is_err());
    }
}
