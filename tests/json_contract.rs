use reswho::{
    model::{
        Diagnostic, FailureReason, Grouping, JSON_SCHEMA_VERSION, Network, Node, Process, Protocol,
        Snapshot, Socket, SystemSummary,
    },
    output, tree,
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::OnceLock};

fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    })
}

fn validate(value: &Value) {
    let errors: Vec<_> = validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn command_extension_preserves_argument_boundaries_and_validates_types() {
    let root = tree::build(vec![process(71, Path::new("/"))]).unwrap();
    let selected = tree::select(&root, "pid:71").unwrap();
    let mut value = serde_json::to_value(snapshot(selected)).unwrap();
    value["selections"][0]["root"]["process"]["command"] = json!({
        "argv": ["python3", "worker job.py", "--token", "[filtered]"],
        "redacted_indices": [3],
        "entrypoint": "worker job.py",
        "unavailable": []
    });
    validate(&value);
    value["selections"][0]["root"]["process"]["command"]["argv"] = json!("not an argument array");
    assert!(!validator().is_valid(&value));
    value["selections"][0]["root"]["process"]["command"] = json!({
        "argv": null, "entrypoint": null, "redacted_indices": [],
        "unavailable": [Diagnostic::new("permission_denied", "command.read", "denied")]
    });
    validate(&value);
    value["selections"][0]["root"]["process"]["command"]["unavailable"] = json!([]);
    assert!(!validator().is_valid(&value));
    value["selections"][0]["root"]["process"]
        .as_object_mut()
        .unwrap()
        .remove("command");
    validate(&value);
}

fn snapshot(root: Node) -> Snapshot {
    Snapshot {
        schema: JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 1_700_000_000_000,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary {
            cpu_percent: Some(20.0),
            logical_cpus: Some(8),
            ..SystemSummary::default()
        },
        scope: root.reference.clone(),
        warnings: vec![],
        root,
    }
}

fn process(pid: u32, path: &Path) -> Process {
    Process {
        pid,
        ppid: Some(1),
        uid: Some(501),
        started_unix_us: Some(u64::from(pid)),
        uptime_seconds: Some(60),
        name: "example".into(),
        cwd: Some(path.into()),
        executable: Some("/example/bin/tool".into()),
        cpu_percent: Some(0.0),
        resident_bytes: Some(1024),
        ..Process::default()
    }
}

#[test]
fn schema_validates_classification_scopes_and_depth_without_parsing_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let git = temp.path().join("checkout space_中文");
    fs::create_dir_all(git.join(".git")).unwrap();
    let manifest = temp.path().join("manifest");
    fs::create_dir(&manifest).unwrap();
    fs::write(manifest.join("go.mod"), "module example.test/tool").unwrap();
    let mut app = process(10, temp.path());
    app.executable = Some(temp.path().join("Example.app/Contents/MacOS/example"));
    app.network = Some(Network {
        complete: true,
        unavailable: vec![],
        sockets: vec![Socket {
            fd: 4,
            protocol: Protocol::Tcp,
            local: "[::1]:8080".parse().unwrap(),
            remote: Some("[::1]:45678".parse().unwrap()),
            tcp_state: Some("Established".into()),
        }],
    });
    let mut helper = process(11, temp.path());
    helper.ppid = Some(10);
    helper.executable = None;
    let mut denied = Process {
        pid: 99,
        ..Process::default()
    };
    denied.unavailable.push(Diagnostic::io(
        "identity",
        &std::io::Error::from_raw_os_error(libc::EPERM),
    ));
    let invalid_git = temp.path().join("broken_checkout");
    fs::create_dir(&invalid_git).unwrap();
    fs::write(invalid_git.join(".git"), "not a gitdir").unwrap();
    let mut root = tree::build(vec![
        process(50, &invalid_git),
        app,
        helper,
        process(20, &git),
        process(30, &manifest),
        process(40, temp.path()),
        process(41, temp.path()),
        denied,
    ])
    .unwrap();
    let mut pending = vec![&mut root];
    while let Some(node) = pending.pop() {
        node.evidence = vec!["This prose is deliberately unrelated to paths.".into()];
        pending.extend(&mut node.children);
    }
    let full = snapshot(root);
    for depth in [0, 1, 3, usize::MAX] {
        let mut selected = full.clone();
        tree::limit_depth(&mut selected.root, depth);
        let value = serde_json::to_value(&selected).unwrap();
        validate(&value);
        assert_eq!(
            value["selections"][0]["root"]["totals"],
            serde_json::to_value(&full.root.totals).unwrap()
        );
        if depth == 0 {
            assert!(
                value["selections"][0]["root"]["omitted_children"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
    }
    for (pid, source, path) in [
        (20, "git_root", git.canonicalize().unwrap()),
        (30, "manifest", manifest.canonicalize().unwrap()),
    ] {
        let selected = snapshot(tree::select(&full.root, &format!("pid:{pid}")).unwrap());
        let value = serde_json::to_value(selected).unwrap();
        validate(&value);
        assert_eq!(
            value["selections"][0]["root"]["attribution"]["source"],
            source
        );
        assert_eq!(
            value["selections"][0]["root"]["attribution"]["path"],
            path.to_str().unwrap()
        );
        assert_eq!(
            value["selections"][0]["root"]["attribution"]["inferred"],
            false
        );
    }
    let helper =
        serde_json::to_value(snapshot(tree::select(&full.root, "pid:11").unwrap())).unwrap();
    validate(&helper);
    assert_eq!(
        helper["selections"][0]["root"]["attribution"]["source"],
        "app_bundle"
    );
    assert_eq!(
        helper["selections"][0]["root"]["attribution"]["inferred"],
        true
    );
    assert_eq!(
        helper["selections"][0]["root"]["attribution"]["via_pid"],
        10
    );
    let denied =
        serde_json::to_value(snapshot(tree::select(&full.root, "pid:99").unwrap())).unwrap();
    validate(&denied);
    assert_eq!(
        denied["selections"][0]["root"]["process"]["unavailable"][0]["code"],
        "permission_denied"
    );
    assert_eq!(
        denied["selections"][0]["root"]["process"]["unavailable"][0]["os_error"],
        libc::EPERM
    );
    assert!(denied["selections"][0]["root"]["process"]["resident_bytes"].is_null());
    assert_eq!(
        denied["selections"][0]["root"]["totals"]["known_resident_bytes"],
        0
    );
    assert_eq!(
        denied["selections"][0]["root"]["totals"]["memory_unknown"],
        1
    );
    let empty = snapshot(tree::build(vec![]).unwrap());
    validate(&serde_json::to_value(empty).unwrap());
    temp.close().unwrap();
}

#[test]
fn schema_rejects_wrong_types_missing_fields_and_invalid_node_variants() {
    let good = serde_json::to_value(snapshot(
        tree::build(vec![Process {
            pid: 10,
            ..Process::default()
        }])
        .unwrap(),
    ))
    .unwrap();
    validate(&good);
    for (pointer, invalid) in [
        ("/schema", json!("reswho/7")),
        ("/sample_ms", json!(-1)),
        ("/sample_ms", json!("250ms")),
        ("/sample_ms", json!(0.5)),
        ("/system/cpu_percent", json!(101)),
        ("/selections/0/root/kind", json!("process")),
        ("/selections/0/root/grouping", Value::Null),
        ("/selections/0/root/runtime", json!({})),
        (
            "/selections/0/root/children/0/totals/known_resident_bytes",
            json!("1MiB"),
        ),
        ("/warnings", json!(["unstructured error"])),
        (
            "/warnings",
            json!([{"code": "collection_failed", "operation": "docker", "message": "x", "os_error": null, "reason": "sandboxed"}]),
        ),
    ] {
        let mut value = good.clone();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(!validator().is_valid(&value), "accepted invalid {pointer}");
    }
    let mut missing = good.clone();
    missing["selections"][0]["root"]
        .as_object_mut()
        .unwrap()
        .remove("children");
    assert!(!validator().is_valid(&missing));
    let mut extra = good;
    extra["extension_field"] = json!({"future": true});
    validate(&extra);
}

#[test]
fn published_examples_validate_and_match_current_serialized_shapes() {
    for name in ["host", "containers", "collapsed", "unavailable"] {
        let text = fs::read_to_string(format!(
            "{}/docs/examples/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        validate(&value);
        assert_eq!(
            value["selections"][0]["selector"],
            value["selections"][0]["root"]["reference"]
        );
    }
    let node = Node::group(
        "group:all".into(),
        "Resources".into(),
        vec![],
        Grouping::Category,
    );
    let value = serde_json::to_value(snapshot(node)).unwrap();
    validate(&value);
    let diagnostic = serde_json::to_value(Diagnostic::failure(
        "collection_failed",
        "docker",
        "wording can change",
        FailureReason::PermissionDenied,
        Some(13),
    ))
    .unwrap();
    assert_eq!(diagnostic["reason"], "permission_denied");
    assert!(
        serde_json::to_value(Diagnostic::new("unavailable", "cpu", "x"))
            .unwrap()
            .get("reason")
            .is_none()
    );
    let shape: Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
    for (def, actual) in [
        ("node", &value["selections"][0]["root"]),
        ("system", &value["system"]),
        ("totals", &value["selections"][0]["root"]["totals"]),
        ("diagnostic", &diagnostic),
    ] {
        let mut expected: Vec<_> = shape["$defs"][def]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| def != "node" || key.as_str() != "host_resources")
            .collect();
        if def == "node" {
            assert!(actual.get("host_resources").is_none());
            assert!(
                !shape["$defs"][def]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("host_resources"))
            );
        }
        let mut actual: Vec<_> = actual.as_object().unwrap().keys().collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected, "{def}");
    }
}

#[test]
fn schema_cli_is_offline_and_never_collects_resources() {
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--json-schema")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        serde_json::from_str::<Value>(output::JSON_SCHEMA).unwrap()
    );
    let invalid = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .args(["--json-schema", "--json"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
}

#[test]
fn schema8_to_12_remain_available_while_current_output_is_schema13() {
    let legacy_schema: Value =
        serde_json::from_str(include_str!("../schemas/reswho-8.schema.json")).unwrap();
    let legacy = jsonschema::validator_for(&legacy_schema).unwrap();
    let mut example: Value =
        serde_json::from_str(include_str!("../docs/examples/host.json")).unwrap();
    assert_eq!(example["schema"], "reswho/13");
    assert!(validator().is_valid(&example));
    assert!(!legacy.is_valid(&example));
    let selection = example["selections"][0].clone();
    let object = example.as_object_mut().unwrap();
    object.remove("selections");
    object.insert("scope".into(), selection["selector"].clone());
    object.insert("root".into(), selection["root"].clone());
    example["schema"] = json!("reswho/13");
    assert!(!validator().is_valid(&example));
    example["schema"] = json!("reswho/12");
    let twelve: Value =
        serde_json::from_str(include_str!("../schemas/reswho-12.schema.json")).unwrap();
    assert!(
        jsonschema::validator_for(&twelve)
            .unwrap()
            .is_valid(&example)
    );
    assert!(!validator().is_valid(&example));
    example["schema"] = json!("reswho/10");
    let ten: Value =
        serde_json::from_str(include_str!("../schemas/reswho-10.schema.json")).unwrap();
    assert!(jsonschema::validator_for(&ten).unwrap().is_valid(&example));
    example["schema"] = json!("reswho/9");
    let nine: Value =
        serde_json::from_str(include_str!("../schemas/reswho-9.schema.json")).unwrap();
    assert!(jsonschema::validator_for(&nine).unwrap().is_valid(&example));
    example["schema"] = json!("reswho/8");
    assert!(legacy.is_valid(&example));
    assert!(!validator().is_valid(&example));
}

#[test]
fn tag_details_are_projected_before_rendering_and_children_always_remain_summary() {
    use reswho::extensions::tags::{Effective, Tag};
    let mut parent = process(71, Path::new("/tmp"));
    parent.executable = Some("/tmp/tool".into());
    let mut child = parent.clone();
    child.pid = 72;
    child.ppid = Some(71);
    child.started_unix_us = Some(72);
    let mut root = tree::build(vec![parent, child]).unwrap();
    fn attach(node: &mut Node) {
        if let Some(p) = &mut node.process {
            let mut tag: Tag = serde_json::from_value(json!({"dimension":"role","id":"role:test","label":format!("private-tag-{}", p.pid),"path":null,"basis":"observed"})).unwrap();
            tag.label = format!("private-tag-{}", p.pid);
            p.tag_report.as_mut().unwrap().effective.push(Effective {
                source: "fixture".into(),
                tag,
            });
            node.evidence.push(format!("private-evidence-{}", p.pid));
        }
        for child in &mut node.children {
            attach(child);
        }
    }
    attach(&mut root);
    let summary = snapshot(root.clone());
    let value = serde_json::to_value(&summary).unwrap();
    validate(&value);
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(
        !encoded.contains("tag_report")
            && !encoded.contains("private-tag")
            && !encoded.contains("private-evidence")
            && !encoded.contains("\"roles\"")
    );
    assert!(!output::markdown(&summary).contains("private-tag"));
    let selected = snapshot(tree::select(&root, "pid:71").unwrap());
    let value = serde_json::to_value(&selected).unwrap();
    validate(&value);
    assert!(value["selections"][0]["root"]["process"]["tag_report"].is_object());
    assert!(
        value["selections"][0]["root"]["children"][0]["process"]
            .get("tag_report")
            .is_none()
    );
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(
        encoded.contains("private-tag-71")
            && !encoded.contains("private-tag-72")
            && !encoded.contains("private-evidence-72")
    );
    let md = output::markdown_with_depth(&selected, usize::MAX);
    assert!(md.contains("private-tag-71") && !md.contains("private-tag-72"));
    let mut empty = tree::select(&root, "pid:71").unwrap();
    empty.process.as_mut().unwrap().tag_report = Some(Default::default());
    let value = serde_json::to_value(snapshot(empty)).unwrap();
    validate(&value);
    assert_eq!(
        value["selections"][0]["root"]["process"]["tag_report"]["effective"],
        json!([])
    );
    let mut leaked = value.clone();
    leaked["selections"][0]["root"]["children"][0]["process"]["tag_report"] =
        value["selections"][0]["root"]["process"]["tag_report"].clone();
    assert!(!validator().is_valid(&leaked));
}

#[test]
fn schema13_rejects_removed_extension_versions_but_keeps_software_versions() {
    let schema: Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
    let mut contracts = vec![
        schema["properties"]["extensions"]["items"].clone(),
        schema["$defs"]["tagReport"]["properties"]["proposals"]["items"].clone(),
    ];
    contracts.extend(
        [
            "extensionAttribution",
            "extensionDetails",
            "resource",
            "role",
        ]
        .map(|key| schema["$defs"][key].clone()),
    );
    for contract in contracts {
        assert!(contract["properties"].get("version").is_none());
        assert_eq!(contract["additionalProperties"], false);
    }
    let mut value = serde_json::to_value(snapshot(
        tree::build(vec![process(1, Path::new("/"))]).unwrap(),
    ))
    .unwrap();
    value["extensions"] = json!([{"provider":"project","phase":"classify","result":"success","reason":null,"duration_ms":0.1}]);
    validate(&value);
    for field in [
        "version",
        "author",
        "provenance",
        "revision",
        "key_id",
        "source",
    ] {
        let mut removed = value.clone();
        removed["extensions"][0][field] = json!("obsolete");
        assert!(!validator().is_valid(&removed), "{field}");
    }
    value["extensions"][0]["phase"] = json!("verification");
    assert!(!validator().is_valid(&value));
    assert!(
        schema["$defs"]["process"]["properties"]
            .get("version")
            .is_some()
    );
    assert!(
        schema["$defs"]["node"]["properties"]
            .get("version")
            .is_some()
    );
}
