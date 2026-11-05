use super::*;
use crate::{extensions, model::Node, snapshot::Cancellation, tree};

pub(crate) fn processes() -> Vec<Process> {
    [
        (10, 1, "/Applications/Lumina.app/Contents/MacOS/main"),
        (20, 10, "/Applications/Lumina.app/Contents/Helpers/agent"),
        (30, 20, "/fixture/bin/multiplexer"),
        (40, 30, "/fixture/bin/worker"),
    ]
    .into_iter()
    .map(|(pid, ppid, executable)| Process {
        pid,
        ppid: Some(ppid),
        uid: Some(501),
        started_unix_us: Some(u64::from(pid)),
        name: "fixture".into(),
        executable: Some(executable.into()),
        cwd: Some(format!("/fixture/{pid}").into()),
        resident_bytes: Some(100),
        cpu_percent: Some(1.0),
        ..Process::default()
    })
    .collect()
}

fn tag(facts: &Facts<'_>, pid: u32) -> Option<Tag> {
    let p = proposal(facts, facts.by_pid[&pid])?;
    let ClassificationEvidence::Tags { proposal } = p.evidence else {
        panic!()
    };
    let Operation::Add { tag } = proposal.operation else {
        panic!()
    };
    Some(tag)
}

#[test]
fn nearest_app_through_arbitrary_helpers_and_multiple_levels() {
    let mut processes = processes();
    let facts = Facts::new(&processes);
    let t = tag(&facts, 40).unwrap();
    assert_eq!(t.label, "Launched by Lumina");
    assert_eq!(
        t.path.as_deref(),
        Some(std::path::Path::new("/Applications/Lumina.app"))
    );
    assert_eq!(
        t.evidence_chain
            .iter()
            .map(|e| e.identity.pid)
            .collect::<Vec<_>>(),
        [40, 30, 20]
    );
    assert_eq!(
        t.evidence_chain[2].executable,
        processes[1].executable.clone().unwrap()
    );
    processes[2].executable = Some("/Applications/Nova.app/Contents/Helpers/arbitrary".into());
    let facts = Facts::new(&processes);
    let t = tag(&facts, 40).unwrap();
    assert_eq!(t.label, "Launched by Nova");
    assert_eq!(t.evidence_chain.len(), 2);
}

#[test]
fn unsafe_or_unverifiable_ancestry_is_omitted() {
    for case in 0..11 {
        let mut p = processes();
        match case {
            0 => p[2].started_unix_us = Some(99),
            1 => p[2].started_unix_us = None,
            2 => p[2].ppid = Some(40),
            3 => p[2].ppid = None,
            4 => p[2].ppid = Some(999),
            5 => p[2].ppid = Some(1),
            6 => p[2].uid = Some(502),
            7 => p[2].uid = None,
            8 => p[2].executable = None,
            9 => p[3].started_unix_us = None,
            10 => p[2].executable = Some("relative/tool".into()),
            _ => unreachable!(),
        }
        assert!(tag(&Facts::new(&p), 40).is_none(), "case {case}");
    }
    let mut p = processes();
    for p in &mut p {
        p.executable = Some("/fixture/bin/terminal".into());
    }
    assert!(tag(&Facts::new(&p), 40).is_none());
}

fn structure(node: &Node) -> serde_json::Value {
    serde_json::json!({"ref":node.reference,"name":node.name,"totals":node.totals,"grouping":node.grouping,
        "attribution":node.attribution,"children":node.children.iter().map(structure).collect::<Vec<_>>()})
}

#[test]
fn supplementary_tags_do_not_change_ownership_tree_refs_or_totals_and_disable_cleanly() {
    let cancel = Cancellation::default();
    let mut settings = extensions::default_settings().unwrap();
    let enabled = tree::build_with_context(processes(), &Context::new(&settings, &cancel)).unwrap();
    assert!(enabled.diagnostics.is_empty(), "{:?}", enabled.diagnostics);
    let child = tree::select(&enabled, "pid:40").unwrap();
    let report = child.process.unwrap().tag_report.unwrap();
    assert!(report.effective.iter().any(|e| e.source == ID));
    let helper = tree::select(&enabled, "pid:20").unwrap();
    let report = helper.process.unwrap().tag_report.unwrap();
    assert!(!report.effective.iter().any(|e| e.source == ID));
    assert!(report.proposals.iter().all(|p| p.source != ID));
    settings.set_builtin_enabled(ID, false).unwrap();
    let context = Context::new(&settings, &cancel);
    let disabled = tree::build_with_context(processes(), &context).unwrap();
    assert_eq!(structure(&enabled), structure(&disabled));
    tree::validate_references(&disabled).unwrap();
    let mut pending = vec![&disabled];
    while let Some(node) = pending.pop() {
        if let Some(report) = node.process.as_ref().and_then(|p| p.tag_report.as_ref()) {
            assert!(report.proposals.iter().all(|p| p.source != ID));
            assert!(report.effective.iter().all(|e| e.source != ID));
        }
        pending.extend(&node.children);
    }
    assert!(
        context
            .report
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.provider == ID
                && r.phase == "classify"
                && r.result == "skipped"
                && r.duration_ms.is_none())
    );
    settings.set_builtin_enabled(ID, true).unwrap();
    let enabled_again =
        tree::build_with_context(processes(), &Context::new(&settings, &cancel)).unwrap();
    assert_eq!(structure(&enabled), structure(&enabled_again));
    assert!(
        tree::select(&enabled_again, "pid:40")
            .unwrap()
            .process
            .unwrap()
            .tag_report
            .unwrap()
            .effective
            .iter()
            .any(|e| e.source == ID)
    );
}

#[test]
fn observed_chain_roundtrips_without_losing_snapshot_metadata() {
    let p = processes();
    let facts = Facts::new(&p);
    let t = tag(&facts, 40).unwrap();
    let proposal = Proposal {
        id: "fixture".into(),
        identity: ProcessIdentity::observed(&p[3]).unwrap(),
        operation: Operation::Add { tag: t },
    };
    let wire = serde_json::to_value(&proposal).unwrap();
    let decoded: Proposal = serde_json::from_value(wire.clone()).unwrap();
    extensions::tags::validate(std::slice::from_ref(&decoded), &p, ID, None).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    let mut corrupt = proposal.clone();
    if let Operation::Add { tag } = &mut corrupt.operation {
        tag.evidence_chain[1].identity.started_unix_us += 1;
    }
    assert!(extensions::tags::validate(&[corrupt], &p, ID, None).is_err());
}

pub(crate) fn snapshot(enabled: bool, scope: &str) -> crate::model::Snapshot {
    let mut settings = extensions::default_settings().unwrap();
    settings.set_builtin_enabled(ID, enabled).unwrap();
    let cancel = Cancellation::default();
    let root = tree::build_with_context(processes(), &Context::new(&settings, &cancel)).unwrap();
    crate::model::Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 1,
        sample_ms: 250,
        collection_ms: 300,
        system: Default::default(),
        extensions: vec![],
        warnings: vec![],
        scope: scope.into(),
        root: tree::select(&root, scope).unwrap(),
    }
}

#[test]
fn launched_by_markdown_json_schema_and_projection_agree() {
    let snapshot = snapshot(true, "pid:40");
    let md = crate::output::markdown(&snapshot);
    assert!(md.contains("Launched by Lumina"), "{md}");
    for (pid, path) in [
        (40, "/fixture/bin/worker"),
        (30, "/fixture/bin/multiplexer"),
        (20, "/Applications/Lumina.app/Contents/Helpers/agent"),
    ] {
        assert!(
            md.contains(&format!(
                "via pid:{pid} · started {pid} · executable {path}"
            )),
            "{md}"
        );
    }
    let value = serde_json::to_value(snapshot).unwrap();
    let schema = serde_json::from_str::<serde_json::Value>(crate::output::JSON_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    let report = &value["selections"][0]["root"]["process"]["tag_report"];
    let tag = &report["effective"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["source"] == ID)
        .unwrap()["tag"];
    assert_eq!(tag["evidence_chain"].as_array().unwrap().len(), 3);
    assert_eq!(tag["supplementary_to"], "app");
    let mut invalid = value.clone();
    let tags = invalid["selections"][0]["root"]["process"]["tag_report"]["effective"]
        .as_array_mut()
        .unwrap();
    tags.iter_mut().find(|e| e["source"] == ID).unwrap()["tag"]["evidence_chain"][0]["identity"]
        ["pid"] = serde_json::json!("40");
    assert!(!validator.is_valid(&invalid));
    let suppressed = crate::output::markdown(&self::snapshot(true, "pid:20"));
    assert!(!suppressed.contains("Launched by"), "{suppressed}");
    let disabled = serde_json::to_string(&self::snapshot(false, "pid:40")).unwrap();
    assert!(!disabled.contains("launched_by"));
    assert!(!disabled.contains("evidence_chain"));
    let summary = serde_json::to_string(&self::snapshot(true, "group:all")).unwrap();
    assert!(!summary.contains("tag_report"));
    assert!(!summary.contains("supplementary_labels"));
}

#[test]
fn direct_app_ownership_is_retained_when_another_app_launched_it() {
    let mut p = processes();
    p[3].executable = Some("/Applications/Nova.app/Contents/MacOS/main".into());
    let root = tree::build(p).unwrap();
    let node = tree::select(&root, "pid:40").unwrap();
    assert_eq!(
        node.attribution.unwrap().path.unwrap(),
        std::path::Path::new("/Applications/Nova.app")
    );
    let report = node.process.unwrap().tag_report.unwrap();
    assert!(
        report
            .effective
            .iter()
            .any(|e| e.source == ID && e.tag.label == "Launched by Lumina")
    );
}

#[test]
fn sensitive_ancestry_metadata_is_rejected_without_arguments_or_environment() {
    use std::collections::BTreeMap;
    let secret = "multiplexer";
    let definition: crate::settings::Definition = serde_json::from_value(serde_json::json!({"key":"extensions.fixture.secret","type":"string","default":secret,"description":"fixture","sensitive":true,"provider":"fixture"})).unwrap();
    let mut registry = extensions::settings_registry().unwrap();
    registry
        .register_extension("fixture", vec![definition], vec![])
        .unwrap();
    let settings = registry
        .resolve(&crate::settings::Config::default(), &BTreeMap::new())
        .unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let p = processes();
    let (reports, _) = extensions::tags::classify(
        &[&extensions::Builtin::LaunchedBy],
        &context,
        &Facts::new(&p),
    );
    for report in reports.values() {
        assert!(!serde_json::to_string(report).unwrap().contains(secret));
        assert!(report.proposals.is_empty());
    }
    assert!(
        context
            .report
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.provider == ID && r.result == "failed")
    );
}

#[test]
fn inherited_primary_suppresses_launcher_but_a_different_primary_keeps_it() {
    let mut p = processes();
    for p in &mut p {
        p.cwd = Some("/fixture/shared".into());
    }
    let cancel = Cancellation::default();
    let mut settings = extensions::default_settings().unwrap();
    let root = tree::build_with_context(p.clone(), &Context::new(&settings, &cancel)).unwrap();
    let child = tree::select(&root, "pid:40").unwrap();
    let report = child.process.unwrap().tag_report.unwrap();
    assert!(report.effective.iter().any(|e| e.source == "ancestor"));
    assert!(report.proposals.iter().all(|e| e.source != ID));
    settings.ownership_priority = vec!["bin".into(), "app".into()];
    let root = tree::build_with_context(p, &Context::new(&settings, &cancel)).unwrap();
    let child = tree::select(&root, "pid:40").unwrap();
    let report = child.process.unwrap().tag_report.unwrap();
    assert!(report.effective.iter().any(|e| e.source == ID));
    assert!(report.effective.iter().any(|e| e.source == "ancestor"));
}

#[test]
fn typed_ancestry_proposal_keeps_evidence_without_wire_metadata() {
    let p = processes();
    let proposed = proposal(&Facts::new(&p), &p[3]).unwrap();
    let value = serde_json::to_value(&proposed).unwrap();
    let decoded: extensions::ClassificationProposal =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    assert!(value.get("version").is_none());
}

#[test]
fn deep_ancestry_is_iterative_and_all_evidence_is_retained() {
    let mut p = processes();
    p.truncate(2);
    for pid in 21..221 {
        let mut child = p[1].clone();
        child.pid = pid;
        child.ppid = Some(pid - 1);
        child.started_unix_us = Some(u64::from(pid));
        child.executable = Some("/fixture/bin/tool".into());
        p.push(child);
    }
    let t = tag(&Facts::new(&p), 220).unwrap();
    assert_eq!(t.evidence_chain.len(), 201);
    assert_eq!(t.evidence_chain.last().unwrap().identity.pid, 20);
}

#[test]
fn supplementary_evidence_rejects_incomplete_forged_and_oversized_metadata() {
    let p = processes();
    let original = proposal(&Facts::new(&p), &p[3]).unwrap();
    let ClassificationEvidence::Tags { proposal: original } = original.evidence else {
        panic!()
    };
    for case in 0..7 {
        let mut proposed = original.clone();
        let Operation::Add { tag } = &mut proposed.operation else {
            panic!()
        };
        match case {
            0 => tag.supplementary_to = None,
            1 => tag.supplementary_to = Some(format!("extensions.fixture.{}", "x".repeat(128))),
            2 => tag.evidence_chain.clear(),
            3 => tag.evidence_chain[1].executable = "/fixture/bin/forged".into(),
            4 => tag.evidence_chain[1].identity = tag.evidence_chain[0].identity,
            5 => tag.path = Some("/Applications/Other.app".into()),
            6 => tag.evidence_chain[1].executable = "relative/tool".into(),
            _ => unreachable!(),
        }
        assert!(
            extensions::tags::validate(&[proposed], &p, ID, None).is_err(),
            "case {case}"
        );
    }
}
