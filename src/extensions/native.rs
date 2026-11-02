use anyhow::{Result, bail};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub(super) fn open_regular(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| anyhow::anyhow!("extension file unavailable"))?;
    if !file.metadata()?.is_file() {
        bail!("extension file must be regular");
    }
    Ok(file)
}

pub fn validate(path: &Path) -> Result<()> {
    let mut file = open_regular(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        bail!("extension must be an executable native macOS binary");
    }
    let length = metadata.len();
    let mut header = [0u8; 32];
    file.read_exact(&mut header)
        .map_err(|_| anyhow::anyhow!("invalid native binary header"))?;
    let cpu = if cfg!(target_arch = "aarch64") {
        0x0100000cu32
    } else {
        0x01000007u32
    };
    let mut offset = 0;
    let mut size = length;
    if matches!(&header[..4], b"\xca\xfe\xba\xbe" | b"\xca\xfe\xba\xbf") {
        let wide = header[3] == 0xbf;
        let count = u32::from_be_bytes(header[4..8].try_into()?);
        if count == 0 || count > 64 {
            bail!("invalid universal binary");
        }
        file.seek(SeekFrom::Start(8))?;
        let mut found = false;
        for _ in 0..count {
            let mut arch = [0u8; 32];
            file.read_exact(&mut arch[..if wide { 32 } else { 20 }])?;
            if u32::from_be_bytes(arch[..4].try_into()?) == cpu {
                offset = if wide {
                    u64::from_be_bytes(arch[8..16].try_into()?)
                } else {
                    u32::from_be_bytes(arch[8..12].try_into()?) as u64
                };
                size = if wide {
                    u64::from_be_bytes(arch[16..24].try_into()?)
                } else {
                    u32::from_be_bytes(arch[12..16].try_into()?) as u64
                };
                found = true;
                break;
            }
        }
        if !found || size < 32 || offset.checked_add(size).is_none_or(|end| end > length) {
            bail!("incompatible native binary architecture");
        }
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut header)?;
    }
    let word =
        |index| u32::from_le_bytes(header[index..index + 4].try_into().expect("header word"));
    if word(0) != 0xfeedfacf || word(4) != cpu || word(12) != 2 {
        bail!("extension requires a compatible native macOS executable; scripts are unsupported");
    }
    let count = word(16);
    let commands = word(20) as u64;
    if count == 0 || count > 65536 || commands > size - 32 {
        bail!("invalid native binary load commands");
    }
    let end = offset + 32 + commands;
    file.seek(SeekFrom::Start(offset + 32))?;
    let mut entry = false;
    for _ in 0..count {
        let start = file.stream_position()?;
        let mut command = [0u8; 8];
        file.read_exact(&mut command)?;
        let kind = u32::from_le_bytes(command[..4].try_into()?);
        let size = u32::from_le_bytes(command[4..].try_into()?) as u64;
        if size < 8 || start + size > end {
            bail!("invalid native binary load command");
        }
        entry |= (kind == 0x80000028 && size >= 24) || kind == 5;
        file.seek(SeekFrom::Start(start + size))?;
    }
    if !entry || file.stream_position()? != end {
        bail!("native executable entry point missing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rejects_scripts_truncation_foreign_architecture_and_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("looks-native");
        for bytes in [
            b"#!/bin/sh\necho should-not-run\n".as_slice(),
            b"not an executable",
        ] {
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(validate(&path).is_err());
        }
        let original = fs::read(std::env::current_exe().unwrap()).unwrap();
        fs::write(&path, &original).unwrap();
        validate(&path).unwrap();
        let mut foreign = original.clone();
        foreign[4..8].copy_from_slice(&0u32.to_le_bytes());
        fs::write(&path, foreign).unwrap();
        assert!(validate(&path).is_err());
        for length in [4, 31, 32, 64] {
            fs::write(&path, &original[..length]).unwrap();
            assert!(validate(&path).is_err());
        }
        let mut malformed = original;
        malformed[16..24].fill(0);
        fs::write(&path, malformed).unwrap();
        assert!(validate(&path).is_err());
        dir.close().unwrap();
    }
}
