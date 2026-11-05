use std::process::{Child, Command, Stdio};

struct OwnedProcess(Child);

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        self.0.kill().expect("stop owned fixture process");
        self.0.wait().expect("reap owned fixture process");
    }
}

#[test]
#[ignore = "macOS live process inspection; launches and reaps one owned sleep fixture"]
fn live_process_identity_cwd_and_project_can_be_queried() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".git")).unwrap();
    let child = OwnedProcess(
        Command::new("/bin/sleep")
            .arg("60")
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let reference = format!("pid:{}", child.0.id());
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--json", "--from", &reference, "--depth", "0"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let snapshot: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let schema: serde_json::Value = serde_json::from_str(reswho::output::JSON_SCHEMA).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&snapshot)
        .unwrap();
    assert_eq!(snapshot["schema"], reswho::model::JSON_SCHEMA_VERSION);
    assert_eq!(
        snapshot["selections"][0]["root"]["attribution"]["source"],
        "git_root"
    );
    assert_eq!(snapshot["selections"][0]["selector"], reference);
    assert_eq!(snapshot["selections"][0]["root"]["reference"], reference);
    let process = &snapshot["selections"][0]["root"]["process"];
    assert_eq!(process["pid"], child.0.id());
    assert_eq!(process["ppid"], std::process::id());
    assert_eq!(
        process["cwd"],
        directory.path().canonicalize().unwrap().to_str().unwrap()
    );
    assert!(process["resident_bytes"].as_u64().unwrap() > 0);
    assert!(process["cpu_percent"].as_f64().unwrap() >= 0.0);
    assert!(process["started_unix_us"].as_u64().unwrap() > 0);
    assert_eq!(
        process["command"]["argv"],
        serde_json::json!(["/bin/sleep", "60"])
    );
    assert!(
        process["command"]["unavailable"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(process["network"]["complete"], true);
    assert!(process["network"]["sockets"].as_array().unwrap().is_empty());
    assert_eq!(snapshot["selections"][0]["root"]["totals"]["processes"], 1);
    assert!(
        snapshot["selections"][0]["root"]["children"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(snapshot["sample_ms"].as_u64().unwrap() >= 200);
    let tree = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--json", "--from", "group:projects", "--depth", "all"])
        .output()
        .unwrap();
    assert!(tree.status.success());
    let tree: serde_json::Value = serde_json::from_slice(&tree.stdout).unwrap();
    let groups = tree["selections"][0]["root"]["children"]
        .as_array()
        .unwrap();
    let group = groups
        .iter()
        .find(|group| {
            group["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["reference"] == reference)
        })
        .expect("fixture project is classified");
    let group_ref = group["reference"].as_str().unwrap();
    let member = group["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["reference"] == reference)
        .unwrap();
    assert!(member["process"]["command"].is_null());
    let md = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--from", group_ref, "--depth", "0"])
        .output()
        .unwrap();
    assert!(md.status.success());
    let md = String::from_utf8(md.stdout).unwrap();
    assert!(md.starts_with("# reswho\n"));
    assert!(md.contains(group_ref));
    assert!(md.contains("Σ1 [+1]\n"));
    assert!(md.contains("  ⏱︎ "));
    assert!(!md.contains("reswho --md --from"));
    assert!(md.contains(&format!(
        "> git `{}`",
        directory.path().canonicalize().unwrap().display()
    )));
    assert!(!md.contains("Git root from working directory:"));
    assert!(!md.contains('\u{1b}'));
    std::fs::remove_dir(directory.path().join(".git")).unwrap();
    std::fs::write(
        directory.path().join("go.mod"),
        "module example.test/fixture\n",
    )
    .unwrap();
    let nongit = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--json", "--from", "group:projects", "--depth", "all"])
        .output()
        .unwrap();
    assert!(
        nongit.status.success(),
        "{}",
        String::from_utf8_lossy(&nongit.stderr)
    );
    let nongit: serde_json::Value = serde_json::from_slice(&nongit.stdout).unwrap();
    let group = nongit["selections"][0]["root"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| {
            group["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["reference"] == reference)
        })
        .expect("the same process is classified without Git");
    assert!(
        group["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|evidence| {
                evidence
                    == &format!(
                        "project go.mod {}",
                        directory.path().canonicalize().unwrap().display()
                    )
            })
    );
    assert_eq!(group["totals"]["processes"], 1);
    let path = directory.path().to_path_buf();
    drop(child);
    directory.close().unwrap();
    assert!(!path.exists());
}

#[test]
#[ignore = "macOS command inspection; owns one shell blocked on its stdin pipe"]
fn live_command_redacts_arguments_and_never_outputs_environment() {
    let child = OwnedProcess(
        Command::new("/bin/sh")
            .args([
                "-c",
                "read -r owned_fixture_input",
                "fixture",
                "--token",
                "fixture-token-782",
                "--password=fixture-password-782",
                "https://user:fixture-url-782@example.test/",
                "",
            ])
            .env("RESWHO_FIXTURE_SECRET", "fixture-environment-782")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let reference = format!("pid:{}", child.0.id());
    for mode in ["--json", "--md"] {
        let output = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .args([
                mode,
                "--no-containers",
                "--from",
                &reference,
                "--depth",
                "0",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for hidden in [
            "fixture-token-782",
            "fixture-password-782",
            "fixture-url-782",
            "fixture-environment-782",
            "RESWHO_FIXTURE_SECRET",
            "read -r owned_fixture_input",
        ] {
            assert!(!text.contains(hidden), "exposed {hidden}");
        }
        if mode == "--json" {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let command = &value["selections"][0]["root"]["process"]["command"];
            assert_eq!(
                command["argv"],
                serde_json::json!([
                    "/bin/sh",
                    "-c",
                    "[filtered]",
                    "fixture",
                    "--token",
                    "[filtered]",
                    "--password=[filtered]",
                    "[filtered]",
                    ""
                ])
            );
            assert_eq!(command["redacted_indices"], serde_json::json!([2, 5, 6, 7]));
            assert!(command["entrypoint"].is_null());
            let schema: serde_json::Value =
                serde_json::from_str(reswho::output::JSON_SCHEMA).unwrap();
            jsonschema::validator_for(&schema)
                .unwrap()
                .validate(&value)
                .unwrap();
        } else {
            assert!(text.contains("cmd:"));
            assert!(text.contains("[filtered]"));
        }
    }
    drop(child);
}

#[test]
#[ignore = "macOS script entrypoint inspection; owns one temporary script and shell"]
fn live_script_entrypoint_preserves_literal_path() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("worker job.sh");
    std::fs::write(&script, "read -r owned_fixture_input\n").unwrap();
    let child = OwnedProcess(
        Command::new("/bin/sh")
            .arg(&script)
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .args([
            "--json",
            "--no-containers",
            "--from",
            &format!("pid:{}", child.0.id()),
            "--depth",
            "0",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let command = &value["selections"][0]["root"]["process"]["command"];
    assert_eq!(command["entrypoint"], script.to_str().unwrap());
    assert_eq!(command["argv"][1], script.to_str().unwrap());
    assert!(command["redacted_indices"].as_array().unwrap().is_empty());
    drop(child);
    let path = directory.path().to_path_buf();
    directory.close().unwrap();
    assert!(!path.exists());
}

#[test]
#[ignore = "macOS live socket inspection; binds only owned ephemeral loopback sockets"]
fn live_tcp_udp_ipv4_ipv6_endpoints_and_duplicate_descriptors() {
    use std::net::{TcpListener, TcpStream, UdpSocket};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let duplicate = listener.try_clone().unwrap();
    let local = listener.local_addr().unwrap();
    let client = TcpStream::connect(local).unwrap();
    let (server, _) = listener.accept().unwrap();
    let ipv6 = TcpListener::bind("[::1]:0").unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp6 = UdpSocket::bind("[::1]:0").unwrap();
    let reference = format!("pid:{}", std::process::id());
    let output = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--json", "--from", &reference, "--depth", "0"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let network = &snapshot["selections"][0]["root"]["process"]["network"];
    assert_eq!(network["complete"], true, "{network}");
    let sockets = network["sockets"].as_array().unwrap();
    assert_eq!(sockets.iter().filter(|socket| socket["local"] == local.to_string() && socket["tcp_state"] == "Listen").count(), 1);
    for (local, remote) in [
        (server.local_addr().unwrap(), server.peer_addr().unwrap()),
        (client.local_addr().unwrap(), client.peer_addr().unwrap()),
    ] {
        assert!(
            sockets
                .iter()
                .any(|socket| socket["local"] == local.to_string()
                    && socket["remote"] == remote.to_string()
                    && socket["tcp_state"] == "Established")
        );
    }
    for (protocol, address, state) in [
        ("tcp", ipv6.local_addr().unwrap(), Some("Listen")),
        ("udp", udp.local_addr().unwrap(), None),
        ("udp", udp6.local_addr().unwrap(), None),
    ] {
        assert!(sockets.iter().any(|socket| socket["protocol"] == protocol
            && socket["local"] == address.to_string()
            && socket["tcp_state"].as_str() == state));
    }
    drop((duplicate, server, client, listener, ipv6, udp, udp6));
}

#[test]
#[ignore = "macOS live process inspection; launches and reaps one owned versioned binary"]
fn live_versioned_install_binary_takes_tool_name_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let versions = directory.path().join("tool/versions");
    std::fs::create_dir_all(&versions).unwrap();
    std::fs::create_dir(directory.path().join("bin")).unwrap();
    let source = directory.path().join("tool.c");
    std::fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { pause(); return 0; }\n",
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(versions.join("1.2.3"))
            .status()
            .unwrap()
            .success()
    );
    let link = directory.path().join("bin/tool");
    std::os::unix::fs::symlink(versions.join("1.2.3"), &link).unwrap();
    let child = OwnedProcess(
        Command::new(&link)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let reference = format!("pid:{}", child.0.id());
    let run = |format: &str| {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--no-containers")
            .args([format, "--from", &reference, "--depth", "0"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    let snapshot: serde_json::Value = serde_json::from_str(&run("--json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(reswho::output::JSON_SCHEMA).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&snapshot)
        .unwrap();
    let process = &snapshot["selections"][0]["root"]["process"];
    assert_eq!(process["name"], "tool", "{process}");
    assert_eq!(process["original_name"], "1.2.3");
    assert_eq!(
        process["version"],
        serde_json::json!({"subject": "tool", "value": "1.2.3", "basis": "executable", "source": "versioned_install"})
    );
    let md = run("--md");
    assert!(md.contains(&format!("tool 1.2.3 {reference} · ")), "{md}");
    assert!(
        md.contains("> version 1.2.3 · subject tool · from versioned\\_install (executable)"),
        "{md}"
    );
    assert!(
        md.contains("reported name 1.2.3 · display name from versioned\\_install (executable)"),
        "{md}"
    );
    drop(child);
    directory.close().unwrap();
}

#[test]
#[ignore = "macOS live process inspection; launches and reaps one owned node-named binary"]
fn live_node_entry_names_work_without_exposing_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("bin")).unwrap();
    let source = root.join("node.c");
    std::fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { pause(); return 0; }\n",
    )
    .unwrap();
    let node = root.join("bin/node");
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&node)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("sync-mail.js"), "").unwrap();
    let child = OwnedProcess(
        Command::new(&node)
            .args(["sync-mail.js", "--token", "hunter2-fixture"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = u64::from(child.0.id());
    let run = |extra: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
            .arg("--no-containers")
            .args(extra)
            .args(["--json", "--depth", "all"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8(result.stdout).unwrap();
        for leaked in ["hunter2-fixture", "--token", "sync-mail.js"] {
            assert!(!text.contains(leaked), "{leaked}");
        }
        let snapshot: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut pending = vec![snapshot["selections"][0]["root"].clone()];
        while let Some(node) = pending.pop() {
            if node["process"]["pid"] == pid {
                return node;
            }
            pending.extend(node["children"].as_array().unwrap().iter().cloned());
        }
        panic!("owned node fixture not found");
    };
    let named = run(&[]);
    assert_eq!(named["process"]["name"], "sync-mail", "{named}");
    assert_eq!(named["process"]["original_name"], "node");
    assert_eq!(named["process"]["name_source"]["reason"], "work");
    let plain = run(&["--set", "extensions.node.entry_argv=false"]);
    assert_eq!(plain["process"]["name"], "node", "{plain}");
    assert!(plain["process"]["name_source"].is_null());
    drop(child);
    directory.close().unwrap();
}

#[test]
#[ignore = "macOS live process inspection; launches and reaps one owned python-named binary"]
fn live_python_entry_names_work_and_reads_patchlevel_version() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("include/python3.12")).unwrap();
    std::fs::write(
        root.join("include/python3.12/patchlevel.h"),
        "#define PY_VERSION \"3.12.4\"\n",
    )
    .unwrap();
    let source = root.join("python.c");
    std::fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { pause(); return 0; }\n",
    )
    .unwrap();
    let python = root.join("bin/python3.12");
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&python)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("sync_mail.py"), "").unwrap();
    let child = OwnedProcess(
        Command::new(&python)
            .args(["-u", "sync_mail.py", "--password", "hunter2-fixture"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = u64::from(child.0.id());
    let result = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .args(["--no-containers", "--json", "--depth", "all"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    for leaked in ["hunter2-fixture", "--password", "sync_mail.py"] {
        assert!(!text.contains(leaked), "{leaked}");
    }
    let snapshot: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut pending = vec![snapshot["selections"][0]["root"].clone()];
    let mut found = None;
    while let Some(node) = pending.pop() {
        if node["process"]["pid"] == pid {
            found = Some(node);
            break;
        }
        pending.extend(node["children"].as_array().unwrap().iter().cloned());
    }
    let node = found.expect("owned python fixture not found");
    assert_eq!(node["process"]["name"], "sync_mail", "{node}");
    assert_eq!(node["process"]["version"]["subject"], "python");
    assert_eq!(node["process"]["version"]["value"], "3.12.4");
    drop(child);
    directory.close().unwrap();
}
