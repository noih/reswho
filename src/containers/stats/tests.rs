use super::*;
use crate::{
    containers::http::tests::{Reply, Server},
    snapshot::Cancellation,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(crate) fn sample(id: &str, tick: u64) -> Value {
    json!({
        "id": id, "read": format!("2026-01-01T00:00:{tick:02}.123456789Z"),
        "cpu_stats": {"cpu_usage": {"total_usage": tick * 100}, "system_cpu_usage": tick * 1000, "online_cpus": 4},
        "memory_stats": {"usage": 12345, "stats": {"inactive_file": 345}}
    })
}

#[test]
fn paired_sampling_is_fresh_each_call_and_respects_interval() {
    let counter = Arc::new(AtomicU64::new(0));
    let counts = counter.clone();
    let id = "a".repeat(64);
    let sample_id = id.clone();
    let server = Server::new(move |path| {
        if path == "/version" {
            return Reply::Json(json!({"ApiVersion":"1.56", "MinAPIVersion":"1.44", "Os":"linux"}));
        }
        assert_eq!(
            path,
            format!("/v1.56/containers/{sample_id}/stats?stream=false&one-shot=true")
        );
        let tick = counts.fetch_add(1, Ordering::Relaxed) + 1;
        Reply::Json(sample(&sample_id, tick))
    });
    let cancel = Cancellation::default();
    for _ in 0..2 {
        let start = std::time::Instant::now();
        let batch = collect(
            &server.client(&cancel),
            std::slice::from_ref(&id),
            Duration::from_millis(30),
            None,
        )
        .unwrap()
        .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert_eq!(batch.metrics[&id].cpu_percent, Some(40.0));
        assert_eq!(batch.metrics[&id].memory_bytes, Some(12000));
        assert_eq!(batch.memory_basis, API_MEMORY_BASIS);
        assert!(batch.unavailable.is_empty());
    }
    assert_eq!(counter.load(Ordering::Relaxed), 4);
    assert_eq!(
        server
            .paths
            .lock()
            .unwrap()
            .iter()
            .filter(|path| *path == "/version")
            .count(),
        2
    );
}

#[test]
fn cpu_requires_monotonic_matching_counters_and_read_times() {
    let before: Sample = serde_json::from_value(sample("a", 1)).unwrap();
    for pointer in [
        "/cpu_stats/cpu_usage/total_usage",
        "/cpu_stats/system_cpu_usage",
        "/cpu_stats/online_cpus",
        "/read",
    ] {
        let mut after = sample("a", 2);
        *after.pointer_mut(pointer).unwrap() = Value::Null;
        let after: Sample = serde_json::from_value(after).unwrap();
        assert!(after.metrics_since(Some(&before)).cpu_percent.is_none());
    }
    let mut after = sample("a", 2);
    after["cpu_stats"]["cpu_usage"]["total_usage"] = 100.into();
    let after: Sample = serde_json::from_value(after).unwrap();
    assert_eq!(after.metrics_since(Some(&before)).cpu_percent, Some(0.0));
    for (pointer, value) in [
        ("/cpu_stats/cpu_usage/total_usage", json!(99)),
        ("/cpu_stats/system_cpu_usage", json!(1000)),
        ("/cpu_stats/system_cpu_usage", json!(999)),
        ("/cpu_stats/online_cpus", json!(2)),
        ("/read", json!("2026-01-01T00:00:01.123456789Z")),
        ("/read", json!("0001-01-01T00:00:00Z")),
    ] {
        let mut after = sample("a", 2);
        *after.pointer_mut(pointer).unwrap() = value;
        let after: Sample = serde_json::from_value(after).unwrap();
        assert!(after.metrics_since(Some(&before)).cpu_percent.is_none());
    }
    let missing: Sample =
        serde_json::from_value(json!({"id":"a", "read":"0001-01-01T00:00:00Z"})).unwrap();
    let metrics = missing.metrics_since(Some(&before));
    assert!(metrics.cpu_percent.is_none());
    assert!(metrics.memory_bytes.is_none());
}

#[test]
fn memory_uses_cli_cgroup_v1_v2_rules_without_rounding() {
    for (stats, expected) in [
        (
            json!({"total_inactive_file":345, "inactive_file":45}),
            12000,
        ),
        (json!({"inactive_file":345}), 12000),
        (
            json!({"total_inactive_file":99999,"inactive_file":345}),
            12000,
        ),
        (json!({"inactive_file":12345}), 12345),
        (json!({"inactive_file":99999}), 12345),
        (json!({}), 12345),
    ] {
        let memory: Memory = serde_json::from_value(json!({"usage":12345, "stats":stats})).unwrap();
        assert_eq!(memory.bytes(), Some(expected));
    }
    assert_eq!(Memory::default().bytes(), None);
    assert_eq!(
        serde_json::from_value::<Memory>(json!({"usage":0}))
            .unwrap()
            .bytes(),
        Some(0)
    );
}

#[test]
fn api_negotiation_honors_server_range_and_explicit_version() {
    let server = Version {
        api_version: "1.56".into(),
        min_api_version: Some("1.44".into()),
        os: "linux".into(),
    };
    assert_eq!(negotiated(&server, None).as_deref(), Some("v1.56"));
    assert_eq!(negotiated(&server, Some("1.45")).as_deref(), Some("v1.45"));
    for requested in ["1.40", "1.43", "1.57", "invalid", "1.56/../../secret"] {
        assert_eq!(negotiated(&server, Some(requested)), None);
    }
    for server in [
        Version {
            api_version: "1.40".into(),
            min_api_version: None,
            os: "linux".into(),
        },
        Version {
            api_version: "1.99".into(),
            min_api_version: Some("1.57".into()),
            os: "linux".into(),
        },
        Version {
            api_version: "1.56".into(),
            min_api_version: None,
            os: "windows".into(),
        },
    ] {
        assert_eq!(negotiated(&server, None), None);
    }
}

#[test]
fn older_engine_returns_fallback_without_stats_requests() {
    let server = Server::new(|path| {
        assert_eq!(path, "/version");
        Reply::Json(json!({"ApiVersion":"1.40", "Os":"linux"}))
    });
    let cancel = Cancellation::default();
    assert!(
        collect(
            &server.client(&cancel),
            &["a".repeat(64)],
            Duration::ZERO,
            None
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(server.paths.lock().unwrap().len(), 1);
}

#[test]
fn deadline_during_sample_wait_prevents_second_request() {
    let id = "a".repeat(64);
    let sample_id = id.clone();
    let server = Server::new(move |path| {
        if path == "/version" {
            Reply::Json(json!({"ApiVersion":"1.56", "Os":"linux"}))
        } else {
            Reply::Json(sample(&sample_id, 1))
        }
    });
    let cancel = Cancellation::default();
    let mut client = server.client(&cancel);
    client.deadline = std::time::Instant::now() + Duration::from_millis(100);
    let error = collect(&client, &[id], Duration::from_secs(1), None)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("deadline"));
    assert_eq!(server.paths.lock().unwrap().len(), 2);
}

#[test]
fn unavailable_or_mismatched_container_does_not_erase_healthy_samples() {
    let good = "a".repeat(64);
    let bad = "b".repeat(64);
    let good_id = good.clone();
    let tick = AtomicU64::new(0);
    let server = Server::new(move |path| {
        if path == "/version" {
            return Reply::Json(json!({"ApiVersion":"1.56", "Os":"linux"}));
        }
        if path.contains(&good_id) {
            return Reply::Json(sample(&good_id, tick.fetch_add(1, Ordering::Relaxed) + 1));
        }
        Reply::Json(sample("unexpected-container", 1))
    });
    let cancel = Cancellation::default();
    let batch = collect(
        &server.client(&cancel),
        &[good.clone(), bad.clone()],
        Duration::ZERO,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(batch.metrics[&good].cpu_percent, Some(40.0));
    assert!(!batch.metrics.contains_key(&bad));
    assert_eq!(batch.unavailable.len(), 1);
    assert!(batch.unavailable[0].message.contains("identity mismatch"));
    assert!(
        !batch.unavailable[0]
            .message
            .contains("unexpected-container")
    );
}
