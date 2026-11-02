use super::*;

#[test]
fn builtin_list_is_deterministic_and_does_not_probe_sources() {
    let settings = default_settings().unwrap();
    let list = list(&settings, false);
    assert_eq!(
        list.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        ["docker", "apple", "app_bundle", "command"]
    );
    assert!(list.iter().all(|entry| entry.availability == "unknown"));
    assert!(!list[0].enabled && !list[1].enabled);
    assert!(settings.report().disabled_extensions.is_empty());
}

use crate::model::{Snapshot, SystemSummary};
use std::sync::{Arc, Condvar, Mutex};

struct Fixture {
    id: &'static str,
    contribution: Contribution,
    fail: bool,
    seen: Arc<Mutex<Vec<Duration>>>,
    concurrent: Option<Arc<(Mutex<usize>, Condvar)>>,
}

impl Fixture {
    fn new(id: &'static str, contribution: Contribution) -> Self {
        Self {
            id,
            contribution,
            fail: false,
            seen: Arc::default(),
            concurrent: None,
        }
    }
}

impl Extension for Fixture {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.id,
            version: "fixture/1",
            protocol_version: 1,
            source: "builtin",
            platforms: vec!["macos"],
            capabilities: vec![self.contribution.capability()],
            enabled: true,
            availability: "unknown",
        }
    }
    fn execute(&self, context: &Context<'_>, _request: Request<'_>) -> Result<Contribution> {
        self.seen
            .lock()
            .unwrap()
            .push(context.settings.interval("sampling.interval_ms"));
        if let Some(concurrent) = &self.concurrent {
            let (lock, ready) = &**concurrent;
            let mut count = lock.lock().unwrap();
            *count += 1;
            ready.notify_all();
            let (count, _) = ready
                .wait_timeout_while(count, Duration::from_secs(1), |count| *count < 2)
                .unwrap();
            if *count < 2 {
                bail!("collectors were not dispatched concurrently");
            }
        }
        if self.fail {
            bail!("SECRET provider failure");
        }
        Ok(self.contribution.clone())
    }
}

fn process() -> Process {
    Process {
        pid: 10,
        ppid: Some(1),
        started_unix_us: Some(100),
        name: "demo".into(),
        executable: Some("/Applications/Demo.app/Contents/MacOS/demo".into()),
        cpu_percent: Some(2.0),
        resident_bytes: Some(100),
        ..Process::default()
    }
}

fn runtime(name: &str) -> Runtime {
    Runtime {
        name: name.into(),
        context: None,
        endpoint_ref: Some(format!("{name}-local")),
        daemon_id: None,
        status: "ready".into(),
        inventory_complete: true,
        collection_ms: 0,
        unavailable: vec![],
    }
}

#[test]
fn collectors_share_settings_run_concurrently_and_isolate_provider_failures() {
    let settings = settings_registry()
        .unwrap()
        .resolve(
            &Config::default(),
            &BTreeMap::from([("sampling.interval_ms".into(), 450.into())]),
        )
        .unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let barrier = Arc::new((Mutex::new(0), Condvar::new()));
    let mut good = Fixture::new(
        "good",
        Contribution::Collect {
            runtime: runtime("good"),
            containers: vec![Container {
                id: "one".into(),
                name: "fixture".into(),
                state: Some("running".into()),
                cpu_percent: Some(2.0),
                memory_bytes: Some(100),
                ..Container::default()
            }],
        },
    );
    let mut bad = Fixture::new(
        "bad",
        Contribution::Collect {
            runtime: runtime("bad"),
            containers: vec![],
        },
    );
    good.concurrent = Some(barrier.clone());
    bad.concurrent = Some(barrier);
    bad.fail = true;
    let tree = collect(&[&good, &bad], &context).unwrap();
    assert_eq!(*good.seen.lock().unwrap(), [Duration::from_millis(450)]);
    assert_eq!(*bad.seen.lock().unwrap(), [Duration::from_millis(450)]);
    let totals = tree.container_totals.as_ref().unwrap();
    assert_eq!(totals.containers, 1);
    assert!(!totals.inventory_complete);
    assert_eq!(tree.children.len(), 2);
    assert!(!serde_json::to_string(&tree).unwrap().contains("SECRET"));
    assert_eq!(
        tree.children
            .iter()
            .filter_map(|node| node.runtime.as_ref())
            .filter(|runtime| !runtime.unavailable.is_empty())
            .count(),
        1
    );
    good.concurrent = None;
    let duplicate = Fixture::new(
        "duplicate",
        Contribution::Collect {
            runtime: runtime("duplicate"),
            containers: vec![Container::default(), Container::default()],
        },
    );
    assert_eq!(
        collect(&[&duplicate, &good], &context)
            .unwrap()
            .container_totals
            .unwrap()
            .containers,
        1
    );
    cancel.cancel();
    assert!(collect(&[&good], &context).is_err());
}

#[test]
fn classification_rejects_stale_missing_and_wrong_path_proposals_without_duplicate_metrics() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process();
    let valid = ClassificationProposal {
        identity: ProcessIdentity::observed(&process).unwrap(),
        evidence: ClassificationEvidence::AppBundle {
            path: "/Applications/Demo.app".into(),
        },
    };
    let mut stale = valid.clone();
    stale.identity.started_unix_us += 1;
    let mut missing = valid.clone();
    missing.identity.pid = 999;
    let mut path = valid.clone();
    path.evidence = ClassificationEvidence::AppBundle {
        path: "/Applications/Other.app".into(),
    };
    let bad = Fixture::new(
        "bad",
        Contribution::Classify {
            proposals: vec![stale, missing, path],
        },
    );
    let good = Fixture::new(
        "good",
        Contribution::Classify {
            proposals: vec![valid.clone(), valid],
        },
    );
    let tree =
        crate::tree::build_with_providers(vec![process.clone()], &context, &[&bad, &good]).unwrap();
    assert_eq!(tree.totals.processes, 1);
    assert_eq!(tree.totals.resident_bytes, 100);
    assert_eq!(tree.diagnostics.len(), 3);
    assert_eq!(
        crate::tree::select(&tree, "group:applications")
            .unwrap()
            .totals
            .processes,
        1
    );
    crate::tree::validate_references(&tree).unwrap();
    let mut unknown = process;
    unknown.started_unix_us = None;
    let tree = crate::tree::build_with_providers(vec![unknown], &context, &[&good]).unwrap();
    assert!(crate::tree::select(&tree, "group:applications").is_err());
    assert_eq!(tree.diagnostics.len(), 2);
}

#[test]
fn real_classifier_and_validated_details_preserve_schema_and_markdown_redaction() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let mut process = process();
    let raw = ProcessCommand {
        argv: Some(vec!["demo".into(), "--token".into(), "SECRET".into()]),
        entrypoint: Some("SECRET".into()),
        redacted_indices: vec![usize::MAX],
        ..ProcessCommand::default()
    };
    let fixture = Fixture::new(
        "details",
        Contribution::Details {
            proposal: DetailProposal {
                identity: ProcessIdentity::observed(&process).unwrap(),
                command: raw,
            },
        },
    );
    process.command = Some(details(&fixture, &context, &process, 8).unwrap());
    let root =
        crate::tree::build_with_providers(vec![process.clone()], &context, &[&Builtin::AppBundle])
            .unwrap();
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 250,
        system: SystemSummary::default(),
        scope: "pid:10".into(),
        root: crate::tree::select(&root, "pid:10").unwrap(),
        warnings: vec![],
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&json));
    let json = serde_json::to_string(&json).unwrap();
    let markdown = crate::output::markdown(&snapshot);
    for rendered in [&json, &markdown] {
        assert!(rendered.contains("[filtered]"));
        assert!(!rendered.contains("SECRET"));
    }
    assert!(json.contains("app_bundle"));
    let mut stale = process;
    stale.started_unix_us = Some(101);
    assert!(details(&fixture, &context, &stale, 8).is_err());
    assert!(dispatch(&fixture, &context, Request::Collect).is_err());
    let mut expired = Context::new(&settings, &cancel);
    expired.deadline = Instant::now();
    assert!(details(&fixture, &expired, &stale, 8).is_err());
    assert_eq!(fixture.seen.lock().unwrap().len(), 2);
}

#[test]
fn malformed_and_oversized_details_are_rejected_before_rendering() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process();
    for argv in [
        None,
        Some(vec!["x".repeat(16_385)]),
        Some(vec![String::new(); 4097]),
    ] {
        let fixture = Fixture::new(
            "details",
            Contribution::Details {
                proposal: DetailProposal {
                    identity: ProcessIdentity::observed(&process).unwrap(),
                    command: ProcessCommand {
                        argv,
                        ..Default::default()
                    },
                },
            },
        );
        assert!(details(&fixture, &context, &process, 8).is_err());
    }
}

struct Panicking {
    metadata: bool,
    capability: Capability,
    calls: std::sync::atomic::AtomicUsize,
}

impl Extension for Panicking {
    fn descriptor(&self) -> Descriptor {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.metadata {
            panic!("SECRET_DESCRIPTOR_SENTINEL");
        }
        Descriptor {
            id: "panicking",
            version: "1",
            protocol_version: 1,
            source: "builtin",
            platforms: vec!["macos"],
            capabilities: vec![self.capability],
            enabled: true,
            availability: "unknown",
        }
    }
    fn execute(&self, _: &Context<'_>, _: Request<'_>) -> Result<Contribution> {
        panic!("SECRET_EXECUTE_SENTINEL");
    }
}

#[test]
fn boundary_subprocess_scenario() {
    let Ok(mode) = std::env::var("RESWHO_BOUNDARY_SCENARIO") else {
        return;
    };
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process();
    let healthy = Fixture::new(
        "healthy",
        Contribution::Collect {
            runtime: runtime("healthy"),
            containers: vec![],
        },
    );
    for metadata in [true, false] {
        for capability in [
            Capability::Collect,
            Capability::Classify,
            Capability::Details,
        ] {
            let bad = Panicking {
                metadata,
                capability,
                calls: std::sync::atomic::AtomicUsize::new(0),
            };
            match capability {
                Capability::Collect => {
                    let tree = collect(&[&bad, &healthy], &context).unwrap();
                    assert_eq!(tree.children.len(), 2);
                    assert_eq!(
                        tree.children
                            .iter()
                            .filter_map(|node| node.runtime.as_ref())
                            .filter(|runtime| runtime.status == "ready")
                            .count(),
                        1
                    );
                }
                Capability::Classify => {
                    let (accepted, diagnostics) = classify(
                        &[&bad, &Builtin::AppBundle],
                        &context,
                        std::slice::from_ref(&process),
                    );
                    assert_eq!(accepted.len(), 1);
                    assert_eq!(diagnostics.len(), 1);
                }
                Capability::Details => {
                    assert!(details(&bad, &context, &process, 8).is_err());
                    let good = Fixture::new(
                        "healthy",
                        Contribution::Details {
                            proposal: DetailProposal {
                                identity: ProcessIdentity::observed(&process).unwrap(),
                                command: ProcessCommand {
                                    argv: Some(vec!["demo".into()]),
                                    ..Default::default()
                                },
                            },
                        },
                    );
                    assert!(details(&good, &context, &process, 8).is_ok());
                }
            }
            assert_eq!(bad.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    }
    if mode == "outside" {
        panic!("OUTSIDE_BOUNDARY_SENTINEL");
    }
}

#[test]
fn panic_boundaries_keep_siblings_alive_and_suppress_only_extension_panic_hooks() {
    for mode in ["inside", "outside"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "extensions::tests::boundary_subprocess_scenario",
                "--nocapture",
            ])
            .env("RESWHO_BOUNDARY_SCENARIO", mode)
            .output()
            .unwrap();
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!output.contains("SECRET_"), "{mode}: {output}");
        assert_eq!(result.status.success(), mode == "inside", "{output}");
        assert_eq!(
            output.contains("OUTSIDE_BOUNDARY_SENTINEL"),
            mode == "outside"
        );
    }
}

#[test]
fn argv_and_unavailable_cannot_reach_a_snapshot_together() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process();
    let fixture = Fixture::new(
        "details",
        Contribution::Details {
            proposal: DetailProposal {
                identity: ProcessIdentity::observed(&process).unwrap(),
                command: ProcessCommand {
                    argv: Some(vec!["demo".into()]),
                    unavailable: vec![Diagnostic::new(
                        "invalid_data",
                        "fixture",
                        "SECRET_DETAIL_SENTINEL",
                    )],
                    ..Default::default()
                },
            },
        },
    );
    let result = details(&fixture, &context, &process, 8);
    assert!(result.is_err());
    assert!(
        !result
            .err()
            .unwrap()
            .to_string()
            .contains("SECRET_DETAIL_SENTINEL")
    );
    let root = crate::tree::build(vec![process]).unwrap();
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 250,
        system: SystemSummary::default(),
        scope: "pid:10".into(),
        root: crate::tree::select(&root, "pid:10").unwrap(),
        warnings: vec![],
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&json));
    assert!(
        !serde_json::to_string(&json)
            .unwrap()
            .contains("SECRET_DETAIL_SENTINEL")
    );
}

#[test]
fn every_capability_rejects_expired_admission_and_late_results() {
    struct Late {
        output: Contribution,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Extension for Late {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: "late",
                version: "1",
                protocol_version: 1,
                source: "builtin",
                platforms: vec!["macos"],
                capabilities: vec![self.output.capability()],
                enabled: true,
                availability: "unknown",
            }
        }
        fn execute(&self, context: &Context<'_>, _: Request<'_>) -> Result<Contribution> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::thread::sleep(
                context.deadline.saturating_duration_since(Instant::now())
                    + Duration::from_millis(1),
            );
            Ok(self.output.clone())
        }
    }
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let process = process();
    let outputs = [
        Contribution::Collect {
            runtime: runtime("late"),
            containers: vec![],
        },
        Contribution::Classify { proposals: vec![] },
        Contribution::Details {
            proposal: DetailProposal {
                identity: ProcessIdentity::observed(&process).unwrap(),
                command: ProcessCommand {
                    argv: Some(vec!["demo".into()]),
                    ..Default::default()
                },
            },
        },
    ];
    for output in outputs {
        let capability = output.capability();
        let late = Late {
            output,
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        for expired in [true, false] {
            let context = Context {
                settings: &settings,
                cancel: &cancel,
                deadline: Instant::now()
                    + if expired {
                        Duration::ZERO
                    } else {
                        Duration::from_millis(5)
                    },
            };
            let request = match capability {
                Capability::Collect => Request::Collect,
                Capability::Classify => Request::Classify(std::slice::from_ref(&process)),
                Capability::Details => Request::Details {
                    process: &process,
                    pointer_width: 8,
                },
            };
            let result = dispatch(&late, &context, request);
            assert!(result.is_err());
            assert!(result.err().unwrap().to_string().contains("deadline"));
            assert_eq!(
                late.calls.load(std::sync::atomic::Ordering::Relaxed),
                usize::from(!expired)
            );
        }
    }
}

#[test]
fn failed_metadata_and_hyphenated_provider_operations_validate_as_full_snapshots() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for capability in [Capability::Collect, Capability::Classify] {
        let bad_metadata = Panicking {
            metadata: true,
            capability,
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let output = match capability {
            Capability::Collect => Contribution::Collect {
                runtime: runtime("my-extension"),
                containers: vec![],
            },
            _ => Contribution::Classify { proposals: vec![] },
        };
        let mut hyphenated = Fixture::new("my-extension", output);
        hyphenated.fail = true;
        let mut root = crate::tree::build(vec![process()]).unwrap();
        if capability == Capability::Collect {
            root.children
                .push(collect(&[&bad_metadata, &hyphenated], &context).unwrap());
        } else {
            root.diagnostics = classify(&[&bad_metadata, &hyphenated], &context, &[process()]).1;
            assert!(
                root.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("my-extension"))
            );
        }
        let snapshot = Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            captured_unix_ms: 0,
            sample_ms: 250,
            collection_ms: 250,
            system: SystemSummary::default(),
            scope: "group:all".into(),
            root,
            warnings: vec![],
        };
        let json = serde_json::to_value(snapshot).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&json)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
