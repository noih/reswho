use super::*;
use crate::extensions::{
    ClassificationEvidence,
    tags::{self, Operation},
};

struct Arguments(HashMap<u32, Vec<String>>);
impl Argv for Arguments {
    fn read(&mut self, process: &Process) -> Option<Vec<String>> {
        self.0.get(&process.pid).cloned()
    }
}

fn fixture() -> (tempfile::TempDir, Vec<Process>, Arguments) {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let entry = root_path.join("node_modules/playwright-core/lib/entry/cliDaemon.js");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, "").unwrap();
    std::fs::write(
        entry.ancestors().nth(3).unwrap().join("package.json"),
        "{\"name\":\"playwright-core\",\"version\":\"1.64.0-alpha-1790635538000\"}",
    )
    .unwrap();
    std::fs::write(root_path.join("package.json"), "{\"name\":\"demo\"}").unwrap();
    let processes = [
        (10, 1, "/fixture/node"),
        (20, 10, "/fixture/chrome"),
        (30, 20, "/fixture/chrome-helper"),
    ]
    .into_iter()
    .map(|(pid, ppid, exe)| Process {
        pid,
        ppid: Some(ppid),
        uid: Some(unsafe { libc::geteuid() }),
        started_unix_us: Some(u64::from(pid)),
        executable: Some(exe.into()),
        cwd: Some(root_path.clone()),
        ..Default::default()
    })
    .collect();
    let args = Arguments(HashMap::from([(
        10,
        vec![
            "node".into(),
            entry.to_string_lossy().into_owned(),
            "review".into(),
        ],
    )]));
    (root, processes, args)
}

#[test]
fn daemon_session_and_ancestor_project_are_independent_and_validated() {
    let (root, processes, mut args) = fixture();
    for enabled in [true, false] {
        let proposals = proposals(&Facts::new(&processes), &mut args, enabled);
        assert_eq!(proposals.len(), if enabled { 9 } else { 6 });
        for proposal in proposals {
            let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
                panic!()
            };
            tags::validate(
                std::slice::from_ref(&proposal),
                &processes,
                "playwright",
                None,
            )
            .unwrap();
            let Operation::Add { tag } = proposal.operation else {
                panic!()
            };
            if tag.hint.as_ref().unwrap().kind == "project" {
                assert!(enabled);
                assert!(tag.path.is_some());
            } else {
                assert!(tag.path.is_none());
            }
        }
    }
    root.close().unwrap();
}

#[test]
fn relative_entry_preserves_direct_and_inherited_contributions() {
    let (root, processes, mut args) = fixture();
    assert_ne!(
        processes[0].cwd.as_deref().unwrap(),
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
    for enabled in [true, false] {
        let absolute = proposals(&Facts::new(&processes), &mut args, enabled);
        assert_eq!(absolute.len(), if enabled { 9 } else { 6 });
        let entry = args.0[&10][1].clone();
        args.0.get_mut(&10).unwrap()[1] =
            "node_modules/playwright-core/lib/entry/cliDaemon.js".into();
        let relative = proposals(&Facts::new(&processes), &mut args, enabled);
        assert_eq!(
            serde_json::to_value(relative).unwrap(),
            serde_json::to_value(absolute).unwrap()
        );
        args.0.get_mut(&10).unwrap()[1] = entry;
    }
    let path = root.path().to_path_buf();
    root.close().unwrap();
    assert!(!path.exists());
}

#[test]
fn broken_cross_uid_reused_missing_or_cyclic_parent_never_transfers_hints() {
    let (root, processes, mut args) = fixture();
    for case in 0..6 {
        let mut processes = processes.clone();
        match case {
            0 => processes[1].uid = processes[1].uid.map(|uid| uid + 1),
            1 => processes[0].started_unix_us = Some(100),
            2 => processes[0].started_unix_us = None,
            3 => processes[1].ppid = Some(99),
            4 => processes[0].ppid = Some(30),
            _ => processes[1].executable = None,
        }
        let result = proposals(&Facts::new(&processes), &mut args, true);
        assert!(
            result.iter().all(|proposal| proposal.identity.pid != 30),
            "case {case}"
        );
    }
    root.close().unwrap();
}

#[test]
fn missing_package_unknown_version_and_unsafe_session_names_are_omitted() {
    let (root, processes, mut args) = fixture();
    let entry = std::path::PathBuf::from(&args.0[&10][1]);
    let package = entry.ancestors().nth(3).unwrap().join("package.json");
    std::fs::write(
        &package,
        "{\"name\":\"playwright-core\",\"version\":\"99.0.0\"}",
    )
    .unwrap();
    assert!(proposals(&Facts::new(&processes), &mut args, true).is_empty());
    std::fs::remove_file(&package).unwrap();
    assert!(proposals(&Facts::new(&processes), &mut args, true).is_empty());
    std::fs::write(
        &package,
        "{\"name\":\"playwright-core\",\"version\":\"1.64.0\"}",
    )
    .unwrap();
    for name in ["api_key=secret", "bad\u{1b}", "private name", ""] {
        args.0.get_mut(&10).unwrap()[2] = name.into();
        assert!(proposals(&Facts::new(&processes), &mut args, true).is_empty());
    }
    root.close().unwrap();
}
