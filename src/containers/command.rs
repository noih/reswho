use crate::model::FailureReason;
use anyhow::Result;
use std::time::{Duration, Instant};

mod bounded;
#[cfg(test)]
use bounded::{MAX_OUTPUT_BYTES, drain};
pub(crate) use bounded::{Output, fail, reason_of, run_bounded};

#[cfg(target_os = "macos")]
pub(super) fn find_program(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

pub(crate) fn probe_endpoint(host: Option<&str>) -> Option<(FailureReason, Option<i32>)> {
    let path = host?
        .strip_prefix("unix://")
        .filter(|path| std::path::Path::new(path).is_absolute())?;
    let error = match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => return None,
        Err(error) => error,
    };
    let reason = match error.raw_os_error() {
        Some(libc::ENOENT) => FailureReason::EndpointNotFound,
        Some(libc::EACCES | libc::EPERM) => FailureReason::PermissionDenied,
        Some(libc::ECONNREFUSED) => FailureReason::ConnectionRefused,
        Some(libc::ETIMEDOUT) => FailureReason::Timeout,
        _ => FailureReason::Unknown,
    };
    Some((reason, error.raw_os_error()))
}

type Probed = Option<(FailureReason, Option<i32>)>;

pub(crate) struct Probe {
    limits: Option<(crate::snapshot::Cancellation, Instant)>,
    seen: Vec<(String, Probed)>,
    pub probes: usize,
}

impl Probe {
    pub(super) fn new(runner: &impl Runner) -> Self {
        Self {
            limits: runner.limits(),
            seen: Vec::new(),
            probes: 0,
        }
    }

    pub fn with_limits(cancel: &crate::snapshot::Cancellation, deadline: Instant) -> Self {
        Self {
            limits: Some((cancel.clone(), deadline)),
            seen: Vec::new(),
            probes: 0,
        }
    }

    pub fn canceled(&self) -> bool {
        self.limits
            .as_ref()
            .is_some_and(|(cancel, _)| cancel.check().is_err())
    }

    pub fn expired(&self) -> bool {
        self.limits
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
    }

    fn endpoint(&mut self, host: Option<&str>) -> Probed {
        let host = host?;
        if let Some((_, result)) = self.seen.iter().find(|(seen, _)| seen == host) {
            return *result;
        }
        if self.canceled() || self.expired() {
            return None;
        }
        self.probes += 1;
        let result = probe_endpoint(Some(host));
        self.seen.push((host.to_owned(), result));
        result
    }

    pub fn refine(&mut self, diagnostic: &mut crate::model::Diagnostic, host: Option<&str>) {
        if diagnostic.reason == Some(FailureReason::Unknown)
            && let Some((reason, os_error)) = self.endpoint(host)
        {
            diagnostic.refine(reason, os_error);
        }
    }

    pub fn diagnose(
        &mut self,
        code: &'static str,
        operation: &str,
        error: &anyhow::Error,
        host: Option<&str>,
    ) -> crate::model::Diagnostic {
        let reason = if self.canceled() {
            FailureReason::Canceled
        } else {
            reason_of(error)
        };
        let mut diagnostic =
            crate::model::Diagnostic::failure(code, operation, format!("{error:#}"), reason, None);
        self.refine(&mut diagnostic, host);
        diagnostic
    }
}

pub(super) trait Runner {
    fn run(&mut self, args: &[String]) -> Result<Output>;

    fn limits(&self) -> Option<(crate::snapshot::Cancellation, Instant)> {
        None
    }

    fn direct_stats(
        &mut self,
        _host: &str,
        _ids: &[String],
    ) -> Result<Option<super::stats::Batch>> {
        Ok(None)
    }
}

pub(super) struct RuntimeCommand {
    pub cancel: crate::snapshot::Cancellation,
    pub deadline: Instant,
    pub program: std::path::PathBuf,
    pub sample_interval: Duration,
    pub api_version: Option<String>,
}

impl Runner for RuntimeCommand {
    fn limits(&self) -> Option<(crate::snapshot::Cancellation, Instant)> {
        Some((self.cancel.clone(), self.deadline))
    }

    fn direct_stats(&mut self, host: &str, ids: &[String]) -> Result<Option<super::stats::Batch>> {
        let Some(socket) = host
            .strip_prefix("unix://")
            .filter(|path| std::path::Path::new(path).is_absolute())
        else {
            return Ok(None);
        };
        let client = super::http::Client {
            socket: std::path::Path::new(socket),
            cancel: &self.cancel,
            deadline: self.deadline,
        };
        super::stats::collect(
            &client,
            ids,
            self.sample_interval,
            self.api_version.as_deref(),
        )
    }

    fn run(&mut self, args: &[String]) -> Result<Output> {
        run_bounded(&self.program, args, None, &self.cancel, self.deadline)
    }
}

#[cfg(test)]
pub(crate) mod failure_fixtures {
    use std::{
        os::unix::{fs::PermissionsExt, net::UnixListener},
        path::{Path, PathBuf},
    };

    pub(crate) struct Endpoints {
        pub directory: tempfile::TempDir,
        _denied: UnixListener,
    }

    impl Endpoints {
        pub fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let denied = UnixListener::bind(directory.path().join("denied.sock")).unwrap();
            std::fs::set_permissions(
                directory.path().join("denied.sock"),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();
            drop(UnixListener::bind(directory.path().join("refused.sock")).unwrap());
            Self {
                directory,
                _denied: denied,
            }
        }

        pub fn host(&self, name: &str) -> String {
            format!("unix://{}", self.directory.path().join(name).display())
        }

        pub fn cases(&self) -> [(String, crate::model::FailureReason); 3] {
            use crate::model::FailureReason::*;
            [
                (self.host("missing.sock"), EndpointNotFound),
                (self.host("denied.sock"), PermissionDenied),
                (self.host("refused.sock"), ConnectionRefused),
            ]
        }
    }

    pub(crate) fn cli(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn endpoint_probe_classifies_socket_state_without_reading_stderr() {
        let endpoints = failure_fixtures::Endpoints::new();
        for (host, expected) in endpoints.cases() {
            let (reason, errno) = probe_endpoint(Some(&host)).unwrap();
            assert_eq!(reason, expected, "{host}");
            assert!(errno.is_some());
        }
        let live =
            std::os::unix::net::UnixListener::bind(endpoints.directory.path().join("live.sock"))
                .unwrap();
        assert!(probe_endpoint(Some(&endpoints.host("live.sock"))).is_none());
        drop(live);
        assert!(probe_endpoint(Some("ssh://user@remote")).is_none());
        assert!(probe_endpoint(Some("unix://relative.sock")).is_none());
        assert!(probe_endpoint(None).is_none());
        let mut probe = Probe {
            limits: None,
            seen: Vec::new(),
            probes: 0,
        };
        let failure = probe.diagnose(
            "collection_failed",
            "docker",
            &anyhow::anyhow!("info failed"),
            Some(&endpoints.host("denied.sock")),
        );
        assert_eq!(failure.reason, Some(FailureReason::PermissionDenied));
        assert!(failure.message.contains("reason permission_denied: "));
        assert!(failure.message.contains("allow socket access"));
        assert_eq!(
            probe
                .diagnose(
                    "unavailable",
                    "x",
                    &fail(FailureReason::Timeout, "slow"),
                    None
                )
                .reason,
            Some(FailureReason::Timeout)
        );
        endpoints.directory.close().unwrap();
    }

    #[test]
    fn probe_runs_once_per_endpoint_and_respects_cancel_and_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let host = format!("unix://{}", directory.path().join("missing.sock").display());
        let unknown = || anyhow::anyhow!("info failed");
        let cancel = crate::snapshot::Cancellation::default();
        let mut probe = Probe {
            limits: Some((cancel.clone(), Instant::now() + Duration::from_secs(5))),
            seen: Vec::new(),
            probes: 0,
        };
        for _ in 0..3 {
            let diagnostic =
                probe.diagnose("command_failed", "docker.stats", &unknown(), Some(&host));
            assert_eq!(diagnostic.reason, Some(FailureReason::EndpointNotFound));
            assert_eq!(diagnostic.os_error, Some(libc::ENOENT));
            assert!(
                diagnostic
                    .message
                    .ends_with(FailureReason::EndpointNotFound.hint())
            );
            assert!(!diagnostic.message.contains("reason unknown"));
        }
        assert_eq!(probe.probes, 1);
        let mut expired = Probe {
            limits: Some((cancel.clone(), Instant::now())),
            seen: Vec::new(),
            probes: 0,
        };
        let diagnostic = expired.diagnose("collection_failed", "docker", &unknown(), Some(&host));
        assert_eq!(
            (diagnostic.reason, expired.probes),
            (Some(FailureReason::Unknown), 0)
        );
        cancel.cancel();
        let mut canceled = Probe {
            limits: Some((cancel, Instant::now() + Duration::from_secs(5))),
            seen: Vec::new(),
            probes: 0,
        };
        let diagnostic = canceled.diagnose("collection_failed", "docker", &unknown(), Some(&host));
        assert_eq!(
            (diagnostic.reason, canceled.probes),
            (Some(FailureReason::Canceled), 0)
        );
        assert!(diagnostic.message.contains("reason canceled: "));
        directory.close().unwrap();
    }

    #[test]
    fn runner_reports_missing_cli_and_deadline_as_typed_reasons() {
        let directory = tempfile::tempdir().unwrap();
        let error = run_bounded(
            &directory.path().join("missing-cli"),
            &[],
            None,
            &crate::snapshot::Cancellation::default(),
            Instant::now() + Duration::from_secs(2),
        )
        .err()
        .unwrap();
        assert_eq!(reason_of(&error), FailureReason::CliNotFound);
        let output = run_bounded(
            &failure_fixtures::cli(directory.path(), "slow", "exec /bin/sleep 5"),
            &[],
            None,
            &crate::snapshot::Cancellation::default(),
            Instant::now() + Duration::from_millis(150),
        )
        .unwrap();
        assert!(output.timed_out);
        assert_eq!(reason_of(&output.failure("x")), FailureReason::Timeout);
        let output = run_bounded(
            &failure_fixtures::cli(
                directory.path(),
                "fails",
                "echo private-credential >&2; exit 1",
            ),
            &[],
            None,
            &crate::snapshot::Cancellation::default(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        assert!(!output.timed_out);
        assert_eq!(reason_of(&output.failure("x")), FailureReason::Unknown);
        directory.close().unwrap();
    }

    #[test]
    fn guarded_host_fixture() {
        let Ok(root) = std::env::var("RESWHO_GUARD_TEST_ROOT") else {
            return;
        };
        let mode = std::env::var("RESWHO_GUARD_TEST_MODE").unwrap();
        std::thread::scope(|scope| {
            for index in 0..4 {
                let root = &root;
                let mode = &mode;
                scope.spawn(move || {
                    std::fs::write(
                        std::path::Path::new(root).join(format!("{index}.worker")),
                        "starting",
                    )
                    .unwrap();
                    let result = bounded::run_observed(
                        &std::path::Path::new(root).join("provider"),
                        &[root.clone(), index.to_string(), mode.clone()],
                        Some(b"{}"),
                        &crate::snapshot::Cancellation::default(),
                        Instant::now() + Duration::from_secs(5),
                        |pid| {
                            let identity =
                                libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(
                                    pid as i32, 0,
                                )
                                .map_err(|error| anyhow::anyhow!("guard identity: {error}"))?;
                            let temporary =
                                std::path::Path::new(root).join(format!("{index}.guard.tmp"));
                            std::fs::write(
                                &temporary,
                                format!(
                                    "{pid} {} {}",
                                    identity.pbi_start_tvsec, identity.pbi_start_tvusec
                                ),
                            )?;
                            std::fs::rename(
                                temporary,
                                std::path::Path::new(root).join(format!("{index}.guard")),
                            )?;
                            Ok(())
                        },
                    );
                    let diagnostic = match &result {
                        Ok(output) => format!(
                            "completed timed_out={} exit_code={:?} issue={:?}",
                            output.timed_out, output.exit_code, output.issue
                        ),
                        Err(error) => format!("failed {error:#}"),
                    };
                    std::fs::write(
                        std::path::Path::new(root).join(format!("{index}.worker")),
                        diagnostic,
                    )
                    .unwrap();
                    if mode == "return" {
                        assert!(result.unwrap().issue.is_none());
                    }
                });
            }
        });
        std::fs::write(std::path::Path::new(&root).join("returned"), "").unwrap();
        std::thread::sleep(Duration::from_secs(10));
    }

    struct FixtureHost(std::process::Child);

    impl Drop for FixtureHost {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    fn fixture_identity_alive(identity: (i32, u64, u64)) -> bool {
        match libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(identity.0, 0) {
            Ok(info) => (info.pbi_start_tvsec, info.pbi_start_tvusec) == (identity.1, identity.2),
            Err(error) => {
                let exists = unsafe { libc::kill(identity.0, 0) };
                let os_error = std::io::Error::last_os_error().raw_os_error();
                assert!(
                    exists == -1 && os_error == Some(libc::ESRCH),
                    "owned identity unavailable for {}: {error}, probe={exists}/{os_error:?}",
                    identity.0
                );
                false
            }
        }
    }

    #[test]
    fn host_death_reclaims_multiple_providers_descendants_and_guards() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("provider.c");
        std::fs::write(
            &source,
            r#"
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <libproc.h>
#include <sys/proc_info.h>
int publish(const char *root, const char *index, const char *suffix, int *pids, int count) {
    char path[4096], temporary[4096];
    snprintf(path, sizeof(path), "%s/%s.%s", root, index, suffix);
    snprintf(temporary, sizeof(temporary), "%s/%s.%s.tmp", root, index, suffix);
    FILE *f = fopen(temporary, "w");
    if (!f) return -1;
    for (int i = 0; i < count; ++i) {
        struct proc_bsdinfo info;
        if (proc_pidinfo(pids[i], PROC_PIDTBSDINFO, 0, &info, sizeof(info)) != sizeof(info)) { fclose(f); return -1; }
        fprintf(f, "%d %llu %llu\n", pids[i], (unsigned long long)info.pbi_start_tvsec, (unsigned long long)info.pbi_start_tvusec);
    }
    if (fclose(f) != 0) return -1;
    return rename(temporary, path);
}
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    int pids[3] = {getpid(), getppid(), 0};
    if (publish(argv[1], argv[2], "started", pids, 2) != 0) return 3;
    int gate[2];
    if (pipe(gate) != 0) return 4;
    int descendant = fork();
    if (descendant < 0) return 5;
    if (descendant == 0) {
        close(gate[1]);
        char ready;
        if (read(gate[0], &ready, 1) != 1) return 6;
        close(gate[0]); close(0); close(1); close(2); sleep(30); return 0;
    }
    close(gate[0]);
    pids[2] = pids[1]; pids[1] = descendant;
    if (publish(argv[1], argv[2], "owned", pids, 3) != 0) { close(gate[1]); return 9; }
    if (strcmp(argv[3], "unready") == 0) { sleep(30); close(gate[1]); return 0; }
    if (publish(argv[1], argv[2], "pid", pids, 3) != 0) { close(gate[1]); return 7; }
    if (write(gate[1], "r", 1) != 1) { close(gate[1]); return 8; }
    close(gate[1]);
    if (strcmp(argv[3], "return") == 0) return 0;
    sleep(30);
    return 0;
}
"#,
        )
        .unwrap();
        assert!(
            Command::new("cc")
                .arg(&source)
                .arg("-o")
                .arg(dir.path().join("provider"))
                .status()
                .unwrap()
                .success()
        );
        for (mode, signal) in [
            ("launch", libc::SIGKILL),
            ("running", libc::SIGINT),
            ("running", libc::SIGTERM),
            ("running", libc::SIGABRT),
            ("running", libc::SIGKILL),
            ("return", libc::SIGKILL),
            ("unready", libc::SIGKILL),
        ] {
            let stderr_path = dir.path().join("host.stderr");
            let mut host = FixtureHost(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "containers::command::tests::guarded_host_fixture",
                        "--nocapture",
                    ])
                    .env("RESWHO_GUARD_TEST_ROOT", dir.path())
                    .env("RESWHO_GUARD_TEST_MODE", mode)
                    .stdout(Stdio::null())
                    .stderr(std::fs::File::create(&stderr_path).unwrap())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            let required = if mode == "launch" { 1 } else { 4 };
            while (0..required).any(|index| !dir.path().join(format!("{index}.pid")).exists())
                && start.elapsed() < Duration::from_secs(3)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            if mode == "return" {
                while !dir.path().join("returned").exists()
                    && start.elapsed() < Duration::from_secs(3)
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            let returned = dir.path().join("returned").exists();
            let launched =
                (0..required).all(|index| dir.path().join(format!("{index}.pid")).exists());
            let early_status = host.0.try_wait().unwrap();
            if early_status.is_none() {
                assert_eq!(unsafe { libc::kill(host.0.id() as i32, signal) }, 0);
            }
            let status = host.0.wait().unwrap();
            let mut diagnostic = format!(
                "mode={mode} signal={signal} host={} early_status={early_status:?} status={status} launched={launched} returned={returned}",
                host.0.id()
            );
            let mut identities = std::collections::BTreeSet::new();
            let mut owned_providers = 0;
            for index in 0..4 {
                for suffix in [
                    "worker",
                    "guard",
                    "guard.tmp",
                    "started",
                    "started.tmp",
                    "pid.tmp",
                    "owned",
                    "owned.tmp",
                ] {
                    let path = dir.path().join(format!("{index}.{suffix}"));
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        diagnostic.push_str(&format!("\nindex={index} {suffix}: {text}"));
                        if suffix == "owned" {
                            owned_providers += 1;
                        }
                        if suffix != "worker" {
                            for line in text.lines() {
                                let fields: Vec<_> = line.split_whitespace().collect();
                                if fields.len() == 3 {
                                    identities.insert((
                                        fields[0].parse::<i32>().unwrap(),
                                        fields[1].parse::<u64>().unwrap(),
                                        fields[2].parse::<u64>().unwrap(),
                                    ));
                                }
                            }
                        }
                        std::fs::remove_file(path).unwrap();
                    }
                }
            }
            diagnostic.push_str(&format!(
                "\nstderr: {}",
                std::fs::read_to_string(&stderr_path).unwrap()
            ));
            println!("{diagnostic}");
            let mut pids = Vec::new();
            for index in 0..4 {
                let path = dir.path().join(format!("{index}.pid"));
                if let Ok(text) = std::fs::read_to_string(&path) {
                    for line in text.lines() {
                        let fields: Vec<_> = line.split_whitespace().collect();
                        pids.push((
                            fields[0].parse::<i32>().unwrap(),
                            fields[1].parse::<u64>().unwrap(),
                            fields[2].parse::<u64>().unwrap(),
                        ));
                    }
                    std::fs::remove_file(path).unwrap();
                }
            }
            identities.extend(pids.iter().copied());
            let start = Instant::now();
            while identities.iter().copied().any(fixture_identity_alive)
                && start.elapsed() < Duration::from_secs(3)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            let survivors: Vec<_> = identities
                .iter()
                .copied()
                .filter(|identity| fixture_identity_alive(*identity))
                .collect();
            for identity in &survivors {
                if fixture_identity_alive(*identity) {
                    unsafe {
                        libc::kill(identity.0, libc::SIGKILL);
                    }
                }
            }
            if !survivors.is_empty() {
                let cleanup = Instant::now();
                while survivors.iter().copied().any(fixture_identity_alive)
                    && cleanup.elapsed() < Duration::from_secs(3)
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            if mode == "unready" {
                assert!(
                    pids.is_empty() && !launched,
                    "negative readiness fixture published a marker: {diagnostic}"
                );
                assert_eq!(
                    owned_providers, 4,
                    "negative readiness fixture did not retain all owned identities: {diagnostic}"
                );
            } else {
                assert!(
                    !pids.is_empty(),
                    "fixture did not launch: {diagnostic}; tracked guards/providers={identities:?}"
                );
            }
            assert!(
                survivors.is_empty(),
                "owned identities survived {mode}/{signal}: {survivors:?}; {diagnostic}"
            );
            assert!(
                mode == "unready" || launched,
                "required providers did not launch: {diagnostic}"
            );
            assert!(
                mode != "return" || returned,
                "host did not complete provider return"
            );
            let _ = std::fs::remove_file(dir.path().join("returned"));
        }
        dir.close().unwrap();
    }

    #[test]
    #[ignore]
    fn measure_guard_overhead() {
        use libproc::libproc::pid_rusage::{RUsageInfoV0, pidrusage};
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("noop.c");
        let pid_file = dir.path().join("parent");
        std::fs::write(&source, "#include <stdio.h>\n#include <unistd.h>\nint main(int n,char **v){if(n!=2)return 2; FILE *f=fopen(v[1],\"w\");fprintf(f,\"%d\",getppid());fclose(f);usleep(100000);return 0;}").unwrap();
        let program = dir.path().join("noop");
        assert!(
            Command::new("cc")
                .arg(&source)
                .arg("-o")
                .arg(&program)
                .status()
                .unwrap()
                .success()
        );
        let budget = Instant::now();
        for guarded in [false, true] {
            for round in 0..3 {
                let start = Instant::now();
                std::thread::scope(|scope| {
                    let handle = scope.spawn(|| {
                        run_bounded(
                            &program,
                            &[pid_file.to_string_lossy().into_owned()],
                            guarded.then_some(b"".as_slice()),
                            &crate::snapshot::Cancellation::default(),
                            Instant::now() + Duration::from_secs(2),
                        )
                    });
                    let pid = loop {
                        if let Ok(text) = std::fs::read_to_string(&pid_file)
                            && let Ok(pid) = text.parse::<i32>()
                        {
                            break pid;
                        }
                        assert!(start.elapsed() < Duration::from_secs(2));
                        std::thread::sleep(Duration::from_millis(1));
                    };
                    assert_eq!(pid == std::process::id() as i32, !guarded);
                    let rss = if guarded {
                        Some(pidrusage::<RUsageInfoV0>(pid).unwrap().ri_resident_size)
                    } else {
                        None
                    };
                    assert!(handle.join().unwrap().unwrap().issue.is_none());
                    println!(
                        "guarded={guarded} round={round} elapsed_ms={:.3} extra_processes={} guard_rss_bytes={rss:?}",
                        start.elapsed().as_secs_f64() * 1000.0,
                        usize::from(guarded)
                    );
                });
                std::fs::remove_file(&pid_file).unwrap();
            }
        }
        assert!(budget.elapsed() < Duration::from_secs(15));
        dir.close().unwrap();
    }

    #[test]
    fn guard_monitor_failure_and_exec_failure_are_closed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("failure.c");
        let guard = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/containers/guard.c");
        std::fs::write(
            &source,
            format!(
                r#"
#include <sys/event.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>
#include <assert.h>
int failing_kqueue(void) {{ return -1; }}
#define kqueue failing_kqueue
#include {:?}
int main(void) {{
    int life[2]; assert(socketpair(AF_UNIX, SOCK_STREAM, 0, life) == 0);
    int host = fork(); assert(host >= 0);
    if (host == 0) {{ close(life[0]); reswho_guard(life[1]); _exit(99); }}
    close(life[1]);
    int status; assert(waitpid(host, &status, 0) == host);
    assert(WIFEXITED(status) && WEXITSTATUS(status) == 125);
    close(life[0]);
    return 0;
}}
"#,
                guard
            ),
        )
        .unwrap();
        let program = dir.path().join("failure");
        assert!(
            Command::new("cc")
                .arg(&source)
                .arg("-o")
                .arg(&program)
                .status()
                .unwrap()
                .success()
        );
        assert!(Command::new(&program).status().unwrap().success());
        let result = run_bounded(
            &dir.path().join("missing"),
            &[],
            Some(b"{}"),
            &crate::snapshot::Cancellation::default(),
            Instant::now() + Duration::from_secs(2),
        );
        assert!(result.is_err() || result.unwrap().issue.is_some());
        let backpressure = run_bounded(
            std::path::Path::new("/bin/sleep"),
            &["30".into()],
            Some(&vec![0; MAX_OUTPUT_BYTES]),
            &crate::snapshot::Cancellation::default(),
            Instant::now() + Duration::from_millis(100),
        )
        .unwrap();
        assert!(backpressure.issue.unwrap().contains("deadline"));
        dir.close().unwrap();
    }

    #[test]
    fn cancellation_reaps_running_owned_command() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pid");
        let cancel = crate::snapshot::Cancellation::default();
        let mut runner = RuntimeCommand {
            cancel: cancel.clone(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let argument = path.to_string_lossy().into_owned();
        let handle = std::thread::spawn(move || {
            runner.run(&[
                "-c".into(),
                "echo $$ > \"$1\"; exec /bin/sleep 30".into(),
                "fixture".into(),
                argument,
            ])
        });
        let start = Instant::now();
        let pid = loop {
            if let Ok(value) = std::fs::read_to_string(&path)
                && let Ok(pid) = value.trim().parse::<i32>()
            {
                break Some(pid);
            }
            if start.elapsed() >= Duration::from_secs(2) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        cancel.cancel();
        let result = handle.join().unwrap();
        assert!(result.err().unwrap().to_string().contains("canceled"));
        let pid = pid.expect("owned command started");
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        directory.close().unwrap();
    }

    #[test]
    fn timeout_reaps_owned_child_and_preserves_partial_output() {
        let mut runner = RuntimeCommand {
            cancel: crate::snapshot::Cancellation::default(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
            deadline: Instant::now() + Duration::from_millis(200),
        };
        let start = Instant::now();
        let result = runner
            .run(&["-c".into(), "echo $$; exec /bin/sleep 30".into()])
            .unwrap();
        assert!(result.issue.unwrap().contains("deadline"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let pid: i32 = String::from_utf8(result.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn bounded_reader_and_failed_command_do_not_expose_stderr() {
        let mut output = vec![0; MAX_OUTPUT_BYTES];
        assert!(drain(&mut std::io::Cursor::new(b"extra"), &mut output).is_err());
        let mut runner = RuntimeCommand {
            cancel: crate::snapshot::Cancellation::default(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
            deadline: Instant::now() + Duration::from_secs(2),
        };
        let result = runner
            .run(&["-c".into(), "echo private-credential >&2; exit 7".into()])
            .unwrap();
        let issue = result.issue.unwrap();
        assert!(issue.contains('7'));
        assert!(!issue.contains("private-credential"));
        assert!(result.stdout.is_empty());
    }
    #[test]
    fn normal_and_nonzero_exit_clean_descendants_after_closed_pipes() {
        for code in [0, 7] {
            let result = run_bounded(std::path::Path::new("/bin/sh"), &[
                "-c".into(), "/bin/sleep 30 </dev/null >/dev/null 2>&1 & descendant=$!; printf '{\"child\":%s,\"parent\":%s}\\n' \"$descendant\" \"$$\"; exit \"$1\"".into(), "fixture".into(), code.to_string()
            ], Some(b"{}"), &crate::snapshot::Cancellation::default(), Instant::now()+Duration::from_secs(3)).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let parent = value["parent"].as_i64().unwrap() as i32;
            let descendant = value["child"].as_i64().unwrap() as i32;
            assert_eq!(unsafe { libc::kill(parent, 0) }, -1);
            let start = Instant::now();
            while unsafe { libc::kill(descendant, 0) } == 0
                && start.elapsed() < Duration::from_secs(2)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            let gone = unsafe { libc::kill(descendant, 0) } == -1;
            if !gone && unsafe { libc::getpgid(descendant) } == parent {
                unsafe {
                    libc::kill(-parent, libc::SIGKILL);
                }
            }
            assert!(gone, "owned descendant {descendant} survived provider exit");
            assert_eq!(result.issue.is_none(), code == 0);
        }
    }
}
