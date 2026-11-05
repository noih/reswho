use crate::model::{Diagnostic, Process, SystemSummary};
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, time::Duration};

#[cfg(target_os = "macos")]
mod command;
#[cfg(target_os = "macos")]
mod network;

pub struct Collection {
    pub processes: Vec<Process>,
    pub system: SystemSummary,
    pub sample_ms: u64,
    pub warnings: Vec<Diagnostic>,
}

pub(crate) const COLLECTION_NOTES: [&str; 3] = [
    "Command arguments are queried on demand for selected PID details; container-internal processes are not implemented.",
    "TCP/UDP sockets are a non-atomic snapshot, not traffic or inactivity history; sockets may change during collection.",
    "Process memory is RSS; shared pages may overlap between processes. Group totals are sums of known RSS, not unique physical memory.",
];

pub fn cpu_percent(previous: u64, current: u64, elapsed_ticks: u64) -> Option<f64> {
    let delta = current.checked_sub(previous)?;
    (elapsed_ticks > 0).then(|| delta as f64 * 100.0 / elapsed_ticks as f64)
}

pub fn collect(interval: Duration) -> Result<Collection> {
    collect_with_cancel(interval, &crate::snapshot::Cancellation::default(), None)
}

pub(crate) fn collect_with_cancel(
    interval: Duration,
    cancel: &crate::snapshot::Cancellation,
    command_pid: Option<u32>,
) -> Result<Collection> {
    let settings = crate::extensions::settings_registry()?.resolve(
        &crate::settings::Config::default(),
        &std::collections::BTreeMap::from([(
            "sampling.interval_ms".into(),
            serde_json::Value::from(interval.as_millis() as u64),
        )]),
    )?;
    collect_with_context(
        &crate::extensions::Context::new(&settings, cancel),
        command_pid.as_slice(),
    )
}

pub(crate) fn extension_command(
    process: &Process,
    pointer_width: usize,
) -> crate::model::ProcessCommand {
    #[cfg(target_os = "macos")]
    {
        command::collect(
            process.pid as i32,
            process.executable.as_deref(),
            pointer_width,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (process, pointer_width);
        crate::model::ProcessCommand::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct ArgvCost {
    pub candidates: u64,
    pub reads: u64,
    pub bytes: u64,
    pub duration_ms: f64,
}

pub(crate) struct ArgvPrefix {
    buffer: Option<Option<Vec<u8>>>,
    pub(crate) verify: fn(&Process) -> bool,
    pub(crate) argmax: fn() -> Option<usize>,
    pub(crate) fill: fn(i32, &mut [u8]) -> Option<usize>,
    pub(crate) pointer_width: fn(i32) -> Option<usize>,
    pub(crate) costs: std::collections::BTreeMap<String, ArgvCost>,
}

impl Default for ArgvPrefix {
    fn default() -> Self {
        #[cfg(test)]
        {
            Self::with(|_| false, || None, |_, _| None, |_| None)
        }
        #[cfg(all(not(test), target_os = "macos"))]
        {
            Self::with(
                verify_identity,
                || command::argmax().ok(),
                |pid, buffer| command::fill(pid, buffer).ok(),
                macos::pointer_width,
            )
        }
        #[cfg(all(not(test), not(target_os = "macos")))]
        {
            Self::with(|_| false, || None, |_, _| None, |_| None)
        }
    }
}

impl ArgvPrefix {
    pub(crate) fn with(
        verify: fn(&Process) -> bool,
        argmax: fn() -> Option<usize>,
        fill: fn(i32, &mut [u8]) -> Option<usize>,
        pointer_width: fn(i32) -> Option<usize>,
    ) -> Self {
        Self {
            buffer: None,
            verify,
            argmax,
            fill,
            pointer_width,
            costs: Default::default(),
        }
    }

    pub(crate) fn read(
        &mut self,
        provider: &str,
        process: &Process,
        limit: usize,
    ) -> Option<Vec<String>> {
        let started = std::time::Instant::now();
        let cost = self.costs.entry(provider.to_owned()).or_default();
        cost.candidates += 1;
        let result = 'read: {
            if !(self.verify)(process) {
                break 'read None;
            }
            let Some(pointer_width) = (self.pointer_width)(process.pid as i32) else {
                break 'read None;
            };
            let argmax = self.argmax;
            let Some(buffer) = self
                .buffer
                .get_or_insert_with(|| {
                    argmax().map(|argmax| vec![0u8; argmax + std::mem::size_of::<i32>()])
                })
                .as_mut()
            else {
                break 'read None;
            };
            cost.reads += 1;
            let Some(size) = (self.fill)(process.pid as i32, buffer) else {
                break 'read None;
            };
            cost.bytes += size as u64;
            let args = parse_prefix(&buffer[..size], pointer_width, limit)?;
            (self.verify)(process).then_some(args)
        };
        cost.duration_ms += started.elapsed().as_secs_f64() * 1000.0;
        result
    }
}

fn parse_prefix(buffer: &[u8], pointer_width: usize, limit: usize) -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        command::parse_prefix(buffer, pointer_width, limit).ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (buffer, pointer_width, limit);
        None
    }
}

pub(crate) fn sanitize_command(
    argv: Vec<String>,
    executable: Option<&std::path::Path>,
) -> crate::model::ProcessCommand {
    command::sanitize(argv, executable)
}

pub(crate) fn verify_identity(process: &Process) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::verify_identity(process)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = process;
        false
    }
}

pub(crate) fn validate_extension_command(
    mut value: crate::model::ProcessCommand,
    executable: Option<&std::path::Path>,
) -> Result<crate::model::ProcessCommand> {
    if let Some(argv) = value.argv.take() {
        if !value.unavailable.is_empty() {
            bail!("extension command mixes arguments and unavailable diagnostics");
        }
        if argv.len() > 4096
            || argv.iter().any(|arg| arg.len() > 16_384)
            || argv.iter().map(String::len).sum::<usize>() > 1_048_576
        {
            bail!("extension command exceeds limits");
        }
        #[cfg(target_os = "macos")]
        {
            let mut sanitized = command::sanitize(argv, executable);
            sanitized.unavailable = value.unavailable;
            return Ok(sanitized);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (argv, executable);
            bail!("command details require macOS");
        }
    }
    if value.unavailable.is_empty() {
        bail!("extension command has neither arguments nor diagnostics");
    }
    value.entrypoint = None;
    value.redacted_indices.clear();
    Ok(value)
}

#[cfg(target_os = "macos")]
pub(crate) fn collect_with_context(
    context: &crate::extensions::Context<'_>,
    command_pids: &[u32],
) -> Result<Collection> {
    use std::time::Instant;
    let cancel = context.cancel;
    let interval = context.settings.interval("sampling.interval_ms");
    let initial_cpu = macos::host_cpu_ticks();
    let first = macos::scan(false, context, &[])?;
    let start = Instant::now();
    cancel.wait(interval)?;
    let final_cpu = macos::host_cpu_ticks();
    let mut second = macos::scan(true, context, command_pids)?;
    let sample_ms = start.elapsed().as_millis() as u64;
    for (pid, observation) in &mut second {
        observation.process.cpu_percent = first.get(pid).and_then(|old| {
            if old.process.started_unix_us.is_none()
                || old.process.started_unix_us != observation.process.started_unix_us
            {
                return None;
            }
            cpu_percent(
                old.cpu_ticks?,
                observation.cpu_ticks?,
                observation.observed_ticks.checked_sub(old.observed_ticks)?,
            )
        });
        if observation.process.cpu_percent.is_none() {
            observation.process.unavailable.push(Diagnostic::new(
                "sample_unavailable",
                "cpu",
                "cpu: no valid pair of counters for this process identity",
            ));
        }
    }
    let system = SystemSummary {
        cpu_percent: initial_cpu
            .zip(final_cpu)
            .and_then(|(before, after)| host_cpu_percent(before, after)),
        logical_cpus: macos::logical_cpus(),
        memory_total_bytes: macos::total_memory(),
        memory_used_bytes: macos::used_memory(),
        swap_used_bytes: macos::swap_used(),
    };
    let mut warnings: Vec<Diagnostic> = COLLECTION_NOTES
        .into_iter()
        .map(|note| Diagnostic::new("collection_note", "collection", note))
        .collect();
    for (operation, name, unavailable) in [
        ("host.cpu", "CPU", system.cpu_percent.is_none()),
        (
            "host.logical_cpus",
            "logical CPUs",
            system.logical_cpus.is_none(),
        ),
        (
            "host.memory_total",
            "total memory",
            system.memory_total_bytes.is_none(),
        ),
        (
            "host.memory_used",
            "used memory",
            system.memory_used_bytes.is_none(),
        ),
        ("host.swap", "swap", system.swap_used_bytes.is_none()),
    ] {
        if unavailable {
            warnings.push(Diagnostic::new(
                "unavailable",
                operation,
                format!("Host {name}: OS query unavailable or sample incomplete."),
            ));
        }
    }
    Ok(Collection {
        processes: second.into_values().map(|value| value.process).collect(),
        system,
        sample_ms,
        warnings,
    })
}

fn host_cpu_percent(before: [u32; 4], after: [u32; 4]) -> Option<f64> {
    let ticks: Vec<u64> = after
        .iter()
        .zip(before)
        .map(|(new, old)| u64::from(new.wrapping_sub(old)))
        .collect();
    let total: u64 = ticks.iter().sum();
    (total > 0).then(|| (total - ticks[2]) as f64 * 100.0 / total as f64)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn collect_with_context(
    _context: &crate::extensions::Context<'_>,
    _command_pids: &[u32],
) -> Result<Collection> {
    bail!("resource collection currently supports macOS only")
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        io,
        mem::{MaybeUninit, size_of},
        path::PathBuf,
        ptr,
        time::{SystemTime, UNIX_EPOCH},
    };

    pub(super) struct Observation {
        pub process: Process,
        pub cpu_ticks: Option<u64>,
        pub observed_ticks: u64,
    }

    unsafe fn info<T>(pid: i32, flavor: i32) -> io::Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                flavor,
                0,
                value.as_mut_ptr().cast(),
                size_of::<T>() as i32,
            )
        };
        if bytes == size_of::<T>() as i32 {
            Ok(unsafe { value.assume_init() })
        } else if bytes <= 0 {
            Err(io::Error::last_os_error())
        } else {
            Err(io::Error::other("incomplete process information"))
        }
    }

    fn bsd_info(pid: i32) -> io::Result<libc::proc_bsdinfo> {
        unsafe { info(pid, libc::PROC_PIDTBSDINFO) }
    }

    fn text(bytes: impl IntoIterator<Item = libc::c_char>) -> String {
        let bytes: Vec<u8> = bytes
            .into_iter()
            .take_while(|byte| *byte != 0)
            .map(|byte| byte as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn started(info: &libc::proc_bsdinfo) -> u64 {
        info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec
    }

    fn pids() -> Result<Vec<i32>> {
        let count = unsafe { libc::proc_listallpids(ptr::null_mut(), 0) };
        if count <= 0 {
            bail!("cannot enumerate processes: {}", io::Error::last_os_error());
        }
        let mut capacity = count as usize + 128;
        for _ in 0..3 {
            let mut pids = vec![0i32; capacity];
            let count = unsafe {
                libc::proc_listallpids(
                    pids.as_mut_ptr().cast(),
                    (pids.len() * size_of::<i32>()) as i32,
                )
            };
            if count <= 0 {
                bail!("cannot enumerate processes: {}", io::Error::last_os_error());
            }
            if (count as usize) < pids.len() {
                pids.truncate(count as usize);
                pids.retain(|pid| *pid > 0);
                pids.sort_unstable();
                pids.dedup();
                return Ok(pids);
            }
            capacity *= 2;
        }
        bail!("process inventory changed too quickly; retry collection")
    }

    fn executable(pid: i32) -> io::Result<PathBuf> {
        let mut buffer = vec![0i8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let count =
            unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PathBuf::from(text(buffer)))
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn pointer_width(pid: i32) -> Option<usize> {
        const PROC_FLAG_LP64: u32 = 0x10;
        Some(if bsd_info(pid).ok()?.pbi_flags & PROC_FLAG_LP64 != 0 {
            8
        } else {
            4
        })
    }

    pub(super) fn verify_identity(process: &Process) -> bool {
        bsd_info(process.pid as i32).is_ok_and(|identity| {
            Some(started(&identity)) == process.started_unix_us
                && Some(identity.pbi_uid) == process.uid
                && executable(process.pid as i32).ok() == process.executable
        })
    }

    pub(super) fn scan(
        details: bool,
        context: &crate::extensions::Context<'_>,
        command_pids: &[u32],
    ) -> Result<HashMap<u32, Observation>> {
        let cancel = context.cancel;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before Unix epoch")?
            .as_secs();
        let mut observations = HashMap::new();
        for pid in pids()? {
            cancel.check()?;
            let mut process = Process {
                pid: pid as u32,
                name: format!("process {pid}"),
                ..Process::default()
            };
            let initial = bsd_info(pid);
            if let Ok(info) = &initial {
                process.ppid = Some(info.pbi_ppid);
                process.uid = Some(info.pbi_uid);
                process.started_unix_us = Some(started(info));
                process.uptime_seconds = now.checked_sub(info.pbi_start_tvsec);
                process.name = text(info.pbi_name);
                if process.name.is_empty() {
                    process.name = text(info.pbi_comm);
                }
            } else if let Err(error) = &initial {
                process.unavailable.push(Diagnostic::io("identity", error));
            }
            let task: io::Result<libc::proc_taskinfo> =
                unsafe { info(pid, libc::PROC_PIDTASKINFO) };
            let observed_ticks = unsafe { mach2::mach_time::mach_absolute_time() };
            let mut cpu_ticks = None;
            match task {
                Ok(info) => {
                    process.resident_bytes = Some(info.pti_resident_size);
                    cpu_ticks = info.pti_total_user.checked_add(info.pti_total_system);
                }
                Err(error) => process
                    .unavailable
                    .push(Diagnostic::io("task_counters", &error)),
            }
            if details {
                if let Ok(identity) = &initial {
                    match network::collect(pid, identity.pbi_nfiles as usize) {
                        Ok(network) => process.network = Some(network),
                        Err(error) => process.unavailable.push(Diagnostic::io("network", &error)),
                    }
                }
                match executable(pid) {
                    Ok(path) => process.executable = Some(path),
                    Err(error) => process
                        .unavailable
                        .push(Diagnostic::io("executable", &error)),
                }
                let cwd: io::Result<libc::proc_vnodepathinfo> =
                    unsafe { info(pid, libc::PROC_PIDVNODEPATHINFO) };
                match cwd {
                    Ok(info) => {
                        let path = text(info.pvi_cdir.vip_path.into_iter().flatten());
                        if path.is_empty() {
                            process.unavailable.push(Diagnostic::new(
                                "unavailable",
                                "cwd",
                                "cwd: unavailable",
                            ));
                        } else {
                            process.cwd = Some(PathBuf::from(path));
                        }
                    }
                    Err(error) => process.unavailable.push(Diagnostic::io("cwd", &error)),
                }
            }
            let command_requested = command_pids.contains(&(pid as u32));
            if command_requested {
                process.command = Some(if let Ok(identity) = &initial {
                    const PROC_FLAG_LP64: u32 = 0x10;
                    let pointer_width = if identity.pbi_flags & PROC_FLAG_LP64 != 0 {
                        8
                    } else {
                        4
                    };
                    crate::extensions::details(
                        &crate::core::Core::Command,
                        context,
                        &process,
                        pointer_width,
                    )
                    .unwrap_or_else(|_| {
                        command::unavailable(Diagnostic::new(
                            "command_failed",
                            "extensions.command.details",
                            "Command extension unavailable",
                        ))
                    })
                } else {
                    command::unavailable(Diagnostic::new(
                        "unavailable",
                        "command",
                        "command: process identity unavailable",
                    ))
                });
            }
            let final_identity = bsd_info(pid);
            let consistent = initial
                .as_ref()
                .ok()
                .zip(final_identity.as_ref().ok())
                .is_some_and(|(before, after)| started(before) == started(after));
            let command_consistent = !command_requested
                || (consistent
                    && initial
                        .as_ref()
                        .ok()
                        .zip(final_identity.as_ref().ok())
                        .is_some_and(|(before, after)| before.pbi_uid == after.pbi_uid)
                    && process.executable.is_some()
                    && executable(pid).ok() == process.executable);
            if !consistent {
                process = Process {
                    pid: pid as u32,
                    name: format!("process {pid}"),
                    unavailable: vec![match (initial, final_identity) {
                        (Err(error), _) => Diagnostic::io("identity.before", &error),
                        (_, Err(error)) => Diagnostic::io("identity.after", &error),
                        _ => Diagnostic::new(
                            "identity_changed",
                            "identity",
                            "identity changed during observation",
                        ),
                    }],
                    ..Process::default()
                };
                cpu_ticks = None;
            }
            command::discard_if_changed(&mut process, command_requested, command_consistent);
            observations.insert(
                pid as u32,
                Observation {
                    process,
                    cpu_ticks,
                    observed_ticks,
                },
            );
        }
        Ok(observations)
    }

    pub(super) fn swap_used() -> Option<u64> {
        let mut value = MaybeUninit::<libc::xsw_usage>::zeroed();
        let mut size = size_of::<libc::xsw_usage>();
        let status = unsafe {
            libc::sysctlbyname(
                c"vm.swapusage".as_ptr(),
                value.as_mut_ptr().cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == size_of::<libc::xsw_usage>())
            .then(|| unsafe { value.assume_init() }.xsu_used)
    }

    pub(super) fn host_cpu_ticks() -> Option<[u32; 4]> {
        let mut value = MaybeUninit::<libc::host_cpu_load_info>::zeroed();
        let mut count = (size_of::<libc::host_cpu_load_info>() / size_of::<i32>()) as u32;
        let host = unsafe { mach2::mach_init::mach_host_self() };
        let status = unsafe {
            libc::host_statistics(
                host,
                libc::HOST_CPU_LOAD_INFO,
                value.as_mut_ptr().cast(),
                &mut count,
            )
        };
        unsafe { mach2::mach_port::mach_port_deallocate(mach2::traps::mach_task_self(), host) };
        (status == 0 && count as usize == size_of::<libc::host_cpu_load_info>() / size_of::<i32>())
            .then(|| unsafe { value.assume_init() }.cpu_ticks)
    }

    pub(super) fn used_memory() -> Option<u64> {
        let mut value = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let host = unsafe { mach2::mach_init::mach_host_self() };
        let status = unsafe {
            libc::host_statistics64(
                host,
                libc::HOST_VM_INFO64,
                value.as_mut_ptr().cast(),
                &mut count,
            )
        };
        unsafe { mach2::mach_port::mach_port_deallocate(mach2::traps::mach_task_self(), host) };
        let required_words = (std::mem::offset_of!(libc::vm_statistics64, internal_page_count)
            + size_of::<u32>())
            / size_of::<i32>();
        if status != 0 || (count as usize) < required_words || page_size <= 0 {
            return None;
        }
        let value = unsafe { value.assume_init() };
        Some(
            u64::from(value.internal_page_count)
                .saturating_sub(u64::from(value.purgeable_count))
                .saturating_add(u64::from(value.wire_count))
                .saturating_add(u64::from(value.compressor_page_count))
                * page_size as u64,
        )
    }

    pub(super) fn total_memory() -> Option<u64> {
        let mut value = 0u64;
        let mut size = size_of::<u64>();
        let status = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut value as *mut u64).cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == size_of::<u64>() && value > 0).then_some(value)
    }

    pub(super) fn logical_cpus() -> Option<usize> {
        std::thread::available_parallelism()
            .ok()
            .map(|value| value.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static VERIFIED: std::cell::RefCell<std::collections::VecDeque<bool>> = Default::default();
        static FILLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn verify(_: &Process) -> bool {
        VERIFIED.with(|queue| queue.borrow_mut().pop_front().unwrap_or(false))
    }

    fn fill(_: i32, buffer: &mut [u8]) -> Option<usize> {
        FILLS.with(|count| count.set(count.get() + 1));
        let mut bytes = 3i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/opt/node\0");
        while !(bytes.len() - 4).is_multiple_of(8) {
            bytes.push(0);
        }
        bytes.extend_from_slice(b"node\0app.js\0--secret\0");
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Some(bytes.len())
    }

    #[test]
    fn argv_prefix_rechecks_identity_around_a_single_shared_read() {
        let process = Process {
            pid: 42,
            ..Process::default()
        };
        let mut reader = ArgvPrefix::with(verify, || Some(4096), fill, |_| Some(8));
        for (sequence, expected, fills) in [
            (vec![false], None, 0),
            (vec![true, false], None, 1),
            (
                vec![true, true],
                Some(vec!["node".to_owned(), "app.js".to_owned()]),
                1,
            ),
        ] {
            VERIFIED.with(|queue| *queue.borrow_mut() = sequence.into());
            FILLS.with(|count| count.set(0));
            assert_eq!(reader.read("node", &process, 2), expected);
            assert_eq!(FILLS.with(std::cell::Cell::get), fills);
        }
        let cost = &reader.costs["node"];
        assert_eq!((cost.candidates, cost.reads), (3, 2));
        assert!(cost.bytes > 0 && cost.duration_ms >= 0.0);
        let closed = &mut ArgvPrefix::default();
        FILLS.with(|count| count.set(0));
        assert_eq!(closed.read("node", &process, 2), None);
        assert_eq!(closed.costs["node"].reads, 0);
    }

    #[test]
    fn host_cpu_accounts_for_idle_and_counter_wraparound() {
        assert_eq!(host_cpu_percent([0, 0, 0, 0], [20, 10, 70, 0]), Some(30.0));
        assert_eq!(host_cpu_percent([0; 4], [0; 4]), None);
        assert_eq!(
            host_cpu_percent([u32::MAX, 0, 0, 0], [0, 0, 3, 0]),
            Some(25.0)
        );
    }

    #[test]
    fn cpu_zero_is_known_but_reset_or_missing_interval_is_not() {
        assert_eq!(cpu_percent(10, 10, 100), Some(0.0));
        assert_eq!(cpu_percent(10, 60, 100), Some(50.0));
        assert_eq!(cpu_percent(10, 210, 100), Some(200.0));
        assert_eq!(cpu_percent(10, 9, 100), None);
        assert_eq!(cpu_percent(10, 20, 0), None);
    }
}
