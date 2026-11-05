use std::{io::Read, path::Path};

pub(crate) const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, PartialEq)]
pub(crate) enum Trusted {
    Missing,
    Rejected,
    Bytes(Vec<u8>),
}

pub(crate) fn read(file: &Path, owner: Option<u32>) -> Trusted {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(file);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Trusted::Missing,
        Err(_) => return Trusted::Rejected,
    };
    let Ok(metadata) = file.metadata() else {
        return Trusted::Rejected;
    };
    if !metadata.is_file()
        || metadata.len() > MAX_BYTES
        || metadata.uid() != 0 && Some(metadata.uid()) != owner
    {
        return Trusted::Rejected;
    }
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES {
        return Trusted::Rejected;
    }
    Trusted::Bytes(bytes)
}

pub(crate) fn define<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == "#define" && words.next()? == name).then(|| words.next())?
    })
}

pub(crate) fn version_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

pub(crate) fn tail(path: &Path, owner: Option<u32>) -> Trusted {
    use std::{
        io::{Seek, SeekFrom},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
    else {
        return Trusted::Rejected;
    };
    let Ok(before) = file.metadata() else {
        return Trusted::Rejected;
    };
    if !before.is_file() || Some(before.uid()) != owner {
        return Trusted::Rejected;
    }
    let offset = before.len().saturating_sub(MAX_BYTES);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Trusted::Rejected;
    }
    let mut bytes = Vec::new();
    if (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_BYTES
    {
        return Trusted::Rejected;
    }
    let (Ok(after), Ok(current)) = (file.metadata(), path.symlink_metadata()) else {
        return Trusted::Rejected;
    };
    if before.ino() != current.ino()
        || before.dev() != current.dev()
        || !current.is_file()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Trusted::Rejected;
    }
    if offset > 0 {
        let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') else {
            return Trusted::Rejected;
        };
        bytes.drain(..=newline);
    }
    Trusted::Bytes(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn other_uid() -> Option<u32> {
        let user = unsafe { libc::geteuid() };
        (user != 0).then_some(if user == 1 { 2 } else { 1 })
    }

    #[test]
    fn hardened_reads_reject_fifos_symlinks_foreign_owners_and_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let user = Some(unsafe { libc::geteuid() });
        let good = root.join("good.h");
        std::fs::write(&good, "#define NAME 1\n").unwrap();
        assert_eq!(
            read(&good, user),
            Trusted::Bytes(b"#define NAME 1\n".to_vec())
        );
        assert_eq!(read(&root.join("missing.h"), user), Trusted::Missing);
        let link = root.join("link.h");
        std::os::unix::fs::symlink(&good, &link).unwrap();
        assert_eq!(read(&link, user), Trusted::Rejected);
        let large = root.join("large.h");
        std::fs::write(&large, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert_eq!(read(&large, user), Trusted::Rejected);
        std::fs::create_dir(root.join("dir.h")).unwrap();
        assert_eq!(read(&root.join("dir.h"), user), Trusted::Rejected);
        if let Some(other) = other_uid() {
            assert_eq!(read(&good, Some(other)), Trusted::Rejected);
            assert_eq!(read(&good, None), Trusted::Rejected);
        }
        let fifo = root.join("fifo.h");
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        let pipe = fifo.clone();
        std::thread::spawn(move || sender.send(read(&pipe, user)).unwrap());
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            Trusted::Rejected
        );
        std::fs::remove_file(&fifo).unwrap();
        temp.close().unwrap();
    }

    #[test]
    fn define_returns_only_the_named_token() {
        let text = "#define A 1\n# define B 2\n#define AB 3\n#define C\n";
        assert_eq!(define(text, "A"), Some("1"));
        assert_eq!(define(text, "AB"), Some("3"));
        assert_eq!(define(text, "B"), None);
        assert_eq!(define(text, "C"), None);
    }
    #[test]
    fn bounded_index_tail_skips_partial_prefix_and_rejects_untrusted_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("index.jsonl");
        let owner = Some(unsafe { libc::geteuid() });
        let mut bytes = vec![b'x'; MAX_BYTES as usize + 10];
        bytes.extend_from_slice(b"\n{\"id\":\"exact\"}\n");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            tail(&path, owner),
            Trusted::Bytes(b"{\"id\":\"exact\"}\n".to_vec())
        );
        let link = root.path().join("symlink.jsonl");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(tail(&link, owner), Trusted::Rejected);
        if let Some(other) = other_uid() {
            assert_eq!(tail(&path, Some(other)), Trusted::Rejected);
        }
        root.close().unwrap();
    }
}
