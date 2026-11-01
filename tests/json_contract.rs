use reswho::{
    model::{
        Diagnostic, Grouping, JSON_SCHEMA_VERSION, Network, Node, Process, Protocol, Snapshot,
        Socket, SystemSummary,
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

fn snapshot(root: Node) -> Snapshot {
    Snapshot {
        schema: JSON_SCHEMA_VERSION,
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
            value["root"]["totals"],
            serde_json::to_value(&full.root.totals).unwrap()
        );
        if depth == 0 {
            assert!(value["root"]["omitted_children"].as_u64().unwrap() > 0);
        }
    }
    for (pid, source, path) in [
        (20, "git_root", git.canonicalize().unwrap()),
        (30, "manifest", manifest.canonicalize().unwrap()),
    ] {
        let selected = snapshot(tree::select(&full.root, &format!("pid:{pid}")).unwrap());
        let value = serde_json::to_value(selected).unwrap();
        validate(&value);
        assert_eq!(value["root"]["attribution"]["source"], source);
        assert_eq!(value["root"]["attribution"]["path"], path.to_str().unwrap());
        assert_eq!(value["root"]["attribution"]["inferred"], false);
    }
    let helper =
        serde_json::to_value(snapshot(tree::select(&full.root, "pid:11").unwrap())).unwrap();
    validate(&helper);
    assert_eq!(helper["root"]["attribution"]["source"], "app_bundle");
    assert_eq!(helper["root"]["attribution"]["inferred"], true);
    assert_eq!(helper["root"]["attribution"]["via_pid"], 10);
    let denied =
        serde_json::to_value(snapshot(tree::select(&full.root, "pid:99").unwrap())).unwrap();
    validate(&denied);
    assert_eq!(
        denied["root"]["process"]["unavailable"][0]["code"],
        "permission_denied"
    );
    assert_eq!(
        denied["root"]["process"]["unavailable"][0]["os_error"],
        libc::EPERM
    );
    assert!(denied["root"]["process"]["resident_bytes"].is_null());
    assert_eq!(denied["root"]["totals"]["known_resident_bytes"], 0);
    assert_eq!(denied["root"]["totals"]["memory_unknown"], 1);
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
        ("/root/kind", json!("process")),
        ("/root/grouping", Value::Null),
        ("/root/runtime", json!({})),
        (
            "/root/children/0/totals/known_resident_bytes",
            json!("1MiB"),
        ),
        ("/warnings", json!(["unstructured error"])),
    ] {
        let mut value = good.clone();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(!validator().is_valid(&value), "accepted invalid {pointer}");
    }
    let mut missing = good.clone();
    missing["root"].as_object_mut().unwrap().remove("children");
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
        assert_eq!(value["scope"], value["root"]["reference"]);
    }
    let node = Node::group(
        "group:all".into(),
        "Resources".into(),
        vec![],
        Grouping::Category,
    );
    let value = serde_json::to_value(snapshot(node)).unwrap();
    validate(&value);
    let diagnostic =
        serde_json::to_value(Diagnostic::new("unavailable", "cpu", "wording can change")).unwrap();
    let shape: Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
    for (def, actual) in [
        ("node", &value["root"]),
        ("system", &value["system"]),
        ("totals", &value["root"]["totals"]),
        ("diagnostic", &diagnostic),
    ] {
        let mut expected: Vec<_> = shape["$defs"][def]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
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
