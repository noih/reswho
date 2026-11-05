use crate::{
    extensions::{host::files, tags::Hint},
    model::Process,
};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

#[derive(serde::Deserialize)]
struct Header {
    #[serde(rename = "type")]
    kind: String,
    #[serde(deserialize_with = "deserialize_object")]
    payload: SessionMetadata,
}

#[derive(serde::Deserialize)]
struct SessionMetadata {
    id: String,
    cli_version: String,
    originator: String,
}

#[derive(serde::Deserialize)]
struct IndexEntry {
    id: String,
    thread_name: String,
    updated_at: String,
}

fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);

    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn session_id(path: &Path) -> Option<&str> {
    let name = path
        .file_name()?
        .to_str()?
        .strip_prefix("rollout-")?
        .strip_suffix(".jsonl")?;
    if name.as_bytes().get(19) != Some(&b'-') {
        return None;
    }
    let format = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day]T[hour]-[minute]-[second]",
    )
    .ok()?;
    time::PrimitiveDateTime::parse(name.get(..19)?, &format).ok()?;
    let id = name.get(20..)?;
    uuid(id).then_some(id)
}

fn metadata(
    path: &Path,
    owner: Option<u32>,
    device: u64,
    inode: u64,
) -> Result<String, &'static str> {
    metadata_with(path, owner, device, inode, || {})
}

fn metadata_with(
    path: &Path,
    owner: Option<u32>,
    device: u64,
    inode: u64,
    after_read: impl FnOnce(),
) -> Result<String, &'static str> {
    let id = session_id(path).ok_or("Session-log filename unsupported")?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "Session-log metadata inaccessible")?;
    let before = file
        .metadata()
        .map_err(|_| "Session-log file identity unavailable")?;
    if !before.is_file()
        || Some(before.uid()) != owner
        || before.dev() != device
        || before.ino() != inode
    {
        return Err("Session-log file identity changed or owner mismatched");
    }
    let mut bytes = Vec::new();
    use std::io::Read;
    BufReader::new((&file).take(files::MAX_BYTES + 1))
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "Session-log metadata unreadable")?;
    if bytes.len() as u64 > files::MAX_BYTES || bytes.last() != Some(&b'\n') {
        return Err("Session-log metadata incomplete or exceeds read limit");
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let header: Header = deserialize_object(&mut deserializer)
        .map_err(|_| "Session-log metadata invalid or ambiguous")?;
    deserializer
        .end()
        .map_err(|_| "Session-log metadata invalid or ambiguous")?;
    if header.kind != "session_meta" || header.payload.originator.is_empty() {
        return Err("Session-log metadata format unsupported");
    }
    if header.payload.id != id {
        return Err("Session-log ID conflicts with filename");
    }
    if header.payload.cli_version != "0.166.0" {
        return Err("Session-log format version unsupported");
    }
    after_read();
    let after = file
        .metadata()
        .map_err(|_| "Session-log file identity unavailable")?;
    let current = path
        .symlink_metadata()
        .map_err(|_| "Session-log file disappeared")?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.mtime() != after.mtime()
        || current.dev() != before.dev()
        || current.ino() != before.ino()
        || !current.is_file()
    {
        return Err("Session-log changed during inspection");
    }
    Ok(id.into())
}

fn title(index: &Path, id: &str, owner: Option<u32>) -> Option<String> {
    read_title(id, || files::tail(index, owner))
}

fn read_title(id: &str, mut read: impl FnMut() -> files::Trusted) -> Option<String> {
    let files::Trusted::Bytes(before) = read() else {
        return None;
    };
    let text = std::str::from_utf8(&before).ok()?;
    if !text.ends_with('\n') {
        return None;
    }
    let mut latest = None;
    for line in text.lines() {
        let mut deserializer = serde_json::Deserializer::from_str(line);
        let entry: IndexEntry = deserialize_object(&mut deserializer).ok()?;
        deserializer.end().ok()?;
        if entry.id == id {
            time::OffsetDateTime::parse(
                &entry.updated_at,
                &time::format_description::well_known::Rfc3339,
            )
            .ok()?;
            latest = Some(entry.thread_name);
        }
    }
    let files::Trusted::Bytes(after) = read() else {
        return None;
    };
    if before != after {
        return None;
    }
    latest.filter(|name| safe_title(name))
}

fn safe_title(title: &str) -> bool {
    !title.is_empty() && title.len() <= 1024
        && !title.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}'))
}

pub(super) fn observe(process: &Process) -> Vec<Hint> {
    let unavailable = |reason| crate::extensions::host::hints::value("session_unavailable", reason);
    #[cfg(target_os = "macos")]
    {
        let Some(files) = crate::collect::writable_files(process) else {
            return vec![unavailable(
                "Session-log descriptors inaccessible or process identity changed",
            )];
        };
        let mut observed = std::collections::BTreeMap::new();
        let mut issues = std::collections::BTreeSet::new();
        for file in files {
            let Some(sessions) = file
                .path
                .ancestors()
                .find(|path| path.file_name().is_some_and(|name| name == "sessions"))
            else {
                continue;
            };
            if file
                .path
                .strip_prefix(sessions)
                .ok()
                .is_none_or(|relative| relative.components().count() != 4)
            {
                continue;
            }
            let id = match metadata(&file.path, process.uid, file.device, file.inode) {
                Ok(id) => id,
                Err(reason) => {
                    issues.insert(reason);
                    continue;
                }
            };
            let mut hint = crate::extensions::host::hints::value("session_log", &id);
            hint.record = Some(file.clone());
            hint.title = sessions
                .parent()
                .and_then(|home| title(&home.join("session_index.jsonl"), &id, process.uid));
            if !crate::collect::still_open(process, &file) {
                issues.insert("Session-log handle closed or process identity changed");
                continue;
            }
            if hint.title.is_none() {
                issues.insert(
                    "Session-log title metadata unavailable, ambiguous, unsafe, or changed",
                );
            }
            observed.entry(hint.value.clone()).or_insert(hint);
        }
        if observed.is_empty() && issues.is_empty() {
            issues.insert("No supported writable session log observed");
        }
        observed
            .into_values()
            .chain(issues.into_iter().map(unavailable))
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = process;
        vec![unavailable(
            "Session-log descriptor inspection unsupported on this platform",
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "019aa1bb-2222-7333-8444-555566667777";

    #[test]
    fn session_identity_requires_matching_bounded_header_and_exact_file_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join(format!("rollout-2026-11-05T00-00-00-{ID}.jsonl"));
        let header = serde_json::json!({"type":"session_meta","payload":{"id":ID,"cli_version":"0.166.0","originator":"codex_cli_rs"}});
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let stat = path.metadata().unwrap();
        assert_eq!(
            metadata(&path, Some(stat.uid()), stat.dev(), stat.ino()).as_deref(),
            Ok(ID)
        );
        assert!(metadata(&path, Some(stat.uid()), stat.dev(), stat.ino() + 1).is_err());
        for value in [
            "{}\n".into(),
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"other","cli_version":"0.166.0","originator":"codex"}})
            ),
            "x".repeat(files::MAX_BYTES as usize + 1),
        ] {
            std::fs::write(&path, value).unwrap();
            assert!(metadata(&path, Some(stat.uid()), stat.dev(), stat.ino()).is_err());
        }
        root.close().unwrap();
    }

    #[test]
    fn exact_session_title_uses_last_metadata_rename_without_prompt_fallback() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session_index.jsonl");
        let owner = Some(unsafe { libc::geteuid() });
        let entry = |id: &str, name: &str| {
            format!(
                "{}\n",
                serde_json::json!({"id":id,"thread_name":name,"updated_at":"2026-11-05T00:00:00Z"})
            )
        };
        std::fs::write(
            &path,
            entry(ID, "Manual name") + &entry("different", "Unrelated") + &entry(ID, "Renamed"),
        )
        .unwrap();
        assert_eq!(title(&path, ID, owner).as_deref(), Some("Renamed"));
        assert!(title(&path, "missing", owner).is_none());
        for name in [
            "[rw-61] Repair summary",
            "[ordinary] not JSON",
            "key=value",
            "{ordinary title}",
        ] {
            std::fs::write(&path, entry(ID, "Old") + &entry(ID, name)).unwrap();
            assert_eq!(title(&path, ID, owner).as_deref(), Some(name));
        }
        for value in [
            entry(ID, "bad\u{1b}[31m"),
            entry(ID, "bad\u{202e}title"),
            "{\"first_prompt\":\"private\"}\n".into(),
            entry(ID, "old") + "{incomplete",
        ] {
            std::fs::write(&path, value).unwrap();
            assert!(title(&path, ID, owner).is_none());
        }
        root.close().unwrap();
    }
    #[test]
    fn changed_metadata_index_and_unknown_versions_suppress_associations_or_titles() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join(format!("rollout-2026-11-05T00-00-00-{ID}.jsonl"));
        let header = serde_json::json!({"type":"session_meta","payload":{"id":ID,"cli_version":"0.166.0","originator":"bb"}});
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let stat = path.metadata().unwrap();
        let result = metadata_with(&path, Some(stat.uid()), stat.dev(), stat.ino(), || {
            use std::io::Write;
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap(),
                "changed"
            )
            .unwrap();
        });
        assert_eq!(result.unwrap_err(), "Session-log changed during inspection");
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let result = metadata_with(&path, Some(stat.uid()), stat.dev(), stat.ino(), || {
            std::fs::rename(&path, root.path().join("moved")).unwrap();
            std::fs::write(&path, format!("{header}\n")).unwrap();
        });
        assert_eq!(result.unwrap_err(), "Session-log changed during inspection");
        let mut unknown = header;
        unknown["payload"]["cli_version"] = serde_json::json!("99.0.0");
        std::fs::write(&path, format!("{unknown}\n")).unwrap();
        let stat = path.metadata().unwrap();
        assert!(metadata(&path, Some(stat.uid()), stat.dev(), stat.ino()).is_err());
        let entry = |title: &str| {
            format!(
                "{}\n",
                serde_json::json!({"id":ID,"thread_name":title,"updated_at":"2026-11-05T00:00:00Z"})
            )
            .into_bytes()
        };
        let mut reads = [
            files::Trusted::Bytes(entry("Old")),
            files::Trusted::Bytes(entry("Renamed")),
        ]
        .into_iter();
        assert!(read_title(ID, || reads.next().unwrap()).is_none());
        root.close().unwrap();
    }
    #[test]
    fn native_owned_session_log_reaches_adjudicated_output_and_disappears_without_stale_hints() {
        use crate::{
            extensions::{self, host::Facts},
            model::{Process, Snapshot},
            snapshot::Cancellation,
        };
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let directory = root
            .path()
            .canonicalize()
            .unwrap()
            .join("sessions/2026/11/05");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("rollout-2026-11-05T00-00-00-{ID}.jsonl"));
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"session_meta","payload":{"id":ID,"cli_version":"0.166.0","originator":"owned_fixture"}})).unwrap();
        let index = root.path().join("session_index.jsonl");
        std::fs::write(&index, format!("{}\n", serde_json::json!({"id":ID,"thread_name":"[rw-61] Exact synthetic title","updated_at":"2026-11-05T00:00:00Z"}))).unwrap();
        let identity =
            libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(std::process::id() as i32, 0)
                .unwrap();
        let process = Process {
            pid: std::process::id(),
            uid: Some(identity.pbi_uid),
            started_unix_us: Some(identity.pbi_start_tvsec * 1_000_000 + identity.pbi_start_tvusec),
            executable: Some(std::env::current_exe().unwrap()),
            cwd: Some("/".into()),
            name: "owned-observer".into(),
            ..Default::default()
        };
        let settings = extensions::default_settings().unwrap();
        let cancel = Cancellation::default();
        let node = crate::tree::build(vec![process.clone()]).unwrap();
        let mut node = crate::tree::select(&node, &format!("pid:{}", process.pid)).unwrap();
        let render = |process: &Process, node: &mut crate::model::Node| {
            let facts = Facts::new(std::slice::from_ref(process));
            let proposals = super::super::session_proposals(&facts, process);
            let fixture = extensions::tests::Fixture::new(
                "codex",
                extensions::Contribution::Classify { proposals },
            );
            let (mut reports, diagnostics) = extensions::tags::classify(
                &[&fixture],
                &extensions::Context::new(&settings, &cancel),
                &facts,
            );
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            node.process.as_mut().unwrap().tag_report = reports.remove(&process.pid);
            extensions::host::hints::summarize(node);
            extensions::summaries::attach(
                node,
                &[&extensions::Builtin::Codex],
                &extensions::Context::new(&settings, &cancel),
            );
        };
        render(&process, &mut node);
        assert_eq!(node.hints.len(), 1);
        assert_eq!(node.hints[0].value, ID);
        assert_eq!(
            node.hints[0].title.as_deref(),
            Some("[rw-61] Exact synthetic title")
        );
        assert!(
            crate::output::hint_lines(&node)
                .join("\n")
                .contains("session_log [codex · open_session_log]")
        );
        assert_eq!(
            node.summary.as_ref().unwrap().value,
            "[rw-61] Exact synthetic title"
        );
        let second_id = "019aa1bb-2222-7333-8444-888899990000";
        let second_path = directory.join(format!("rollout-2026-11-05T00-00-01-{second_id}.jsonl"));
        let mut second = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&second_path)
            .unwrap();
        writeln!(second, "{}", serde_json::json!({"type":"session_meta","payload":{"id":second_id,"cli_version":"0.166.0","originator":"owned_fixture"}})).unwrap();
        render(&process, &mut node);
        assert_eq!(node.hints.len(), 2);
        assert_eq!(node.summary.as_ref().unwrap().value, "2 session logs");
        assert_eq!(node.summary.as_ref().unwrap().evidence.len(), 2);
        let screen = crate::tui::tests::snapshot_screen(Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            captured_unix_ms: 1,
            sample_ms: 250,
            collection_ms: 1,
            system: Default::default(),
            extensions: vec![],
            warnings: vec![],
            scope: node.reference.clone(),
            root: node.clone(),
        });
        assert!(screen.contains("Codex: 2 session logs"), "{screen}");
        assert!(!screen.contains("Exact synthetic title"), "{screen}");

        assert!(
            node.hints
                .iter()
                .any(|hint| hint.value == second_id && hint.title.is_none())
        );
        drop(second);
        std::fs::remove_file(second_path).unwrap();
        render(&process, &mut node);
        assert_eq!(
            node.summary.as_ref().unwrap().value,
            "[rw-61] Exact synthetic title"
        );
        let snapshot = Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            captured_unix_ms: 1,
            sample_ms: 250,
            collection_ms: 1,
            system: Default::default(),
            extensions: vec![],
            warnings: vec![],
            scope: format!("pid:{}", process.pid),
            root: node.clone(),
        };
        let screen = crate::tui::tests::snapshot_screen(snapshot.clone());
        assert!(
            screen.contains("Codex log: [rw-61] Exact synthetic title"),
            "{screen}"
        );
        let value = serde_json::to_value(&snapshot).unwrap();
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        assert!(
            jsonschema::validator_for(&schema).unwrap().is_valid(&value),
            "{value}"
        );
        let markdown = crate::output::markdown(&snapshot);
        assert!(
            markdown.contains(ID)
                && markdown.contains(r"\[rw-61\] Exact synthetic title")
                && markdown.contains("inode")
        );
        for (id, version, expected) in [
            (ID, "99.0.0", "Session-log format version unsupported"),
            (
                "different",
                "0.166.0",
                "Session-log ID conflicts with filename",
            ),
        ] {
            std::fs::write(&path, format!("{}\n", serde_json::json!({"type":"session_meta","payload":{"id":id,"cli_version":version,"originator":"owned_fixture"}}))).unwrap();
            render(&process, &mut node);
            assert!(node.hints.is_empty());
            assert!(
                crate::output::tag_report_lines(
                    node.process.as_ref().unwrap().tag_report.as_ref().unwrap()
                )
                .join("\n")
                .contains(expected)
            );
        }
        std::fs::write(&path, format!("{}\n", serde_json::json!({"type":"session_meta","payload":{"id":ID,"cli_version":"0.166.0","originator":"owned_fixture"}}))).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
            let observed = observe(&process);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(observed.iter().all(|hint| hint.kind != "session_log"));
            assert!(
                observed
                    .iter()
                    .any(|hint| hint.value == "Session-log metadata inaccessible")
            );
        }
        std::fs::remove_file(&index).unwrap();
        render(&process, &mut node);
        assert_eq!(node.hints.len(), 1);
        assert!(node.hints[0].title.is_none());
        assert!(
            crate::output::tag_report_lines(
                node.process.as_ref().unwrap().tag_report.as_ref().unwrap()
            )
            .join("\n")
            .contains("Session-log title metadata unavailable")
        );
        drop(file);
        render(&process, &mut node);
        assert!(node.hints.is_empty());
        assert!(
            crate::output::tag_report_lines(
                node.process.as_ref().unwrap().tag_report.as_ref().unwrap()
            )
            .join("\n")
            .contains("No supported writable session log observed")
        );
        let mut changed = process.clone();
        changed.started_unix_us = changed.started_unix_us.map(|start| start + 1);
        let observed = observe(&changed);
        assert!(observed.iter().all(|hint| hint.kind != "session_log"));
        assert!(
            observed
                .iter()
                .any(|hint| hint.value.contains("process identity changed"))
        );
        root.close().unwrap();
    }
    #[test]
    fn duplicate_evidence_keys_reject_headers_and_titles_without_rejecting_independent_renames() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join(format!("rollout-2026-11-05T00-00-00-{ID}.jsonl"));
        let owner = Some(unsafe { libc::geteuid() });
        for (key, wrong, correct) in [
            ("id", "different", ID),
            ("cli_version", "99.0.0", "0.166.0"),
            ("originator", "other", "owned_fixture"),
        ] {
            let good =
                format!(r#""id":"{ID}","cli_version":"0.166.0","originator":"owned_fixture""#);
            let field = format!(r#""{key}":"{correct}""#);
            let duplicated = good.replace(&field, &format!(r#""{key}":"{wrong}",{field}"#));
            std::fs::write(
                &path,
                format!(r#"{{"type":"session_meta","payload":{{{duplicated}}}}}"#) + "\n",
            )
            .unwrap();
            let stat = path.metadata().unwrap();
            assert_eq!(
                metadata(&path, owner, stat.dev(), stat.ino()).unwrap_err(),
                "Session-log metadata invalid or ambiguous"
            );
        }
        for header in [
            format!(
                r#"{{"type":"other","type":"session_meta","payload":{{"id":"{ID}","cli_version":"0.166.0","originator":"owned_fixture"}}}}"#
            ),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"wrong"}},"payload":{{"id":"{ID}","cli_version":"0.166.0","originator":"owned_fixture"}}}}"#
            ),
        ] {
            std::fs::write(&path, header + "\n").unwrap();
            let stat = path.metadata().unwrap();
            assert!(metadata(&path, owner, stat.dev(), stat.ino()).is_err());
        }
        let index = root.path().join("session_index.jsonl");
        let good = format!(
            r#"{{"id":"{ID}","thread_name":"Renamed","updated_at":"2026-11-05T00:00:00Z"}}"#
        );
        for (field, replacement) in [
            (
                format!(r#""id":"{ID}""#),
                format!(r#""id":"other","id":"{ID}""#),
            ),
            (
                r#""thread_name":"Renamed""#.into(),
                r#""thread_name":"Arbitrary","thread_name":"Renamed""#.into(),
            ),
            (
                r#""updated_at":"2026-11-05T00:00:00Z""#.into(),
                r#""updated_at":"old","updated_at":"2026-11-05T00:00:00Z""#.into(),
            ),
        ] {
            std::fs::write(&index, good.replace(&field, &replacement) + "\n").unwrap();
            assert!(title(&index, ID, owner).is_none());
        }
        let old = good.replace("Renamed", "Original");
        std::fs::write(&index, old + "\n" + &good + "\n").unwrap();
        assert_eq!(title(&index, ID, owner).as_deref(), Some("Renamed"));
        root.close().unwrap();
    }
    #[test]
    fn session_evidence_requires_object_header_payload_and_index_records() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join(format!("rollout-2026-11-05T00-00-00-{ID}.jsonl"));
        let owner = Some(unsafe { libc::geteuid() });
        let payload = serde_json::json!({"id":ID,"cli_version":"0.166.0","originator":"fixture"});
        for header in [
            serde_json::json!(["session_meta", [ID, "0.166.0", "fixture"]]),
            serde_json::json!(["session_meta", payload.clone()]),
            serde_json::json!({"type":"session_meta","payload":[ID,"0.166.0","fixture"]}),
        ] {
            std::fs::write(&path, format!("{header}\n")).unwrap();
            let stat = path.metadata().unwrap();
            assert_eq!(
                metadata(&path, owner, stat.dev(), stat.ino()).unwrap_err(),
                "Session-log metadata invalid or ambiguous"
            );
        }
        let header = serde_json::json!({"type":"session_meta","payload":payload});
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let stat = path.metadata().unwrap();
        assert_eq!(
            metadata(&path, owner, stat.dev(), stat.ino()).as_deref(),
            Ok(ID)
        );
        let index = root.path().join("session_index.jsonl");
        std::fs::write(
            &index,
            format!(
                "{}\n",
                serde_json::json!([ID, "Unsupported array title", "2026-11-05T00:00:00Z"])
            ),
        )
        .unwrap();
        assert!(title(&index, ID, owner).is_none());
        let entry = |name| {
            format!(
                "{}\n",
                serde_json::json!({"id":ID,"thread_name":name,"updated_at":"2026-11-05T00:00:00Z"})
            )
        };
        std::fs::write(&index, entry("Original") + &entry("Renamed")).unwrap();
        assert_eq!(title(&index, ID, owner).as_deref(), Some("Renamed"));
        root.close().unwrap();
    }
}
