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
