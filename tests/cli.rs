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
