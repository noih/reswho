use super::super::command::Output;
use super::*;
use serde_json::{Value, json};
use std::collections::VecDeque;
const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
struct Fake {
    replies: VecDeque<Result<Output>>,
    calls: Vec<Vec<String>>,
}
impl Runner for Fake {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.calls.push(args.to_vec());
        self.replies.pop_front().expect("unexpected Podman call")
    }
}
fn output(text: String) -> Result<Output> {
    Ok(Output {
        stdout: text.into_bytes(),
        issue: None,
    })
}
fn inspect() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/podman/inspect.json")).unwrap()
}
fn counter(cpu: u64, time: u64) -> Value {
    json!({"id":ID,"cpu_nano":cpu,"system_nano":time,"memory":503808,"pids":1})
}
fn configured() -> Fake {
    Fake { replies: [
        json!({"version":"5.8.9","os":"linux","remote":false,"hostname":"owned-linux","graphroot":"/var/lib/containers/storage"}).to_string(),
        format!("{ID}\n"), inspect().to_string(),
        counter(1000000000, 1000000000).to_string(), counter(1500000000, 1500000000).to_string(), inspect().to_string()
    ].into_iter().map(output).collect(), calls: vec![] }
}
fn run(fake: &mut Fake) -> (Runtime, Vec<Container>) {
    collect(
        fake,
        &Cancellation::default(),
        Instant::now() + Duration::from_secs(1),
        Duration::ZERO,
        (None, None),
        OffsetDateTime::parse("2026-11-03T02:00:00Z", &Rfc3339).unwrap(),
    )
}
#[test]
fn raw_paired_counters_inventory_attribution_and_secret_omission() {
    let mut fake = configured();
    let (runtime, containers) = run(&mut fake);
    assert_eq!(runtime.status, "available");
    assert!(runtime.inventory_complete);
    let item = &containers[0];
    assert_eq!(item.cpu_percent, Some(100.0));
    assert_eq!(item.memory_bytes, Some(503808));
    assert_eq!(item.memory_basis, MEMORY_BASIS);
    assert_eq!(item.uptime_seconds, Some(3600));
    assert_eq!(item.project.as_ref().unwrap().name, "rw26");
    assert_eq!(item.service.as_deref(), Some("api"));
    assert_eq!(item.ports.as_ref().unwrap().len(), 2);
    assert_eq!(item.ports.as_ref().unwrap()[0].host_port, Some(18080));
    assert!(item.candidates.is_empty());
    let template = fake.calls[2].join(" ");
    assert!(template.contains("json .ID"));
    assert!(!template.contains(".Config.Env"));
    assert!(!template.contains(".Config.Cmd"));
    assert!(!template.contains("json .Config.Labels"));
    assert!(fake.replies.is_empty());
}
#[test]
fn reset_partial_unknown_and_zero_are_distinct() {
    let before: Counter = serde_json::from_value(counter(100, 100)).unwrap();
    let mut after: Counter = serde_json::from_value(counter(100, 200)).unwrap();
    assert_eq!(metrics(&before, &after), (Some(0.0), Some(503808)));
    after.cpu_nano = Some(50);
    assert_eq!(metrics(&before, &after).0, None);
    after.cpu_nano = None;
    assert_eq!(metrics(&before, &after), (None, Some(503808)));
    after.cpu_nano = Some(200);
    after.system_nano = Some(100);
    assert_eq!(metrics(&before, &after).0, None);
    after.system_nano = Some(99);
    assert_eq!(metrics(&before, &after).0, None);
    after.pids = Some(0);
    assert_eq!(metrics(&before, &after), (None, None));
    after.pids = Some(1);
    after.system_nano = Some(200);
    after.memory = None;
    assert_eq!(metrics(&before, &after), (Some(100.0), None));
}
#[test]
fn restart_disappearance_and_missing_metadata_do_not_reuse_metrics() {
    for replacement in [Some("2026-11-03T01:30:00Z"), None] {
        let mut fake = configured();
        fake.replies[5] = match replacement {
            Some(start) => {
                let mut value = inspect();
                value["started_at"] = start.into();
                output(value.to_string())
            }
            None => Err(anyhow::anyhow!("fixture-private-error")),
        };
        let (_, items) = run(&mut fake);
        assert_eq!(items[0].cpu_percent, None);
        assert_eq!(items[0].memory_bytes, None);
    }
    let mut fake = configured();
    fake.replies.truncate(3);
    fake.replies[2] = Err(anyhow::anyhow!("missing"));
    let (runtime, items) = run(&mut fake);
    assert!(runtime.inventory_complete);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, None);
}
#[test]
fn stopped_initialized_and_unknown_states_have_no_fake_usage() {
    for state in [
        "exited",
        "created",
        "stopped",
        "initialized",
        "stopping",
        "unknown",
    ] {
        let mut fake = configured();
        let mut value = inspect();
        value["state"] = state.into();
        fake.replies[2] = output(value.to_string());
        fake.replies.truncate(3);
        let (_, items) = run(&mut fake);
        let item = &items[0];
        assert_eq!(item.cpu_percent, None);
        assert_eq!(item.memory_bytes, None);
        assert_eq!(item.uptime_seconds, None);
        assert_eq!(
            item.is_inactive(),
            matches!(state, "exited" | "created" | "stopped" | "initialized")
        );
    }
}
#[test]
fn remote_connection_precedence_and_paths_are_conservative() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Cargo.toml"), "").unwrap();
    let mut fake = configured();
    let mut info: Value =
        serde_json::from_slice(&fake.replies[0].as_ref().unwrap().stdout).unwrap();
    info["remote"] = true.into();
    fake.replies[0] = output(info.to_string());
    fake.replies.insert(1, output(json!([{"Name":"chosen","URI":"ssh://private@example/run/podman.sock","Default":false}]).to_string()));
    for index in [3, 6] {
        let mut value = inspect();
        value["compose_project"] = Value::Null;
        value["compose_workdir"] = Value::Null;
        value["mounts"][0]["source"] = json!(temp.path());
        fake.replies[index] = output(value.to_string());
    }
    let (runtime, items) = collect(
        &mut fake,
        &Cancellation::default(),
        Instant::now() + Duration::from_secs(1),
        Duration::ZERO,
        (Some("chosen"), Some("unix:///ignored")),
        OffsetDateTime::now_utc(),
    );
    assert_eq!(runtime.context.as_deref(), Some("remote"));
    assert!(items[0].project.is_none());
    assert!(items[0].candidates.is_empty());
    assert!(
        fake.calls
            .iter()
            .skip(2)
            .all(|args| args[0] == "--connection=chosen")
    );
    assert!(
        !serde_json::to_string(&runtime)
            .unwrap()
            .contains("private@example")
    );
    temp.close().unwrap();
}
#[test]
fn malformed_version_capability_and_cli_failures_keep_unknown_semantics() {
    let mut fake = configured();
    fake.replies[3] = Err(anyhow::anyhow!("private-payload"));
    fake.replies.remove(4);
    let (_, items) = run(&mut fake);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].cpu_percent, None);
    let mut fake = configured();
    let mut info: Value =
        serde_json::from_slice(&fake.replies[0].as_ref().unwrap().stdout).unwrap();
    info["version"] = Value::Null;
    fake.replies[0] = output(info.to_string());
    fake.replies.truncate(3);
    let (runtime, items) = run(&mut fake);
    assert_eq!(items.len(), 1);
    assert!(runtime.unavailable.iter().any(|d| d.code == "unsupported"));
    for message in ["not found", "disconnected", "private credential"] {
        let mut fake = Fake {
            replies: VecDeque::from([Err(anyhow::anyhow!(message.to_owned()))]),
            calls: vec![],
        };
        let (runtime, items) = run(&mut fake);
        assert_eq!(runtime.status, "unavailable");
        assert!(!runtime.inventory_complete);
        assert!(items.is_empty());
        assert!(!serde_json::to_string(&runtime).unwrap().contains(message));
    }
}
#[test]
fn duplicate_and_invalid_inventory_are_rejected_and_limit_is_explicit() {
    for inventory in [format!("{ID}\n{ID}"), "not-id".into()] {
        let mut fake = configured();
        fake.replies[1] = output(inventory);
        fake.replies.truncate(2);
        let (runtime, items) = run(&mut fake);
        assert!(!runtime.inventory_complete);
        assert!(items.is_empty());
    }
    let mut fake = configured();
    fake.replies[1] = output((0..257).map(|index| format!("{index:064x}\n")).collect());
    fake.replies.truncate(2);
    for _ in 0..8 {
        fake.replies
            .push_back(Err(anyhow::anyhow!("inspect missing")));
    }
    let (runtime, items) = run(&mut fake);
    assert!(!runtime.inventory_complete);
    assert_eq!(items.len(), 256);
    assert!(
        runtime
            .unavailable
            .iter()
            .any(|d| d.code == "limit_exceeded")
    );
}
#[test]
fn configured_wait_is_once() {
    let mut fake = configured();
    let cancel = Cancellation::default();
    let start = Instant::now();
    let (_, items) = collect(
        &mut fake,
        &cancel,
        start + Duration::from_secs(1),
        Duration::from_millis(35),
        (None, None),
        OffsetDateTime::now_utc(),
    );
    assert_eq!(items.len(), 1);
    assert!(start.elapsed() >= Duration::from_millis(35));
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[test]
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
#[test]
fn nested_host_fixture() {
    if std::env::var_os("RESWHO_PODMAN_GUARD_ROOT").is_none() {
        return;
    }
    let timed = std::env::var_os("RESWHO_PODMAN_OUTER_TIMEOUT").is_some();
    let result = command::run_bounded(
        &std::env::current_exe().unwrap(),
        &[
            "--exact".into(),
            "containers::podman::tests::nested_provider_fixture".into(),
        ],
        Some(b""),
        &Cancellation::default(),
        Instant::now()
            + if timed {
                Duration::from_millis(250)
            } else {
                Duration::from_secs(9)
            },
    );
    if timed {
        assert!(result.unwrap().issue.unwrap().contains("deadline"));
    }
}
#[test]
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
    for timed in [false, true] {
        let mut host_command = Command::new(std::env::current_exe().unwrap());
        if timed {
            host_command.env("RESWHO_PODMAN_OUTER_TIMEOUT", "1");
        }
        let mut host = host_command
            .args(["--exact", "containers::podman::tests::nested_host_fixture"])
            .env("RESWHO_PODMAN_GUARD_ROOT", dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
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
        let mut pids = Vec::new();
        for file in ["provider.pid", "cli.pid"] {
            if let Ok(text) = std::fs::read_to_string(dir.path().join(file)) {
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
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    dir.close().unwrap();
}

#[test]
fn native_missing_fixture() {
    if std::env::var_os("RESWHO_PODMAN_MISSING_FIXTURE").is_none() {
        return;
    }
    let (runtime, items) = native(
        &Cancellation::default(),
        Instant::now() + Duration::from_secs(1),
        Duration::from_millis(200),
    );
    assert_eq!(runtime.status, "unavailable");
    assert!(!runtime.inventory_complete);
    assert!(items.is_empty());
    assert_eq!(runtime.unavailable[0].code, "not_found");
}
#[test]
fn missing_cli_is_explicit_without_contacting_other_runtimes() {
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "containers::podman::tests::native_missing_fixture"
            ])
            .env("RESWHO_PODMAN_MISSING_FIXTURE", "1")
            .env("PATH", "/nonexistent-reswho-podman")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn deadline_during_sample_keeps_inventory_and_discards_unrevalidated_usage() {
    struct Deadline {
        fake: Fake,
        deadline: Instant,
    }
    impl Runner for Deadline {
        fn run(&mut self, args: &[String]) -> Result<Output> {
            if Instant::now() >= self.deadline {
                bail!("fixture deadline");
            }
            self.fake.run(args)
        }
    }
    let start = Instant::now();
    let deadline = start + Duration::from_millis(30);
    let mut runner = Deadline {
        fake: configured(),
        deadline,
    };
    let (runtime, items) = collect(
        &mut runner,
        &Cancellation::default(),
        deadline,
        Duration::from_secs(1),
        (None, None),
        OffsetDateTime::now_utc(),
    );
    assert_eq!(runtime.status, "available");
    assert!(runtime.inventory_complete);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].cpu_percent, None);
    assert_eq!(items[0].memory_bytes, None);
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[test]
fn podman_tree_uses_common_schema_markdown_and_separate_host_totals() {
    let (runtime, items) = run(&mut configured());
    let root = super::super::build_all(vec![(runtime, items)]).unwrap();
    assert_eq!(root.totals.processes, 0);
    assert_eq!(root.totals.resident_bytes, 0);
    assert_eq!(
        root.container_totals.as_ref().unwrap().known_memory_bytes,
        503808
    );
    let snapshot = crate::model::Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 1,
        sample_ms: 400,
        collection_ms: 500,
        system: crate::model::SystemSummary::default(),
        scope: root.reference.clone(),
        warnings: vec![],
        root,
    };
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let value = serde_json::to_value(&snapshot).unwrap();
    let errors = validator
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{errors:?}");
    let markdown = crate::output::markdown_with_depth(&snapshot, usize::MAX);
    assert!(markdown.contains("podman"));
    assert!(markdown.contains("worker"));
    assert!(markdown.contains("18080"));
}
