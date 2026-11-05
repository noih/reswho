use super::*;

#[test]
fn builtin_list_is_deterministic_and_does_not_probe_sources() {
    let settings = default_settings().unwrap();
    let list = list(&settings, false);
    assert_eq!(
        list.iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [
            "project",
            "ancestor",
            "versioned_install",
            "node",
            "python",
            "launched_by",
            "docker",
            "podman",
            "apple",
            "vm"
        ]
    );
    assert!(list.iter().all(|entry| entry.availability == "unknown"));
    assert!(
        list.iter()
            .filter(|entry| entry.capabilities.contains(&Capability::Collect))
            .all(|entry| !entry.enabled)
    );
    assert!(settings.report().disabled_extensions.is_empty());
}

use crate::model::{Snapshot, SystemSummary};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) struct Fixture {
    id: &'static str,
    contribution: Contribution,
    fail: bool,
    seen: Arc<Mutex<Vec<Duration>>>,
    concurrent: Option<Arc<(Mutex<usize>, Condvar)>>,
}

impl Fixture {
    pub(crate) fn new(id: &'static str, contribution: Contribution) -> Self {
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
    fn collects_containers(&self) -> bool {
        matches!(
            self.contribution,
            Contribution::Collect { .. }
                | Contribution::Collected {
                    runtime: Some(_),
                    ..
                }
        )
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.id.into(),
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
        metrics_scope: None,
        host_scope: None,
        configuration: None,
        name: name.into(),
        context: None,
        endpoint_ref: Some(format!("{name}-local")),
        daemon_id: None,
        status: "available".into(),
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
                runtime: "good".into(),
                endpoint_ref: "good-local".into(),
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
    assert_eq!(tree.diagnostics.len(), 1);
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
    assert_eq!(tree.diagnostics.len(), 1);
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
    let root = crate::tree::build_with_providers(
        vec![process.clone()],
        &context,
        &[&crate::core::Core::AppBundle],
    )
    .unwrap();
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
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
            id: "panicking".into(),
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
                            .filter(|runtime| runtime.status == "available")
                            .count(),
                        1
                    );
                }
                Capability::Classify => {
                    let (accepted, _, diagnostics) = classify(
                        &[&bad, &crate::core::Core::AppBundle],
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
        extensions: vec![],
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
                id: "late".into(),
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
                report: std::sync::Arc::default(),
                settings: &settings,
                cancel: &cancel,
                failed: Mutex::new(HashSet::new()),
                deadline: Instant::now()
                    + if expired {
                        Duration::ZERO
                    } else {
                        Duration::from_millis(5)
                    },
                argv: Default::default(),
            };
            let facts = crate::extensions::host::Facts::new(std::slice::from_ref(&process));
            let request = match capability {
                Capability::Collect => Request::Collect,
                Capability::Classify => Request::Classify(&facts),
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
            root.diagnostics = classify(&[&bad_metadata, &hyphenated], &context, &[process()]).2;
            assert!(
                root.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("my-extension"))
            );
        }
        let healthy = Fixture::new(
            "unidentified-0",
            Contribution::Collect {
                runtime: runtime("healthy"),
                containers: vec![],
            },
        );
        assert!(dispatch(&healthy, &context, Request::Collect).is_ok());
        let snapshot = Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
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

#[test]
fn unrepresentable_container_aggregate_rejects_only_the_overflowing_source() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let good = Fixture::new(
        "good",
        Contribution::Collect {
            runtime: runtime("good"),
            containers: vec![Container {
                runtime: "good".into(),
                endpoint_ref: "good-local".into(),
                id: "good".into(),
                state: Some("running".into()),
                memory_bytes: Some(100),
                ..Default::default()
            }],
        },
    );
    let overflow = Fixture::new(
        "overflow",
        Contribution::Collect {
            runtime: runtime("overflow"),
            containers: vec![Container {
                runtime: "overflow".into(),
                endpoint_ref: "overflow-local".into(),
                id: "huge".into(),
                state: Some("running".into()),
                memory_bytes: Some(u64::MAX),
                ..Default::default()
            }],
        },
    );
    crate::extensions::collection::validate("good", &good.contribution).unwrap();
    crate::extensions::collection::validate("overflow", &overflow.contribution).unwrap();
    let root = collect(&[&good, &overflow], &context).unwrap();
    let totals = root.container_totals.unwrap();
    assert_eq!(totals.containers, 1);
    assert_eq!(totals.known_memory_bytes, 100);
    assert!(!totals.inventory_complete);
}

#[test]
fn capability_validation_failures_skip_later_calls_only_in_the_current_round() {
    struct Invalid {
        kind: &'static str,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Extension for Invalid {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: "invalid".into(),
                capabilities: vec![
                    Capability::Collect,
                    Capability::Classify,
                    Capability::Details,
                ],
                enabled: true,
                availability: "unknown",
            }
        }
        fn execute(&self, _: &Context<'_>, request: Request<'_>) -> Result<Contribution> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match request {
                Request::Details { process, .. } => {
                    if self.kind == "parsed" {
                        return Ok(Contribution::ParsedDetails {
                            identity: ProcessIdentity::observed(process).unwrap(),
                            fields: vec![crate::model::DetailField {
                                key: "extensions.other.field".into(),
                                label: "invalid".into(),
                                value: serde_json::json!("value"),
                                sensitive: false,
                            }],
                        });
                    }
                    let mut identity = ProcessIdentity::observed(process).unwrap();
                    if self.kind == "identity" {
                        identity.started_unix_us += 1;
                    }
                    let mut command = ProcessCommand {
                        argv: Some(vec!["demo".into()]),
                        ..Default::default()
                    };
                    if self.kind == "data" {
                        command.unavailable.push(Diagnostic::new(
                            "invalid_data",
                            "fixture",
                            "SECRET_VALIDATION_SENTINEL",
                        ));
                    }
                    Ok(Contribution::Details {
                        proposal: DetailProposal { identity, command },
                    })
                }
                Request::Collect | Request::CollectResources => Ok(Contribution::Collect {
                    runtime: runtime("invalid"),
                    containers: if self.kind == "overflow" {
                        vec![Container {
                            runtime: "invalid".into(),
                            endpoint_ref: "invalid-local".into(),
                            id: "huge".into(),
                            state: Some("running".into()),
                            memory_bytes: Some(u64::MAX),
                            ..Default::default()
                        }]
                    } else {
                        vec![Container::default(), Container::default()]
                    },
                }),
                Request::Classify(_) => Ok(Contribution::Classify {
                    proposals: if self.kind == "classify" {
                        vec![ClassificationProposal {
                            identity: ProcessIdentity {
                                pid: 999,
                                started_unix_us: 123,
                            },
                            evidence: ClassificationEvidence::AppBundle {
                                path: "/wrong.app".into(),
                            },
                        }]
                    } else {
                        vec![]
                    },
                }),
            }
        }
    }
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let process = process();
    for kind in [
        "identity", "data", "collect", "classify", "overflow", "parsed",
    ] {
        let invalid = Invalid {
            kind,
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let context = Context::new(&settings, &cancel);
        match kind {
            "parsed" => {
                let mut selected = process.clone();
                assert_eq!(
                    details::attach_details(&[&invalid], &context, &mut selected).len(),
                    1
                );
            }
            "overflow" => {
                let prefix = Fixture::new(
                    "prefix",
                    Contribution::Collect {
                        runtime: runtime("prefix"),
                        containers: vec![Container {
                            runtime: "prefix".into(),
                            endpoint_ref: "prefix-local".into(),
                            id: "small".into(),
                            state: Some("running".into()),
                            memory_bytes: Some(100),
                            ..Default::default()
                        }],
                    },
                );
                collect(&[&prefix, &invalid], &context).unwrap();
            }
            "collect" => {
                collect(&[&invalid], &context).unwrap();
            }
            "classify" => {
                assert_eq!(
                    classify(&[&invalid], &context, std::slice::from_ref(&process))
                        .2
                        .len(),
                    1
                );
            }
            _ => {
                let error = details(&invalid, &context, &process, 8).err().unwrap();
                assert!(!error.to_string().contains("SECRET"));
            }
        }
        let healthy = Fixture::new("healthy", Contribution::Classify { proposals: vec![] });
        assert_eq!(
            classify(
                &[&invalid, &healthy],
                &context,
                std::slice::from_ref(&process)
            )
            .2
            .len(),
            1
        );
        assert_eq!(
            invalid.calls.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "{kind}"
        );
        assert_eq!(healthy.seen.lock().unwrap().len(), 1);
        let fresh = Context::new(&settings, &cancel);
        let _ = classify(&[&invalid], &fresh, std::slice::from_ref(&process));
        assert_eq!(
            invalid.calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "{kind}"
        );
    }
}

#[test]
fn current_phase_reports_acceptance_failures_skips_and_parallel_durations() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process();
    let invalid = Fixture::new(
        "invalid",
        Contribution::Classify {
            proposals: vec![ClassificationProposal {
                identity: ProcessIdentity::observed(&process).unwrap(),
                evidence: ClassificationEvidence::AppBundle {
                    path: "/Wrong.app".into(),
                },
            }],
        },
    );
    let (_, _, errors) = classify(&[&invalid], &context, std::slice::from_ref(&process));
    assert_eq!(errors.len(), 1);
    let records = context.report.lock().unwrap().clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].result, "failed");
    assert_eq!(records[0].reason.as_deref(), Some("invalid_data"));
    assert!(records[0].duration_ms.is_some());
    assert!(
        dispatch(
            &invalid,
            &context,
            Request::Classify(&crate::extensions::host::Facts::new(&[]))
        )
        .is_err()
    );
    let records = context.report.lock().unwrap().clone();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].result, "skipped");
    assert!(records[1].duration_ms.is_none());
    let barrier = Arc::new((Mutex::new(0), Condvar::new()));
    let mut good = Fixture::new("one", Contribution::Classify { proposals: vec![] });
    let mut bad = Fixture::new("two", Contribution::Classify { proposals: vec![] });
    good.concurrent = Some(barrier.clone());
    bad.concurrent = Some(barrier);
    bad.fail = true;
    let fresh = Context::new(&settings, &cancel);
    let (_, _, errors) = classify(&[&good, &bad], &fresh, &[]);
    assert_eq!(errors.len(), 1);
    let records = fresh.report.lock().unwrap().clone();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|record| record.duration_ms.is_some_and(|ms| ms >= 0.0))
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.result == "success")
            .count(),
        1
    );
    assert_eq!(good.seen.lock().unwrap().len(), 1);
    assert_eq!(bad.seen.lock().unwrap().len(), 1);
    assert!(!serde_json::to_string(&records).unwrap().contains("SECRET"));
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: records,
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 1,
        system: SystemSummary::default(),
        scope: "group:all".into(),
        warnings: vec![],
        root: crate::tree::build(vec![]).unwrap(),
    };
    let text = crate::output::markdown(&snapshot);
    assert!(text.contains("Extension two / classify"));
    assert!(!text.contains("Extension timings"));
    assert!(crate::output::extension_profile(&snapshot.extensions).contains("Duration ms"));
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
    snapshot.extensions[0].duration_ms = Some(-1.0);
    assert!(
        !jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&snapshot).unwrap())
    );
    struct Canceled;
    impl Extension for Canceled {
        fn descriptor(&self) -> Descriptor {
            Fixture::new("cancel", Contribution::Classify { proposals: vec![] }).descriptor()
        }
        fn execute(&self, context: &Context<'_>, _: Request<'_>) -> Result<Contribution> {
            context.cancel.cancel();
            Ok(Contribution::Classify { proposals: vec![] })
        }
    }
    let next = Context::new(&settings, &cancel);
    assert!(
        dispatch(
            &Canceled,
            &next,
            Request::Classify(&crate::extensions::host::Facts::new(&[]))
        )
        .is_err()
    );
    let record = next.report.lock().unwrap()[0].clone();
    assert_eq!(record.result, "failed");
    assert_eq!(record.reason.as_deref(), Some("canceled"));
    assert!(record.duration_ms.is_some());
}

#[test]
fn phase_validation_preserves_independent_provider_timings() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let started = Instant::now();
    let timings = [
        ("first", "classify", Some(1.0)),
        ("sibling", "classify", Some(150.0)),
        ("later_batch", "classify", Some(300.0)),
        ("first", "details", Some(20.0)),
        ("disabled", "classify", None),
    ];
    *context.report.lock().unwrap() = timings
        .iter()
        .map(|&(provider, phase, duration_ms)| report::Record {
            provider: provider.into(),
            phase: phase.into(),
            result: if duration_ms.is_some() {
                "success"
            } else {
                "skipped"
            }
            .into(),
            reason: None,
            duration_ms,
            argv: None,
            started: duration_ms.map(|_| started),
        })
        .collect();

    for (provider, phase, validation_ms) in [
        ("first", "classify", 2),
        ("later_batch", "classify", 7),
        ("first", "details", 4),
        ("disabled", "classify", 9),
    ] {
        context
            .finish_phase(
                Some(provider),
                phase,
                Duration::from_millis(validation_ms),
                Ok(()),
            )
            .unwrap();
    }
    let records = context.report.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.duration_ms)
            .collect::<Vec<_>>(),
        [Some(3.0), Some(150.0), Some(307.0), Some(24.0), None],
        "{records:?}"
    );
}

#[test]
fn supplemental_roles_coexist_on_apps_projects_without_reparenting_or_new_totals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("go.mod"), "module fixture").unwrap();
    let mut app = process();
    app.cwd = Some(dir.path().into());
    app.uid = Some(501);
    let mut project = app.clone();
    project.pid = 11;
    project.executable = Some("/tmp/tool".into());
    let processes = vec![app, project];
    let role = |process: &Process, label: &str| ClassificationProposal {
        identity: ProcessIdentity::observed(process).unwrap(),
        evidence: ClassificationEvidence::Role {
            key: "worker".into(),
            label: label.into(),
            path: process.cwd.clone().unwrap(),
            basis: "cwd".into(),
        },
    };
    let first = Fixture::new(
        "first",
        Contribution::Classify {
            proposals: processes
                .iter()
                .map(|process| role(process, "First worker"))
                .collect(),
        },
    );
    let second = Fixture::new(
        "second",
        Contribution::Classify {
            proposals: processes
                .iter()
                .map(|process| role(process, "Second worker"))
                .collect(),
        },
    );
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let original = crate::tree::build_with_providers(
        processes.clone(),
        &context,
        &[&crate::core::Core::AppBundle],
    )
    .unwrap();
    let root = crate::tree::build_with_providers(
        processes.clone(),
        &context,
        &[&crate::core::Core::AppBundle, &first, &second],
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&root.totals).unwrap(),
        serde_json::to_value(&original.totals).unwrap()
    );
    for pid in [10, 11] {
        let selected = crate::tree::select(&root, &format!("pid:{pid}")).unwrap();
        let old = crate::tree::select(&original, &format!("pid:{pid}")).unwrap();
        assert_eq!(selected.evidence, old.evidence);
        assert_eq!(
            serde_json::to_value(&selected.attribution).unwrap(),
            serde_json::to_value(&old.attribution).unwrap()
        );
        let roles: Vec<_> = selected
            .process
            .as_ref()
            .unwrap()
            .tag_report
            .as_ref()
            .unwrap()
            .effective
            .iter()
            .filter(|entry| entry.tag.dimension == "role")
            .collect();
        assert_eq!(
            roles
                .iter()
                .map(|role| role.source.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(roles[0].tag.path.as_deref(), Some(dir.path()));
        let snapshot = Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 0,
            sample_ms: 1,
            collection_ms: 1,
            system: SystemSummary::default(),
            scope: selected.reference.clone(),
            warnings: vec![],
            root: selected,
        };
        let md = crate::output::markdown(&snapshot);
        assert!(md.contains("First worker") && md.contains("Second worker"));
    }
    for invalid in ["stale", "basis", "path"] {
        let mut proposal = role(&processes[0], "Invalid");
        match invalid {
            "stale" => proposal.identity.started_unix_us += 1,
            "basis" => {
                if let ClassificationEvidence::Role { basis, .. } = &mut proposal.evidence {
                    *basis = "command".into();
                }
            }
            _ => {
                if let ClassificationEvidence::Role { path, .. } = &mut proposal.evidence {
                    *path = "/elsewhere".into();
                }
            }
        }
        let bad = Fixture::new(
            "bad",
            Contribution::Classify {
                proposals: vec![proposal],
            },
        );
        let fresh = Context::new(&settings, &cancel);
        let (_, _, errors, roles) = classify_with_roles(&[&bad], &fresh, &processes);
        assert!(roles.is_empty());
        assert!(!errors.is_empty());
    }
    dir.close().unwrap();
}

#[test]
fn role_limit_rejects_entire_offending_source_and_retains_healthy_sources() {
    let process = process();
    let role = |key: String| ClassificationProposal {
        identity: ProcessIdentity::observed(&process).unwrap(),
        evidence: ClassificationEvidence::Role {
            key,
            label: "Worker".into(),
            path: process.executable.clone().unwrap(),
            basis: "executable".into(),
        },
    };
    let full = Fixture::new(
        "full",
        Contribution::Classify {
            proposals: (0..MAX_ROLES_PER_PROCESS)
                .map(|index| role(index.to_string()))
                .collect(),
        },
    );
    let mut fallback = role("fallback".into());
    fallback.evidence = ClassificationEvidence::Custom {
        key: "fallback".into(),
        label: "Unaccepted fallback".into(),
        path: process.executable.clone().unwrap(),
        basis: "executable".into(),
    };
    let overflow = Fixture::new(
        "overflow",
        Contribution::Classify {
            proposals: vec![fallback, role("extra".into())],
        },
    );
    let healthy = Fixture::new(
        "healthy",
        Contribution::Classify {
            proposals: vec![ClassificationProposal {
                identity: ProcessIdentity::observed(&process).unwrap(),
                evidence: ClassificationEvidence::AppBundle {
                    path: "/Applications/Demo.app".into(),
                },
            }],
        },
    );
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let (apps, custom, errors, roles) = classify_with_roles(
        &[&full, &overflow, &healthy],
        &context,
        std::slice::from_ref(&process),
    );
    assert_eq!(roles[&process.pid].len(), MAX_ROLES_PER_PROCESS);
    assert!(
        roles[&process.pid]
            .iter()
            .all(|role| role.provider == "full")
    );
    assert!(custom.is_empty());
    assert!(apps.contains_key(&process.pid));
    assert_eq!(errors.len(), 1);
    assert!(context.failed.lock().unwrap().contains("overflow"));
    let records = context.report.lock().unwrap();
    assert!(records.iter().any(|record| record.provider == "overflow"
        && record.result == "failed"
        && record.reason.as_deref() == Some("invalid_data")));
    assert!(
        records
            .iter()
            .any(|record| record.provider == "healthy" && record.result == "success")
    );
}

#[test]
fn repair_failed_collectors_only_invalidate_their_requested_scopes() {
    for (dual, include_containers) in [(false, true), (true, true), (true, false)] {
        let healthy = Fixture::new(
            "healthy",
            Contribution::Collect {
                runtime: runtime("healthy"),
                containers: vec![Container {
                    runtime: "healthy".into(),
                    endpoint_ref: "healthy-local".into(),
                    id: "owned".into(),
                    name: "healthy".into(),
                    state: Some("running".into()),
                    cpu_percent: Some(2.0),
                    memory_bytes: Some(100),
                    ..Default::default()
                }],
            },
        );
        let mut failing = Fixture::new(
            "failing",
            Contribution::Collected {
                runtime: dual.then(|| runtime("failing")),
                containers: vec![],
                resources: vec![],
            },
        );
        failing.fail = true;
        let settings = default_settings().unwrap();
        let cancel = Cancellation::default();
        let context = Context::new(&settings, &cancel);
        let providers: Vec<&dyn Extension> = if include_containers {
            vec![&healthy, &failing]
        } else {
            vec![&failing]
        };
        let (containers, resources) =
            collect_scoped(&providers, &context, include_containers).unwrap();
        assert_eq!(
            containers.children.len(),
            usize::from(include_containers) + usize::from(dual && include_containers)
        );
        let totals = containers.container_totals.as_ref().unwrap();
        assert_eq!(totals.inventory_complete, !dual || !include_containers);
        assert_eq!(
            totals.known_memory_bytes,
            if include_containers { 100 } else { 0 }
        );
        assert!(resources.children.is_empty());
        assert!(
            context
                .report
                .lock()
                .unwrap()
                .iter()
                .any(|record| record.provider == "failing" && record.result == "failed")
        );
    }
}

#[test]
fn tree_provider_metadata_panics_keep_core_processes_and_diagnostics() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let bad = Panicking {
        metadata: true,
        capability: Capability::Classify,
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let tree = crate::tree::build_with_providers(
        vec![process()],
        &Context::new(&settings, &cancel),
        &[&bad],
    )
    .unwrap();
    assert_eq!(tree.totals.processes, 1);
    assert_eq!(tree.diagnostics.len(), 1);
    assert!(crate::tree::select(&tree, "group:applications").is_ok());
}

#[test]
fn internal_resources_keep_precise_values_and_isolate_invalid_providers() {
    use crate::model::{Resource, ResourceMetric};
    let resource = Resource {
        provider: "fixture".into(),
        id: "volume".into(),
        kind: "volume".into(),
        label: "Volume".into(),
        path: None,
        metrics: vec![ResourceMetric {
            key: "capacity".into(),
            label: "Capacity".into(),
            unit: "bytes".into(),
            value: Some(u64::MAX.into()),
        }],
    };
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let fixture = Fixture::new(
        "fixture",
        Contribution::Collected {
            runtime: None,
            containers: vec![],
            resources: vec![resource.clone()],
        },
    );
    let (_, root) = collect_scoped(&[&fixture], &Context::new(&settings, &cancel), false).unwrap();
    crate::tree::validate_references(&root).unwrap();
    assert_eq!(root.totals.processes, 0);
    let json = serde_json::to_value(&root).unwrap();
    assert_eq!(
        json["children"][0]["children"][0]["resource"]["metrics"][0]["value"],
        u64::MAX
    );
    assert!(
        json["children"][0]["children"][0]["resource"]
            .get("version")
            .is_none()
    );
    for invalid in ["owner", "duplicate", "relative", "metric"] {
        let mut resources = vec![resource.clone()];
        match invalid {
            "owner" => resources[0].provider = "other".into(),
            "duplicate" => resources.push(resource.clone()),
            "relative" => resources[0].path = Some("relative".into()),
            _ => {
                let duplicate = resources[0].metrics[0].clone();
                resources[0].metrics.push(duplicate);
            }
        }
        let bad = Fixture::new(
            "fixture",
            Contribution::Collected {
                runtime: None,
                containers: vec![],
                resources,
            },
        );
        let healthy = Fixture::new(
            "healthy",
            Contribution::Collected {
                runtime: None,
                containers: vec![],
                resources: vec![Resource {
                    provider: "healthy".into(),
                    ..resource.clone()
                }],
            },
        );
        let context = Context::new(&settings, &cancel);
        let (_, root) = collect_scoped(&[&bad, &healthy], &context, false).unwrap();
        crate::tree::validate_references(&root).unwrap();
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].name, "healthy");
        assert!(
            context
                .report
                .lock()
                .unwrap()
                .iter()
                .any(|record| record.provider == "fixture" && record.result == "failed")
        );
    }
}

#[test]
fn internal_parsed_details_mask_sensitive_fields_and_keep_integer_precision() {
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let mut process = process();
    let fields = vec![
        crate::model::DetailField {
            key: "extensions.fixture.token".into(),
            label: "Token".into(),
            value: serde_json::json!("SECRET_TOKEN"),
            sensitive: false,
        },
        crate::model::DetailField {
            key: "extensions.fixture.private".into(),
            label: "Private".into(),
            value: serde_json::json!("SECRET_PRIVATE"),
            sensitive: true,
        },
        crate::model::DetailField {
            key: "extensions.fixture.count".into(),
            label: "Count".into(),
            value: serde_json::json!(u64::MAX),
            sensitive: false,
        },
    ];
    let fixture = Fixture::new(
        "fixture",
        Contribution::ParsedDetails {
            identity: ProcessIdentity::observed(&process).unwrap(),
            fields,
        },
    );
    assert!(
        details::attach_details(&[&fixture], &Context::new(&settings, &cancel), &mut process)
            .is_empty()
    );
    assert_eq!(process.extension_details[0].fields[2].value, u64::MAX);
    let root =
        crate::tree::build_with_providers(vec![process], &Context::new(&settings, &cancel), &[])
            .unwrap();
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 0,
        sample_ms: 0,
        collection_ms: 0,
        system: SystemSummary::default(),
        warnings: vec![],
        extensions: vec![],
        scope: "pid:10".into(),
        root: crate::tree::select(&root, "pid:10").unwrap(),
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&json));
    for rendered in [json.to_string(), crate::output::markdown(&snapshot)] {
        assert!(rendered.contains("[filtered]"));
        assert!(!rendered.contains("SECRET"));
    }
}
