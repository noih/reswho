use super::*;
use serde_json::json;

fn process(pid: u32, executable: &str, cwd: Option<&Path>) -> Process {
    Process {
        pid,
        ppid: Some(1),
        started_unix_us: Some(u64::from(pid)),
        uid: Some(501),
        executable: Some(executable.into()),
        cwd: cwd.map(Into::into),
        name: "fixture".into(),
        resident_bytes: Some(100),
        cpu_percent: Some(1.0),
        ..Default::default()
    }
}
struct Work;
impl Extension for Work {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: "fixture".into(),
            capabilities: vec![Capability::Classify],
            enabled: true,
            availability: "available",
        }
    }
    fn execute(&self, _: &Context<'_>, request: Request<'_>) -> Result<Contribution> {
        let Request::Classify(facts) = request else {
            bail!("classify")
        };
        Ok(Contribution::Classify {
            proposals: facts
                .processes
                .iter()
                .filter(|p| p.pid == 12 || p.pid == 13)
                .map(|p| {
                    host::proposals::identity_proposal(
                        ProcessIdentity::observed(p).unwrap(),
                        "fixture",
                        "observed",
                        "work",
                        "worker",
                        None,
                    )
                })
                .collect(),
        })
    }
}
#[test]
fn placement_defaults_priority_and_disabled_fallbacks_preserve_totals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("go.mod"), "module fixture").unwrap();
    let processes = vec![
        process(
            10,
            "/Applications/Demo.app/Contents/MacOS/demo",
            Some(dir.path()),
        ),
        process(11, "/opt/tool/run", Some(dir.path())),
        process(12, "/opt/tool/run", None),
        process(13, "/opt/tool/run", None),
        process(14, "/opt/other/run", None),
        process(15, "/usr/libexec/fixture", None),
    ];
    let cancel = crate::snapshot::Cancellation::default();
    let mut settings = default_settings().unwrap();
    settings.source_priority = vec!["bin".into()];
    let build = |settings: &Resolved, providers: &[&dyn Extension]| {
        crate::tree::build_with_providers(
            processes.clone(),
            &Context::new(settings, &cancel),
            providers,
        )
        .unwrap()
    };
    let root = build(&settings, &[&Work]);
    for (reference, count) in [
        ("group:applications", 1),
        ("group:projects", 1),
        ("group:unclassified", 3),
        ("group:system", 1),
    ] {
        assert_eq!(
            crate::tree::select(&root, reference)
                .unwrap()
                .totals
                .processes,
            count
        );
    }
    let work_tag = crate::tree::select(&root, "pid:12")
        .unwrap()
        .process
        .unwrap()
        .tag_report
        .unwrap()
        .effective
        .into_iter()
        .find(|e| e.tag.dimension == "work")
        .unwrap()
        .tag;
    let work = tags::ownership_ref("work", &work_tag.id, Path::new("/opt/tool/run"));
    assert_eq!(
        crate::tree::select(&root, &work).unwrap().totals.processes,
        2
    );
    assert_eq!(root.totals.processes, 6);
    assert_eq!(root.totals.resident_bytes, 600);
    settings.ownership_priority = vec!["project".into(), "extensions.absent.owner".into()];
    let moved = build(&settings, &[&Work]);
    assert!(crate::tree::select(&moved, "group:applications").is_err());
    assert_eq!(
        crate::tree::select(&moved, "group:projects")
            .unwrap()
            .totals
            .processes,
        2
    );
    settings.set_builtin_enabled("project", false).unwrap();
    let discovery_before = host::projects::DISCOVERIES.load(std::sync::atomic::Ordering::Relaxed);
    let fallback = build(&settings, &[]);
    assert_eq!(
        host::projects::DISCOVERIES.load(std::sync::atomic::Ordering::Relaxed),
        discovery_before
    );
    assert!(crate::tree::select(&fallback, "group:projects").is_err());
    assert_eq!(
        crate::tree::select(&fallback, "group:applications")
            .unwrap()
            .totals
            .processes,
        1
    );
    assert_eq!(fallback.totals.resident_bytes, root.totals.resident_bytes);
    assert_eq!(settings.ownership_priority[1], "extensions.absent.owner");
    settings.set_builtin_enabled("project", true).unwrap();
    let restored = build(&settings, &[]);
    assert_eq!(
        crate::tree::select(&restored, "group:projects")
            .unwrap()
            .totals
            .processes,
        2
    );

    for builtin in BUILTINS {
        settings
            .set_builtin_enabled(&builtin.descriptor().id, false)
            .unwrap();
    }
    assert!(settings.set_builtin_enabled("bin", false).is_err());
    let context = Context::new(&settings, &cancel);
    let core = crate::tree::build_with_providers(processes.clone(), &context, &[]).unwrap();
    assert_eq!(core.children.len(), 3);
    assert!(core.children.iter().all(|c| matches!(
        c.reference.as_str(),
        "group:applications" | "group:system" | "group:unclassified"
    )));
    assert_eq!(
        serde_json::to_value(&core.totals).unwrap(),
        serde_json::to_value(&root.totals).unwrap()
    );
    fn inspect<'a>(node: &'a crate::model::Node, nodes: &mut Vec<&'a crate::model::Node>) {
        nodes.push(node);
        for child in &node.children {
            inspect(child, nodes);
        }
    }
    let mut nodes = Vec::new();
    inspect(&core, &mut nodes);
    let mut references = HashSet::new();
    let mut pids = HashSet::new();
    for node in nodes {
        assert!(references.insert(&node.reference));
        assert!(crate::tree::select(&core, &node.reference).is_ok());
        assert!(node.container.is_none() && node.runtime.is_none() && node.resource.is_none());
        assert!(node.host_resources.is_empty());
        if let Some(process) = &node.process {
            assert!(pids.insert(process.pid));
            let original = processes.iter().find(|p| p.pid == process.pid).unwrap();
            let before = serde_json::to_value(original).unwrap();
            let after = serde_json::to_value(process).unwrap();
            for key in [
                "pid",
                "ppid",
                "uid",
                "started_unix_us",
                "uptime_seconds",
                "name",
                "executable",
                "cwd",
                "command",
                "cpu_percent",
                "resident_bytes",
                "network",
                "unavailable",
            ] {
                assert_eq!(before[key], after[key], "pid {}: {key}", process.pid);
            }
            assert!(process.roles.is_empty() && process.supplementary_labels.is_empty());
            assert!(process.host_resource.is_none() && process.extension_details.is_empty());
            assert!(process.version.is_none() && process.name_source.is_none());
            let report = process.tag_report.as_ref().unwrap();
            assert!(
                report
                    .proposals
                    .iter()
                    .all(|tag| matches!(tag.source.as_str(), "app_bundle" | "bin"))
            );
            assert!(
                report
                    .effective
                    .iter()
                    .all(|tag| matches!(tag.source.as_str(), "app_bundle" | "bin"))
            );
        }
    }
    assert_eq!(pids, processes.iter().map(|p| p.pid).collect());
    let containers = collect(
        &[&Builtin::Docker, &Builtin::Podman, &Builtin::Apple],
        &context,
    )
    .unwrap();
    assert!(containers.children.is_empty());
    for builtin in BUILTINS {
        let id = builtin.descriptor().id;
        assert!(settings.is_disabled(&id));
        let records = context.report.lock().unwrap();
        assert!(records.iter().any(|r| r.provider == id), "{id}");
        assert!(
            records
                .iter()
                .filter(|r| r.provider == id)
                .all(|r| r.result == "skipped" && r.duration_ms.is_none()),
            "{id}"
        );
    }
    let core_report = crate::tree::select(&core, "pid:10")
        .unwrap()
        .process
        .unwrap()
        .tag_report
        .unwrap();
    assert!(
        core_report
            .effective
            .iter()
            .any(|e| e.source == "app_bundle")
    );
    dir.close().unwrap();
}

#[test]
fn ownership_priority_config_validation_and_save_preserve_unknown_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("config.json");
    let config:Config=serde_json::from_value(json!({"version":1,"source_priority":["bin"],"ownership_priority":["extensions.absent.owner","project"],"settings":{"unknown.setting":"preserved"}})).unwrap();
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let registry = settings_registry().unwrap();
    let mut editor = crate::settings::edit::Settings::new(
        registry,
        config.clone(),
        BTreeMap::new(),
        Some(path.clone()),
    );
    editor
        .save(&BTreeMap::from([(
            "sampling.interval_ms".into(),
            Some(json!(300)),
        )]))
        .unwrap();
    let saved = Config::load_path(Some(&path), true).unwrap();
    assert_eq!(saved.ownership_priority, config.ownership_priority);
    assert_eq!(saved.settings["unknown.setting"], "preserved");
    let mut concurrent = Config::load_path(Some(&path), true).unwrap();
    concurrent.ownership_priority.reverse();
    std::fs::write(&path, serde_json::to_vec(&concurrent).unwrap()).unwrap();
    editor
        .save_priority(&["project".into(), "bin".into()])
        .unwrap();
    assert_eq!(
        Config::load_path(Some(&path), true)
            .unwrap()
            .ownership_priority,
        concurrent.ownership_priority
    );
    for bad in [
        vec!["project".into(), "project".into()],
        vec!["bad dimension".into()],
    ] {
        let mut invalid = config.clone();
        invalid.ownership_priority = bad;
        assert!(crate::settings::validate_ownership_priority(&invalid.ownership_priority).is_err());
        let recovered = settings_registry()
            .unwrap()
            .resolve(&invalid, &BTreeMap::new())
            .unwrap();
        assert!(recovered.ownership_priority.is_empty());
    }
    dir.close().unwrap();
}
