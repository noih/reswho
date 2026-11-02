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
    assert_eq!(snapshot["root"]["attribution"]["source"], "git_root");
    assert_eq!(snapshot["scope"], reference);
    assert_eq!(snapshot["root"]["reference"], reference);
    let process = &snapshot["root"]["process"];
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
    assert_eq!(snapshot["root"]["totals"]["processes"], 1);
    assert!(snapshot["root"]["children"].as_array().unwrap().is_empty());
    assert!(snapshot["sample_ms"].as_u64().unwrap() >= 200);
    let tree = Command::new(env!("CARGO_BIN_EXE_reswho"))
        .arg("--no-containers")
        .args(["--json", "--from", "group:projects", "--depth", "all"])
        .output()
        .unwrap();
    assert!(tree.status.success());
    let tree: serde_json::Value = serde_json::from_slice(&tree.stdout).unwrap();
    let groups = tree["root"]["children"].as_array().unwrap();
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
    let group = nongit["root"]["children"]
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
            let command = &value["root"]["process"]["command"];
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
    let command = &value["root"]["process"]["command"];
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
    let network = &snapshot["root"]["process"]["network"];
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
