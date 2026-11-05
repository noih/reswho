use super::*;
use crate::{
    extensions::{
        ClassificationEvidence,
        tags::{self, Operation},
    },
    model::Process,
};

struct Arguments(Vec<String>);
impl Argv for Arguments {
    fn read(&mut self, _: &Process) -> Option<Vec<String>> {
        Some(self.0.clone())
    }
}

#[test]
fn native_and_npm_relative_entry_roles_never_promote_resume_or_registry_state_to_current_session() {
    let root = tempfile::tempdir().unwrap();
    let entry = root
        .path()
        .canonicalize()
        .unwrap()
        .join("node_modules/@anthropic-ai/claude-code/cli.js");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, "").unwrap();
    std::fs::write(
        entry.parent().unwrap().join("package.json"),
        "{\"name\":\"@anthropic-ai/claude-code\",\"version\":\"2.1.316\"}",
    )
    .unwrap();
    assert_ne!(
        root.path().canonicalize().unwrap(),
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
    for (executable, script) in [
        ("/fixture/claude", None),
        ("/fixture/claude/versions/2.1.316", None),
        ("/fixture/node", entry.to_str()),
        (
            "/fixture/node",
            Some("node_modules/@anthropic-ai/claude-code/cli.js"),
        ),
    ] {
        let process = Process {
            pid: 42,
            uid: Some(unsafe { libc::geteuid() }),
            started_unix_us: Some(100),
            executable: Some(executable.into()),
            cwd: Some(root.path().into()),
            ..Default::default()
        };
        let mut argv = vec![executable.to_owned()];
        if let Some(script) = script {
            argv.push(script.into());
        }
        for (argument, role) in [
            ("--resume", "Claude Code CLI"),
            ("--daemon", "Claude Code daemon"),
            ("--bg-worker", "Claude Code background worker"),
            ("--bg-spare", "Claude Code background worker"),
        ] {
            let mut argv = argv.clone();
            argv.extend([
                argument.into(),
                "019aa1bb-2222-7333-8444-555566667777".into(),
            ]);
            let proposals = proposals(
                &crate::extensions::host::Facts::new(std::slice::from_ref(&process)),
                &mut Arguments(argv),
            );
            assert_eq!(proposals.len(), 2, "{executable}");
            for proposal in proposals {
                let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
                    panic!()
                };
                tags::validate(
                    std::slice::from_ref(&proposal),
                    std::slice::from_ref(&process),
                    "claude",
                    None,
                )
                .unwrap();
                let Operation::Add { tag } = proposal.operation else {
                    panic!()
                };
                let hint = tag.hint.unwrap();
                assert!(matches!(hint.kind.as_str(), "role" | "session_unavailable"));
                if hint.kind == "role" {
                    assert_eq!(hint.value, role);
                }
                assert_ne!(tag.dimension, "work");
            }
        }
    }
    root.close().unwrap();
}
