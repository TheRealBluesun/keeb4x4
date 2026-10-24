use crate::{
    Cancel,
    events::{Decoder, Event, Kind},
};
use anyhow::{Context, Result, bail, ensure};
use serialport::{SerialPort, SerialPortType};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct Port {
    pub path: String,
    pub serial: String,
}
pub fn ports(serial: Option<&str>, path: Option<&str>) -> Result<Vec<Port>> {
    Ok(serialport::available_ports()?
        .into_iter()
        .filter_map(|p| {
            let SerialPortType::UsbPort(info) = p.port_type else {
                return None;
            };
            if (info.vid, info.pid) != (0xc0de, 0xcafe)
                || info.product.as_deref() != Some("keeb-v2 events")
            {
                return None;
            }
            let uid = info.serial_number?;
            (serial.is_none_or(|s| s == uid) && path.is_none_or(|s| s == p.port_name)).then_some(
                Port {
                    path: p.port_name,
                    serial: uid,
                },
            )
        })
        .collect())
}
pub fn select(serial: Option<&str>, path: Option<&str>) -> Result<Port> {
    let mut found = ports(serial, path)?;
    ensure!(
        found.len() == 1,
        "expected one keypad; use --serial/--port or --recovery if already in DFU"
    );
    Ok(found.remove(0))
}
struct Connection {
    port: Box<dyn SerialPort>,
    decoder: Decoder,
    query_at: Instant,
    hello: bool,
    last_event: Instant,
}
impl Connection {
    fn open(port: &Port) -> Result<Self> {
        let mut serial = serialport::new(&port.path, 115200)
            .timeout(Duration::from_millis(100))
            .open()
            .with_context(|| {
                format!(
                    "cannot open {}; close another listener/updater using the port",
                    port.path
                )
            })?;
        serial.write_data_terminal_ready(true)?;
        Ok(Self {
            port: serial,
            decoder: Decoder::default(),
            query_at: Instant::now(),
            hello: false,
            last_event: Instant::now(),
        })
    }
    fn command(&mut self, cmd: &str) -> Result<()> {
        self.port
            .write_all(format!("{{\"v\":1,\"cmd\":\"{cmd}\"}}\n").as_bytes())?;
        // tcdrain can block indefinitely for CDC, even with a port timeout.
        // The hello response / observed USB disconnect confirms delivery.
        Ok(())
    }
    fn read(&mut self, cancel: &Cancel) -> Result<Vec<Event>> {
        cancel.check()?;
        ensure!(
            self.last_event.elapsed() < Duration::from_secs(5),
            "no valid USB events for five seconds; reopening serial connection"
        );
        if !self.hello && Instant::now() >= self.query_at {
            self.command("hello")?;
            self.query_at = Instant::now() + Duration::from_secs(1);
        }
        let mut buf = [0; 512];
        let count = match self.port.read(&mut buf) {
            Ok(0) => bail!("serial device disconnected"),
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Ok(vec![]);
            }
            Err(e) => return Err(e.into()),
        };
        let mut events = Vec::new();
        for record in self.decoder.feed(&buf[..count]) {
            match record {
                Ok(event) => {
                    self.last_event = Instant::now();
                    if matches!(event.kind, Kind::Hello { .. }) {
                        self.hello = true;
                    }
                    events.push(event);
                }
                Err(e) => eprintln!("Protocol warning: {e}"),
            }
        }
        Ok(events)
    }
}

pub fn request_bootloader(port: &Port, cancel: &Cancel) -> Result<()> {
    let mut connection = Connection::open(port)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        for event in connection.read(cancel)? {
            if let Kind::Hello { update, .. } = event.kind {
                ensure!(
                    update == Some(1),
                    "firmware lacks USB update support; install factory.bin over SWD once (see docs/PROGRAMMER.md)"
                );
                connection.command("bootloader")?;
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    // Keep DTR and the file open until the reboot actually occurs.
                    if ports(Some(&port.serial), None)?.is_empty() {
                        return Ok(());
                    }
                    ensure!(
                        Instant::now() < deadline,
                        "firmware did not disconnect after reboot request"
                    );
                    cancel.sleep(Duration::from_millis(50))?;
                }
            }
        }
    }
    bail!("no firmware handshake; close other programs using the serial port")
}

pub fn wait_application(serial: &str, crc: u32, cancel: &Cancel) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        cancel.check()?;
        let found = ports(Some(serial), None)?;
        ensure!(
            found.len() <= 1,
            "multiple devices have the expected serial"
        );
        if let Some(port) = found.first()
            && let Ok(mut connection) = Connection::open(port)
        {
            let mut verified = false;
            while Instant::now() < deadline {
                let events = match connection.read(cancel) {
                    Ok(e) => e,
                    Err(_) => {
                        cancel.check()?;
                        break;
                    }
                };
                for event in events {
                    match event.kind {
                        Kind::Hello { image_crc32, .. } => {
                            ensure!(
                                image_crc32 == Some(crc),
                                "reconnected firmware has the wrong checksum"
                            );
                            verified = true;
                        }
                        Kind::State { .. } if verified => return Ok(port.path.clone()),
                        _ => {}
                    }
                }
            }
        }
        cancel.sleep(Duration::from_millis(100))?;
    }
    bail!(
        "image committed but application did not reconnect; release encoder and reconnect USB, or hold it and retry with --recovery"
    )
}

pub fn listen(
    serial: Option<&str>,
    path: Option<&str>,
    json: bool,
    duration: Option<Duration>,
    cancel: &Cancel,
) -> Result<()> {
    let deadline = duration
        .map(|d| {
            Instant::now()
                .checked_add(d)
                .context("duration is too large")
        })
        .transpose()?;
    let active = || deadline.is_none_or(|d| Instant::now() < d);
    let mut waiting = false;
    while active() {
        cancel.check()?;
        let found = ports(serial, path)?;
        ensure!(
            found.len() <= 1,
            "multiple keypads; select --serial or --port"
        );
        if let Some(port) = found.first() {
            let session = (|| -> Result<()> {
                let mut connection = Connection::open(port)?;
                eprintln!("Listening on {} ({})", port.path, port.serial);
                waiting = false;
                let mut last_seq: Option<u32> = None;
                let mut last_state = None;
                let stdout = io::stdout();
                let mut output = stdout.lock();
                while active() {
                    for event in connection.read(cancel)? {
                        if matches!(event.kind, Kind::Hello { .. }) {
                            last_seq = None;
                            last_state = None;
                        }
                        if let Some(seq) = event.seq() {
                            if last_seq.is_some_and(|s| s.wrapping_add(1) != seq) {
                                eprintln!(
                                    "Event sequence gap; reconcile with the next state snapshot."
                                );
                            }
                            last_seq = Some(seq);
                        }
                        if json {
                            serde_json::to_writer(&mut output, &event)
                                .context("writing events to stdout")?;
                            writeln!(output).context("writing events to stdout")?;
                        } else {
                            let description = event.describe();
                            if !matches!(event.kind, Kind::State { .. })
                                || last_state.as_ref() != Some(&description)
                            {
                                writeln!(output, "{description}")
                                    .context("writing events to stdout")?;
                            }
                            if matches!(event.kind, Kind::State { .. }) {
                                last_state = Some(description);
                            }
                        }
                        output.flush().context("writing events to stdout")?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = session {
                cancel.check()?;
                if e.to_string() == "writing events to stdout"
                    && (e
                        .downcast_ref::<io::Error>()
                        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
                        || e.downcast_ref::<serde_json::Error>()
                            .is_some_and(|e| e.io_error_kind() == Some(io::ErrorKind::BrokenPipe)))
                {
                    return Ok(());
                }
                eprintln!("USB disconnected/unavailable: {e:#}; retrying");
            }
        } else if !waiting {
            eprintln!("Waiting for keeb-v2...");
            waiting = true;
        }
        if active() {
            cancel.sleep(Duration::from_millis(200))?;
        }
    }
    Ok(())
}
