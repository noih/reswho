use super::*;
use crate::{
    extensions::{
        self, Capability, Contribution, Descriptor, Extension, ProcessIdentity, Request,
        report::Record,
    },
    model::{DetailField, Process},
};
use anyhow::bail;

struct Details(&'static str, bool);
impl Extension for Details {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.0.into(),
            capabilities: vec![Capability::Details],
            enabled: true,
            availability: "unknown",
        }
    }
    fn execute(&self, _: &extensions::Context<'_>, request: Request<'_>) -> Result<Contribution> {
        if self.1 {
            bail!("SECRET_SENTINEL");
        }
        let Request::Details { process, .. } = request else {
            bail!("expected details")
        };
        Ok(Contribution::ParsedDetails {
            identity: ProcessIdentity::observed(process).unwrap(),
            fields: vec![DetailField {
                key: format!("extensions.{}.value", self.0),
                label: "Value".into(),
                value: serde_json::json!(1),
                sensitive: false,
            }],
        })
    }
}

#[test]
fn large_detail_selection_queries_each_requested_identity_once() {
    let mut processes: Vec<_> = (10..1010)
        .map(|pid| Process {
            pid,
            started_unix_us: Some(u64::from(pid)),
            name: "fixture".into(),
            resident_bytes: Some(100),
            ..Default::default()
        })
        .collect();
    let mut root = crate::model::Node::group(
        "group:all".into(),
        "Resources".into(),
        vec![],
        crate::model::Grouping::Category,
    );
    for process in &processes {
        let mut node = crate::model::Node::group(
            format!("pid:{}", process.pid),
            process.name.clone(),
            vec![],
            crate::model::Grouping::Category,
        );
        node.kind = "process";
        node.grouping = None;
        node.process = Some(Box::new(process.clone()));
        root.totals.add_process(process);
        root.children.push(node);
    }
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let provider = Details("fixture", false);
    selected_details(
        &mut root,
        &mut processes,
        &[],
        &[&provider],
        &context,
        |_| panic!("overview must not query details"),
    );
    let mut requested: Vec<_> = (10..1010).filter(|pid| pid % 2 == 0).collect();
    requested.extend([10, 10, 9999]);
    let verified = std::cell::Cell::new(0);
    selected_details(
        &mut root,
        &mut processes,
        &requested,
        &[&provider],
        &context,
        |_| {
            verified.set(verified.get() + 1);
            true
        },
    );
    assert_eq!(verified.get(), 500);
    crate::model::project_node(&mut root, &requested);
    for node in &root.children {
        let process = node.process.as_ref().unwrap();
        assert_eq!(
            process.extension_details.len(),
            usize::from(process.pid % 2 == 0)
        );
    }
    assert_eq!(root.totals.processes, 1000);
    assert_eq!(root.totals.resident_bytes, 100_000);
}

#[test]
fn final_identity_rejection_invalidates_only_contributing_details_and_suppresses_retry() {
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let mut process = Process {
        pid: 42,
        started_unix_us: Some(123),
        ..Default::default()
    };
    let good = Details("good", false);
    let bad = Details("bad", true);
    assert_eq!(
        extensions::details::attach_details(&[&good, &bad], &context, &mut process).len(),
        1
    );
    context.report.lock().unwrap().extend([
        Record::new(
            "unused",
            "details",
            "never_invoked",
            Some("not_requested"),
            None,
        ),
        Record::new("healthy", "classify", "success", None, Some(Instant::now())),
    ]);
    validate_details_identity(&context, &mut process, |observed| {
        assert_eq!(observed.pid, 42);
        assert_eq!(observed.started_unix_us, Some(123));
        false
    });
    assert!(process.extension_details.is_empty());
    assert!(
        process
            .unavailable
            .iter()
            .any(|diagnostic| diagnostic.code == "identity_changed")
    );
    let records = context.report.lock().unwrap().clone();
    let good_record = records
        .iter()
        .find(|record| record.provider == "good")
        .unwrap();
    assert_eq!(good_record.result, "failed");
    assert_eq!(good_record.reason.as_deref(), Some("identity_changed"));
    assert!(good_record.duration_ms.is_some());
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "bad")
            .unwrap()
            .reason
            .as_deref(),
        Some("provider_failed")
    );
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "unused")
            .unwrap()
            .result,
        "never_invoked"
    );
    assert_eq!(
        records
            .iter()
            .find(|record| record.provider == "healthy")
            .unwrap()
            .result,
        "success"
    );
    assert!(!serde_json::to_string(&records).unwrap().contains("SECRET"));
    assert!(
        extensions::dispatch(
            &good,
            &context,
            Request::Details {
                process: &process,
                pointer_width: 8
            }
        )
        .is_err()
    );
    assert_eq!(
        context.report.lock().unwrap().last().unwrap().result,
        "skipped"
    );
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let mut snapshot = Snapshot {
        extensions: records,
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        scope: "group:all".into(),
        warnings: vec![],
        root: crate::tree::build(vec![]).unwrap(),
    };
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
    snapshot.extensions = context.report.lock().unwrap().clone();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
}

#[test]
fn tag_names_and_versions_survive_projects_apps_and_selected_details() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("work");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"work\"\n").unwrap();
    let bundle = temp.path().join("Apps/Tool.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS/versions")).unwrap();
    let raw = |pid: u32, executable: std::path::PathBuf, cwd: Option<std::path::PathBuf>| Process {
        pid,
        ppid: Some(1),
        uid: Some(501),
        started_unix_us: Some(u64::from(pid)),
        uptime_seconds: Some(5),
        name: "1.2.3".into(),
        executable: Some(executable),
        cwd,
        resident_bytes: Some(1),
        ..Default::default()
    };
    let in_project = raw(
        50,
        temp.path().join("tool/versions/1.2.3"),
        Some(project.clone()),
    );
    let in_app = raw(51, bundle.join("Contents/MacOS/versions/1.2.3"), None);
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let identities = crate::tree::versions::fixtures::Identities(vec![(
        51,
        Some("tool"),
        Some(("tool", "1.2.3")),
    )]);
    let mut root = crate::tree::build_with_providers(
        vec![in_project.clone(), in_app],
        &extensions::Context::new(&settings, &cancel),
        &[&crate::core::Core::AppBundle, &identities],
    )
    .unwrap();
    for (selector, category) in [
        ("pid:50", "group:projects"),
        ("pid:51", "group:applications"),
    ] {
        let category = crate::tree::select(&root, category).unwrap();
        let node = crate::tree::select(&category, selector).unwrap();
        let process = node.process.as_ref().unwrap();
        assert_eq!(process.name, "tool", "{selector}");
        assert_eq!(process.original_name.as_deref(), Some("1.2.3"));
        assert_eq!(process.version.as_ref().unwrap().value, "1.2.3");
    }
    let mut detailed = in_project.clone();
    detailed.command = Some(Default::default());
    attach_details(
        &mut root,
        &std::collections::HashMap::from([(detailed.pid, (&detailed, vec![]))]),
    );
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 1,
        sample_ms: 1,
        collection_ms: 1,
        system: Default::default(),
        scope: "pid:50".into(),
        warnings: vec![],
        root: crate::tree::select(&root, "pid:50").unwrap(),
    };
    let process = snapshot.root.process.as_ref().unwrap();
    assert!(process.command.is_some());
    assert_eq!(process.name, "tool");
    assert_eq!(process.original_name.as_deref(), Some("1.2.3"));
    assert_eq!(
        process.name_source.as_ref().unwrap().source,
        "versioned_install"
    );
    assert!(process.tag_report.is_some());
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["selections"][0]["root"]["process"]["name"], "tool");
    assert_eq!(
        json["selections"][0]["root"]["process"]["version"]["value"],
        "1.2.3"
    );
    let md = crate::output::markdown(&snapshot);
    assert!(md.contains("tool 1.2.3 pid:50 · "), "{md}");
    snapshot.root = root;
    snapshot.scope = "group:all".into();
    assert!(crate::output::markdown(&snapshot).contains("- tool 1.2.3 pid:50 · "));
    temp.close().unwrap();
}

#[test]
fn multi_pid_details_keep_provider_diagnostics_in_their_own_selection() {
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let process = |pid: u32, ppid: u32| Process {
        pid,
        ppid: Some(ppid),
        uid: Some(501),
        started_unix_us: Some(u64::from(pid)),
        name: format!("fixture{pid}"),
        executable: Some("/opt/fixture/run".into()),
        resident_bytes: Some(1),
        ..Default::default()
    };
    let mut processes = vec![process(60, 1), process(61, 60), process(62, 1)];
    let mut root = crate::tree::build(processes.clone()).unwrap();
    let good = Details("good", false);
    let bad = Details("bad", true);
    selected_details(
        &mut root,
        &mut processes,
        &[60, 61],
        &[&good, &bad],
        &context,
        |_| true,
    );
    fn count(value: &serde_json::Value, operation: &str) -> usize {
        match value {
            serde_json::Value::Object(map) => {
                usize::from(map.get("operation").is_some_and(|op| op == operation))
                    + map.values().map(|v| count(v, operation)).sum::<usize>()
            }
            serde_json::Value::Array(items) => items.iter().map(|v| count(v, operation)).sum(),
            _ => 0,
        }
    }
    for (selector, expected) in [
        ("pid:60", 1),
        ("pid:61", 1),
        ("pid:62", 0),
        ("group:all", 0),
    ] {
        let selection = crate::model::Selection {
            selector: selector.into(),
            root: Some(crate::tree::select(&root, selector).unwrap()),
            nested_in: vec![],
            diagnostics: vec![],
        };
        let value = serde_json::to_value(&selection).unwrap();
        assert_eq!(
            count(&value, "extensions.bad.details"),
            expected,
            "{selector}: {value}"
        );
        let details = &value["root"]["process"]["extension_details"];
        assert_eq!(
            details.as_array().map_or(0, Vec::len),
            usize::from(selector == "pid:60" || selector == "pid:61"),
            "{selector}"
        );
    }
}

#[test]
fn vm_host_resource_survives_selected_detail_refresh() {
    let mut snapshot = crate::extensions::builtin::vm::tests::fixture_snapshot(true, true);
    let process = crate::tree::select(&snapshot.root, "pid:40")
        .unwrap()
        .process
        .unwrap();
    let before = serde_json::to_value(&process.host_resource).unwrap();
    let fresh = crate::model::Process {
        pid: 40,
        started_unix_us: Some(40),
        ..Default::default()
    };
    super::attach_details(
        &mut snapshot.root,
        &std::collections::HashMap::from([(fresh.pid, (&fresh, vec![]))]),
    );
    let process = crate::tree::select(&snapshot.root, "pid:40")
        .unwrap()
        .process
        .unwrap();
    assert_eq!(
        serde_json::to_value(&process.host_resource).unwrap(),
        before
    );
}

#[cfg(target_os = "macos")]
#[test]
fn selected_process_detail_does_not_enumerate_host_or_collect_runtimes() {
    let mut settings = extensions::default_settings().unwrap();
    for builtin in extensions::BUILTINS {
        settings
            .set_builtin_enabled(&builtin.descriptor().id, false)
            .unwrap();
    }
    let options = Options {
        settings: Arc::new(settings),
        containers: true,
        command_pids: vec![],
        detail_refs: vec![],
    };
    let cancel = Cancellation::default();
    let expected = collect::detail_processes(
        &extensions::Context::new(&options.settings, &cancel),
        &Process {
            pid: std::process::id(),
            ..Default::default()
        },
    )
    .unwrap()
    .remove(0);
    assert!(collect::verify_identity(&expected));
    let mut target = crate::model::Node::group(
        format!("pid:{}", expected.pid),
        expected.name.clone(),
        vec![],
        crate::model::Grouping::Bin,
    );
    target.process = Some(Box::new(expected));
    let mut prior_counts = None;
    for size in [500, 1000, 5000] {
        target.children = (0..size)
            .map(|i| {
                crate::model::Node::group(
                    format!("group:fixture{i}"),
                    "fixture".into(),
                    vec![],
                    crate::model::Grouping::Bin,
                )
            })
            .collect();
        collect::NATIVE_WORK.with(|work| *work.borrow_mut() = [0; 4]);
        let report = Arc::default();
        let result = capture_detail(&options, &cancel, &target, Arc::clone(&report)).unwrap();
        let fresh = result.node.unwrap();
        assert!(fresh.children.is_empty());
        let p = fresh.process.unwrap();
        assert!(
            p.command.is_some() && p.network.is_some() && p.cpu_percent.is_some(),
            "{:?}",
            p.unavailable
        );
        let counts = collect::NATIVE_WORK.with(|work| *work.borrow());
        assert_eq!(counts[0], 0, "must not enumerate the host");
        assert_eq!(counts[2..], [1, 1], "only target sockets and command");
        assert!((2..=130).contains(&counts[1]));
        if let Some(prior) = prior_counts {
            assert_eq!(counts, prior);
        }
        prior_counts = Some(counts);
        assert!(!report.lock().unwrap().iter().any(|r| r.phase == "collect"));
    }
    target.process.as_mut().unwrap().started_unix_us = Some(1);
    collect::NATIVE_WORK.with(|work| *work.borrow_mut() = [0; 4]);
    assert!(
        capture_detail(&options, &cancel, &target, Arc::default())
            .unwrap()
            .node
            .is_none()
    );
    assert_eq!(collect::NATIVE_WORK.with(|work| *work.borrow()), [0; 4]);
    cancel.cancel();
    assert!(capture_detail(&options, &cancel, &target, Arc::default()).is_err());
}

#[test]
fn runtime_detail_is_scoped_and_does_not_retarget_changed_identity() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    struct Runtime<'a> {
        name: &'static str,
        daemon: &'static str,
        calls: &'a AtomicUsize,
    }
    impl Extension for Runtime<'_> {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: self.name.into(),
                capabilities: vec![Capability::Collect],
                enabled: true,
                availability: "available",
            }
        }
        fn execute(&self, _: &extensions::Context<'_>, _: Request<'_>) -> Result<Contribution> {
            self.calls.fetch_add(1, Relaxed);
            Ok(Contribution::Collect {runtime: serde_json::from_value(serde_json::json!({
                "name":self.name, "endpoint_ref":"endpoint", "daemon_id":self.daemon,
                "status":"available", "inventory_complete":true, "collection_ms":0, "unavailable":[]
            })).unwrap(), containers: vec![]})
        }
    }
    let calls = AtomicUsize::new(0);
    let unrelated_calls = AtomicUsize::new(0);
    let provider = Runtime {
        name: "fixture",
        daemon: "original",
        calls: &calls,
    };
    let unrelated = Runtime {
        name: "unrelated",
        daemon: "other",
        calls: &unrelated_calls,
    };
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let root = extensions::collect(&[&provider], &context).unwrap();
    let target = root.children[0].without_children();
    calls.store(0, Relaxed);
    assert!(
        resource_detail(&target, &[&unrelated, &provider], &context)
            .unwrap()
            .is_some()
    );
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(unrelated_calls.load(Relaxed), 0);
    let changed = Runtime {
        daemon: "restarted",
        ..provider
    };
    assert!(
        resource_detail(&target, &[&unrelated, &changed], &context)
            .unwrap()
            .is_none()
    );
    assert!(resource_detail(&target, &[&unrelated], &context).is_err());
    assert_eq!(unrelated_calls.load(Relaxed), 0);
}

#[test]
fn scoped_details_keep_linked_overview_data_with_explicit_freshness() {
    let mut target = crate::model::Node::group(
        "group:fixture".into(),
        "fixture".into(),
        vec![],
        crate::model::Grouping::Runtime,
    );
    target.host_resources.push(crate::model::HostUsage {
        reference: "pid:42".into(),
        identity: ProcessIdentity {
            pid: 42,
            started_unix_us: 100,
        },
        cpu_percent: Some(1.0),
        resident_bytes: Some(200),
    });
    let mut fresh = target.without_children();
    fresh.host_resources.clear();
    retain_overview_context(&mut fresh, &target);
    assert_eq!(fresh.host_resources.len(), 1);
    assert_eq!(
        fresh.host_resources[0].identity,
        target.host_resources[0].identity
    );
    assert!(fresh.diagnostics.iter().any(|d| d.code == "not_refreshed"));
}

#[test]
fn overview_configuration_is_only_retained_for_the_same_related_resource() {
    let mut target = crate::model::Node::group(
        "pid:42".into(),
        "fixture".into(),
        vec![],
        crate::model::Grouping::Bin,
    );
    target.process = Some(Box::new(Process {
        pid: 42,
        host_resource: Some(Box::new(crate::model::HostResource {
            link_key: Some("scope-a".into()),
            kind: "fixture".into(),
            scope: Some("scope".into()),
            label: "fixture".into(),
            confidence: "verified".into(),
            diagnostics: vec![],
            responsible: None,
            evidence: vec![],
            configuration: Some(Box::new(crate::model::ResourceConfiguration {
                cpus: Some(4),
                scope: "fixture".into(),
                ..Default::default()
            })),
        })),
        ..Default::default()
    }));
    for key in [None, Some("scope-b"), Some("scope-a")] {
        let mut fresh = target.without_children();
        let resource = fresh
            .process
            .as_mut()
            .unwrap()
            .host_resource
            .as_mut()
            .unwrap();
        resource.configuration = None;
        resource.link_key = key.map(Into::into);
        retain_overview_context(&mut fresh, &target);
        assert_eq!(
            fresh
                .process
                .as_ref()
                .unwrap()
                .host_resource
                .as_ref()
                .unwrap()
                .configuration
                .is_some(),
            key == Some("scope-a")
        );
        assert_eq!(!fresh.diagnostics.is_empty(), key == Some("scope-a"));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "read-only timing experiment; run alone after building"]
fn targeted_detail_timing() {
    let options = Options {
        settings: Arc::new(extensions::default_settings().unwrap()),
        containers: false,
        command_pids: vec![std::process::id()],
        detail_refs: vec![],
    };
    let cancel = Cancellation::default();
    let warmup = capture(&options, &cancel).unwrap();
    let target = tree::select_detail(&warmup.root, &format!("pid:{}", std::process::id())).unwrap();
    capture_detail(&options, &cancel, &target, Arc::default()).unwrap();
    for round in 0..6 {
        for scoped in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            collect::NATIVE_WORK.with(|work| *work.borrow_mut() = [0; 4]);
            let report = Arc::default();
            let start = Instant::now();
            let (selected, process_count, collection_ms) = if scoped {
                let result =
                    capture_detail(&options, &cancel, &target, Arc::clone(&report)).unwrap();
                (result.node.unwrap(), None, None)
            } else {
                let result = capture_reported(&options, &cancel, Arc::clone(&report)).unwrap();
                (
                    tree::select_detail(&result.root, &target.reference).unwrap(),
                    Some(result.root.totals.processes),
                    Some(result.collection_ms),
                )
            };
            let elapsed = start.elapsed();
            let process = selected.process.as_ref().unwrap();
            let missing = [
                process.cpu_percent.is_none(),
                process.resident_bytes.is_none(),
                process.network.is_none(),
                process
                    .command
                    .as_ref()
                    .and_then(|command| command.argv.as_ref())
                    .is_none(),
            ];
            let failed = report
                .lock()
                .unwrap()
                .iter()
                .filter(|record| record.result == "failed")
                .count();
            eprintln!(
                "round={round} scoped={scoped} call_ms={:.2} snapshot_collection_ms={collection_ms:?} host_processes={process_count:?} native_counts={:?} selected_diagnostics={} failed_phases={failed} missing_cpu_rss_network_command={missing:?}",
                elapsed.as_secs_f64() * 1000.0,
                collect::NATIVE_WORK.with(|work| *work.borrow()),
                process.unavailable.len() + selected.diagnostics.len()
            );
        }
    }
}

#[test]
fn resource_detail_routes_containers_and_independent_resources_without_broadening_selection() {
    struct Provider(std::sync::atomic::AtomicUsize);
    impl Extension for Provider {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: "fixture".into(),
                capabilities: vec![Capability::Collect],
                enabled: true,
                availability: "available",
            }
        }
        fn execute(
            &self,
            context: &extensions::Context<'_>,
            request: Request<'_>,
        ) -> Result<Contribution> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let runtime: crate::containers::Runtime = serde_json::from_value(serde_json::json!({"name":"fixture", "endpoint_ref":"endpoint", "daemon_id":"original", "status":"available", "inventory_complete":true, "collection_ms":0, "unavailable":[]})).unwrap();
            assert!(
                !context.detail_refs.contains(&runtime.reference()),
                "container/resource selection must not request runtime disk usage"
            );
            Ok(match request {
                Request::Collect => Contribution::Collect {
                    runtime,
                    containers: vec![crate::containers::Container {
                        runtime: "fixture".into(),
                        endpoint_ref: "endpoint".into(),
                        id: "container".into(),
                        name: "owned".into(),
                        state: Some("running".into()),
                        memory_basis: "fixture".into(),
                        memory_bytes: Some(123),
                        ..Default::default()
                    }],
                },
                Request::CollectResources => Contribution::Collected {
                    runtime: None,
                    containers: vec![],
                    resources: vec![crate::model::Resource {
                        provider: "fixture".into(),
                        id: "volume".into(),
                        kind: "fixture.volume".into(),
                        label: "Fixture volume".into(),
                        path: None,
                        metrics: vec![crate::model::ResourceMetric {
                            key: "bytes".into(),
                            label: "Bytes".into(),
                            unit: "bytes".into(),
                            value: Some(123.into()),
                        }],
                    }],
                },
                _ => bail!("unexpected query"),
            })
        }
    }
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let provider = Provider(std::sync::atomic::AtomicUsize::new(0));
    for containers in [true, false] {
        let context = extensions::Context::new(&settings, &cancel);
        let (container_root, resource_root) =
            extensions::collect_scoped(&[&provider], &context, containers).unwrap();
        let root = if containers {
            container_root
        } else {
            resource_root
        };
        let mut pending = vec![&root];
        let target = loop {
            let node = pending.pop().expect("fixture resource missing");
            if node.container.is_some() || node.resource.is_some() {
                break node.without_children();
            }
            pending.extend(&node.children);
        };
        let refs = [target.reference.clone()];
        let mut context = extensions::Context::new(&settings, &cancel);
        context.detail_refs = &refs;
        provider.0.store(0, std::sync::atomic::Ordering::Relaxed);
        let fresh = resource_detail(&target, &[&provider], &context)
            .unwrap()
            .unwrap();
        assert_eq!(fresh.reference, target.reference);
        assert!(fresh.children.is_empty());
        assert_eq!(provider.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
