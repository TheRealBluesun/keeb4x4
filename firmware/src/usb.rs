//! USB CDC ACM, newline-delimited JSON v1. Producers never wait for the host.
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
};
use embassy_stm32::{
    Peri, bind_interrupts, peripherals,
    usb::{Driver, InterruptHandler},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embassy_usb::{
    Builder,
    class::cdc_acm::{CdcAcmClass, Receiver, Sender, State},
    driver::EndpointError,
};
use heapless::String;

bind_interrupts!(struct Irqs { USB_LP => InterruptHandler<peripherals::USB>; });
type UsbDriver<'d> = Driver<'d, peripherals::USB>;
static CONNECTED: AtomicBool = AtomicBool::new(false);
static KEYS: AtomicU32 = AtomicU32::new(0);
static ENCODER: AtomicU32 = AtomicU32::new(0);
static SEQ: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static EVENTS: Channel<CriticalSectionRawMutex, Event, 32> = Channel::new();

#[derive(Clone, Copy)]
enum Kind {
    Hello,
    Key(u8, bool),
    Encoder(i16, bool, u32),
}
#[derive(Clone, Copy)]
struct Event {
    seq: u32,
    ms: u64,
    kind: Kind,
}

fn publish(kind: Kind) {
    let seq = SEQ.fetch_add(1, Relaxed).wrapping_add(1);
    if CONNECTED.load(Relaxed)
        && EVENTS
            .try_send(Event {
                seq,
                ms: Instant::now().as_millis(),
                kind,
            })
            .is_err()
    {
        DROPPED.fetch_add(1, Relaxed);
    }
}

pub fn key(index: u8, pressed: bool) {
    if pressed {
        KEYS.fetch_or(1 << index, Relaxed);
    } else {
        KEYS.fetch_and(!(1 << index), Relaxed);
    }
    publish(Kind::Key(index, pressed));
}

pub fn encoder(count: u16, pressed: bool, delta: i16) {
    ENCODER.store(count as u32 | ((pressed as u32) << 16), Relaxed);
    publish(Kind::Encoder(
        delta,
        pressed,
        crate::encoder::control::DISPLAY.load(Relaxed),
    ));
}

fn snapshot() -> String<256> {
    let mut line = String::new();
    let encoder = ENCODER.load(Relaxed);
    let display = crate::encoder::control::DISPLAY.load(Relaxed);
    core::writeln!(&mut line, "{{\"v\":1,\"type\":\"state\",\"ms\":{},\"keys\":{},\"encoder_count\":{},\"encoder_pressed\":{},\"screens_on\":{},\"brightness\":{},\"dropped\":{}}}",
        Instant::now().as_millis(), KEYS.load(Relaxed), encoder & 65535, encoder & 65536 != 0,
        display & 256 != 0, display & 255, DROPPED.load(Relaxed)).unwrap();
    line
}

fn format(event: Event) -> String<256> {
    if matches!(event.kind, Kind::Hello) {
        return hello();
    }
    let mut line = String::new();
    write!(
        &mut line,
        "{{\"v\":1,\"seq\":{},\"ms\":{},",
        event.seq, event.ms
    )
    .unwrap();
    match event.kind {
        Kind::Hello => unreachable!(),
        Kind::Key(index, pressed) => writeln!(&mut line,
            "\"type\":\"key\",\"key\":{},\"row\":{},\"col\":{},\"pressed\":{}}}", index+1, index/4, index%4, pressed).unwrap(),
        Kind::Encoder(delta, pressed, display) => writeln!(&mut line,
            "\"type\":\"encoder\",\"delta\":{},\"pressed\":{},\"screens_on\":{},\"brightness\":{}}}", delta, pressed, display&256 != 0, display&255).unwrap(),
    }
    line
}

fn hello() -> String<256> {
    let mut hello = String::new();
    let crc = unsafe { core::ptr::read_volatile((crate::boot::protocol::META + 8) as *const u32) };
    writeln!(
        &mut hello,
        "{{\"v\":1,\"type\":\"hello\",\"device\":\"keeb-v2\",\"update\":1,\"image_crc32\":{}}}",
        crc
    )
    .unwrap();
    hello
}

async fn send<'d>(
    class: &mut Sender<'d, UsbDriver<'d>>,
    bytes: &[u8],
) -> Result<(), EndpointError> {
    for packet in bytes.chunks(64) {
        with_timeout(Duration::from_secs(2), class.write_packet(packet))
            .await
            .map_err(|_| EndpointError::Disabled)??;
    }
    if bytes.len().is_multiple_of(64) {
        with_timeout(Duration::from_secs(2), class.write_packet(&[]))
            .await
            .map_err(|_| EndpointError::Disabled)??;
    }
    Ok(())
}

async fn serve<'d>(class: &mut Sender<'d, UsbDriver<'d>>) {
    loop {
        class.wait_connection().await;
        if !class.dtr() {
            Timer::after_millis(20).await;
            continue;
        }
        while EVENTS.try_receive().is_ok() {}
        CONNECTED.store(true, Relaxed);
        defmt::info!("USB host listening");
        let session = async {
            send(class, hello().as_bytes()).await?;
            send(class, snapshot().as_bytes()).await?;
            let mut deadline = Instant::now() + Duration::from_secs(1);
            loop {
                if !class.dtr() {
                    return Ok::<(), EndpointError>(());
                }
                let now = Instant::now();
                let line = if now >= deadline {
                    deadline = now + Duration::from_secs(1);
                    snapshot()
                } else {
                    match with_timeout(deadline - now, EVENTS.receive()).await {
                        Ok(event) => format(event),
                        Err(_) => {
                            deadline = Instant::now() + Duration::from_secs(1);
                            snapshot()
                        }
                    }
                };
                send(class, line.as_bytes()).await?;
            }
        };
        let _ = session.await;
        CONNECTED.store(false, Relaxed);
        defmt::info!("USB listener disconnected");
        Timer::after_millis(20).await;
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Command<'a> {
    v: u8,
    cmd: &'a str,
}

async fn receive<'d>(class: &mut Receiver<'d, UsbDriver<'d>>) {
    let mut packet = [0; 64];
    loop {
        class.wait_connection().await;
        let mut line = heapless::Vec::<u8, 128>::new();
        let mut discard = false;
        while let Ok(count) = class.read_packet(&mut packet).await {
            for &byte in &packet[..count] {
                if byte == b'\n' {
                    if !discard
                        && class.dtr()
                        && let Ok((command, used)) = serde_json_core::from_slice::<Command>(&line)
                        && command.v == 1
                        && line[used..].iter().all(u8::is_ascii_whitespace)
                    {
                        match command.cmd {
                            "hello" => publish(Kind::Hello),
                            "bootloader" => {
                                defmt::info!("USB update requested; resetting into recovery");
                                Timer::after_millis(30).await;
                                crate::boot::request_update();
                            }
                            _ => {}
                        }
                    }
                    line.clear();
                    discard = false;
                } else if !discard && line.push(byte).is_err() {
                    discard = true;
                }
            }
        }
    }
}

#[embassy_executor::task]
pub async fn run(
    peripheral: Peri<'static, peripherals::USB>,
    dp: Peri<'static, peripherals::PA12>,
    dm: Peri<'static, peripherals::PA11>,
) {
    let driver = Driver::new(peripheral, Irqs, dp, dm);
    // Embassy example VID/PID for this local prototype, not a production allocation.
    let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("keeb-v2 prototype");
    config.product = Some("keeb-v2 events");
    config.serial_number = Some(embassy_stm32::uid::uid_hex());
    config.max_power = 500;
    let mut descriptor = [0; 256];
    let mut bos = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();
    let mut builder = Builder::new(
        driver,
        config,
        &mut descriptor,
        &mut bos,
        &mut [],
        &mut control_buf,
    );
    let class = CdcAcmClass::new(&mut builder, &mut state, 64);
    let mut device = builder.build();
    defmt::info!("USB CDC starting, serial={}", embassy_stm32::uid::uid_hex());
    let (mut sender, mut receiver) = class.split();
    embassy_futures::join::join3(device.run(), serve(&mut sender), receive(&mut receiver)).await;
}
