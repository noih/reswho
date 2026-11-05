use crate::{extensions::ProcessIdentity, model::Process};
use std::path::{Path, PathBuf};

pub(crate) const CANDIDATE: &str = "com.apple.Virtualization.VirtualMachine";

#[derive(Default)]
pub(crate) struct Probe {
    pub paths: Vec<PathBuf>,
    pub responsible: Option<u32>,
    pub limited: bool,
}

pub(crate) fn candidate(process: &Process) -> bool {
    process
        .executable
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|name| name == CANDIDATE)
}

pub(crate) fn responsible<'a>(
    process: &Process,
    pid: Option<u32>,
    processes: &'a std::collections::HashMap<u32, &'a Process>,
    valid: impl Fn(&Process) -> bool,
) -> Option<&'a Process> {
    let parent = *processes.get(&pid.filter(|pid| *pid > 1 && *pid != process.pid)?)?;
    (parent.uid? == process.uid?
        && ProcessIdentity::observed(parent)?.started_unix_us <= process.started_unix_us?
        && parent.executable.is_some()
        && valid(parent))
    .then_some(parent)
}

pub(crate) fn probe(process: &Process) -> Probe {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn reswho_vm_probe(pid: i32, paths: *mut libc::c_char, limited: *mut i32) -> i32;
        }
        if process.uid != Some(unsafe { libc::geteuid() })
            || !crate::collect::verify_identity(process)
        {
            return Probe::default();
        }
        let mut paths = [[0i8; 1024]; 64];
        let address = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"responsibility_get_pid_responsible_for_pid".as_ptr(),
            )
        };
        let resolve = if address.is_null() {
            None
        } else {
            Some(unsafe {
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(i32) -> i32>(address)
            })
        };
        let responsible = responsible_pid(process.pid, resolve);
        let mut limited = 0;
        let count =
            unsafe { reswho_vm_probe(process.pid as i32, paths.as_mut_ptr().cast(), &mut limited) };
        if !crate::collect::verify_identity(process) {
            return Probe::default();
        }
        let paths = paths
            .iter()
            .take(count.max(0) as usize)
            .filter_map(|path| {
                let value = unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
                    .to_str()
                    .ok()?;
                Some(PathBuf::from(value))
            })
            .collect();
        Probe {
            paths,
            responsible,
            limited: limited != 0,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = process;
        Probe::default()
    }
}

fn responsible_pid(pid: u32, resolve: Option<unsafe extern "C" fn(i32) -> i32>) -> Option<u32> {
    let value = unsafe { resolve?(i32::try_from(pid).ok()?) };
    u32::try_from(value).ok().filter(|p| *p > 1 && *p != pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vm_missing_symbol_and_minus_one_responsibility_degrade_to_none() {
        unsafe extern "C" fn unavailable(_: i32) -> i32 {
            -1
        }
        unsafe extern "C" fn invalid(_: i32) -> i32 {
            1
        }
        unsafe extern "C" fn same(pid: i32) -> i32 {
            pid
        }
        assert_eq!(responsible_pid(42, None), None);
        for resolve in [
            unavailable as unsafe extern "C" fn(i32) -> i32,
            invalid,
            same,
        ] {
            assert_eq!(responsible_pid(42, Some(resolve)), None);
        }
    }
}
