use crate::image::{APP, END, FLASH, Image, META, elf_flash, hash};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn build(root: &Path) -> Result<()> {
    let root = root.canonicalize().context("project directory not found")?;
    for folder in ["bootloader", "firmware"] {
        ensure!(
            root.join(folder).join("Cargo.toml").exists(),
            "--project must contain firmware/ and bootloader/"
        );
        ensure!(
            Command::new("cargo")
                .args(["build", "--release", "--locked"])
                .current_dir(root.join(folder))
                .status()?
                .success(),
            "{folder} build failed"
        );
    }
    let elf = |folder: &str, name: &str| -> PathBuf {
        root.join(folder)
            .join("target/thumbv7em-none-eabihf/release")
            .join(name)
    };
    let mut boot = elf_flash(&fs::read(elf("bootloader", "keeb-v2-boot"))?, FLASH, APP)?;
    boot.resize((APP - FLASH) as usize, 0xff);
    let image = Image::new(
        elf_flash(&fs::read(elf("firmware", "keeb-v2"))?, APP, META)?,
        hash(&boot),
    )?;
    let mut factory = vec![0xff; (END - FLASH) as usize];
    factory[..boot.len()].copy_from_slice(&boot);
    factory[(APP - FLASH) as usize..(APP - FLASH) as usize + image.app.len()]
        .copy_from_slice(&image.app);
    factory[(META - FLASH) as usize..(META - FLASH) as usize + 16]
        .copy_from_slice(&image.metadata());
    let out = root.join("firmware/artifacts/update");
    fs::create_dir_all(&out)?;
    for (name, data) in [
        ("keeb-v2.kbf", image.pack()),
        ("application.bin", image.app.clone()),
        ("bootloader.bin", boot),
        ("factory.bin", factory),
    ] {
        let tmp = out.join(format!("{name}.tmp"));
        fs::write(&tmp, data)?;
        fs::rename(tmp, out.join(name))?;
    }
    println!(
        "Application: {} / {} bytes, CRC32 {:08x}",
        image.app.len(),
        META - APP,
        image.crc()
    );
    println!("USB update: {}", out.join("keeb-v2.kbf").display());
    println!(
        "Initial J-Link image: {}",
        out.join("factory.bin").display()
    );
    Ok(())
}
