use std::process::Command;

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
        "reswho-md/19",
        "root=0",
        "Group ⏱︎ is the longest",
        "ΣN is the total host process count",
        "[+N] counts collapsed direct children",
        "Classification",
        "[resource:kind]",
        "not a security",
        "docs/containers.md",
        "--json-schema and docs/json-format.md",
    ] {
        assert!(text.contains(meaning), "missing {meaning}");
    }
    assert!(!text.contains("Command arguments, environment"));
    for flag in ["-h", "--help"] {
        let result = run(&[flag]);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        let text = String::from_utf8(result.stdout).unwrap();
        for heading in [
            "Output:",
            "Selection/expansion:",
            "Sorting:",
            "Sampling/refresh:",
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
    let result = run(&["--sample-ms", "500", "--refresh-ms", "6000"]);
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["settings"]["sampling.interval_ms"]["value"], 500);
    assert_eq!(report["settings"]["sampling.interval_ms"]["source"], "cli");
    assert_eq!(report["settings"]["refresh.interval_ms"]["value"], 6000);
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
    assert!(String::from_utf8_lossy(&text.stdout).contains("source: default, definition: core/1"));
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
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("sampling.interval_ms"));
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("SECRET"));
    assert!(
        run(&["settings", "--config", path, "--sample-ms", "400"])
            .status
            .success()
    );
    for flag in ["--help", "--help-md", "--json-schema"] {
        assert!(run(&[flag, "--config", path]).status.success());
    }
    for contents in ["SECRET", r#"{"version":2,"settings":{}}"#] {
        std::fs::write(path, contents).unwrap();
        let invalid = run(&["settings", "--config", path]);
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
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
                "app_bundle",
                "project",
                "ancestor",
                "bin",
                "docker",
                "podman",
                "apple",
                "command"
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry["availability"] == "unknown"
                    && entry["source"] == "builtin"
                    && entry["version"] == env!("CARGO_PKG_VERSION"))
        );
        assert!(entries[0]["enabled"].as_bool().unwrap());
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry["id"] == "app_bundle")
                .unwrap()["capabilities"][0],
            "classify"
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry["id"] == "command")
                .unwrap()["capabilities"][0],
            "details"
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
    assert!(output.contains(
        "docker/0.1.0 source=builtin capabilities=collect enabled=false availability=unknown"
    ));
    assert!(!output.contains("Loading"));
    dir.close().unwrap();
}

#[test]
fn lifecycle_commands_are_offline_and_updates_preserve_state_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, r#"{"version":1,"settings":{}}"#).unwrap();
    let manifest = dir.path().join("manifest.json");
    let value = serde_json::json!({"id":"local","version":"1.0.0","engines":{"reswho":">99"},"protocol_version":1,"platforms":["macos"],"capabilities":["collect"],"collect_scopes":["containers"],"executable":"/usr/bin/false"});
    std::fs::write(&manifest, value.to_string()).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        run(&["extensions", "install", manifest.to_str().unwrap()])
            .status
            .success()
    );
    let result = run(&["extensions", "--json"]);
    let entries: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let entry = entries.as_array().unwrap().last().unwrap();
    assert_eq!(entry["version"], "1.0.0");
    assert_eq!(entry["compatible"], false);
    assert_eq!(entry["explicit_enabled"], true);
    assert_eq!(entry["enabled"], false);
    assert!(entry["reason"].as_str().unwrap().contains("engines"));
    let state = dir.path().join("extensions.json");
    let before = std::fs::read(&state).unwrap();
    std::fs::write(&manifest, "{}").unwrap();
    assert!(
        !run(&["extensions", "install", manifest.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(std::fs::read(&state).unwrap(), before);
    for action in ["disable", "enable", "remove"] {
        assert!(run(&["extensions", action, "local"]).status.success());
    }
    let result = run(&["extensions", "--json"]);
    let entries: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 8);
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        r#"{"version":1,"settings":{}}"#
    );
    dir.close().unwrap();
}

#[test]
fn fifo_entrypoints_and_manifests_never_block_offline_lifecycle() {
    use std::{
        ffi::CString,
        fs,
        os::unix::ffi::OsStrExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    fs::write(&config, r#"{"version":1,"settings":{}}"#).unwrap();
    let binary = dir.path().join("provider");
    fs::copy("/usr/bin/false", &binary).unwrap();
    let manifest = dir.path().join("manifest.json");
    let mut value = serde_json::json!({"id":"fifo","version":"1.0.0","engines":{"reswho":"*"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":binary});
    fs::write(&manifest, value.to_string()).unwrap();
    let setup_budget = Duration::from_secs(5);
    let fifo_budget = Duration::from_secs(1);
    let run = |args: &[&str], phase: &str, budget: Duration| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let polled = loop {
            match child.try_wait() {
                Ok(None) if start.elapsed() < budget => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => break result,
            }
        };
        let timed_out = matches!(polled, Ok(None));
        if !matches!(polled, Ok(Some(_))) {
            let _ = child.kill();
        }
        let result = child.wait_with_output().unwrap();
        assert!(
            polled.is_ok(),
            "offline lifecycle {phase} poll failed: {polled:?}"
        );
        assert!(
            !timed_out,
            "offline lifecycle {phase} watchdog (startup + operation, budget={budget:?}): args={args:?}, elapsed={:?}, status={}, stdout_bytes={}, stderr_bytes={}; FIFO blocking cause unproven",
            start.elapsed(),
            result.status,
            result.stdout.len(),
            result.stderr.len()
        );
        result
    };
    assert!(
        run(
            &["extensions", "install", manifest.to_str().unwrap()],
            "setup",
            setup_budget,
        )
        .status
        .success()
    );
    value["id"] = "healthy".into();
    value["executable"] = "/usr/bin/false".into();
    fs::write(&manifest, value.to_string()).unwrap();
    assert!(
        run(
            &["extensions", "install", manifest.to_str().unwrap()],
            "setup",
            setup_budget,
        )
        .status
        .success()
    );
    fs::remove_file(&binary).unwrap();
    let name = CString::new(binary.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
    let result = run(&["extensions", "--json"], "FIFO check", fifo_budget);
    assert!(result.status.success());
    let list: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let entries = list.as_array().unwrap();
    assert_eq!(
        entries.iter().find(|entry| entry["id"] == "fifo").unwrap()["enabled"],
        false
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry["id"] == "healthy")
            .unwrap()["enabled"],
        true
    );
    value["id"] = "fifo".into();
    value["executable"] = binary.to_string_lossy().as_ref().into();
    fs::write(&manifest, value.to_string()).unwrap();
    assert!(
        !run(
            &["extensions", "install", manifest.to_str().unwrap()],
            "FIFO check",
            fifo_budget,
        )
        .status
        .success()
    );
    fs::remove_file(&manifest).unwrap();
    let name = CString::new(manifest.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
    assert!(
        !run(
            &["extensions", "install", manifest.to_str().unwrap()],
            "FIFO check",
            fifo_budget,
        )
        .status
        .success()
    );
    dir.close().unwrap();
}

#[test]
fn dropin_default_root_discovers_without_executing_and_does_not_rewrite_config() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("reswho/extensions/drop");
    std::fs::create_dir_all(&package).unwrap();
    let marker = dir.path().join("executed");
    let source = dir.path().join("provider.c");
    std::fs::write(
        &source,
        format!(
            "#include <stdio.h>\nint main(void){{FILE *f=fopen({:?},\"w\");fclose(f);return 1;}}",
            marker
        ),
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(package.join("provider"))
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(package.join("manifest.json"),serde_json::json!({"id":"drop","version":"1.0.0","engines":{"reswho":"*"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":"provider"}).to_string()).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .env("XDG_CONFIG_HOME", dir.path())
            .env("PATH", "")
            .args(["extensions", "--json"])
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "drop")
        .unwrap();
    assert_eq!(row["enabled"], true);
    assert!(
        row["report"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["phase"] == "classify"
                && record["result"] == "never_invoked"
                && record["duration_ms"].is_null())
    );
    let state = dir.path().join("reswho/extensions.json");
    let before = std::fs::read(&state).unwrap();
    assert!(run().status.success());
    assert_eq!(before, std::fs::read(state).unwrap());
    assert!(!dir.path().join("reswho/config.json").exists());
    assert!(!marker.exists());
    dir.close().unwrap();
}

#[test]
fn promoted_builtin_keeps_old_registry_usable_without_duplicate_inventory() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    let state = dir.path().join("extensions.json");
    let config_bytes=json!({"version":1,"settings":{"sampling.interval_ms":400,"extensions.podman.old":true},"source_priority":["healthy","podman"]}).to_string();
    std::fs::write(&config, &config_bytes).unwrap();
    let manifest = |id: &str| json!({"id":id,"version":"1.0.0","engines":{"reswho":"*"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":"/usr/bin/false"});
    std::fs::write(&state,json!({"version":1,"installed":[{"manifest":manifest("podman"),"enabled":true,"removed":false},{"manifest":manifest("healthy"),"enabled":true,"removed":false}],"shared_definitions":[]}).to_string()).unwrap();
    let before = std::fs::read(&state).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .env("PATH", "")
            .output()
            .unwrap()
    };
    let result = run(&["extensions", "--json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let entries: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let entries = entries.as_array().unwrap();
    let podman: Vec<_> = entries.iter().filter(|e| e["id"] == "podman").collect();
    assert_eq!(podman.len(), 1);
    assert_eq!(podman[0]["source"], "builtin");
    assert_eq!(podman[0]["enabled"], true);
    assert!(
        podman[0]["report"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "external_builtin_id_conflict")
    );
    assert!(
        entries
            .iter()
            .any(|e| e["id"] == "healthy" && e["enabled"] == true)
    );
    assert_eq!(before, std::fs::read(&state).unwrap());
    let settings = run(&["settings", "--json"]);
    assert!(settings.status.success());
    let settings: serde_json::Value = serde_json::from_slice(&settings.stdout).unwrap();
    assert_eq!(settings["settings"]["sampling.interval_ms"]["value"], 400);
    assert_eq!(settings["source_priority"], json!(["healthy", "podman"]));
    let reserved = dir.path().join("reserved.json");
    std::fs::write(&reserved, manifest("podman").to_string()).unwrap();
    assert!(
        !run(&["extensions", "install", reserved.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(before, std::fs::read(&state).unwrap());
    for action in ["disable", "enable", "remove"] {
        assert!(run(&["extensions", action, "healthy"]).status.success());
    }
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(saved["installed"][0]["manifest"]["id"], "podman");
    assert_eq!(saved["installed"][0]["enabled"], true);
    assert_eq!(saved["installed"][1]["removed"], true);
    assert_eq!(std::fs::read_to_string(&config).unwrap(), config_bytes);
    dir.close().unwrap();
}
