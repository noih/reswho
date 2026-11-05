use super::super::command::Output;
use super::*;
use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf};
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
        timed_out: false,
        exit_code: None,
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
    let result = collect(
        fake,
        &Cancellation::default(),
        Instant::now() + Duration::from_secs(1),
        Duration::ZERO,
        (None, None),
        OffsetDateTime::parse("2026-11-03T02:00:00Z", &Rfc3339).unwrap(),
    );
    crate::extensions::collection::assert_adapter("podman", result)
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

fn assert_cli_failure(
    program: PathBuf,
    host: Option<String>,
    expected: crate::model::FailureReason,
    budget: u64,
) {
    let cancel = Cancellation::default();
    let deadline = Instant::now() + Duration::from_millis(budget);
    let mut runner = PodmanCommand {
        program,
        cancel: &cancel,
        deadline,
    };
    let (runtime, containers) = collect(
        &mut runner,
        &cancel,
        deadline,
        Duration::ZERO,
        (None, host.as_deref()),
        OffsetDateTime::parse("2026-11-03T02:00:00Z", &Rfc3339).unwrap(),
    );
    assert!(containers.is_empty());
    let [failure] = runtime.unavailable.as_slice() else {
        panic!("{:?}", runtime.unavailable)
    };
    assert_eq!(
        (failure.code.as_str(), failure.reason),
        ("collection_failed", Some(expected))
    );
    assert!(
        failure
            .message
            .contains(&format!("reason {}: ", expected.code()))
    );
    assert!(
        !serde_json::to_string(&runtime)
            .unwrap()
            .contains("private-credential")
    );
}

#[test]
fn cli_failures_report_structured_reasons_without_stderr() {
    use crate::containers::command::failure_fixtures::cli;
    use crate::model::FailureReason;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let missing = Some(format!("unix://{}", root.join("missing.sock").display()));
    for (program, host, expected, budget) in [
        (
            root.join("missing-cli"),
            None,
            FailureReason::CliNotFound,
            2000,
        ),
        (
            cli(root, "failing", "echo private-credential >&2; exit 1"),
            None,
            FailureReason::Unknown,
            2000,
        ),
        (
            cli(root, "failing", "echo private-credential >&2; exit 1"),
            missing.clone(),
            FailureReason::EndpointNotFound,
            2000,
        ),
        (
            cli(root, "slow", "exec /bin/sleep 5"),
            None,
            FailureReason::Timeout,
            150,
        ),
        (
            cli(root, "garbled", "echo not-json"),
            None,
            FailureReason::DaemonError,
            2000,
        ),
    ] {
        assert_cli_failure(program, host, expected, budget);
    }
    directory.close().unwrap();
}

#[test]
fn cli_failures_classify_owned_socket_states() {
    use crate::containers::command::failure_fixtures::{Endpoints, cli};
    let endpoints = Endpoints::new();
    let failing = cli(endpoints.directory.path(), "podman", "exit 125");
    for (host, expected) in endpoints.cases() {
        assert_cli_failure(failing.clone(), Some(host), expected, 2000);
    }
    endpoints.directory.close().unwrap();
}
