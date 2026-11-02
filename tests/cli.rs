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
    assert!(text.contains("reswho-md/18"));
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
