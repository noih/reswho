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
    attach_details(&mut root, &detailed, &[]);
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
    super::attach_details(&mut snapshot.root, &fresh, &[]);
    let process = crate::tree::select(&snapshot.root, "pid:40")
        .unwrap()
        .process
        .unwrap();
    assert_eq!(
        serde_json::to_value(&process.host_resource).unwrap(),
        before
    );
}
