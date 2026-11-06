use super::*;
use crate::extensions;

struct Arguments {
    args: Option<Vec<String>>,
    reads: usize,
}
impl Argv for Arguments {
    fn read(&mut self, _: &Process) -> Option<Vec<String>> {
        self.reads += 1;
        self.args.clone()
    }
}
fn arguments(args: &[&str]) -> Arguments {
    Arguments {
        args: Some(args.iter().map(|s| (*s).into()).collect()),
        reads: 0,
    }
}
fn process() -> Process {
    Process {
        pid: 42,
        uid: Some(unsafe { libc::geteuid() }),
        started_unix_us: Some(100),
        executable: Some("/fixture/herdr".into()),
        name: "herdr".into(),
        resident_bytes: Some(100),
        ..Default::default()
    }
}

#[test]
fn server_requires_known_launch_mode_not_name_cwd_or_lifetime() {
    for (args, recognized) in [
        (vec!["herdr", "server"], true),
        (vec!["herdr", "--session", "work", "server"], true),
        (vec!["herdr", "server", "--session=work"], true),
        (
            vec![
                "herdr",
                "server",
                "--handoff-import",
                "/tmp/state",
                "opaque",
            ],
            true,
        ),
        (vec!["herdr"], false),
        (vec!["herdr", "server", "stop"], false),
        (vec!["herdr", "server", "reload-config"], false),
        (vec!["herdr", "client"], false),
        (vec!["herdr", "session", "attach", "work"], false),
        (vec!["herdr", "server", "--unknown"], false),
        (vec!["herdr", "server", "--session"], false),
        (vec!["herdr", "server", "--session=.."], false),
        (vec!["herdr", "server", "--session=/work"], false),
        (vec!["herdr", "server", "--", "--session=work"], false),
    ] {
        assert_eq!(
            server(&arguments(&args).args.unwrap()),
            recognized,
            "{args:?}"
        );
    }
    let mut truncated = vec!["herdr".into(), "server".into()];
    truncated.resize(PREFIX_ARGS, "--session=work".into());
    assert!(!server(&truncated));
    let mut p = process();
    for invalid in [0, 1, 2] {
        match invalid {
            0 => p.started_unix_us = None,
            1 => {
                p = process();
                p.uid = None;
            }
            _ => {
                p = process();
                p.executable = Some("/fixture/not-herdr".into());
            }
        }
        assert!(
            proposals(
                &Facts::new(&[p.clone()]),
                &mut arguments(&["herdr", "server"])
            )
            .is_empty()
        );
    }
    let mut denied = Arguments {
        args: None,
        reads: 0,
    };
    assert!(proposals(&Facts::new(&[process()]), &mut denied).is_empty());
    assert!(
        details(&process(), &mut denied)[0]
            .value
            .as_str()
            .unwrap()
            .contains("Unknown")
    );
}

#[test]
fn overview_only_reads_candidates_at_large_process_counts() {
    for count in [500, 1000, 5000] {
        let mut processes: Vec<_> = (100..count + 100)
            .map(|pid| Process {
                pid,
                executable: Some("/fixture/other".into()),
                ..process()
            })
            .collect();
        processes.push(process());
        let mut reader = arguments(&["herdr", "server"]);
        assert_eq!(proposals(&Facts::new(&processes), &mut reader).len(), 1);
        assert_eq!(reader.reads, 1);
    }
}

fn fill(_: i32, buffer: &mut [u8]) -> Option<usize> {
    let mut bytes = 2i32.to_ne_bytes().to_vec();
    bytes.extend_from_slice(b"/fixture/herdr\0");
    while !(bytes.len() - 4).is_multiple_of(8) {
        bytes.push(0);
    }
    bytes.extend_from_slice(b"herdr\0server\0");
    buffer[..bytes.len()].copy_from_slice(&bytes);
    Some(bytes.len())
}

#[test]
fn service_scope_preserves_project_evidence_without_double_counting_or_claiming_clients() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    let mut p = process();
    p.cwd = Some(project.path().into());
    let processes = vec![p.clone()];
    let settings = extensions::default_settings().unwrap();
    let cancel = crate::snapshot::Cancellation::default();
    let mut context = Context::new(&settings, &cancel);
    context.argv = std::sync::Mutex::new(crate::collect::ArgvPrefix::with(
        |_| true,
        || Some(4096),
        fill,
        |_| Some(8),
    ));
    let root = crate::tree::build_with_providers(processes, &context, &[]).unwrap();
    let mut selected = crate::tree::select_detail(&root, "pid:42").unwrap();
    assert_eq!(root.totals.processes, 1);
    assert_eq!(root.totals.resident_bytes, 100);
    assert_eq!(selected.summary.as_ref().unwrap().label, "Shared scope");
    let report = selected
        .process
        .as_ref()
        .unwrap()
        .tag_report
        .as_ref()
        .unwrap();
    assert!(
        report
            .effective
            .iter()
            .any(|e| e.source == "project" && e.tag.basis == "cwd")
    );
    assert!(
        selected
            .evidence
            .iter()
            .any(|e| e.contains("Shared services"))
    );
    selected
        .process
        .as_mut()
        .unwrap()
        .extension_details
        .push(crate::model::ExtensionDetails {
            provider: "herdr".into(),
            fields: details(&p, &mut arguments(&["herdr", "server"])),
        });
    let value = serde_json::to_string(&selected).unwrap();
    assert!(value.contains("Session-wide") && value.contains("Unknown: launch mode"));
    let snapshot = crate::model::Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 1,
        sample_ms: 1,
        collection_ms: 1,
        system: Default::default(),
        scope: "pid:42".into(),
        warnings: vec![],
        root: selected,
    };
    let markdown = crate::output::markdown(&snapshot);
    assert!(
        markdown.contains("Session-wide") && markdown.contains("Unknown: launch mode"),
        "{markdown}"
    );
    assert!(markdown.contains("Herdr session server"));
    for cwd in [
        None,
        Some(project.path().to_path_buf()),
        Some("/another/project".into()),
    ] {
        p.cwd = cwd;
        assert_eq!(
            details(&p, &mut arguments(&["herdr", "server"]))[2].value,
            "Unknown: launch mode does not establish active clients or their projects"
        );
    }
    project.close().unwrap();
}
