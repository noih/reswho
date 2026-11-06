use super::*;
use crate::{
    extensions::{
        self,
        tags::{self, Effective, Operation, Recorded, Report},
    },
    model::{Grouping, Snapshot},
    snapshot::Cancellation,
    tree,
};

fn fixture() -> (tempfile::TempDir, Vec<Process>) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    for name in ["alpha", "beta"] {
        std::fs::create_dir(path.join(name)).unwrap();
        std::fs::write(
            path.join(name).join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.0.0\"\n",
        )
        .unwrap();
    }
    let processes = [(10, 1, "alpha"), (20, 10, "beta"), (30, 1, "none")]
        .into_iter()
        .map(|(pid, ppid, project)| Process {
            pid,
            ppid: Some(ppid),
            uid: Some(unsafe { libc::geteuid() }),
            started_unix_us: Some(u64::from(pid)),
            name: "helper".into(),
            executable: Some("/Applications/Fixture.app/Contents/MacOS/fixture".into()),
            cwd: Some(if project == "none" {
                "/".into()
            } else {
                path.join(project)
            }),
            cpu_percent: Some(1.0),
            resident_bytes: Some(100),
            ..Default::default()
        })
        .collect();
    (root, processes)
}

fn structure(node: &Node) -> serde_json::Value {
    serde_json::json!({"ref":node.reference,"name":node.name,"grouping":node.grouping,"totals":node.totals,
        "pids":node.process.as_ref().map(|p|p.pid),"children":node.children.iter().map(structure).collect::<Vec<_>>()})
}

#[test]
fn project_category_identifies_cwd_association_without_claiming_exclusivity() {
    let (_root, mut processes) = fixture();
    for process in &mut processes {
        process.executable = Some("/opt/shared-server".into());
    }
    let root = tree::build(processes).unwrap();
    let category = root
        .children
        .iter()
        .find(|node| node.reference == "group:projects")
        .unwrap();
    assert_eq!(category.name, "Project associations");
    assert_eq!(root.totals.processes, 3);
    let server = tree::select(&root, "pid:10").unwrap();
    assert!(
        server
            .hints
            .iter()
            .any(|hint| hint.source == "project" && hint.basis == "cwd")
    );
    assert_eq!(server.process.as_ref().unwrap().name, "helper");
    assert!(
        !server
            .hints
            .iter()
            .any(|hint| matches!(hint.kind.as_str(), "idle" | "shared" | "releasable"))
    );
}

#[test]
fn mixed_app_group_keeps_distinct_project_coverage_without_changing_tree_or_totals() {
    let (root, processes) = fixture();
    let mut settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let enabled = tree::build_with_context(
        processes.clone(),
        &extensions::Context::new(&settings, &cancel),
    )
    .unwrap();
    assert!(enabled.diagnostics.is_empty(), "{:?}", enabled.diagnostics);
    let app = enabled
        .children
        .iter()
        .flat_map(|category| &category.children)
        .find(|node| node.grouping == Some(Grouping::App))
        .unwrap();
    assert_eq!(app.hints.len(), 2);
    assert_eq!(app.totals.processes, 3);
    assert!(
        app.hints
            .iter()
            .all(|hint| hint.processes == 1 && hint.source == "project" && hint.basis == "cwd")
    );
    let parent = tree::select(&enabled, "pid:10").unwrap();
    assert_eq!(parent.hints.len(), 1);
    assert_eq!(parent.hints[0].value, "alpha");
    assert_eq!(parent.totals.processes, 2);
    settings.set_builtin_enabled("project", false).unwrap();
    let disabled =
        tree::build_with_context(processes, &extensions::Context::new(&settings, &cancel)).unwrap();
    assert!(disabled.hints.is_empty());
    assert_eq!(structure(&enabled), structure(&disabled));
    tree::validate_references(&enabled).unwrap();
    let app_reference = app.reference.clone();
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 1,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        extensions: vec![],
        warnings: vec![],
        scope: "group:all".into(),
        root: enabled,
    };
    let markdown = crate::output::markdown(&snapshot);
    assert!(markdown.contains("Project via cwd: alpha"), "{markdown}");
    assert!(markdown.contains("Project via cwd: beta"));
    assert!(!markdown.contains("1/3 processes"));
    let mut selected = snapshot.clone();
    selected.root = tree::select(&snapshot.root, &app_reference).unwrap();
    selected.scope = app_reference;
    let details = crate::output::markdown_with_depth(&selected, 0);
    assert!(details.contains(r"project \[project · cwd\]: alpha"));
    assert!(details.contains("1/3 processes"));
    let value = serde_json::to_value(&snapshot).unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    let projected = &value["selections"][0]["root"];
    assert!(
        !serde_json::to_string(projected)
            .unwrap()
            .contains("tag_report")
    );
    assert!(
        serde_json::to_string(projected)
            .unwrap()
            .contains("\"hints\"")
    );
    root.close().unwrap();
}

fn record(process: &Process, source: &str, basis: &str) -> Recorded {
    let proposal = proposal(
        source,
        value("role", "Observed worker"),
        basis,
        None,
        vec![ProcessEvidence {
            identity: ProcessIdentity::observed(process).unwrap(),
            executable: process.executable.clone().unwrap(),
        }],
    )
    .unwrap();
    let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
        panic!()
    };
    Recorded {
        source: source.into(),
        proposal,
    }
}

#[test]
fn summary_uses_only_effective_explicit_hints_and_never_rewrites_their_source_basis() {
    let (root, processes) = fixture();
    let process = &processes[0];
    let first = record(process, "project", "arguments");
    let mut exclude = first.clone();
    exclude.source = "replacement".into();
    exclude.proposal.id = "exclude-project-role".into();
    exclude.proposal.operation = Operation::Exclude {
        selector: tags::Selector {
            allowed_sources: vec![],
            dimension: "extensions.project.role".into(),
            tag_id: None,
            source: Some("project".into()),
        },
    };
    let higher = record(process, "replacement", "arguments");
    let report = tags::resolve(
        vec![first, exclude, higher],
        &["replacement".into(), "project".into()],
    );
    assert_eq!(report.effective.len(), 1);
    let mut node = Node::group("fixture".into(), "fixture".into(), vec![], Grouping::App);
    let mut process = process.clone();
    process.tag_report = Some(report);
    node.process = Some(Box::new(process));
    summarize(&mut node);
    assert_eq!(node.hints[0].source, "replacement");
    assert_eq!(node.hints[0].basis, "arguments");
    let process = node.process.as_mut().unwrap();
    let mut plain = record(process, "project", "arguments");
    let Operation::Add { tag } = &mut plain.proposal.operation else {
        panic!()
    };
    tag.hint = None;
    process.tag_report = Some(Report {
        effective: vec![Effective {
            source: "project".into(),
            tag: tag.clone(),
        }],
        ..Default::default()
    });
    summarize(&mut node);
    assert!(node.hints.is_empty());
    root.close().unwrap();
}

#[test]
fn explicit_hint_validation_rejects_control_text_forged_identity_and_cwd_ancestry() {
    let (root, processes) = fixture();
    let base = record(&processes[0], "fixture", "arguments");
    tags::validate(
        std::slice::from_ref(&base.proposal),
        &processes,
        "fixture",
        None,
    )
    .unwrap();
    let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let mut proposal_schema = schema["$defs"]["tagProposal"].clone();
    proposal_schema["$defs"] = schema["$defs"].clone();
    let validator = jsonschema::validator_for(&proposal_schema).unwrap();
    let wire = serde_json::to_value(&base.proposal).unwrap();
    assert!(validator.is_valid(&wire), "{wire}");
    assert!(wire["change"]["tag"].get("evidence_chain").is_none());
    assert_eq!(
        wire["change"]["tag"]["hint"]["evidence_chain"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    for case in 0..7 {
        let mut record = base.clone();
        let Operation::Add { tag } = &mut record.proposal.operation else {
            panic!()
        };
        match case {
            0 => tag.hint.as_mut().unwrap().value = "bad\u{1b}".into(),
            1 => tag.hint.as_mut().unwrap().value = "bad\u{202e}".into(),
            2 => {
                tag.hint.as_mut().unwrap().evidence_chain[0]
                    .identity
                    .started_unix_us += 1
            }
            3 => tag.basis = "ancestor_arguments".into(),
            4 => tag.path = Some("/unobserved".into()),
            5 => tag.hint.as_mut().unwrap().evidence_chain.clear(),
            _ => {
                tag.hint.as_mut().unwrap().title = Some("not a session".into());
            }
        }
        assert!(
            tags::validate(&[record.proposal], &processes, "fixture", None).is_err(),
            "case {case}"
        );
    }
    root.close().unwrap();
}

#[test]
fn multiple_session_logs_and_refresh_never_duplicate_usage_or_keep_old_titles() {
    let (root, processes) = fixture();
    let mut node = Node::group("fixture".into(), "fixture".into(), vec![], Grouping::App);
    let mut process = processes[0].clone();
    let mut report = Report::default();
    for (id, title) in [("first", "Manual title"), ("second", "Automatic title")] {
        let mut record = record(&process, "codex", "open_session_log");
        let Operation::Add { tag } = &mut record.proposal.operation else {
            panic!()
        };
        tag.hint = Some(Box::new(tags::Hint {
            record: None,
            evidence_chain: tag.hint.as_ref().unwrap().evidence_chain.clone(),
            kind: "session_log".into(),
            value: id.into(),
            title: Some(title.into()),
        }));
        report.effective.push(Effective {
            source: "codex".into(),
            tag: tag.clone(),
        });
    }
    process.tag_report = Some(report.clone());
    node.totals.add_process(&process);
    node.process = Some(Box::new(process));
    let before = structure(&node);
    summarize(&mut node);
    assert_eq!(node.hints.len(), 2);
    assert!(node.hints.iter().all(|hint| hint.processes == 1));
    assert_eq!(structure(&node), before);
    report.effective.remove(0);
    report.effective[0].tag.hint.as_mut().unwrap().title = Some("Renamed".into());
    node.process.as_mut().unwrap().tag_report = Some(report);
    summarize(&mut node);
    assert_eq!(node.hints.len(), 1);
    assert_eq!(node.hints[0].value, "second");
    assert_eq!(node.hints[0].title.as_deref(), Some("Renamed"));
    node.process.as_mut().unwrap().tag_report = None;
    summarize(&mut node);
    assert!(node.hints.is_empty());
    assert_eq!(structure(&node), before);
    root.close().unwrap();
}

#[test]
fn disabled_tool_extensions_remove_hints_and_report_skipped_work() {
    let (root, processes) = fixture();
    let cancel = Cancellation::default();
    for source in ["codex", "claude", "playwright"] {
        let record = record(&processes[0], source, "arguments");
        let contribution = extensions::Contribution::Classify {
            proposals: vec![ClassificationProposal {
                identity: record.proposal.identity,
                evidence: ClassificationEvidence::Tags {
                    proposal: record.proposal,
                },
            }],
        };
        let fixture = extensions::tests::Fixture::new(source, contribution);
        let mut settings = extensions::default_settings().unwrap();
        for enabled in [true, false] {
            settings.set_builtin_enabled(source, enabled).unwrap();
            let context = extensions::Context::new(&settings, &cancel);
            let (reports, diagnostics) =
                tags::classify(&[&fixture], &context, &super::super::Facts::new(&processes));
            assert!(diagnostics.is_empty());
            assert_eq!(
                reports
                    .get(&processes[0].pid)
                    .is_some_and(|report| !report.effective.is_empty()),
                enabled
            );
            let records = context.report.lock().unwrap();
            assert!(records.iter().any(|record| record.provider == source
                && record.result == if enabled { "success" } else { "skipped" }));
        }
    }
    root.close().unwrap();
}

#[test]
fn unsafe_optional_project_hints_preserve_both_ownerships_and_healthy_hint() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().canonicalize().unwrap();
    for marker in ['\u{202e}', '\u{2066}', '\u{2028}', '\u{2029}'] {
        let names = [
            "HealthyProject".to_owned(),
            format!("Unsafe{marker}Project"),
        ];
        let processes: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let cwd = directory.join(name);
                std::fs::create_dir_all(&cwd).unwrap();
                std::fs::write(
                    cwd.join("Cargo.toml"),
                    "[package]\nname=\"fixture\"\nversion=\"0.0.0\"\n",
                )
                .unwrap();
                Process {
                    pid: 10 + index as u32,
                    ppid: Some(1),
                    uid: Some(unsafe { libc::geteuid() }),
                    started_unix_us: Some(100 + index as u64),
                    name: "worker".into(),
                    executable: Some("/fixture/tool".into()),
                    cwd: Some(cwd),
                    ..Default::default()
                }
            })
            .collect();
        let combined = tree::build(processes.clone()).unwrap();
        assert!(
            combined.diagnostics.is_empty(),
            "{:?}",
            combined.diagnostics
        );
        let category = combined
            .children
            .iter()
            .find(|node| node.reference == "group:projects")
            .unwrap();
        assert_eq!(category.totals.processes, 2);
        for (index, process) in processes.iter().enumerate() {
            let alone = tree::build(vec![process.clone()]).unwrap();
            let prior = alone
                .children
                .iter()
                .find(|node| node.reference == "group:projects")
                .unwrap()
                .children
                .first()
                .unwrap();
            let group = category
                .children
                .iter()
                .find(|group| group.reference == prior.reference)
                .unwrap();
            assert_eq!(group.name, prior.name);
            assert_eq!(group.grouping, prior.grouping);
            let pid = tree::select(&combined, &format!("pid:{}", process.pid)).unwrap();
            let report = pid.process.as_ref().unwrap().tag_report.as_ref().unwrap();
            let tag = &report
                .effective
                .iter()
                .find(|entry| entry.source == "project")
                .unwrap()
                .tag;
            assert_eq!(tag.path, process.cwd);
            assert_eq!(tag.label, names[index]);
            assert_eq!(tag.hint.is_some(), index == 0);
        }
        assert!(
            combined
                .hints
                .iter()
                .any(|hint| hint.value == "HealthyProject")
        );
    }
    root.close().unwrap();
}
