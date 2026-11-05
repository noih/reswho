#[path = "../src/model/diagnostic.rs"]
pub mod model;
#[path = "../src/snapshot/cancellation.rs"]
pub mod snapshot;
#[cfg(target_os = "macos")]
#[link(name = "reswho_guard", kind = "static")]
unsafe extern "C" {}
#[path = "../src/containers/command/bounded.rs"]
pub mod command;
#[path = "../src/containers/podman/command.rs"]
pub mod podman_command;
use podman_command::PodmanCommand;
use snapshot::Cancellation;
use std::time::{Duration, Instant};

const CASE: &str = "nested_native_cli_guard_reaps_on_host_death_timeout_and_cancel";

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match (
        std::env::var_os("RESWHO_PODMAN_GUARD_ROOT").is_some(),
        args.first().map(String::as_str),
    ) {
        (true, Some("host")) => nested_host_fixture(),
        (true, Some("provider")) => nested_provider_fixture(),
        _ => {
            let mut selected = true;
            let mut skip = false;
            let mut filter_seen = false;
            let exact = args.iter().any(|arg| arg == "--exact");
            for arg in &args {
                if skip {
                    selected &= if exact {
                        arg != CASE
                    } else {
                        !CASE.contains(arg)
                    };
                    skip = false;
                } else {
                    match arg.as_str() {
                        "--skip" => skip = true,
                        "--ignored" => selected = false,
                        "--nocapture" | "--show-output" | "--exact" | "--include-ignored"
                        | "--test-threads=1" | "--quiet" | "-q" | "--list" => {}
                        filter if !filter.starts_with('-') => {
                            assert!(!filter_seen, "podman_guard supports one test filter");
                            filter_seen = true;
                            selected &= if exact {
                                filter == CASE
                            } else {
                                CASE.contains(filter)
                            };
                        }
                        _ => panic!("unsupported podman_guard argument: {arg}"),
                    }
                }
            }
            assert!(!skip, "--skip needs a test name");
            if args.iter().any(|arg| arg == "--list") {
                if selected {
                    println!("{CASE}: test");
                }
            } else if selected {
                nested_native_cli_guard_reaps_on_host_death_timeout_and_cancel();
                println!("test {CASE} ... ok");
            }
        }
    }
}

struct OwnedHost(std::process::Child);

impl std::ops::Deref for OwnedHost {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for OwnedHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for OwnedHost {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn nested_provider_fixture() {
    let Ok(root) = std::env::var("RESWHO_PODMAN_GUARD_ROOT") else {
        return;
    };
    std::fs::write(
        std::path::Path::new(&root).join("provider.pid"),
        format!("{} {}", std::process::id(), unsafe { libc::getppid() }),
    )
    .unwrap();
    let cancel = Cancellation::default();
    let mut runner = PodmanCommand {
        program: std::path::Path::new(&root).join("cli"),
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(8),
    };
    let _ = runner.run(&[]);
}
fn stage_monotonic_ns() -> u128 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
        0
    );
    time.tv_sec as u128 * 1_000_000_000 + time.tv_nsec as u128
}

fn nested_host_fixture() {
    let entered = stage_monotonic_ns();
    let Some(root) = std::env::var_os("RESWHO_PODMAN_GUARD_ROOT") else {
        return;
    };
    let env_after = stage_monotonic_ns();
    use std::io::Write;
    let open_before = stage_monotonic_ns();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::path::Path::new(&root).join("host.stage"))
        .unwrap();
    let open_after = stage_monotonic_ns();
    for (name, time) in [
        ("fixture_first_statement", entered),
        ("env_after", env_after),
        ("stage_open_before", open_before),
        ("stage_open_after", open_after),
    ] {
        writeln!(file, "mono_ns={time} {name}").unwrap();
    }
    file.flush().unwrap();
    let mut stage = |name: &str, time: u128| {
        writeln!(
            file,
            "{} mono_ns={time} {name}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
        .unwrap();
        file.flush().unwrap();
    };
    stage("fixture_enter", stage_monotonic_ns());
    let timed = std::env::var_os("RESWHO_PODMAN_OUTER_TIMEOUT").is_some();
    stage("run_bounded_enter", stage_monotonic_ns());
    let program = std::env::current_exe().unwrap();
    let args = ["provider".into()];
    let cancel = Cancellation::default();
    stage("arguments_ready", stage_monotonic_ns());
    let deadline_before = stage_monotonic_ns();
    let deadline = Instant::now()
        + if timed {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(9)
        };
    let deadline_after = stage_monotonic_ns();
    stage("deadline_before", deadline_before);
    stage("deadline_after", deadline_after);
    stage("call_before", stage_monotonic_ns());
    let result = command::run_bounded(&program, &args, Some(b""), &cancel, deadline);
    stage("run_bounded_return", stage_monotonic_ns());
    if timed {
        assert!(result.unwrap().issue.unwrap().contains("deadline"));
    }
    stage("fixture_exit", stage_monotonic_ns());
}
fn nested_native_cli_guard_reaps_on_host_death_timeout_and_cancel() {
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("cli.c");
    std::fs::write(&source,r#"#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(void){const char *root=getenv("RESWHO_PODMAN_GUARD_ROOT");if(!root)return 2;int p=fork();if(p==0){close(0);close(1);close(2);sleep(30);return 0;}char f[4096],t[4096];snprintf(f,sizeof f,"%s/cli.pid",root);snprintf(t,sizeof t,"%s/cli.tmp",root);FILE *o=fopen(t,"w");fprintf(o,"%d %d %d",getpid(),getppid(),p);fclose(o);rename(t,f);sleep(30);return 0;}
"#).unwrap();
    let program = dir.path().join("cli");
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&program)
            .status()
            .unwrap()
            .success()
    );
    eprintln!("owned nested root: {}", dir.path().display());
    for timed in [false, true] {
        let _ = std::fs::remove_file(dir.path().join("host.stage"));
        let log = dir.path().join(format!("host-{timed}.log"));
        let log_file = std::fs::File::create(&log).unwrap();
        let mut host_command = Command::new(std::env::current_exe().unwrap());
        if timed {
            host_command.env("RESWHO_PODMAN_OUTER_TIMEOUT", "1");
        }
        host_command
            .arg("host")
            .env("RESWHO_PODMAN_GUARD_ROOT", dir.path())
            .stdout(Stdio::from(log_file.try_clone().unwrap()))
            .stderr(Stdio::from(log_file));
        let spawn_before = stage_monotonic_ns();
        let host = host_command.spawn().unwrap();
        let start = Instant::now();
        let mut host = OwnedHost(host);
        let spawn_after = stage_monotonic_ns();
        eprintln!(
            "nested host start: timed={timed} pid={} spawn_before_mono_ns={spawn_before} spawn_after_mono_ns={spawn_after} unix_ns={}",
            host.id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let mut observed_exit = None;
        while !dir.path().join("cli.pid").exists() && start.elapsed() < Duration::from_secs(2) {
            if observed_exit.is_none()
                && let Some(status) = host.try_wait().unwrap()
            {
                observed_exit = Some((start.elapsed(), status));
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let launched = dir.path().join("cli.pid").exists();
        eprintln!(
            "nested fixture: timed={timed}, readiness_elapsed={:?}, launched={launched}, host_exit={observed_exit:?}",
            start.elapsed()
        );
        if timed {
            while host.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            if host.try_wait().unwrap().is_none() {
                host.kill().unwrap();
            }
        } else {
            host.kill().unwrap();
        }
        let status = host.wait().unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "nested host end: timed={timed} elapsed={elapsed:?} status={status} stages={} log={}",
            std::fs::read_to_string(dir.path().join("host.stage")).unwrap_or_default(),
            std::fs::read_to_string(&log).unwrap()
        );
        let mut pids = Vec::new();
        for file in ["provider.pid", "cli.pid"] {
            if let Ok(text) = std::fs::read_to_string(dir.path().join(file)) {
                eprintln!(
                    "nested {file}: modified={:?} pids={text}",
                    std::fs::metadata(dir.path().join(file))
                        .unwrap()
                        .modified()
                        .unwrap()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                );
                pids.extend(text.split_whitespace().map(|v| v.parse::<i32>().unwrap()));
                std::fs::remove_file(dir.path().join(file)).unwrap();
            }
        }
        let start = Instant::now();
        while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0)
            && start.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        for pid in pids {
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                -1,
                "owned nested PID {pid} survived"
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
        if timed {
            assert!(status.success(), "outer deadline fixture failed");
            assert!(elapsed < Duration::from_secs(2));
        } else {
            assert!(launched, "nested CLI never launched");
        }
    }
    for canceled in [false, true] {
        let cancel = Cancellation::default();
        if canceled {
            cancel.cancel();
        }
        let mut runner = PodmanCommand {
            program: "/bin/sleep".into(),
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_millis(60),
        };
        let start = Instant::now();
        let result = runner.run(&["30".into()]);
        if canceled {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().issue.unwrap().contains("deadline"));
        }
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(1));
        eprintln!("nested inner branch: canceled={canceled} elapsed={elapsed:?} assertions=passed");
    }
    let root = dir.path().to_path_buf();
    dir.close().unwrap();
    assert!(!root.exists());
    eprintln!("owned nested root removed: {}", root.display());
}
