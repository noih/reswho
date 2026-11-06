use super::*;
use std::collections::VecDeque;

struct Fake {
    replies: VecDeque<Result<Output>>,
    calls: Vec<Vec<String>>,
}

impl Fake {
    fn new(replies: Vec<serde_json::Value>) -> Self {
        Self {
            replies: replies
                .into_iter()
                .map(|value| {
                    Ok(Output {
                        stdout: format!("{value}\n").into_bytes(),
                        issue: None,
                        timed_out: false,
                        exit_code: None,
                    })
                })
                .collect(),
            calls: Vec::new(),
        }
    }

    fn text(&mut self, index: usize, value: String) {
        self.replies[index] = Ok(Output {
            stdout: value.into_bytes(),
            issue: None,
            timed_out: false,
            exit_code: None,
        });
    }
}

impl Runner for Fake {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.calls.push(args.to_vec());
        self.replies.pop_front().expect("unexpected Docker call")
    }

    fn run_until(&mut self, args: &[String], _: Instant) -> Result<Output> {
        self.run(args)
    }
}

fn inspect(id: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"name":"/arbitrary-name","state":"running","started_at":"2026-01-01T00:00:00Z","workdir":"/workspace","ports":{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"9000"}],"5353/udp":null},"mounts":[]})
}

fn now() -> OffsetDateTime {
    OffsetDateTime::parse("2026-01-01T01:00:00Z", &Rfc3339).unwrap()
}

fn configured(id: &str, endpoint: &str) -> Fake {
    let mut fake = Fake::new(vec![
        serde_json::json!(endpoint),
        serde_json::json!({"id":"daemon-A","os":"linux"}),
        serde_json::Value::Null,
        inspect(id),
        serde_json::json!({"id":id,"cpu":"0.00%","memory":"1.5MiB / 2GiB"}),
        inspect(id),
    ]);
    fake.text(2, format!("{id}\n"));
    fake
}

#[test]
fn configured_context_wins_and_stats_are_identity_checked() {
    let id = "a".repeat(64);
    let mut fake = configured(&id, "unix:///custom/docker.sock");
    let (runtime, containers) = collect(
        &mut fake,
        Some("unfamiliar".into()),
        Some("tcp://ignored".into()),
        now(),
    );
    assert!(runtime.inventory_complete);
    assert_eq!(runtime.context.as_deref(), Some("unfamiliar"));
    assert_eq!(containers.len(), 1);
    assert_eq!(containers[0].cpu_percent, Some(0.0));
    assert_eq!(containers[0].memory_bytes, Some(1572864));
    assert_eq!(containers[0].uptime_seconds, Some(3600));
    assert_eq!(containers[0].ports.as_ref().unwrap()[0].host_port, None);
    assert!(
        fake.calls
            .iter()
            .skip(1)
            .all(|call| call[0] == "--context=unfamiliar")
    );
    let template = fake.calls[3].join(" ");
    assert!(!template.contains(".Config.Env"));
    assert!(!template.contains(".Config.Cmd"));
    assert!(!template.contains("json .Config.Labels"));
    assert!(fake.replies.is_empty());
}

#[test]
fn host_override_and_remote_paths_do_not_become_local_projects() {
    let id = "b".repeat(64);
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Cargo.toml"), "").unwrap();
    let mut fake = configured(&id, "tcp://remote.example:2376");
    fake.replies.pop_front();
    let mut detail = inspect(&id);
    detail["mounts"] =
        serde_json::json!([{"kind":"bind","source":temp.path(),"destination":"/workspace"}]);
    fake.text(2, detail.to_string());
    fake.text(4, detail.to_string());
    let (runtime, containers) = collect(
        &mut fake,
        None,
        Some("tcp://remote.example:2376".into()),
        now(),
    );
    assert_eq!(runtime.context, None);
    assert!(containers[0].project.is_none());
    assert!(containers[0].candidates.is_empty());
    assert!(
        fake.calls
            .iter()
            .all(|call| call[0] == "--host=tcp://remote.example:2376")
    );
    temp.close().unwrap();
}

#[test]
fn restart_and_missing_revalidation_discard_metrics() {
    let id = "c".repeat(64);
    for after in [Some("2026-01-01T00:30:00Z"), None] {
        let mut fake = configured(&id, "unix:///socket");
        if let Some(start) = after {
            let mut changed = inspect(&id);
            changed["started_at"] = start.into();
            fake.text(5, changed.to_string());
        } else {
            fake.replies[5] = Err(anyhow::anyhow!("inspection failed"));
        }
        let (_, containers) = collect(&mut fake, Some("default".into()), None, now());
        assert_eq!(containers[0].cpu_percent, None);
        assert_eq!(containers[0].memory_bytes, None);
        assert!(
            containers[0]
                .unavailable
                .iter()
                .any(|issue| issue.code == "identity_unverified")
        );
    }
}

#[test]
fn missing_cli_inventory_errors_and_stats_errors_keep_unknowns() {
    let mut missing = Fake {
        replies: VecDeque::from([Err(anyhow::anyhow!("Docker CLI not found on PATH"))]),
        calls: Vec::new(),
    };
    let (runtime, containers) = collect(&mut missing, None, None, now());
    assert_eq!(runtime.status, "unavailable");
    assert!(!runtime.inventory_complete);
    assert!(containers.is_empty());
    let id = "d".repeat(64);
    let mut fake = configured(&id, "unix:///socket");
    fake.replies[4] = Ok(Output {
        stdout: format!("{{\"id\":\"{id}\",\"cpu\":\"0.00%\",\"memory\":\"0B / 0B\"}}")
            .into_bytes(),
        issue: Some("stats timeout".into()),
        timed_out: false,
        exit_code: None,
    });
    fake.replies.pop_back();
    let (runtime, containers) = collect(&mut fake, Some("default".into()), None, now());
    assert!(runtime.inventory_complete);
    assert_eq!(containers[0].memory_bytes, None);
    assert!(
        runtime
            .unavailable
            .iter()
            .any(|issue| issue.code == "command_failed" && issue.operation == "docker.stats")
    );
    for inventory in ["bad-id\n".to_owned(), format!("{id}\n{id}\n")] {
        let mut fake = configured(&id, "unix:///socket");
        fake.text(2, inventory);
        let (runtime, containers) = collect(&mut fake, Some("default".into()), None, now());
        assert!(!runtime.inventory_complete);
        assert!(containers.is_empty());
    }
}

#[test]
fn stopped_and_disappeared_containers_remain_visible_without_fabricated_usage() {
    let id = "e".repeat(64);
    for missing in [true, false] {
        let mut fake = configured(&id, "unix:///socket");
        fake.replies.truncate(4);
        if missing {
            fake.text(3, String::new());
        } else {
            let mut stopped = inspect(&id);
            stopped["state"] = "exited".into();
            fake.text(3, stopped.to_string());
        }
        let (_, containers) = collect(&mut fake, Some("default".into()), None, now());
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].uptime_seconds, None);
        assert_eq!(containers[0].cpu_percent, None);
        assert!(fake.replies.is_empty());
    }
}

#[test]
fn malformed_metrics_and_ports_are_unknown() {
    assert_eq!(memory_bytes("1.25 GiB / 2 GiB"), Some(1342177280));
    for invalid in ["NaNMiB", "-1MiB", "1e999GiB", "1MB", "unknown"] {
        assert_eq!(memory_bytes(invalid), None);
    }
    for invalid in ["NaN", "inf", "-0.1", ""] {
        assert_eq!(number(invalid), None);
    }
    let id = "f".repeat(64);
    let mut detail = inspect(&id);
    detail["ports"] = serde_json::json!({"8080/tcp":[{"HostIp":"0.0.0.0","HostPort":"invalid"}]});
    assert!(ports(&serde_json::from_value(detail).unwrap()).is_none());
}

#[test]
fn default_context_empty_inventory_and_inventory_limit_are_explicit() {
    let mut empty = Fake::new(vec![
        serde_json::Value::Null,
        serde_json::json!("unix:///socket"),
        serde_json::json!({"id":"daemon","os":"linux"}),
        serde_json::Value::Null,
    ]);
    empty.text(0, "personal-context\n".into());
    empty.text(3, String::new());
    let (runtime, containers) = collect(&mut empty, None, None, now());
    assert!(runtime.inventory_complete);
    assert_eq!(runtime.context.as_deref(), Some("personal-context"));
    assert!(containers.is_empty());
    assert!(empty.replies.is_empty());
    let mut fake = Fake::new(vec![serde_json::Value::Null]);
    fake.text(0, (0..257).map(|id| format!("{id:064x}\n")).collect());
    let mut runtime = runtime;
    let source = Source {
        args: vec![],
        host: "unix:///socket".into(),
        context: None,
    };
    let ids = inventory(&mut fake, &source, &mut runtime).unwrap();
    assert_eq!(ids.len(), 256);
    assert!(!runtime.inventory_complete);
    assert_eq!(runtime.unavailable[0].code, "limit_exceeded");
}

struct ApiRunner {
    cli: Fake,
    transport: crate::containers::command::RuntimeCommand,
}

impl Runner for ApiRunner {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.cli.run(args)
    }

    fn run_until(&mut self, args: &[String], deadline: Instant) -> Result<Output> {
        self.cli.run_until(args, deadline)
    }

    fn direct_stats(&mut self, host: &str, ids: &[String]) -> Result<Option<Batch>> {
        self.transport.direct_stats(host, ids)
    }
}

fn api_runner(cli: Fake) -> ApiRunner {
    ApiRunner {
        cli,
        transport: crate::containers::command::RuntimeCommand {
            cancel: crate::snapshot::Cancellation::default(),
            program: "unused-fixture-command".into(),
            deadline: Instant::now() + std::time::Duration::from_secs(5),
            sample_interval: std::time::Duration::from_millis(10),
            api_version: None,
        },
    }
}

#[test]
fn unix_api_metrics_reach_collection_and_restart_discards_them() {
    use crate::containers::http::tests::{Reply, Server};
    use std::sync::atomic::{AtomicU64, Ordering};
    let id = "a".repeat(64);
    for changed in [false, true] {
        let container_id = id.clone();
        let tick = AtomicU64::new(0);
        let server = Server::new(move |path| {
            if path == "/version" {
                return Reply::Json(
                    serde_json::json!({"ApiVersion":"1.56", "MinAPIVersion":"1.44", "Os":"linux"}),
                );
            }
            let n = tick.fetch_add(1, Ordering::Relaxed) + 1;
            Reply::Json(serde_json::json!({
                "id":container_id, "read":format!("2026-01-01T00:00:0{n}Z"),
                "cpu_stats":{"cpu_usage":{"total_usage":100*n},"system_cpu_usage":1000*n,"online_cpus":4},
                "memory_stats":{"usage":12345,"stats":{"inactive_file":345}}
            }))
        });
        let mut cli = configured(&id, &format!("unix://{}", server.socket.display()));
        cli.replies.remove(4);
        if changed {
            let mut detail = inspect(&id);
            detail["started_at"] = "2026-01-01T00:30:00Z".into();
            cli.text(4, detail.to_string());
        }
        let mut runner = api_runner(cli);
        let (runtime, containers) = collect(&mut runner, Some("test".into()), None, now());
        assert!(runner.cli.replies.is_empty());
        assert!(
            !runner
                .cli
                .calls
                .iter()
                .any(|args| args.iter().any(|arg| arg == "stats"))
        );
        assert_eq!(containers[0].memory_basis, API_MEMORY_BASIS);
        if changed {
            assert_eq!(containers[0].cpu_percent, None);
            assert_eq!(containers[0].memory_bytes, None);
            assert!(
                containers[0]
                    .unavailable
                    .iter()
                    .any(|d| d.code == "identity_unverified")
            );
        } else {
            assert_eq!(containers[0].cpu_percent, Some(40.0));
            assert_eq!(containers[0].memory_bytes, Some(12000));
            assert!(runtime.unavailable.is_empty());
        }
    }
}

#[test]
fn unsupported_endpoints_and_old_engines_keep_cli_collection() {
    use crate::containers::http::tests::{Reply, Server};
    let id = "b".repeat(64);
    let server = Server::new(|path| {
        assert_eq!(path, "/version");
        Reply::Json(serde_json::json!({"ApiVersion":"1.40", "Os":"linux"}))
    });
    for endpoint in [
        format!("unix://{}", server.socket.display()),
        "ssh://test".into(),
        "tcp://test:2376".into(),
    ] {
        let mut runner = api_runner(configured(&id, &endpoint));
        let (runtime, containers) = collect(&mut runner, Some("test".into()), None, now());
        assert!(runtime.unavailable.is_empty());
        assert_eq!(containers[0].memory_basis, CLI_MEMORY_BASIS);
        assert_eq!(containers[0].memory_bytes, Some(1572864));
        assert!(runner.cli.replies.is_empty());
        assert!(
            runner
                .cli
                .calls
                .iter()
                .any(|args| args.iter().any(|arg| arg == "stats"))
        );
    }
    assert_eq!(server.paths.lock().unwrap().len(), 1);
}

fn failing_runner(
    program: std::path::PathBuf,
    budget_ms: u64,
) -> crate::containers::command::RuntimeCommand {
    crate::containers::command::RuntimeCommand {
        cancel: crate::snapshot::Cancellation::default(),
        program,
        deadline: Instant::now() + std::time::Duration::from_millis(budget_ms),
        sample_interval: std::time::Duration::from_millis(10),
        api_version: None,
    }
}

fn assert_cli_failure(
    program: std::path::PathBuf,
    host: String,
    expected: crate::model::FailureReason,
    budget: u64,
    root: &std::path::Path,
) {
    let (runtime, containers) = collect(
        &mut failing_runner(program, budget),
        None,
        Some(host),
        now(),
    );
    assert_eq!(runtime.status, "unavailable");
    assert!(containers.is_empty());
    let [failure] = runtime.unavailable.as_slice() else {
        panic!("{:?}", runtime.unavailable)
    };
    assert_eq!(failure.code, "collection_failed");
    assert_eq!(failure.reason, Some(expected));
    assert!(
        failure
            .message
            .contains(&format!("reason {}: ", expected.code()))
    );
    let json = serde_json::to_string(&runtime).unwrap();
    assert!(json.contains(&format!("\"reason\":\"{}\"", expected.code())));
    assert!(!json.contains("private-credential"));
    assert!(!json.contains(root.to_str().unwrap()));
}

#[test]
fn cli_failures_report_structured_reasons_without_stderr_or_endpoint() {
    use crate::containers::command::failure_fixtures::cli;
    use crate::model::FailureReason;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let missing = format!("unix://{}", root.join("missing.sock").display());
    for (program, host, expected, budget) in [
        (
            cli(root, "docker", "echo private-credential >&2; exit 1"),
            missing.clone(),
            FailureReason::EndpointNotFound,
            2000,
        ),
        (
            root.join("missing-cli"),
            missing.clone(),
            FailureReason::CliNotFound,
            2000,
        ),
        (
            cli(root, "slow", "exec /bin/sleep 5"),
            missing.clone(),
            FailureReason::Timeout,
            150,
        ),
        (
            cli(root, "garbled", "echo not-json"),
            missing.clone(),
            FailureReason::DaemonError,
            2000,
        ),
        (
            cli(root, "remote", "exit 1"),
            "ssh://user@private-host".into(),
            FailureReason::Unknown,
            2000,
        ),
    ] {
        assert_cli_failure(program, host, expected, budget, root);
    }
    directory.close().unwrap();
}

#[test]
fn cli_failures_classify_owned_socket_states() {
    use crate::containers::command::failure_fixtures::{Endpoints, cli};
    let endpoints = Endpoints::new();
    let root = endpoints.directory.path();
    let failing = cli(root, "docker", "echo private-credential >&2; exit 1");
    for (host, expected) in endpoints.cases() {
        assert_cli_failure(failing.clone(), host, expected, 2000, root);
    }
    endpoints.directory.close().unwrap();
}

#[test]
fn api_failures_report_structured_reasons() {
    use crate::containers::command::failure_fixtures::Endpoints;
    use crate::containers::http::tests::{Reply, Server};
    use crate::model::FailureReason;
    let id = "c".repeat(64);
    let endpoints = Endpoints::new();
    let rejecting = Server::new(|_| {
        Reply::Raw(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 18\r\nConnection: close\r\n\r\nprivate-credential".to_vec())
    });
    let stalled = Server::new(|_| Reply::Stall);
    let mut cases: Vec<_> = endpoints
        .cases()
        .into_iter()
        .map(|(host, reason)| (host, reason, 2000))
        .collect();
    cases.push((
        format!("unix://{}", rejecting.socket.display()),
        FailureReason::DaemonError,
        2000,
    ));
    cases.push((
        format!("unix://{}", stalled.socket.display()),
        FailureReason::Timeout,
        300,
    ));
    for (host, expected, budget) in cases {
        let mut runner = api_runner(Fake::new(Vec::new()));
        runner.transport.deadline = Instant::now() + std::time::Duration::from_millis(budget);
        let source = Source {
            args: Vec::new(),
            host: host.clone(),
            context: None,
        };
        let mut probe = crate::containers::command::Probe::new(&runner);
        let batch = stats(
            &mut runner,
            &source,
            std::slice::from_ref(&id),
            &mut Vec::new(),
            &mut probe,
        );
        let [failure] = batch.unavailable.as_slice() else {
            panic!("{host}: {:?}", batch.unavailable)
        };
        assert_eq!(failure.operation, "docker.stats");
        assert_eq!(failure.reason, Some(expected), "{host}");
        assert!(!failure.message.contains("private-credential"));
    }
    drop((rejecting, stalled));
    endpoints.directory.close().unwrap();
}

#[test]
fn api_sample_failures_after_version_carry_reasons() {
    use crate::containers::http::tests::{Reply, Server};
    use crate::model::FailureReason;
    let id = "d".repeat(64);
    let version = || Reply::Json(serde_json::json!({"ApiVersion": "1.56", "Os": "linux"}));
    let rejecting = Server::new(move |path| {
        if path.ends_with("/version") {
            version()
        } else {
            Reply::Raw(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 18\r\nConnection: close\r\n\r\nprivate-credential".to_vec())
        }
    });
    let stalled = Server::new(move |path| {
        if path.ends_with("/version") {
            version()
        } else {
            Reply::Stall
        }
    });
    for (server, expected, budget) in [
        (&rejecting, FailureReason::DaemonError, 2000),
        (&stalled, FailureReason::Timeout, 600),
    ] {
        let mut runner = api_runner(Fake::new(Vec::new()));
        runner.transport.deadline = Instant::now() + std::time::Duration::from_millis(budget);
        let source = Source {
            args: Vec::new(),
            host: format!("unix://{}", server.socket.display()),
            context: None,
        };
        let mut probe = crate::containers::command::Probe::new(&runner);
        let batch = stats(
            &mut runner,
            &source,
            std::slice::from_ref(&id),
            &mut Vec::new(),
            &mut probe,
        );
        assert!(!batch.unavailable.is_empty(), "{expected:?}");
        for diagnostic in &batch.unavailable {
            assert_eq!(diagnostic.reason, Some(expected), "{diagnostic:?}");
            assert!(!diagnostic.message.contains("private-credential"));
        }
        assert!(batch.metrics.is_empty());
        assert_eq!(probe.probes, 0);
    }
    drop((rejecting, stalled));
}

#[test]
fn canceled_collection_reports_canceled_without_probing() {
    use crate::containers::command::failure_fixtures::cli;
    use crate::model::FailureReason;
    let directory = tempfile::tempdir().unwrap();
    let mut runner = failing_runner(cli(directory.path(), "docker", "exit 1"), 2000);
    runner.cancel.cancel();
    let host = format!("unix://{}", directory.path().join("missing.sock").display());
    let (runtime, _) = collect(&mut runner, None, Some(host), now());
    let [failure] = runtime.unavailable.as_slice() else {
        panic!("{:?}", runtime.unavailable)
    };
    assert_eq!(failure.reason, Some(FailureReason::Canceled));
    assert!(failure.os_error.is_none());
    directory.close().unwrap();
}

#[test]
fn vm_desktop_configuration_uses_existing_info_and_excludes_remote_endpoints() {
    let home = std::env::var_os("HOME").unwrap();
    let host = format!(
        "unix://{}/.docker/run/docker.sock",
        std::path::Path::new(&home).display()
    );
    for (host, local) in [(host, true), ("tcp://remote:2376".into(), false)] {
        let id = "a".repeat(64);
        let mut runner = configured(&id, &host);
        runner.replies[1] = Ok(Output {
            stdout: br#"{"id":"daemon","os":"linux","cpus":8,"memory":8589934592}"#.to_vec(),
            issue: None,
            timed_out: false,
            exit_code: None,
        });
        let (runtime, containers) = collect(&mut runner, Some("fixture".into()), None, now());
        assert_eq!(runtime.host_scope.is_some(), local);
        assert_eq!(
            containers[0].metrics_scope.as_deref(),
            local.then_some("in_vm")
        );
        if let Some(config) = runtime.configuration {
            assert_eq!(config.cpus, Some(8));
            assert_eq!(config.memory_bytes, Some(8589934592));
            assert_eq!(config.scope, "vm");
        }
        assert!(
            runner.calls[1]
                .iter()
                .any(|a| a.contains(".NCPU") && a.contains(".MemTotal"))
        );
    }
}

fn collect(
    runner: &mut impl Runner,
    context: Option<String>,
    host: Option<String>,
    now: OffsetDateTime,
) -> (Runtime, Vec<Container>) {
    crate::extensions::collection::assert_adapter(
        "docker",
        super::collect(runner, context, host, now, &[]),
    )
}

#[test]
fn disk_usage_only_queries_selected_runtime_and_preserves_reported_estimates() {
    let id = "a".repeat(64);
    let unrelated = vec![
        "group:all".into(),
        "group:containers".into(),
        "group:stale-runtime".into(),
        "pid:42".into(),
    ];
    let mut fake = configured(&id, "tcp://remote.example:2376");
    let (runtime, _) = super::collect(&mut fake, Some("fixture".into()), None, now(), &unrelated);
    assert!(runtime.disk_usage.is_none());
    assert!(
        !fake
            .calls
            .iter()
            .any(|args| args.iter().any(|arg| arg == "df"))
    );
    let mut fake = configured(&id, "tcp://remote.example:2376");
    let (_, containers) = collect(&mut fake, Some("fixture".into()), None, now());
    assert_eq!(containers.len(), 1);
    assert!(
        !fake
            .calls
            .iter()
            .any(|args| args.iter().any(|arg| arg == "df"))
    );

    let mut fake = configured(&id, "tcp://remote.example:2376");
    fake.replies.push_back(Ok(Output {
        stdout: br#"{"Type":"Images","TotalCount":"5","Active":"2","Size":"16.43MB","Reclaimable":"11.63MB (70%)"}
{"Type":"Build Cache","TotalCount":"0","Active":"0","Size":"0B","Reclaimable":"0B"}
"#.to_vec(),
        issue: None, timed_out: false, exit_code: None,
    }));
    let mut selected = disk_detail_refs();
    selected.push(selected[0].clone());
    let (runtime, containers) = crate::extensions::collection::assert_adapter(
        "docker",
        super::collect(&mut fake, Some("fixture".into()), None, now(), &selected),
    );
    assert!(runtime.inventory_complete);
    assert_eq!(containers[0].memory_bytes, Some(1572864));
    let usage = runtime.disk_usage.as_ref().unwrap();
    assert_eq!(usage.source, "docker system df");
    assert_eq!(usage.entries.len(), 2);
    assert_eq!(usage.entries[0].reclaimable, "11.63MB (70%)");
    assert_eq!(usage.entries[1].reclaimable, "0B");
    assert_eq!(
        fake.calls.last().unwrap(),
        &[
            "--context=fixture",
            "system",
            "df",
            "--format",
            "{{json .}}"
        ]
    );
    assert!(fake.replies.is_empty());
    let root = crate::containers::build(runtime, containers).unwrap();
    let mut snapshot = crate::model::Snapshot {
        extensions: vec![],
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 1,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        scope: "group:containers".into(),
        root,
        warnings: vec![],
    };
    snapshot.project(&[]);
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&json));
    let markdown = crate::output::markdown(&snapshot);
    assert!(markdown.contains("11.63MB (70%)"), "{markdown}");
    assert!(markdown.contains("not host free space"));
    assert_eq!(snapshot.root.totals.resident_bytes, 0);
}

#[test]
fn invalid_or_failed_disk_usage_does_not_discard_container_metrics() {
    for payload in [
        "",
        "{}",
        "not json",
        r#"{"Type":"Images","TotalCount":"1","Active":"2","Size":"1GB","Reclaimable":"0B"}"#,
        r#"{"Type":"Images","TotalCount":"-1","Active":"0","Size":"1GB","Reclaimable":"0B"}"#,
        r#"{"Type":"Images","TotalCount":"1","Active":"0","Size":"1GB","Reclaimable":"bad\nvalue"}"#,
        r#"{"Type":"Images","TotalCount":"1","Active":"0","Size":"1GB","Reclaimable":"0B"} {}"#,
    ] {
        let id = "a".repeat(64);
        let mut fake = configured(&id, "tcp://remote.example:2376");
        fake.replies.push_back(Ok(Output {
            stdout: payload.as_bytes().to_vec(),
            issue: None,
            timed_out: false,
            exit_code: None,
        }));
        let (runtime, containers) = crate::extensions::collection::assert_adapter(
            "docker",
            super::collect(
                &mut fake,
                Some("fixture".into()),
                None,
                now(),
                &disk_detail_refs(),
            ),
        );
        assert_eq!(runtime.status, "available");
        assert!(runtime.disk_usage.is_none());
        assert!(
            runtime
                .unavailable
                .iter()
                .any(|d| d.operation == "docker.disk_usage")
        );
        assert_eq!(containers[0].memory_bytes, Some(1572864));
    }
    let mut fake = configured(&"a".repeat(64), "tcp://remote.example:2376");
    fake.replies.push_back(Ok(Output {
        stdout: vec![],
        issue: Some("command timed out".into()),
        timed_out: true,
        exit_code: None,
    }));
    let (runtime, containers) = crate::extensions::collection::assert_adapter(
        "docker",
        super::collect(
            &mut fake,
            Some("fixture".into()),
            None,
            now(),
            &disk_detail_refs(),
        ),
    );
    assert!(runtime.disk_usage.is_none());
    assert!(runtime.unavailable.iter().any(|d| d.operation == "docker.disk_usage" && d.reason == Some(FailureReason::Timeout)));
    assert_eq!(containers[0].memory_bytes, Some(1572864));
}

fn disk_detail_refs() -> Vec<String> {
    let mut fake = configured(&"a".repeat(64), "tcp://remote.example:2376");
    let (runtime, _) = collect(&mut fake, Some("fixture".into()), None, now());
    vec![runtime.reference()]
}

#[test]
fn disk_usage_reserves_collection_time_and_skips_when_budget_is_insufficient() {
    struct Budget {
        fake: Fake,
        deadline: Instant,
        requested: Option<Instant>,
    }
    impl Runner for Budget {
        fn run(&mut self, args: &[String]) -> Result<Output> {
            self.fake.run(args)
        }
        fn limits(&self) -> Option<(crate::snapshot::Cancellation, Instant)> {
            Some((Default::default(), self.deadline))
        }
        fn run_until(&mut self, args: &[String], deadline: Instant) -> Result<Output> {
            self.requested = Some(deadline);
            self.fake.run_until(args, deadline)
        }
    }
    for seconds in [0.5, 3.0, 20.0] {
        let mut fake = configured(&"a".repeat(64), "tcp://remote.example:2376");
        if seconds > 1.0 {
            fake.replies.push_back(Ok(Output {
                stdout: br#"{"Type":"Images","TotalCount":"1","Active":"0","Size":"1GB","Reclaimable":"1GB"}"#.to_vec(),
                issue: None, timed_out: false, exit_code: None,
            }));
        }
        let before = Instant::now();
        let mut runner = Budget {
            fake,
            deadline: before + Duration::from_secs_f64(seconds),
            requested: None,
        };
        let (runtime, containers) = super::collect(
            &mut runner,
            Some("fixture".into()),
            None,
            now(),
            &disk_detail_refs(),
        );
        assert_eq!(containers[0].memory_bytes, Some(1_572_864));
        assert_eq!(runtime.disk_usage.is_some(), seconds > 1.0);
        if let Some(deadline) = runner.requested {
            assert!(deadline <= runner.deadline - COLLECTION_FINISH_RESERVE);
            assert!(deadline <= Instant::now() + DISK_USAGE_TIMEOUT);
            assert!(deadline > before);
        } else {
            assert_eq!(seconds, 0.5);
            assert!(
                !runner
                    .fake
                    .calls
                    .iter()
                    .any(|args| args.iter().any(|arg| arg == "df"))
            );
            assert!(
                runtime
                    .unavailable
                    .iter()
                    .any(|d| d.operation == "docker.disk_usage"
                        && d.reason == Some(FailureReason::Timeout))
            );
        }
        assert!(runner.fake.replies.is_empty());
    }
}
