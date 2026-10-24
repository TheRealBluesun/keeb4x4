use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Event {
    pub v: u8,
    #[serde(flatten)]
    pub kind: Kind,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Hello {
        device: String,
        #[serde(default)]
        update: Option<u8>,
        #[serde(default)]
        image_crc32: Option<u32>,
    },
    State {
        ms: u64,
        keys: u16,
        encoder_count: u16,
        encoder_pressed: bool,
        screens_on: bool,
        brightness: u8,
        dropped: u32,
    },
    Key {
        seq: u32,
        ms: u64,
        key: u8,
        row: u8,
        col: u8,
        pressed: bool,
    },
    Encoder {
        seq: u32,
        ms: u64,
        delta: i16,
        pressed: bool,
        screens_on: bool,
        brightness: u8,
    },
}
impl Event {
    fn validate(&self) -> Result<()> {
        ensure!(self.v == 1, "unsupported event version");
        match &self.kind {
            Kind::Hello { device, .. } => ensure!(device == "keeb-v2", "unexpected device"),
            Kind::Key { key, row, col, .. } => ensure!(
                *row < 4 && *col < 4 && *key == row * 4 + col + 1,
                "invalid key coordinates"
            ),
            Kind::State { brightness, .. } | Kind::Encoder { brightness, .. } => {
                ensure!(*brightness <= 100, "invalid brightness")
            }
        }
        Ok(())
    }
    pub fn seq(&self) -> Option<u32> {
        match self.kind {
            Kind::Key { seq, .. } | Kind::Encoder { seq, .. } => Some(seq),
            _ => None,
        }
    }
    pub fn describe(&self) -> String {
        match &self.kind {
            Kind::Hello { .. } => "keeb-v2 connected (event protocol v1)".into(),
            Kind::Key {
                key,
                row,
                col,
                pressed,
                ..
            } => format!(
                "SW{key} {} (row {row}, col {col})",
                if *pressed { "DOWN" } else { "UP" }
            ),
            Kind::Encoder {
                delta,
                pressed,
                screens_on,
                brightness,
                ..
            } => format!(
                "Dial {delta:+}; push {}; screens {}, brightness {brightness}%",
                if *pressed { "DOWN" } else { "UP" },
                if *screens_on { "ON" } else { "OFF" }
            ),
            Kind::State {
                keys,
                encoder_count,
                encoder_pressed,
                screens_on,
                brightness,
                dropped,
                ..
            } => {
                let held = (0..16)
                    .filter(|i| keys & (1 << i) != 0)
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "State: held keys {}; dial count {encoder_count}; push {}; screens {}, brightness {brightness}%; dropped {dropped}",
                    if held.is_empty() { "none" } else { &held },
                    if *encoder_pressed { "DOWN" } else { "UP" },
                    if *screens_on { "ON" } else { "OFF" }
                )
            }
        }
    }
}

#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
    discarding: bool,
}
impl Decoder {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Result<Event>> {
        let mut records = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if !self.discarding && !self.pending.is_empty() {
                    records.push((|| {
                        let event: Event = serde_json::from_slice(&self.pending)?;
                        event.validate()?;
                        Ok(event)
                    })());
                }
                self.pending.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.pending.len() == 4096 {
                    self.pending.clear();
                    self.discarding = true;
                    records.push(Err(anyhow::anyhow!("oversized event discarded")));
                } else {
                    self.pending.push(byte);
                }
            }
        }
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HELLO: &[u8] = b"{\"v\":1,\"type\":\"hello\",\"device\":\"keeb-v2\"}\n";
    #[test]
    fn all_fragment_boundaries() {
        for split in 0..HELLO.len() {
            let mut d = Decoder::default();
            assert!(d.feed(&HELLO[..split]).is_empty());
            let mut rest = HELLO[split..].to_vec();
            rest.extend(HELLO);
            let records = d.feed(&rest);
            assert_eq!(records.len(), 2);
            assert!(records.iter().all(Result::is_ok));
        }
    }
    #[test]
    fn bad_records_resync_and_bound_memory() {
        let mut d = Decoder::default();
        assert!(d.feed(b"not json\n")[0].is_err());
        assert!(d.feed(&vec![b'x'; 10000])[0].is_err());
        assert!(d.pending.len() <= 4096);
        let mut input = b"\n".to_vec();
        input.extend(HELLO);
        assert!(d.feed(&input)[0].is_ok());
        assert!(d.feed(b"{\"v\":2,\"type\":\"hello\",\"device\":\"keeb-v2\"}\n")[0].is_err());
        assert!(d.feed(b"{\"v\":1,\"type\":\"key\"}\n")[0].is_err());
    }
}
