use crate::model::{FileEvidence as OpenFile, Process};
use std::{
    mem::{MaybeUninit, size_of},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::PathBuf,
};

const FILE_WRITE: u32 = 2;
const FD_SHARED: u32 = 1;
const FD_CLOSE_ON_EXEC: u32 = 2;
const VNODE_PATH_INFO: i32 = 2;

#[repr(C)]
struct FileInfo {
    flags: u32,
    status: u32,
    offset: i64,
    kind: i32,
    guard: u32,
}

#[repr(C)]
struct VnodePath {
    file: FileInfo,
    vnode: libc::vnode_info_path,
}

fn inspect(process: &Process, fd: i32) -> Option<OpenFile> {
    let mut info = MaybeUninit::<VnodePath>::zeroed();
    let size = size_of::<VnodePath>() as i32;
    if unsafe {
        libc::proc_pidfdinfo(
            process.pid as i32,
            fd,
            VNODE_PATH_INFO,
            info.as_mut_ptr().cast(),
            size,
        )
    } != size
    {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let stat = info.vnode.vip_vi.vi_stat;
    if info.file.flags & FILE_WRITE == 0
        || info.file.flags & libc::O_APPEND as u32 == 0
        || info.file.status & FD_CLOSE_ON_EXEC == 0
        || info.file.status & FD_SHARED != 0
        || stat.vst_mode & libc::S_IFMT != libc::S_IFREG
        || Some(stat.vst_uid) != process.uid
    {
        return None;
    }
    let bytes: Vec<u8> = info
        .vnode
        .vip_path
        .iter()
        .flatten()
        .map(|c| *c as u8)
        .take_while(|c| *c != 0)
        .collect();
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
    if !path.is_absolute() {
        return None;
    }
    let metadata = path.symlink_metadata().ok()?;
    if !metadata.is_file()
        || metadata.dev() != u64::from(stat.vst_dev)
        || metadata.ino() != stat.vst_ino
    {
        return None;
    }
    Some(OpenFile {
        fd,
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub(crate) fn writable_files(process: &Process) -> Option<Vec<OpenFile>> {
    if process.uid != Some(unsafe { libc::geteuid() }) || !super::verify_identity(process) {
        return None;
    }
    let fds = super::network::descriptors(process.pid as i32, 32).ok()?;
    let files = fds
        .into_iter()
        .filter(|fd| fd.proc_fdtype == 1)
        .filter_map(|fd| inspect(process, fd.proc_fd))
        .collect();
    super::verify_identity(process).then_some(files)
}

pub(crate) fn still_open(process: &Process, file: &OpenFile) -> bool {
    super::verify_identity(process)
        && inspect(process, file.fd).as_ref() == Some(file)
        && super::verify_identity(process)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn writable_handle_revalidation_rejects_closed_reused_replaced_and_foreign_identity() {
        let pid = std::process::id();
        let identity =
            libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(pid as i32, 0).unwrap();
        let process = Process {
            pid,
            uid: Some(identity.pbi_uid),
            started_unix_us: Some(identity.pbi_start_tvsec * 1_000_000 + identity.pbi_start_tvusec),
            executable: Some(std::env::current_exe().unwrap()),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("owned.jsonl");
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .unwrap();
        let observed = crate::collect::writable_files(&process)
            .unwrap()
            .into_iter()
            .find(|value| value.fd == file.as_raw_fd())
            .unwrap();
        assert!(crate::collect::still_open(&process, &observed));
        let read_only = std::fs::File::open(&path).unwrap();
        assert!(inspect(&process, read_only.as_raw_fd()).is_none());
        drop(read_only);
        let duplicate = file.try_clone().unwrap();
        assert!(inspect(&process, file.as_raw_fd()).is_none());
        drop(duplicate);

        let mut changed = process.clone();
        changed.started_unix_us = changed.started_unix_us.map(|value| value + 1);
        assert!(crate::collect::writable_files(&changed).is_none());
        assert!(!crate::collect::still_open(&changed, &observed));
        changed = process.clone();
        changed.uid = Some(identity.pbi_uid + 1);
        assert!(crate::collect::writable_files(&changed).is_none());
        std::fs::rename(&path, root.path().join("old.jsonl")).unwrap();
        std::fs::write(&path, "replacement").unwrap();
        assert!(!crate::collect::still_open(&process, &observed));
        drop(file);
        assert!(!crate::collect::still_open(&process, &observed));
        let file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        assert!(!crate::collect::still_open(&process, &observed));
        drop(file);
        root.close().unwrap();
    }
}
