use std::process::Command;

#[test]
fn help_is_available_without_resource_collection() {
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--help-format")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("reswho-md/19"));
    assert!(text.contains("root=0"));
    assert!(text.contains("Group ⏱︎ is the longest known member uptime"));
    assert!(text.contains("ΣN is the total host process count"));
    assert!(text.contains("[+N] counts collapsed direct children"));
    assert!(text.contains("reswho --md --from REF --depth N"));
    assert!(!text.contains("Captured:"));
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
    for flag in ["--help", "--help-format", "--json-schema"] {
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
                "docker",
                "apple",
                "app_bundle",
                "command",
                "project",
                "ancestor",
                "bin"
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
        assert_eq!(entries[2]["capabilities"][0], "classify");
        assert_eq!(entries[3]["capabilities"][0], "details");
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
    assert_eq!(entries.as_array().unwrap().len(), 7);
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
    let run = |args: &[&str]| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--config")
            .arg(&config)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let timed_out = child.try_wait().unwrap().is_none();
        if timed_out {
            child.kill().unwrap();
        }
        let result = child.wait_with_output().unwrap();
        assert!(!timed_out, "offline extension command blocked on FIFO");
        result
    };
    assert!(
        run(&["extensions", "install", manifest.to_str().unwrap()])
            .status
            .success()
    );
    value["id"] = "healthy".into();
    value["executable"] = "/usr/bin/false".into();
    fs::write(&manifest, value.to_string()).unwrap();
    assert!(
        run(&["extensions", "install", manifest.to_str().unwrap()])
            .status
            .success()
    );
    fs::remove_file(&binary).unwrap();
    let name = CString::new(binary.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
    let result = run(&["extensions", "--json"]);
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
        !run(&["extensions", "install", manifest.to_str().unwrap()])
            .status
            .success()
    );
    fs::remove_file(&manifest).unwrap();
    let name = CString::new(manifest.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
    assert!(
        !run(&["extensions", "install", manifest.to_str().unwrap()])
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
