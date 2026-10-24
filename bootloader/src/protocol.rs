//! Flash layout v2; the Python packager/updater verifies the same contract.
#![allow(dead_code)]
pub const APP: u32 = 0x0800_8000;
pub const META: u32 = 0x0801_f800;
pub const MAILBOX: u32 = 0x2000_7ff0;
pub const REQUEST: u32 = 0xb007_df01;
pub const META_MAGIC: u32 = 0x3242_454b;

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

pub fn valid_vectors(sp: u32, reset: u32, length: u32) -> bool {
    (8..=META - APP).contains(&length)
        && sp > 0x2000_0000
        && sp <= MAILBOX
        && sp & 7 == 0
        && reset & 1 == 1
        && reset & !1 >= APP
        && (reset & !1) < APP + length
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crc_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
    #[test]
    fn vector_bounds() {
        assert!(valid_vectors(MAILBOX, APP + 0x101, 1024));
        assert!(!valid_vectors(MAILBOX + 8, APP + 1, 1024));
        assert!(!valid_vectors(MAILBOX, APP + 1, META - APP + 1));
        assert!(!valid_vectors(MAILBOX, APP + 1025, 1024));
        assert!(!valid_vectors(MAILBOX, APP + 256, 1024));
    }
}
