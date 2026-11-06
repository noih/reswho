use super::*;
use crate::{
    model::{Process, Snapshot, SystemSummary},
    output, tree,
};

fn runtime(endpoint: &str) -> Runtime {
    Runtime {
        disk_usage: None,
        metrics_scope: None,
        host_scope: None,
        configuration: None,
        name: "docker".into(),
        context: Some("custom".into()),
        endpoint_ref: Some(endpoint.into()),
        daemon_id: Some("daemon".into()),
        status: "available".into(),
        inventory_complete: true,
        collection_ms: 1,
        unavailable: Vec::new(),
    }
}

fn container(id: &str) -> Container {
    Container {
        runtime: "docker".into(),
        endpoint_ref: "endpoint-a".into(),
        id: id.into(),
        name: "same-name".into(),
        state: Some("running".into()),
        cpu_percent: Some(0.0),
        memory_bytes: Some(100),
        uptime_seconds: Some(60),
        ..Container::default()
    }
}

#[test]
fn container_scope_counts_once_and_remains_queryable_after_collapse() {
    let mut one = container("one");
    one.project = Some(Project {
        name: "site".into(),
        path: Some("/work/site".into()),
        source: ProjectSource::Compose,
    });
    let mut two = one.clone();
    two.id = "two".into();
    two.memory_bytes = None;
    let branch = build(runtime("endpoint-a"), vec![one, two]).unwrap();
    let reference = branch.children[0].children[0].children[0].reference.clone();
    let mut root = tree::build(vec![Process {
        pid: 10,
        resident_bytes: Some(500),
        ..Process::default()
    }])
    .unwrap();
    root.children.push(branch);
    assert_eq!(root.totals.resident_bytes, 500);
    assert_eq!(root.totals.processes, 1);
    let selected = tree::select(&root, &reference).unwrap();
    assert!(selected.container.is_some());
    let mut selected = tree::select(&root, "group:containers").unwrap();
    tree::limit_depth(&mut selected, 0);
    let totals = selected.container_totals.as_ref().unwrap();
    assert_eq!(totals.containers, 2);
    assert_eq!(totals.known_memory_bytes, 100);
    assert_eq!(totals.memory_unknown, 1);
    assert_eq!(selected.omitted_children, 1);
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root: selected,
    };
    let md = output::markdown(&snapshot);
    assert!(md.contains("2 containers [+1]"));
    assert!(md.contains("1 unknown"));
    assert!(!md.contains("2 processes"));
    assert_eq!(
        serde_json::to_value(snapshot).unwrap()["selections"][0]["root"]["container_totals"]["containers"],
        2
    );
}

#[test]
fn same_names_do_not_merge_endpoint_or_container_identity() {
    let first = build(
        runtime("endpoint-a"),
        vec![container("one"), container("two")],
    )
    .unwrap();
    let other = build(runtime("endpoint-b"), vec![container("one")]).unwrap();
    let nodes = &first.children[0].children;
    assert_eq!(nodes.len(), 2);
    assert_ne!(nodes[0].reference, nodes[1].reference);
    assert_ne!(nodes[0].reference, other.children[0].children[0].reference);
    assert!(
        build(
            runtime("endpoint-a"),
            vec![container("one"), container("one")]
        )
        .is_err()
    );
}

#[test]
fn compose_and_bind_inference_retain_evidence_and_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("a/site");
    let second = temp.path().join("b/site");
    for path in [&first, &second] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("go.mod"), "").unwrap();
    }
    let mut item = container("one");
    item.mounts = vec![Mount {
        kind: "bind".into(),
        source: first.clone(),
        destination: "/workspace".into(),
    }];
    let mut detail = attribution::Evidence::default();
    let mut discovery = crate::extensions::host::projects::Discovery::default();
    attribution::classify(&mut item, &detail, true, &mut discovery);
    assert!(matches!(
        item.project.as_ref().unwrap().source,
        ProjectSource::BindProject
    ));
    item.project = None;
    item.mounts.push(Mount {
        kind: "bind".into(),
        source: second,
        destination: "/shared".into(),
    });
    attribution::classify(&mut item, &detail, true, &mut discovery);
    assert!(item.project.is_none());
    assert_eq!(item.candidates.len(), 2);
    detail.workdir = Some("/workspace".into());
    attribution::classify(&mut item, &detail, true, &mut discovery);
    assert!(matches!(
        item.project.as_ref().unwrap().source,
        ProjectSource::BindWorkdir
    ));
    assert_eq!(
        item.project.as_ref().unwrap().path,
        Some(first.canonicalize().unwrap())
    );
    detail.compose_project = Some("declared-project".into());
    detail.compose_workdir = Some("/possibly-remote/checkout".into());
    attribution::classify(&mut item, &detail, true, &mut discovery);
    assert!(matches!(
        item.project.as_ref().unwrap().source,
        ProjectSource::Compose
    ));
    assert_eq!(item.candidates.len(), 2);
    assert_eq!(item.project.as_ref().unwrap().name, "declared-project");
    temp.close().unwrap();
}

#[test]
fn partial_inventory_and_container_markdown_preserve_semantics() {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut runtime = runtime("endpoint-a");
    runtime.inventory_complete = false;
    runtime.unavailable.push("inventory truncated".into());
    let mut item = container("one");
    item.memory_bytes = None;
    item.state = Some("running".into());
    item.project = Some(Project {
        name: "site".into(),
        path: Some("/a/space and`tick".into()),
        source: ProjectSource::BindProject,
    });
    let branch = build(runtime, vec![item]).unwrap();
    let reference = branch.children[0].children[0].children[0].reference.clone();
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root: branch,
    };
    let md = output::markdown(&snapshot);
    assert!(md.contains("· >=0.0%  ?  ⏱︎ >=1m  >=1 active / >=1 containers"));
    assert!(!md.contains(">=>="));
    assert!(md.contains("> ~ Container project inferred:"), "{md}");
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        json["selections"][0]["root"]["children"][0]["children"][0]["evidence"],
        serde_json::json!([
            "Container project inferred: bind path + project marker",
            "container project path /a/space and`tick"
        ])
    );
    assert_eq!(md.matches("space and`tick").count(), 1);
    let mut item_depth = 0;
    let mut quote_depth = 0;
    let mut observed = false;
    for event in Parser::new(&md) {
        match event {
            Event::Start(Tag::Item) => item_depth += 1,
            Event::End(TagEnd::Item) => item_depth -= 1,
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Text(text) if text.contains("same-name") => {
                assert_eq!((item_depth, quote_depth), (3, 0));
                observed = true;
            }
            _ => {}
        }
    }
    assert!(observed);
    snapshot.root = tree::select(&snapshot.root, &reference).unwrap();
    let detail = output::markdown(&snapshot);
    assert!(detail.contains("~ single bind project + marker"));
    assert!(detail.contains("! Runtime unavailable: inventory truncated"));
    assert_eq!(snapshot.root.container.unwrap().id, "one");
}

#[test]
fn absent_or_ambiguous_paths_never_force_project_ownership() {
    let mut item = container("one");
    let detail = attribution::Evidence::default();
    let mut discovery = crate::extensions::host::projects::Discovery::default();
    for source in ["relative/project", "/", "/a/nonexistent/project"] {
        item.mounts = vec![Mount {
            kind: "bind".into(),
            source: source.into(),
            destination: "/workspace".into(),
        }];
        attribution::classify(&mut item, &detail, true, &mut discovery);
        assert!(item.project.is_none());
    }
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("package.json"), "{}").unwrap();
    item.mounts = vec![Mount {
        kind: "volume".into(),
        source: temp.path().into(),
        destination: "/workspace".into(),
    }];
    attribution::classify(&mut item, &detail, true, &mut discovery);
    assert!(item.project.is_none());
    temp.close().unwrap();
}

#[test]
fn inactive_containers_do_not_make_active_metrics_unknown() {
    let mut active = container("active");
    active.state = Some("running".into());
    let mut exited = container("exited");
    exited.state = Some("exited".into());
    exited.cpu_percent = None;
    exited.memory_bytes = None;
    exited.uptime_seconds = None;
    let branch = build(runtime("endpoint-a"), vec![active.clone(), exited.clone()]).unwrap();
    let totals = branch.container_totals.as_ref().unwrap();
    assert_eq!(totals.cpu_unknown, 0);
    assert_eq!(totals.memory_unknown, 0);
    assert_eq!(totals.uptime_unknown, 0);
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root: branch,
    };
    let md = output::markdown(&snapshot);
    assert!(md.contains("⏱︎ 1m  1 active / 2 containers"));
    assert!(md.contains("· -  -  -  exited"));
    assert!(!md.contains("unknown"));
    assert!(!md.contains(">="));
    active.memory_bytes = None;
    snapshot.root = build(runtime("endpoint-a"), vec![active, exited.clone()]).unwrap();
    assert!(output::markdown(&snapshot).contains("· 0.0%  ?  ⏱︎ 1m"));
    snapshot.root = build(runtime("endpoint-a"), vec![exited]).unwrap();
    let md = output::markdown(&snapshot);
    assert!(md.contains("· -  -  ⏱︎ -  0 active / 1 containers"));
    assert!(!md.contains("unknown"));
}

#[test]
fn compose_group_notes_use_one_line_without_losing_pathless_evidence() {
    let mut item = container("compose");
    item.project = Some(Project {
        name: "site".into(),
        path: Some("/work/site".into()),
        source: ProjectSource::Compose,
    });
    let mut snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root: build(runtime("endpoint-a"), vec![item.clone()]).unwrap(),
    };
    let md = output::markdown(&snapshot);
    assert_eq!(md.matches("> compose `/work/site`").count(), 1);
    assert!(!md.contains("Compose project label"));
    assert!(!md.contains("> path "));
    item.project.as_mut().unwrap().path = None;
    snapshot.root = build(runtime("endpoint-a"), vec![item]).unwrap();
    let md = output::markdown(&snapshot);
    assert!(md.contains("> compose\n"), "{md}");
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        json["selections"][0]["root"]["children"][0]["children"][0]["evidence"],
        serde_json::json!(["compose"])
    );
}

#[test]
fn unknown_and_transitional_activity_remains_distinct_from_inactive() {
    let mut totals = ContainerTotals::default();
    for state in [
        Some("running"),
        Some("paused"),
        Some("restarting"),
        Some("created"),
        Some("exited"),
        Some("dead"),
        Some("removing"),
        Some("future-state"),
        None,
    ] {
        let item = Container {
            state: state.map(str::to_owned),
            ..Container::default()
        };
        totals.add(&item);
    }
    assert_eq!(totals.containers, 9);
    assert_eq!(totals.active_containers, 3);
    assert_eq!(totals.inactive_containers, 3);
    assert_eq!(totals.activity_unknown, 3);
    assert_eq!(totals.cpu_unknown, 6);
    assert_eq!(totals.memory_unknown, 6);
    assert_eq!(totals.uptime_unknown, 6);
    let json = serde_json::to_value(&totals).unwrap();
    assert_eq!(json["active_containers"], 3);
    assert_eq!(json["inactive_containers"], 3);
    assert_eq!(json["activity_unknown"], 3);
    let mut root = build(
        runtime("endpoint-a"),
        vec![Container {
            id: "unknown".into(),
            ..Container::default()
        }],
    )
    .unwrap();
    tree::limit_depth(&mut root, 0);
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root,
    };
    let md = output::markdown(&snapshot);
    assert!(md.contains("· ?  ?  ⏱︎ ?  ? active / 1 containers [+1]"));
}

#[test]
fn json_contract_preserves_container_attribution_and_scopes() {
    let schema: serde_json::Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut active = container("one");
    active.memory_basis = stats::API_MEMORY_BASIS.into();
    active.project = Some(Project {
        name: "site".into(),
        path: Some("/example/space_中文".into()),
        source: ProjectSource::Compose,
    });
    let mut stopped = container("two");
    stopped.state = Some("exited".into());
    stopped.cpu_percent = None;
    stopped.memory_bytes = None;
    stopped.uptime_seconds = None;
    let root = build(runtime("endpoint-a"), vec![active, stopped]).unwrap();
    let mut references = Vec::new();
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        references.push(node.reference.clone());
        pending.extend(&node.children);
    }
    for reference in references {
        for depth in [0, usize::MAX] {
            let mut selected = tree::select(&root, &reference).unwrap();
            tree::limit_depth(&mut selected, depth);
            let snapshot = Snapshot {
                schema: crate::model::JSON_SCHEMA_VERSION,
                extensions: vec![],
                captured_unix_ms: 0,
                sample_ms: 250,
                collection_ms: 300,
                system: SystemSummary::default(),
                scope: reference.clone(),
                warnings: vec![],
                root: selected,
            };
            let value = serde_json::to_value(&snapshot).unwrap();
            let errors: Vec<_> = validator
                .iter_errors(&value)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{}", errors.join("\n"));
            if matches!(snapshot.root.grouping, Some(Grouping::Project)) {
                assert_eq!(
                    value["selections"][0]["root"]["attribution"]["path"],
                    "/example/space_中文"
                );
                assert_eq!(
                    value["selections"][0]["root"]["attribution"]["source"],
                    "compose"
                );
                assert_eq!(
                    value["selections"][0]["root"]["attribution"]["inferred"],
                    false
                );
            }
        }
    }
}

#[test]
fn multiple_runtimes_keep_identity_totals_and_partial_inventory_separate() {
    let docker = runtime("endpoint-a");
    let mut apple = runtime("endpoint-b");
    apple.name = "apple".into();
    apple.context = Some("local".into());
    apple.inventory_complete = false;
    let one = container("same-name");
    let mut two = one.clone();
    two.runtime = "apple".into();
    two.endpoint_ref = "endpoint-b".into();
    two.state = Some("stopped".into());
    two.cpu_percent = None;
    two.memory_bytes = None;
    let root = build_all(vec![(docker, vec![one]), (apple, vec![two])]).unwrap();
    assert_eq!(root.children.len(), 2);
    assert_ne!(
        root.children[0].children[0].reference,
        root.children[1].children[0].reference
    );
    let totals = root.container_totals.as_ref().unwrap();
    assert_eq!(totals.containers, 2);
    assert_eq!(totals.active_containers, 1);
    assert_eq!(totals.inactive_containers, 1);
    assert_eq!(totals.memory_unknown, 0);
    assert_eq!(totals.known_memory_bytes, 100);
    assert!(!totals.inventory_complete);
}

#[test]
fn runtime_failure_reason_and_hint_reach_markdown_and_json() {
    let mut failed = runtime("endpoint-a");
    failed.status = "unavailable".into();
    failed.unavailable.push(crate::model::Diagnostic::failure(
        "collection_failed",
        "docker",
        "info: container CLI command failed (exit status: 1); stderr withheld",
        crate::model::FailureReason::PermissionDenied,
        Some(libc::EPERM),
    ));
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:containers".into(),
        warnings: vec![],
        root: build_all(vec![(failed, vec![])]).unwrap(),
    };
    let md = output::markdown(&snapshot);
    assert!(
        md.contains("> ! info: container CLI command failed (exit status: 1); stderr withheld · reason permission\\_denied: endpoint socket access denied; allow socket access (sandbox or file mode)"),
        "{md}"
    );
    let json = serde_json::to_value(&snapshot).unwrap();
    let diagnostic = &json["selections"][0]["root"]["children"][0]["runtime"]["unavailable"][0];
    assert_eq!(diagnostic["reason"], "permission_denied");
    assert_eq!(diagnostic["os_error"], libc::EPERM);
}

#[test]
fn repair_vm_aggregate_scope_survives_selection_projection_and_depth() {
    for scopes in [
        [Some("in_vm"), Some("in_vm"), Some("in_vm")],
        [Some("in_vm"), Some("host"), Some("in_vm")],
        [Some("in_vm"), None, Some("in_vm")],
        [None, Some("in_vm"), Some("in_vm")],
        [None, None, None],
        [Some("host"), Some("host"), Some("host")],
    ] {
        let expected_scope = scopes
            .iter()
            .all(|s| *s == scopes[0])
            .then_some(scopes[0])
            .flatten();
        let expected = expected_scope == Some("in_vm");
        let items = scopes
            .into_iter()
            .enumerate()
            .map(|(i, scope)| {
                let mut item = container(&i.to_string());
                item.metrics_scope = scope.map(Into::into);
                item.project = Some(Project {
                    name: "site".into(),
                    path: None,
                    source: ProjectSource::Compose,
                });
                item
            })
            .collect();
        let mut source = runtime("endpoint-a");
        source.metrics_scope = Some("in_vm".into());
        let root = build_all(vec![(source, items)]).unwrap();
        let project_ref = root.children[0].children[0].reference.clone();
        for reference in ["group:containers", project_ref.as_str()] {
            let selected = tree::select(&root, reference).unwrap();
            let totals = serde_json::to_value(&selected.container_totals).unwrap();
            let snapshot = Snapshot {
                schema: crate::model::JSON_SCHEMA_VERSION,
                extensions: vec![],
                captured_unix_ms: 1,
                sample_ms: 1,
                collection_ms: 1,
                system: Default::default(),
                scope: reference.into(),
                warnings: vec![],
                root: selected,
            };
            for depth in [0, 1] {
                let md = output::markdown_with_depth(&snapshot, depth);
                assert_eq!(
                    md.contains("in-VM usage: CPU"),
                    expected,
                    "{scopes:?} {reference} depth={depth}: {md}"
                );
                let mut trimmed = snapshot.clone();
                trimmed.project(&[]);
                tree::limit_depth(&mut trimmed.root, depth);
                assert_eq!(output::metrics_scope(&trimmed.root), expected_scope);
                crate::tui::tests::assert_aggregate_metrics_scope(trimmed.clone(), expected);
                let value = serde_json::to_value(&trimmed).unwrap();
                let actual = &value["selections"][0]["root"]["container_totals"];
                assert_eq!(actual["metrics_scope"].as_str(), expected_scope);
                assert_eq!(actual, &totals);
                assert_eq!(trimmed.root.reference, reference);
                let schema: serde_json::Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
                assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
            }
        }
    }
}
