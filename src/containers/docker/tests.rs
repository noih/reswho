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
        });
    }
}

impl Runner for Fake {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.calls.push(args.to_vec());
        self.replies.pop_front().expect("unexpected Docker call")
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
