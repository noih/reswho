use std::process::Command;

#[test]
fn disk_usage_deadline_preserves_selected_runtime_and_container_metrics() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.json");
    std::fs::write(&config, serde_json::json!({
        "version": 1, "settings": {},
        "disabled_builtins": ["apple", "podman", "project", "ancestor", "versioned_install", "node", "python", "launched_by", "vm", "codex", "claude", "playwright"]
    }).to_string()).unwrap();
    let docker = root.path().join("docker");
    std::fs::write(&docker, r#"#!/bin/sh
case "$1" in --host=*) shift;; *) exit 8;; esac
id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
case "$1 $2" in
  'info --format') printf '%s\n' '{"id":"review-daemon","os":"linux"}';;
  'container ls') printf '%s\n' "$id";;
  'container inspect') printf '{"id":"%s","name":"/fixture","state":"running","started_at":"2026-01-01T00:00:00Z","mounts":[]}\n' "$id";;
  'container stats') printf '{"id":"%s","cpu":"0.00%%","memory":"1.5MiB / 2GiB"}\n' "$id";;
  'system df') printf '%s\n' "$$" > "$0.pid"; exec /bin/sleep 15;;
  *) exit 9;;
esac
"#).unwrap();
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let run = |selector: &str| {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .args(["--json", "--depth", "all", "--from", selector, "--config"])
            .arg(&config)
            .env("PATH", root.path())
            .env("XDG_CONFIG_HOME", root.path())
            .env("DOCKER_HOST", "tcp://fixture.invalid:2376")
            .env_remove("DOCKER_CONTEXT")
            .env_remove("DOCKER_API_VERSION")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let overview = run("group:containers");
    let runtime = &overview["selections"][0]["root"]["children"][0];
    let reference = runtime["reference"].as_str().unwrap();
    assert_eq!(runtime["runtime"]["status"], "available");
    assert!(!docker.with_extension("pid").exists());
    let detail = run(reference);
    let runtime = &detail["selections"][0]["root"];
    assert_eq!(runtime["reference"], reference);
    assert_eq!(runtime["runtime"]["status"], "available");
    assert_eq!(runtime["runtime"]["daemon_id"], "review-daemon");
    assert!(runtime["runtime"].get("disk_usage").is_none());
    assert!(
        runtime["runtime"]["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| { d["operation"] == "docker.disk_usage" && d["reason"] == "timeout" })
    );
    fn memory(node: &serde_json::Value) -> Option<u64> {
        node.get("container")
            .and_then(|container| container["memory_bytes"].as_u64())
            .or_else(|| node.get("children")?.as_array()?.iter().find_map(memory))
    }
    assert_eq!(memory(runtime), Some(1_572_864));
    assert!(
        detail["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| {
                record["provider"] == "docker"
                    && record["phase"] == "collect"
                    && record["result"] == "success"
            })
    );
    let pid: i32 = std::fs::read_to_string(docker.with_extension("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    root.close().unwrap();
}

#[test]
fn help_is_available_without_resource_collection() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "invalid config: must not be read").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .arg("--config")
            .arg(&config)
            .args(args)
            .output()
            .unwrap()
    };
    let result = run(&["--help-md"]);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let obsolete = run(&["--help-format"]);
    assert_eq!(obsolete.status.code(), Some(2));
    assert!(obsolete.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&obsolete.stderr).contains("unexpected argument '--help-format'")
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for meaning in [
        "reswho-md/23",
        "root=0",
        "Group ⏱︎ is the longest",
        "ΣN is the total host process count",
        "[+N] counts collapsed direct children",
        "Classification",
        "by default. `ownership_priority`",
        "sources within a dimension",
        "[resource:kind]",
        "not a security",
        "README.md for installation and usage",
        "--json-schema and docs/json-format.md",
    ] {
        assert!(text.contains(meaning), "missing {meaning}");
    }
    assert!(!text.contains("Command arguments, environment"));
    assert!(!text.contains("docs/extension-protocol.md"));
    for flag in ["-h", "--help"] {
        let result = run(&[flag]);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        let text = String::from_utf8(result.stdout).unwrap();
        for heading in [
            "Output:",
            "Selection/expansion:",
            "Sorting:",
            "Configuration:",
        ] {
            assert!(text.contains(heading), "missing {heading}");
        }
        assert!(text.contains("--help-md"));
        assert!(!text.contains("--help-format"));
        assert!(!text.contains("collection prototype"));
        assert!(text.lines().all(|line| line.chars().count() <= 80));
    }
    for args in [
        vec!["--json-schema", "--help-md"],
        vec!["--help-md", "--json-schema"],
        vec!["--json-schema", "--help-md", "settings", "--json"],
    ] {
        let result = run(&args);
        assert!(!result.status.success(), "accepted {args:?}");
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "invalid config: must not be read"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn invalid_flags_and_tui_without_terminal_report_on_stderr_only() {
    for args in [
        vec!["--md", "--json"],
        vec!["--depth", "many"],
        vec!["--tui"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
}

#[test]
fn command_line_exposes_no_mutation_commands() {
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("stop")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn settings_are_offline_and_cli_overrides_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"version":1,"settings":{"sampling.interval_ms":400,"refresh.interval_ms":3000,"future.token":"SECRET"}}"#).unwrap();
    let before = std::fs::read(&path).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .args(["settings", "--json", "--config"])
            .arg(&path)
            .args(extra)
            .output()
            .unwrap()
    };
    let result = run(&[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 400);
    assert_eq!(report["settings"]["sampling.interval_ms"]["source"], "user");
    assert_eq!(report["settings"]["refresh.interval_ms"]["value"], 3000);
    assert_eq!(
        report["settings"]["refresh.interval_ms"]["definition"]["provider"],
        "core"
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("SECRET"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("SECRET"));
    let result = run(&[
        "--set",
        "sampling.interval_ms=500",
        "--set",
        "refresh.interval_ms=6000",
    ]);
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 500);
    assert_eq!(report["settings"]["sampling.interval_ms"]["source"], "cli");
    assert_eq!(report["settings"]["refresh.interval_ms"]["value"], 6000);
    assert_eq!(before, std::fs::read(&path).unwrap());
}

#[test]
fn config_failures_defaults_and_help_do_not_collect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let result = run(&["settings", "--json"]);
    assert!(result.status.success());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 250);
    assert_eq!(report["settings"]["refresh.interval_ms"]["value"], 2000);
    let text = run(&["settings"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("source: default, definition: core"));
    let path = path.to_str().unwrap();
    let missing = run(&["settings", "--config", path]);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    std::fs::write(
        path,
        r#"{"version":1,"settings":{"sampling.interval_ms":"SECRET"}}"#,
    )
    .unwrap();
    let invalid = run(&["settings", "--config", path]);
    assert!(invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stdout).contains("sampling.interval_ms = 250"));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("sampling.interval_ms"));
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("SECRET"));
    assert!(
        run(&[
            "settings",
            "--config",
            path,
            "--set",
            "sampling.interval_ms=400"
        ])
        .status
        .success()
    );
    for flag in ["--help", "--help-md", "--json-schema"] {
        assert!(run(&[flag, "--config", path]).status.success());
    }
    for contents in ["SECRET", r#"{"version":2,"settings":{}}"#] {
        std::fs::write(path, contents).unwrap();
        let invalid = run(&["settings", "--config", path]);
        assert!(invalid.status.success());
        assert!(String::from_utf8_lossy(&invalid.stdout).contains("source: default"));
        assert!(!String::from_utf8_lossy(&invalid.stderr).contains("SECRET"));
        assert!(run(&["--json-schema", "--config", path]).status.success());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn settings_json_rejects_format_conflicts_across_subcommand_positions() {
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--json-schema", "--md", "--tui"] {
        for args in [
            vec![flag, "settings", "--json"],
            vec![flag, "--json", "settings"],
            vec!["--json", flag, "settings"],
            vec!["--json", "settings", flag],
            vec!["settings", flag, "--json"],
            vec!["settings", "--json", flag],
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
                .env("XDG_CONFIG_HOME", dir.path())
                .args(&args)
                .output()
                .unwrap();
            assert!(!result.status.success(), "accepted {args:?}");
            assert!(result.stdout.is_empty(), "{args:?}");
            assert!(!result.stderr.is_empty(), "{args:?}");
        }
    }
    for args in [["settings", "--json"], ["--json", "settings"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(result.status.success());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 250);
    }
}

#[test]
fn extensions_list_is_offline_and_distinguishes_enabled_from_availability() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["extensions", "--json"], vec!["--json", "extensions"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .env("PATH", "")
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let entries: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let entries = entries.as_array().unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
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
                "vm",
                "codex",
                "claude",
                "playwright"
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry["availability"] == "unknown"
                    && entry.get("version").is_none()
                    && entry.get("source").is_none())
        );
        assert!(entries[0]["enabled"].as_bool().unwrap());
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry["id"] == "project")
                .unwrap()["capabilities"][0],
            "classify"
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry["id"] == "docker")
                .unwrap()["capabilities"][0],
            "collect"
        );
    }
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .env("XDG_CONFIG_HOME", dir.path())
        .env("PATH", "")
        .args(["extensions", "--no-containers"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("docker capabilities=collect enabled=false availability=unknown"));
    assert!(!output.contains("Loading"));
    dir.close().unwrap();
}

#[test]
fn generic_overrides_preserve_order_and_reject_obsolete_flags() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&[
        "--set",
        "sampling.interval_ms=SECRET",
        "--set=refresh.interval_ms=6000",
        "settings",
        "--json",
        "--set",
        "sampling.interval_ms=400",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 400);
    assert_eq!(report["settings"]["refresh.interval_ms"]["value"], 6000);
    for flag in ["--sample-ms", "--refresh-ms"] {
        let output = run(&["settings", flag, "400"]);
        assert_eq!(output.status.code(), Some(2));
    }
    for args in [
        vec!["--set", "source_priority=SECRET"],
        vec!["--set", "sampling.interval_ms=SECRET"],
        vec!["--set", "sampling.interval_ms=0"],
        vec!["--set", "sampling.interval_ms=200.5"],
        vec!["--set", "SECRET", "--set", "sampling.interval_ms=400"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET"));
    }
    for action in ["enable", "disable"] {
        let output = run(&[
            "--set",
            "sampling.interval_ms=400",
            "extensions",
            action,
            "absent",
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--set cannot be used"));
        assert!(!dir.path().join("reswho/extensions.json").exists());
    }
}

struct OwnedSleep(std::process::Child);

impl Drop for OwnedSleep {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn repeated_from_shares_one_snapshot_and_reports_missing_selections() {
    let spawn = || {
        OwnedSleep(
            Command::new("/bin/sleep")
                .arg("60")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let (first, second) = (spawn(), spawn());
    let (a, b) = (
        format!("pid:{}", first.0.id()),
        format!("pid:{}", second.0.id()),
    );
    let missing = "pid:4294967295";
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .args(["--no-containers", "--depth", "0", "--sort", "name"])
            .args(args)
            .output()
            .unwrap()
    };
    let selections = [
        "--from",
        &a,
        "--from",
        &b,
        "--from",
        &a,
        "--from",
        "group:all",
        "--from",
        missing,
    ];
    let json = run(&[&["--json"][..], &selections].concat());
    assert_eq!(json.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&json.stderr).contains("1 selection(s) not found"));
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let schema: serde_json::Value = serde_json::from_str(reswho::output::JSON_SCHEMA).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&value)
        .unwrap();
    assert!(value.get("scope").is_none() && value.get("root").is_none());
    assert!(value["captured_unix_ms"].as_u64().unwrap() > 0);
    let selections = value["selections"].as_array().unwrap();
    let selectors: Vec<_> = selections
        .iter()
        .map(|s| s["selector"].as_str().unwrap())
        .collect();
    assert_eq!(selectors, [a.as_str(), b.as_str(), "group:all", missing]);
    for selection in &selections[..2] {
        assert_eq!(selection["root"]["reference"], selection["selector"]);
        assert_eq!(selection["nested_in"], serde_json::json!(["group:all"]));
        assert!(selection["root"]["process"]["command"].is_object());
        assert!(selection["diagnostics"].as_array().unwrap().is_empty());
    }
    let all = &selections[2]["root"];
    assert!(all["children"].as_array().unwrap().is_empty());
    assert!(all["omitted_children"].as_u64().unwrap() > 0);
    assert_eq!(selections[3]["root"], serde_json::Value::Null);
    assert_eq!(selections[3]["diagnostics"][0]["code"], "not_found");
    let md = run(&[
        &["--md"][..],
        &[
            "--from",
            &a,
            "--from",
            &b,
            "--from",
            "group:all",
            "--from",
            missing,
        ],
    ]
    .concat());
    assert_eq!(md.status.code(), Some(3));
    let md = String::from_utf8(md.stdout).unwrap();
    assert_eq!(md.matches("Captured: ").count(), 1, "{md}");
    assert!(md.contains(&format!("## Selection 1/4: {a}")), "{md}");
    assert!(md.contains(&format!("## Selection 2/4: {b}")), "{md}");
    assert!(
        md.contains("## Selection 3/4: group:all\n\ngroup:all · "),
        "{md}"
    );
    assert!(
        md.contains(&format!(
            "## Selection 4/4: {missing}\n\n> ! target not found: {missing}"
        )),
        "{md}"
    );
    assert_eq!(
        md.matches("> Inside group:all; totals overlap").count(),
        2,
        "{md}"
    );
    let single = run(&["--json", "--from", &a]);
    assert_eq!(single.status.code(), Some(0));
    let single: serde_json::Value = serde_json::from_slice(&single.stdout).unwrap();
    assert_eq!(single["selections"].as_array().unwrap().len(), 1);
    assert_eq!(single["selections"][0]["nested_in"], serde_json::json!([]));
    let none = run(&["--json", "--from", missing, "--from", "pid:4294967294"]);
    assert_eq!(none.status.code(), Some(1));
    assert!(none.stdout.is_empty());
    assert!(String::from_utf8_lossy(&none.stderr).contains("target not found"));
    for invalid in [
        &["--json", "--from", "pid:0"][..],
        &["--json", "--md", "--from", &a],
        &["--json", "--from", &a, "--from", "group:"],
    ] {
        let result = run(invalid);
        assert_eq!(result.status.code(), Some(2), "{invalid:?}");
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
    let tui = run(&["--tui", "--from", &a, "--from", &b]);
    assert_eq!(tui.status.code(), Some(1));
    assert!(tui.stdout.is_empty());
    assert!(String::from_utf8_lossy(&tui.stderr).contains("--tui accepts one --from"));
    drop((first, second));
}

#[test]
fn builtin_disable_restart_snapshot_fallback_and_enable() {
    use serde_json::{Value, json};
    use std::process::{Child, Stdio};
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    std::fs::write(root.path().join("go.mod"), "module fixture").unwrap();
    std::fs::write(&path, json!({"version":1,"settings":{"future.key":9},"ownership_priority":["future"],"source_priority":["missing"]}).to_string()).unwrap();
    let child = Owned(
        Command::new("/bin/sleep")
            .arg("60")
            .current_dir(root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let capture = || -> Value {
        serde_json::from_slice(
            &run(&[
                "--no-containers",
                "--json",
                "--from",
                &format!("pid:{}", child.0.id()),
            ])
            .stdout,
        )
        .unwrap()
    };
    let has_project = |v: &Value| {
        v["selections"][0]["root"]["process"]["tag_report"]["effective"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["tag"]["dimension"] == "project")
    };
    let initial = capture();
    assert!(has_project(&initial));
    let inventory: Value = serde_json::from_slice(&run(&["extensions", "--json"]).stdout).unwrap();
    assert!(
        inventory
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("version").is_none()
                && entry.get("author").is_none()
                && entry.get("provenance").is_none())
    );
    assert!(
        initial["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("source").is_none())
    );
    run(&["extensions", "disable", "project"]);
    let disabled = capture();
    assert!(!has_project(&disabled));
    assert!(
        disabled["selections"][0]["root"]["process"]["tag_report"]["effective"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["tag"]["dimension"] == "bin")
    );
    let reports: Vec<_> = disabled["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["provider"] == "project")
        .collect();
    assert!(!reports.is_empty());
    assert!(
        reports
            .iter()
            .all(|r| r["result"] == "skipped" && r["duration_ms"].is_null())
    );
    run(&["extensions", "enable", "project"]);
    assert!(has_project(&capture()));
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["settings"]["future.key"], 9);
    assert_eq!(saved["ownership_priority"], json!(["future"]));
    assert_eq!(saved["source_priority"], json!(["missing"]));
    assert_eq!(saved["disabled_builtins"], json!([]));
    drop(child);
    root.close().unwrap();
}

#[test]
fn classifier_builtins_toggle_offline_without_changing_other_config() {
    use serde_json::{Value, json};
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    std::fs::write(&path, json!({"version":1,"settings":{"future.key":9},"ownership_priority":["future"],"disabled_builtins":["unknown"]}).to_string()).unwrap();
    for id in ["launched_by", "vm", "codex", "claude", "playwright"] {
        for (action, enabled) in [("disable", false), ("enable", true)] {
            let run = |args: &[&str]| {
                let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
                    .env("PATH", "")
                    .arg("--config")
                    .arg(&path)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result
            };
            run(&["extensions", action, id]);
            let result = run(&["extensions", "--json"]);
            let entries: Value = serde_json::from_slice(&result.stdout).unwrap();
            let entry = entries
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap();
            assert_eq!(entry["enabled"], enabled);
            assert!(entry.get("version").is_none());
            assert_eq!(entry["capabilities"], json!(["classify"]));
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["settings"], json!({"future.key":9}));
            assert_eq!(saved["ownership_priority"], json!(["future"]));
            assert!(
                saved["disabled_builtins"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("unknown"))
            );
        }
    }
    let owned_path = root.path().to_path_buf();
    root.close().unwrap();
    assert!(!owned_path.exists());
}

#[test]
fn stale_and_malformed_config_recover_save_and_restart_without_losing_original() {
    let owner = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = owner.path().join("config.json");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&path)
            .args(args)
            .output()
            .unwrap()
    };
    for original in [
        r#"{"version":1,"settings":{"sampling.interval_ms":"private-bad","future.key":9}}"#,
        "{private-broken",
        r#"{"version":2,"settings":{}}"#,
    ] {
        std::fs::write(&path, original).unwrap();
        let loaded = run(&["settings", "--json"]);
        assert!(
            loaded.status.success(),
            "{}",
            String::from_utf8_lossy(&loaded.stderr)
        );
        assert!(!String::from_utf8_lossy(&loaded.stderr).contains("private-bad"));
        assert!(!String::from_utf8_lossy(&loaded.stderr).contains("private-broken"));
        let settings: serde_json::Value = serde_json::from_slice(&loaded.stdout).unwrap();
        assert_eq!(settings["settings"]["sampling.interval_ms"]["value"], 250);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert!(run(&["extensions", "disable", "project"]).status.success());
        let restarted = run(&["extensions", "--json"]);
        assert!(restarted.status.success());
        let entries: serde_json::Value = serde_json::from_slice(&restarted.stdout).unwrap();
        assert_eq!(
            entries
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == "project")
                .unwrap()["enabled"],
            false
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if original.contains("future.key") {
            assert_eq!(saved["settings"]["future.key"], 9);
        } else {
            assert!(std::fs::read_dir(owner.path()).unwrap().any(|entry| {
                let p = entry.unwrap().path();
                p.extension().is_some_and(|e| e == "bak")
                    && std::fs::read(p).unwrap() == original.as_bytes()
            }));
        }
    }
    owner.close().unwrap();
}

#[test]
fn obsolete_external_files_and_signed_package_paths_are_inert_even_when_enabled() {
    use serde_json::json;
    use std::{
        fs,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let config = root.path().join("config.json");
    fs::write(&config, r#"{"version":1,"settings":{"extensions.old.enabled":true,"extensions.signed.enabled":true}}"#).unwrap();
    let marker = root.path().join("executed");
    let source = root.path().join("provider.c");
    let binary = root.path().join("provider");
    fs::write(&source, format!("#include <stdio.h>\nint main(void){{FILE *f=fopen({marker:?},\"a\");if(!f)return 1;fputs(\"executed\",f);return fclose(f);}}\n")).unwrap();
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success()
    );
    let manifest = json!({"id":"old","version":"1.0.0","engines":{"reswho":"*"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":binary});
    let xdg = root.path().join("xdg");
    let signed = xdg.join("reswho/extensions/signed");
    fs::create_dir_all(&signed).unwrap();
    fs::write(signed.join("metadata.json"), json!({"format":1,"algorithm":"ML-DSA-65","key_id":"reswho-release-1","id":"signed","version":"1.0.0","revision":1,"author":"Former author","target":"aarch64-apple-darwin","host_api":1,"capabilities":["classify"],"payload":{"file":"provider","bytes":fs::metadata(&binary).unwrap().len(),"sha256":"old-signed-payload"}}).to_string()).unwrap();
    fs::copy(&binary, signed.join("provider")).unwrap();
    fs::write(signed.join("metadata.sig"), vec![0u8; 3309]).unwrap();
    let package = root.path().join("extensions/old");
    fs::create_dir_all(&package).unwrap();
    let package_manifest = package.join("manifest.json");
    fs::write(&package_manifest, manifest.to_string()).unwrap();
    let state = root.path().join("extensions.json");
    let saved = json!({"version":1,"installed":[{"manifest":manifest,"enabled":true,"removed":false}],"shared_definitions":[]}).to_string();
    fs::write(&state, &saved).unwrap();
    let run = |args: &[&str]| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&config)
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", &xdg)
            .env("PATH", "")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("obsolete external files blocked {args:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        child.wait_with_output().unwrap()
    };
    for args in [
        vec!["extensions", "--json"],
        vec!["settings", "--json"],
        vec!["--no-containers", "--json", "--from", "pid:4294967295"],
        vec!["--no-containers", "--md", "--from", "pid:4294967295"],
    ] {
        let output = run(&args);
        assert!(matches!(output.status.code(), Some(0 | 1)));
        assert!(!marker.exists());
        if args == ["extensions", "--json"] {
            let entries: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                entries
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|entry| entry["id"] != "old" && entry["id"] != "signed")
            );
        }
        assert_eq!(fs::read_to_string(&state).unwrap(), saved);
    }
    for args in [
        vec!["extensions", "install", package_manifest.to_str().unwrap()],
        vec!["extensions", "remove", "old"],
        vec!["extensions", "enable", "old"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        if args[1] != "enable" {
            assert_eq!(output.status.code(), Some(2));
        }
        assert!(!marker.exists());
        assert_eq!(fs::read_to_string(&state).unwrap(), saved);
    }
    fs::write(&state, "malformed old state").unwrap();
    assert!(run(&["extensions", "--json"]).status.success());
    assert_eq!(fs::read_to_string(&state).unwrap(), "malformed old state");
    fs::remove_file(&state).unwrap();
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let fifo = CString::new(state.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    for path in [
        signed.join("metadata.json"),
        signed.join("metadata.sig"),
        signed.join("provider"),
        xdg.join("reswho/extensions/entry-fifo"),
    ] {
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    }
    for args in [
        vec!["extensions", "--json"],
        vec!["settings", "--json"],
        vec!["extensions", "disable", "project"],
        vec!["extensions", "enable", "project"],
    ] {
        assert!(run(&args).status.success());
    }
    assert!(!marker.exists());
    assert!(!root.path().join("extensions.lock").exists());
    assert_eq!(
        fs::read_to_string(&package_manifest).unwrap(),
        manifest.to_string()
    );
    let owned = root.path().to_path_buf();
    root.close().unwrap();
    assert!(!owned.exists());
}

#[test]
fn extension_text_inventory_exposes_capabilities_without_package_metadata() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_reswho"))
        .args(["extensions", "--no-containers"])
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.lines()
            .any(|line| line.starts_with("project capabilities=classify")
                && line.contains("enabled=true availability=unknown reason=none")),
        "{text}"
    );
    for field in ["version=", "author=", "provenance=", "revision=", "key_id="] {
        assert!(!text.contains(field), "{text}");
    }
    root.close().unwrap();
}
