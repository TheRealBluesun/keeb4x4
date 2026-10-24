use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
mod build;
mod dfu;
mod events;
mod image;
mod serial;
mod update;

#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);
#[derive(Debug)]
struct Interrupted;
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interrupted")
    }
}
impl std::error::Error for Interrupted {}
impl Cancel {
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Relaxed) {
            return Err(Interrupted.into());
        }
        Ok(())
    }
    pub fn sleep(&self, duration: Duration) -> Result<()> {
        let end = Instant::now() + duration;
        loop {
            self.check()?;
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
}

#[derive(Parser)]
#[command(
    name = "keeb",
    version,
    about = "keeb-v2 USB listener, firmware builder and native DFU updater"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// List application and recovery USB devices.
    List,
    /// Listen to key/encoder events, automatically reconnecting.
    Listen {
        #[arg(long)]
        serial: Option<String>,
        #[arg(long)]
        port: Option<String>,
        #[arg(long)]
        json: bool,
        /// Stop after this many seconds, including time disconnected.
        #[arg(long)]
        duration: Option<f64>,
    },
    /// Build both Rust firmware projects and package .kbf/factory images.
    Build {
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Flash a .kbf package, verify every byte and confirm firmware startup.
    Update {
        image: PathBuf,
        #[arg(long)]
        serial: Option<String>,
        #[arg(long)]
        port: Option<String>,
        /// The board is already in recovery (hold encoder when plugging in).
        #[arg(long)]
        recovery: bool,
    },
    /// Reboot the application into USB recovery without changing flash.
    Bootloader {
        #[arg(long)]
        serial: Option<String>,
        #[arg(long)]
        port: Option<String>,
    },
}
fn run(cli: Cli, cancel: &Cancel) -> Result<()> {
    match cli.command {
        Commands::List => {
            for p in serial::ports(None, None)? {
                println!("application  {}  serial={}", p.path, p.serial);
            }
            for d in dfu::devices(None)? {
                println!(
                    "recovery     USB DFU  serial={}",
                    d.serial_number().unwrap_or("unknown")
                );
            }
        }
        Commands::Build { project } => build::build(&project)?,
        Commands::Listen {
            serial,
            port,
            json,
            duration,
        } => {
            let duration = duration.map(Duration::try_from_secs_f64).transpose()?;
            serial::listen(serial.as_deref(), port.as_deref(), json, duration, cancel)?;
        }
        Commands::Bootloader { serial, port } => {
            let port = serial::select(serial.as_deref(), port.as_deref())?;
            serial::request_bootloader(&port, cancel)?;
            let (_, uid) = dfu::Dfu::wait(Some(&port.serial), cancel.clone())?;
            println!("Recovery ready: {uid}");
        }
        Commands::Update {
            image,
            serial,
            port,
            recovery,
        } => {
            ensure!(
                !recovery || port.is_none(),
                "--port is only for application mode; select recovery with --serial"
            );
            let image = image::Image::load(&image)?; // Validate before device access.
            let serial = if recovery {
                serial
            } else {
                let port = serial::select(serial.as_deref(), port.as_deref())?;
                println!("Requesting recovery on {}...", port.path);
                serial::request_bootloader(&port, cancel)?;
                Some(port.serial)
            };
            let (mut dfu, uid) = dfu::Dfu::wait(serial.as_deref(), cancel.clone())?;
            println!("DFU serial: {uid}");
            update::install(&mut dfu, &image)?;
            drop(dfu);
            let port = serial::wait_application(&uid, image.crc(), cancel)?;
            println!(
                "Update verified; keeb-v2 running on {port} (CRC32 {:08x})",
                image.crc()
            );
        }
    }
    Ok(())
}
fn main() {
    let cli = Cli::parse();
    let updating = matches!(&cli.command, Commands::Update { .. });
    let cancel = Cancel::default();
    let signal = cancel.clone();
    if let Err(e) = ctrlc::set_handler(move || signal.0.store(true, Ordering::Relaxed)) {
        eprintln!("Cannot install interrupt handler: {e}");
        std::process::exit(1);
    }
    if let Err(e) = run(cli, &cancel) {
        if e.is::<Interrupted>() {
            if updating {
                eprintln!(
                    "Update interrupted. Retry with `keeb update --recovery <image.kbf>` if the device remains in recovery."
                );
            } else {
                eprintln!("Interrupted.");
            }
            std::process::exit(130);
        }
        eprintln!("keeb: {e:#}");
        std::process::exit(1);
    }
}
