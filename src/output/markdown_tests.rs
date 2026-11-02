use super::*;
use crate::{model::SystemSummary, tree};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

fn fixture_snapshot() -> Snapshot {
    let parent = Process {
        pid: 10,
        ppid: Some(1),
        uid: Some(501),
        started_unix_us: Some(10),
        uptime_seconds: Some(3600),
        name: "Example".into(),
        cwd: Some("/".into()),
        executable: Some("/Applications/Example.app/Contents/MacOS/Example".into()),
        cpu_percent: Some(0.0),
        resident_bytes: Some(1024),
        network: Some(Network {
            complete: true,
            ..Network::default()
        }),
        ..Process::default()
    };
    let mut helper = parent.clone();
    helper.pid = 11;
    helper.ppid = Some(10);
    helper.started_unix_us = Some(11);
    helper.name = "helper".into();
    helper.executable = None;
    let unreadable = |pid| Process {
        pid,
        name: format!("process {pid}"),
        unavailable: vec!["identity: Operation not permitted (os error 1)".into()],
        ..Process::default()
    };
    Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 1000,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:all".into(),
        warnings: vec![
            "Command details are queried only for a selected PID in Markdown/JSON; container-internal processes are not implemented.".into(),
            "Host swap: OS query unavailable or sample incomplete.".into(),
            "Host swap: OS query unavailable or sample incomplete.".into(),
        ],
        root: tree::build(vec![parent, helper, unreadable(30), unreadable(31)]).unwrap(),
    }
}

#[test]
fn report_categories_quotes_and_process_lists_have_distinct_structure() {
    let snapshot = fixture_snapshot();
    let md = markdown(&snapshot);
    assert!(md.starts_with("# reswho\n\n"));
    assert!(md.contains("reswho-md/18"));
    assert!(!md.contains("Collection prototype:"));
    assert_eq!(md.matches("> ! Host swap:").count(), 1);
    assert!(!md.contains("Resources group:all"));
    assert!(md.contains("Host CPU"));
    assert!(!md.contains("- Resources"));
    assert!(md.contains("## Applications\n\ngroup:applications · "));
    assert!(md.contains("## Unclassified\n\ngroup:unclassified · "));
    assert_eq!(md.matches("Unreadable  Σ2").count(), 1);
    assert!(!md.contains("pid:30"));
    assert!(!md.contains("~"));
    let mut headings = Vec::new();
    let mut item_depth = 0;
    let mut quote_depth = 0;
    let mut observed = Vec::new();
    let mut app_paths = 0;
    for event in Parser::new(&md) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => headings.push(level),
            Event::Start(Tag::Item) => item_depth += 1,
            Event::End(TagEnd::Item) => item_depth -= 1,
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Text(text) if text.contains("pid:10") || text.contains("pid:11") => {
                observed.push((text.to_string(), item_depth, quote_depth))
            }
            Event::Code(path) if path.as_ref() == "/Applications/Example.app" => {
                app_paths += 1;
                assert_eq!(item_depth, 1);
                assert_eq!(quote_depth, 1);
            }
            Event::Start(Tag::CodeBlock(_) | Tag::Strong | Tag::Emphasis)
            | Event::Html(_)
            | Event::InlineHtml(_) => panic!("unexpected formatting: {event:?}"),
            _ => {}
        }
    }
    assert_eq!(
        headings,
        vec![HeadingLevel::H1, HeadingLevel::H2, HeadingLevel::H2]
    );
    assert_eq!(observed.len(), 2);
    assert_eq!(app_paths, 1);
    assert_eq!((observed[0].1, observed[0].2), (2, 0));
    assert_eq!((observed[1].1, observed[1].2), (3, 0));
}

#[test]
fn markdown_flattens_system_before_depth_without_changing_json_or_refs() {
    let mut snapshot = fixture_snapshot();
    snapshot.system.cpu_percent = Some(30.0);
    snapshot.root = tree::build(vec![Process {
        pid: 99,
        name: "agent".into(),
        executable: Some("/usr/libexec/agent".into()),
        cpu_percent: Some(240.0),
        resident_bytes: Some(100),
        ..Process::default()
    }])
    .unwrap();
    let wrapper = snapshot.root.children[0].children[0].reference.clone();
    let before = serde_json::to_value(&snapshot).unwrap();
    let md = markdown_with_depth(&snapshot, 2);
    assert!(md.contains("Host CPU 30.0%"));
    assert!(!md.contains("Resources "));
    assert!(md.contains("## System"));
    assert!(md.contains("pid:99 · 240.0%"));
    assert!(!md.contains("System services [system]"));
    let shallow = markdown_with_depth(&snapshot, 1);
    assert!(shallow.contains("## System"));
    assert!(shallow.contains("[+1]"));
    assert!(!shallow.contains("pid:99"));
    let root_only = markdown_with_depth(&snapshot, 0);
    assert!(root_only.contains("Σ1 [+1]"));
    assert!(!root_only.contains("## System"));
    let after = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(before, after);
    assert_eq!(after["system"]["cpu_percent"], 30.0);
    assert_eq!(after["root"]["totals"]["known_cpu_percent"], 240.0);
    assert_eq!(
        after["root"]["children"][0]["children"][0]["reference"],
        wrapper
    );
    snapshot.root = tree::select(&snapshot.root, &wrapper).unwrap();
    snapshot.scope = wrapper.clone();
    let scoped = markdown_with_depth(&snapshot, 0);
    assert!(scoped.contains(&format!("System services [system] {wrapper}")));
    assert!(scoped.contains("[+1]"));
}

#[test]
fn adjacent_groups_keep_notes_and_children_together() {
    let mut snapshot = fixture_snapshot();
    let parent = tree::select(&snapshot.root, "pid:10")
        .unwrap()
        .process
        .unwrap();
    let helper = tree::select(&snapshot.root, "pid:11")
        .unwrap()
        .process
        .unwrap();
    let mut second = parent.clone();
    second.pid = 20;
    second.name = "Second".into();
    second.executable = Some("/Applications/Second.app/Contents/MacOS/Second".into());
    snapshot.root = tree::build(vec![parent, helper, second]).unwrap();
    let md = markdown(&snapshot);
    assert!(md.contains("Σ2\n    > app `/Applications/Example.app`\n    - Example pid:10"));
    assert!(md.contains("1h  -\n        - helper pid:11"));
    assert!(md.contains("1h  -\n\n- Second [app]"));
    assert!(md.contains("Σ1\n    > app `/Applications/Second.app`\n    - Second pid:20"));

    let mut item_depth = 0;
    let mut quote_depth = 0;
    let mut pids = Vec::new();
    let mut paths = 0;
    for event in Parser::new(&md) {
        match event {
            Event::Start(Tag::Item) => item_depth += 1,
            Event::End(TagEnd::Item) => item_depth -= 1,
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Code(path) if path.starts_with("/Applications/") => {
                paths += 1;
                assert_eq!((item_depth, quote_depth), (1, 1));
            }
            Event::Text(text) if text.contains("pid:") => {
                pids.push((item_depth, quote_depth));
            }
            _ => {}
        }
    }
    assert_eq!(paths, 2);
    assert_eq!(pids, vec![(2, 0), (3, 0), (2, 0)]);
}

#[test]
fn root_selection_and_depth_limits_keep_refs_totals_and_omission_markers() {
    for depth in [0, 1, 2, 3] {
        let mut snapshot = fixture_snapshot();
        tree::limit_depth(&mut snapshot.root, depth);
        let md = markdown(&snapshot);
        assert_eq!(snapshot.root.totals.processes, 4);
        assert!(md.contains("Σ4"));
        assert_eq!(md.contains("## Applications"), depth > 0);
        assert_eq!(md.contains("pid:10"), depth >= 3);
        assert!(!md.contains("pid:11"));
        let expected = if depth == 0 { "Σ4 [+2]" } else { "[+1]" };
        assert!(md.contains(expected), "{md}");
    }
    let mut group_snapshot = fixture_snapshot();
    let group = group_snapshot
        .root
        .children
        .iter()
        .find(|node| node.reference == "group:applications")
        .unwrap()
        .children[0]
        .clone();
    group_snapshot.scope = group.reference.clone();
    group_snapshot.root = tree::select(&group_snapshot.root, &group.reference).unwrap();
    tree::limit_depth(&mut group_snapshot.root, 1);
    let group_md = markdown(&group_snapshot);
    assert!(group_md.contains(&format!("\nExample [app] {}", group.reference)));
    assert!(!group_md.contains("## Applications"));
    assert!(group_md.contains("pid:10"));
    assert!(!group_md.contains("pid:11"));
    assert_eq!(group_snapshot.root.totals.processes, 2);
    let mut snapshot = fixture_snapshot();
    snapshot.root = tree::select(&snapshot.root, "pid:11").unwrap();
    snapshot.scope = "pid:11".into();
    let md = markdown(&snapshot);
    assert!(md.contains("\nhelper pid:11"));
    assert!(!md.contains("## Applications"));
    assert!(md.contains("> ~ Inferred from ancestor pid:10:"));
    assert!(md.contains("same UID and working directory"));
    assert!(Parser::new(&md).any(|event| matches!(event, Event::HardBreak)));
    assert!(!md.contains("> ! Unavailable"));
    snapshot.root = tree::select(&fixture_snapshot().root, "pid:30").unwrap();
    snapshot.scope = "pid:30".into();
    let md = markdown(&snapshot);
    assert!(md.contains("> ! Unavailable: identity: Operation not permitted (os error 1)"));
    assert!(!md.contains("Unreadable  "));
}

#[test]
fn special_names_and_paths_stay_literal_without_creating_markdown_structure() {
    assert_eq!(name_markdown("2.1.316"), "2.1.316");
    for name in [
        "# forged",
        "- nested",
        "+ nested",
        "1. ordered",
        "1) ordered",
        "    code",
        "[link](https://example.test)",
        "<img src=x>",
        "\u{1b}[31m\n# forged",
    ] {
        let mut snapshot = fixture_snapshot();
        snapshot.warnings.clear();
        snapshot.root = tree::select(&snapshot.root, "pid:10").unwrap();
        snapshot.root.children.clear();
        snapshot.root.name = name.into();
        let path = "/tmp/space and_under`tick\n\u{202e}";
        snapshot.root.process.as_mut().unwrap().cwd = Some(path.into());
        let md = markdown(&snapshot);
        let mut headings = 0;
        let mut paths = 0;
        for event in Parser::new(&md) {
            match event {
                Event::Start(Tag::Heading { .. }) => headings += 1,
                Event::Code(value) if value.contains("space and_under`tick") => {
                    paths += 1;
                    assert!(value.contains("\\u{a}\\u{202e}"));
                }
                Event::Start(
                    Tag::CodeBlock(_)
                    | Tag::List(_)
                    | Tag::Link { .. }
                    | Tag::Image { .. }
                    | Tag::Strong
                    | Tag::Emphasis,
                )
                | Event::Html(_)
                | Event::InlineHtml(_) => panic!("{name}: {event:?}"),
                _ => {}
            }
        }
        assert_eq!(headings, 1, "{name}");
        assert_eq!(paths, 1, "{name}");
        assert!(!md.contains(['\u{1b}', '\u{202e}']));
    }
}

#[test]
fn diagnostic_markers_do_not_guess_permission_or_mark_idle_processes() {
    let mut snapshot = fixture_snapshot();
    snapshot.warnings.clear();
    snapshot.root = tree::select(&snapshot.root, "pid:10").unwrap();
    let process = snapshot.root.process.as_mut().unwrap();
    process.cpu_percent = Some(0.0);
    process.uptime_seconds = Some(86400 * 100);
    assert!(!markdown(&snapshot).contains("> !"));
    assert!(!markdown(&snapshot).contains("> ~"));
    snapshot
        .root
        .evidence
        .push("Repository metadata unavailable: malformed gitdir".into());
    let md = markdown(&snapshot);
    assert!(md.contains("> ! Repository metadata unavailable: malformed gitdir"));
    assert!(!md.contains("permission"));
    snapshot.root.process.as_mut().unwrap().unavailable = vec![
        "identity: process exited".into(),
        "identity: process exited".into(),
    ];
    let md = markdown(&snapshot);
    assert_eq!(md.matches("identity: process exited").count(), 1);
    assert!(!md.contains("permission"));
    let mut unreadable_snapshot = fixture_snapshot();
    let process = tree::select(&unreadable_snapshot.root, "pid:30")
        .unwrap()
        .process
        .unwrap();
    unreadable_snapshot.root = tree::build(vec![process]).unwrap();
    unreadable_snapshot.warnings.clear();
    let md = markdown(&unreadable_snapshot);
    assert!(md.contains("?  ?  ⏱︎ ?  Σ1"));
    assert!(md.contains("- Unreadable  Σ1\n"));
    assert!(!md.contains("pid:30"));
    assert!(!md.contains("permission"));
    assert!(!md.contains("[+"));
}
