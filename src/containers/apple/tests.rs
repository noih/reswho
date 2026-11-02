use super::*;
use crate::{
    containers::{command::Output, stats::Metrics},
    model::{Snapshot, SystemSummary},
    output,
};
use serde_json::{Value, json};
use std::collections::VecDeque;

struct Fake {
    replies: VecDeque<Output>,
}
impl Runner for Fake {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        assert!(args == ["--version"] || args == ["list", "--all", "--format", "json"]);
        Ok(self.replies.pop_front().expect("unexpected CLI request"))
    }
}
fn fake(version: &str, inventories: &[Value]) -> Fake {
    Fake {
        replies: std::iter::once(Output {
            stdout: version.as_bytes().to_vec(),
            issue: None,
        })
        .chain(inventories.iter().map(|v| Output {
            stdout: serde_json::to_vec(v).unwrap(),
            issue: None,
        }))
        .collect(),
    }
}
fn item(id: &str, state: &str) -> Value {
    json!({"configuration": {"id":id, "creationDate":"2026-10-01T10:00:00Z", "labels":{}, "mounts":[], "publishedPorts":[], "initProcess":{"workingDirectory":"/"}},
        "status":{"state":state,"startedDate":"2026-10-02T10:00:00Z"}})
}
fn now() -> OffsetDateTime {
    OffsetDateTime::parse("2026-11-03T00:00:00Z", &Rfc3339).unwrap()
}
fn batch(ids: &[String]) -> Result<Batch> {
    Ok(Batch {
        metrics: ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    Metrics {
                        cpu_percent: Some(50.0),
                        memory_bytes: Some(4096),
                    },
                )
            })
            .collect(),
        memory_basis: MEMORY_BASIS,
        unavailable: vec![],
    })
}

#[test]
fn fresh_identity_metrics_attribution_ports_and_output_contract() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Cargo.toml"), "").unwrap();
    let mut running = item("named-container", "running");
    running["configuration"]["mounts"] =
        json!([{"type":{"virtiofs":{}},"source":temp.path(),"destination":"/work"}]);
    running["configuration"]["initProcess"] = json!({"workingDirectory":"/work", "environment":["SECRET=never-print"], "arguments":["--token", "never-print"]});
    running["configuration"]["publishedPorts"] = json!([{"hostAddress":"127.0.0.1", "hostPort":8000,"containerPort":80,"proto":"tcp","count":2}]);
    let items = json!([running, item("stopped", "stopped")]);
    let mut runner = fake(
        "container CLI version 1.6.1 (release)",
        &[items.clone(), items],
    );
    let (runtime, containers) = collect(&mut runner, now(), |ids| {
        assert_eq!(ids, &["named-container"]);
        batch(ids)
    });
    assert!(runner.replies.is_empty());
    assert_eq!(runtime.status, "available");
    assert!(runtime.unavailable.is_empty());
    let running = &containers[0];
    assert_eq!(running.cpu_percent, Some(50.0));
    assert_eq!(
        running.project.as_ref().unwrap().path.as_deref(),
        Some(temp.path().canonicalize().unwrap().as_path())
    );
    assert_eq!(running.ports.as_ref().unwrap()[1].host_port, Some(8001));
    assert!(containers[1].is_inactive());
    let root = super::super::build(runtime, containers).unwrap();
    assert_eq!(root.container_totals.as_ref().unwrap().cpu_unknown, 0);
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root,
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/reswho-8.schema.json")).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&json));
    assert!(!json.to_string().contains("never-print"));
    let md = output::markdown(&snapshot);
    assert!(md.contains("apple (local)"));
    assert!(md.contains("named-container"));
    assert!(md.contains("8001->81"));
    assert!(!md.contains("never-print"));
    temp.close().unwrap();
}

#[test]
fn restart_missing_identity_and_partial_stats_never_become_zero() {
    for field in ["startedDate", "creationDate", "missing"] {
        let original = item("one", "running");
        let mut fresh = original.clone();
        if field == "startedDate" {
            fresh["status"][field] = json!("2026-10-03T10:00:00Z");
        }
        if field == "creationDate" {
            fresh["configuration"][field] = json!("2026-10-03T10:00:00Z");
        }
        let after = if field == "missing" {
            json!([])
        } else {
            json!([fresh])
        };
        let mut runner = fake("1.6.1", &[json!([original]), after]);
        let (_, containers) = collect(&mut runner, now(), batch);
        assert_eq!(containers[0].memory_bytes, None);
        assert_eq!(containers[0].cpu_percent, None);
        assert!(
            containers[0]
                .unavailable
                .iter()
                .any(|d| d.code == "identity_unverified")
        );
    }
    let items = json!([item("one", "running"), item("two", "running")]);
    let mut runner = fake("1.6.1", &[items.clone(), items]);
    let (_, containers) = collect(&mut runner, now(), |_| batch(&["one".into()]));
    assert_eq!(containers[0].memory_bytes, Some(4096));
    assert_eq!(containers[1].memory_bytes, None);
    assert!(!containers[1].unavailable.is_empty());
}

#[test]
fn unsupported_versions_keep_inventory_without_slow_fallback() {
    let mut runner = fake(
        "container CLI version 2.0.0",
        &[json!([item("one", "running")])],
    );
    let (runtime, containers) = collect(&mut runner, now(), |_| {
        panic!("unsupported XPC must not be called")
    });
    assert_eq!(runtime.status, "available");
    assert_eq!(containers.len(), 1);
    assert_eq!(runtime.unavailable[0].code, "unsupported");
    assert!(runner.replies.is_empty());
}

#[test]
fn invalid_inventory_unknown_state_and_limits_are_explicit() {
    for data in [
        json!([item("one", "running"), item("one", "running")]),
        json!([{"configuration":{}}]),
    ] {
        let mut runner = fake("1.6.1", &[data]);
        let (runtime, containers) = collect(&mut runner, now(), |_| panic!("invalid inventory"));
        assert_eq!(runtime.status, "unavailable");
        assert!(!runtime.inventory_complete);
        assert!(containers.is_empty());
    }
    let mut runner = fake(
        "1.6.1",
        &[json!(
            (0..257)
                .map(|i| item(&i.to_string(), "future-state"))
                .collect::<Vec<_>>()
        )],
    );
    let (runtime, containers) = collect(&mut runner, now(), |_| {
        panic!("unknown state is not running")
    });
    assert!(!runtime.inventory_complete);
    assert_eq!(containers.len(), 256);
    assert!(!containers[0].is_inactive());
    assert!(
        runtime
            .unavailable
            .iter()
            .any(|d| d.code == "limit_exceeded")
    );
}

#[test]
fn malformed_ports_and_non_bind_mounts_keep_metadata_honest() {
    let mut value = item("one", "running");
    value["configuration"]["mounts"] = json!([
        {"type":{"volume":{"name":"data"}},"source":"/unrelated","destination":"/data"},
        {"type":{"tmpfs":{}},"source":"tmpfs","destination":"/tmp"}
    ]);
    value["configuration"]["publishedPorts"] =
        json!([{"hostAddress":"::1","hostPort":65535,"containerPort":80,"proto":"tcp","count":2}]);
    let item: Item = serde_json::from_value(value).unwrap();
    let container = apply(&item, "endpoint", now(), &mut Discovery::default());
    assert!(container.ports.is_none());
    assert_eq!(container.mounts[0].kind, "volume");
    assert_eq!(container.mounts[1].kind, "tmpfs");
    assert!(container.project.is_none());
    assert!(
        container
            .unavailable
            .iter()
            .any(|d| d.code == "invalid_data")
    );
    let mut value = super::tests::item("one", "running");
    value["configuration"]
        .as_object_mut()
        .unwrap()
        .remove("mounts");
    let item: Item = serde_json::from_value(value).unwrap();
    assert!(
        apply(&item, "endpoint", now(), &mut Discovery::default())
            .unavailable
            .iter()
            .any(|d| d.message == "mount metadata unavailable")
    );
}
