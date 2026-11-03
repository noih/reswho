use super::*;
use crate::{
    containers::{Container, ContainerTotals},
    model::{Grouping, Node, Process, Snapshot, SystemSummary},
    snapshot::Cancellation,
    tree,
};
use ratatui::{Terminal, backend::TestBackend};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn snapshot() -> Snapshot {
    let processes = vec![
        Process {
            pid: 10,
            ppid: Some(1),
            started_unix_us: Some(100),
            name: "demo-one".into(),
            executable: Some("/Applications/Demo.app/Contents/MacOS/demo".into()),
            cwd: Some("/".into()),
            cpu_percent: Some(1.0),
            resident_bytes: Some(100),
            uptime_seconds: Some(100),
            ..Process::default()
        },
        Process {
            pid: 11,
            ppid: Some(1),
            started_unix_us: Some(101),
            name: "demo-two".into(),
            executable: Some("/Applications/Demo.app/Contents/MacOS/demo".into()),
            cwd: Some("/".into()),
            cpu_percent: Some(20.0),
            resident_bytes: Some(200),
            uptime_seconds: Some(10),
            ..Process::default()
        },
        Process {
            pid: 30,
            ..Process::default()
        },
        Process {
            pid: 31,
            ..Process::default()
        },
    ];
    let mut root = tree::build(processes).unwrap();
    let mut containers = Node::group(
        "group:containers".into(),
        "Containers".into(),
        vec![],
        Grouping::Category,
    );
    let mut runtime = Node::group(
        "group:runtime".into(),
        "docker (test)".into(),
        vec![],
        Grouping::Runtime,
    );
    let mut totals = ContainerTotals {
        inventory_complete: true,
        ..ContainerTotals::default()
    };
    for (id, state) in [
        ("live", Some("running")),
        ("stopped", Some("exited")),
        ("unknown", None),
    ] {
        let container = Container {
            id: id.into(),
            name: id.into(),
            state: state.map(str::to_owned),
            candidates: vec!["/workspace/demo".into()],
            ..Container::default()
        };
        totals.add(&container);
        let mut node = Node::group(
            format!("group:{id}"),
            id.into(),
            vec![],
            Grouping::Container,
        );
        node.kind = "container";
        node.container = Some(container);
        runtime.children.push(node);
    }
    runtime.container_totals = Some(totals.clone());
    containers.container_totals = Some(totals);
    containers.children.push(runtime);
    root.children.push(containers);
    Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 1000,
        sample_ms: 250,
        collection_ms: 300,
        system: SystemSummary::default(),
        scope: "group:all".into(),
        warnings: vec![],
        root,
    }
}

fn app() -> App {
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(snapshot()));
    app
}

#[test]
fn d_opens_selected_process_details_without_moving_the_list() {
    let mut app = app();
    select(&mut app, "pid:11");
    let selected = app.selected;
    let expanded = app.expanded.clone();
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    let mut table = TableState::default();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Process details"));
    assert!(text.contains("Loading command"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Down));
    handle_key(&mut app, KeyEvent::from(KeyCode::Enter));
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    assert_eq!(app.selected, selected);
    assert_eq!(app.expanded, expanded);
}

fn command_snapshot(pid: u32) -> Snapshot {
    let mut source = snapshot();
    change(&mut source.root, pid, |process| {
        process.command = Some(crate::model::ProcessCommand {
            argv: Some(vec![
                "node".into(),
                "script.js".into(),
                "--token".into(),
                "[filtered]".into(),
            ]),
            entrypoint: Some("script.js".into()),
            redacted_indices: vec![3],
            ..crate::model::ProcessCommand::default()
        });
    });
    source
}

#[test]
fn refresh_hint_stays_visible_and_acknowledges_manual_requests() {
    let mut app = app();
    let render = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    for detail in [false, true] {
        if detail {
            select(&mut app, "pid:11");
            app.open_details();
            app.update(Ok(command_snapshot(11)));
        }
        assert!(render(&app).contains("r refresh"));
        app.collecting = true;
        assert!(!render(&app).contains("refreshing"));
        app.collecting = false;
        app.paused = true;
        assert!(render(&app).contains("r refresh"));
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
        assert!(render(&app).contains("refreshing"));
        app.collecting = true;
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
        app.update(Ok(command_snapshot(11)));
        assert!(app.paused);
        assert!(render(&app).contains("r refresh"));
        assert!(!render(&app).contains("refreshing"));
        app.paused = false;
        app.update(Err("fixture failure".into()));
        assert!(render(&app).contains("r retry"));
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
        assert!(render(&app).contains("retrying"));
        app.update(Ok(command_snapshot(11)));
        assert!(!render(&app).contains("r retry"));
        assert!(!render(&app).contains("retrying"));
        handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
        assert!(render(&app).contains("refreshing"));
        app.update(Ok(command_snapshot(11)));
    }
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('?')));
    assert!(render(&app).contains("r: refresh once"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert!(app.manual_refresh.is_none());
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert_eq!(app.query, "r");
    assert!(app.manual_refresh.is_none());
}

#[test]
fn details_replace_the_list_with_a_single_page() {
    let mut app = app();
    select(&mut app, "pid:11");
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let mut table = TableState::default();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    app.open_details();
    app.update(Ok(command_snapshot(11)));
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Host CPU"));
    assert!(!text.contains("Sort RAM"));
    assert!(!text.contains("enter toggle"));
    assert!(!text.contains("Grouped under:"));
    assert!(text.contains("Applications → Demo"));
    assert_eq!(text.matches("pid:11").count(), 1);
    assert_eq!(text.matches("q quit").count(), 0);
    assert!(text.contains("Esc back"));
}

#[test]
fn detail_page_wraps_unicode_paths_and_scrolls_visual_lines() {
    let value = format!("/Applications/{}tail", "專案👩‍💻e\u{301}/".repeat(14));
    let mut source = command_snapshot(11);
    change(&mut source.root, 11, |process| {
        process.command.as_mut().unwrap().argv = Some(vec![value.clone()]);
        process.command.as_mut().unwrap().entrypoint = None;
        process.cwd = Some("/".into());
    });
    let mut app = app();
    select(&mut app, "pid:11");
    app.open_details();
    app.update(Ok(source));
    let mut table = TableState::default();
    for width in [36, 80, 120] {
        let mut terminal = Terminal::new(TestBackend::new(width, 100)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..100)
            .map(|y| {
                let mut x = 0;
                let mut row = String::new();
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    row.push_str(symbol);
                    x += ratatui::text::Span::raw(symbol).width().max(1) as u16;
                }
                row.trim_end().to_string()
            })
            .collect();
        let first = rows
            .iter()
            .position(|row| row.starts_with(" Command  "))
            .unwrap();
        let end = rows
            .iter()
            .position(|row| row.starts_with(" Cwd "))
            .unwrap();
        assert!(end > first + 1);
        let reconstructed: String = rows[first..end]
            .iter()
            .enumerate()
            .map(|(index, row)| {
                if index == 0 {
                    row.strip_prefix(" Command  ").unwrap()
                } else {
                    row.strip_prefix("          ")
                        .expect("indented continuation")
                }
            })
            .collect();
        assert_eq!(reconstructed, value);
        assert_eq!(
            buffer[(1, first as u16)].fg,
            ratatui::style::Color::DarkGray
        );
        assert_eq!(buffer[(10, first as u16)].fg, ratatui::style::Color::Reset);
        assert_eq!(buffer[(0, 1)].fg, ratatui::style::Color::Cyan);
    }
    let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let before = terminal.backend().buffer().clone();
    handle_key(&mut app, KeyEvent::from(KeyCode::End));
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let after = terminal.backend().buffer();
    assert_ne!(before, *after);
    let text: String = after.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("Applications → Demo"));
    assert!(text.contains("Esc back"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Home));
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let restored = terminal.backend().buffer();
    for y in 0..10 {
        let mut x = 0;
        while x < 36 {
            assert_eq!(before[(x, y)], restored[(x, y)]);
            x += ratatui::text::Span::raw(before[(x, y)].symbol())
                .width()
                .max(1) as u16;
        }
    }
}

#[test]
fn details_wait_for_targeted_collection_and_refresh_only_while_open() {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let worker = Worker::with_collector(move |_, pid| {
        recorded.lock().unwrap().push(pid);
        Ok(pid.map_or_else(snapshot, command_snapshot))
    });
    let mut app = app();
    app.paused = true;
    request_snapshot(&worker, &mut app);
    select(&mut app, "pid:11");
    app.open_details();
    app.update(worker.results.recv_timeout(Duration::from_secs(2)).unwrap());
    assert!(app.details.as_ref().unwrap().needs_query);
    assert_eq!(app.details.as_ref().unwrap().query_pid(), Some(11));
    request_snapshot(&worker, &mut app);
    assert!(!app.details.as_ref().unwrap().needs_query);
    app.update(worker.results.recv_timeout(Duration::from_secs(2)).unwrap());
    let lines = app.details.as_ref().unwrap().lines().join("\n");
    assert!(!lines.contains("Loading"));
    assert!(lines.contains("Command  node script.js --token [filtered]"));
    assert!(lines.contains("Applications → Demo"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert!(app.details.as_ref().unwrap().needs_query);
    request_snapshot(&worker, &mut app);
    app.update(worker.results.recv_timeout(Duration::from_secs(2)).unwrap());
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    request_snapshot(&worker, &mut app);
    app.update(worker.results.recv_timeout(Duration::from_secs(2)).unwrap());
    assert_eq!(*calls.lock().unwrap(), [None, Some(11), Some(11), None]);
    app.open_details();
    assert!(app.details.as_ref().unwrap().needs_query);
}

#[test]
fn details_never_retarget_reused_missing_or_unverified_pids() {
    for scope in ["group:all", "pid:11"] {
        let mut app = App::new(scope.into(), usize::MAX);
        app.update(Ok(snapshot()));
        select(&mut app, "pid:11");
        app.open_details();
        app.update(Ok(command_snapshot(11)));
        let mut reused = command_snapshot(11);
        change(&mut reused.root, 11, |process| {
            process.started_unix_us = Some(999);
            process.name = "replacement".into();
        });
        app.update(Ok(reused));
        let details = app.details.as_ref().unwrap();
        assert_eq!(details.query_pid(), None);
        assert!(!details.needs_query);
        let lines = details.lines().join("\n");
        assert!(lines.contains("identity changed"));
        assert!(!lines.contains("replacement"));
        assert!(!lines.contains("script.js"));
        app.update(Ok(command_snapshot(11)));
        assert_eq!(app.details.as_ref().unwrap().query_pid(), None);
    }
    let mut app = app();
    select(&mut app, "pid:11");
    app.open_details();
    let mut missing = snapshot();
    missing.root = tree::build(Vec::new()).unwrap();
    app.update(Ok(missing));
    assert!(
        app.details
            .as_ref()
            .unwrap()
            .lines()
            .join("\n")
            .contains("no longer present")
    );
    assert_eq!(app.details.as_ref().unwrap().query_pid(), None);
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    let mut unknown = snapshot();
    change(&mut unknown.root, 11, |process| {
        process.started_unix_us = None
    });
    app.update(Ok(unknown));
    select(&mut app, "pid:11");
    app.open_details();
    assert_eq!(app.details.as_ref().unwrap().query_pid(), None);
    assert!(!app.details.as_ref().unwrap().needs_query);
    app.paused = true;
    assert_eq!(app.refresh_hint(), "");
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert!(app.manual_refresh.is_none());
}

#[test]
fn details_errors_and_late_results_do_not_reveal_stale_commands() {
    let mut app = app();
    select(&mut app, "pid:11");
    app.open_details();
    app.update(Ok(command_snapshot(11)));
    app.update(Err("fixture failure".into()));
    let details = app.details.as_ref().unwrap();
    assert!(details.lines().join("\n").contains("Refresh failed"));
    assert!(!details.lines().join("\n").contains("script.js"));
    assert!(!details.needs_query);
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert!(app.details.as_ref().unwrap().needs_query);
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    select(&mut app, "pid:10");
    app.open_details();
    app.update(Ok(command_snapshot(11)));
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.query_pid(), Some(10));
    assert!(details.needs_query);
    assert!(!details.lines().join("\n").contains("script.js"));
    let mut denied = snapshot();
    change(&mut denied.root, 10, |process| {
        process.command = Some(crate::model::ProcessCommand {
            unavailable: vec![crate::model::Diagnostic::new(
                "permission_denied",
                "command.read",
                "command: permission denied",
            )],
            ..crate::model::ProcessCommand::default()
        });
    });
    app.update(Ok(denied));
    assert!(!app.details.as_ref().unwrap().needs_query);
    assert!(
        app.details
            .as_ref()
            .unwrap()
            .lines()
            .join("\n")
            .contains("permission denied")
    );
    assert_eq!(app.refresh_hint(), "r retry");
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('r')));
    assert_eq!(app.refresh_hint(), "retrying");
}

#[test]
fn detail_scrolling_is_isolated_and_small_layouts_escape_controls() {
    let mut app = App::new("group:all".into(), usize::MAX);
    let mut source = snapshot();
    source.root = tree::build(
        (1..50)
            .map(|pid| Process {
                pid,
                started_unix_us: Some(u64::from(pid)),
                name: format!("fixture{pid}"),
                executable: Some("/bin/fixture".into()),
                ..Process::default()
            })
            .collect(),
    )
    .unwrap();
    app.update(Ok(source));
    select(&mut app, "pid:40@");
    let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();
    let mut table = TableState::default();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let offset = table.offset();
    let selected = app.selected;
    assert!(offset > 0);
    app.open_details();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    for key in [
        KeyCode::Down,
        KeyCode::Char(']'),
        KeyCode::End,
        KeyCode::Char('c'),
        KeyCode::Char('/'),
        KeyCode::Enter,
        KeyCode::Esc,
    ] {
        handle_key(&mut app, KeyEvent::from(key));
    }
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(app.selected, selected);
    assert_eq!(table.offset(), offset);
    assert!(!app.editing);
    assert_eq!(app.sort, Sort::Memory);
    let mut untrusted = command_snapshot(11);
    change(&mut untrusted.root, 11, |process| {
        process.name = "name\x1b[31m\u{202e}".into();
        process.cwd = Some(format!("/{}\x1b", "long".repeat(100)).into());
    });
    app.update(Ok(untrusted.clone()));
    select(&mut app, "pid:11");
    app.open_details();
    app.update(Ok(untrusted));
    for (width, height) in [(120, 24), (80, 14), (36, 10), (20, 6)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains(['\x1b', '\u{202e}']));
        if width >= 36 {
            assert!(text.contains("Process details"));
            assert!(text.contains("Esc back"));
        }
    }
}

fn select(app: &mut App, reference: &str) {
    app.selected = app
        .rows
        .iter()
        .position(|row| row.key.starts_with(reference))
        .unwrap();
}

fn change(node: &mut Node, pid: u32, update: impl FnOnce(&mut Process) + Copy) {
    if let Some(process) = &mut node.process
        && process.pid == pid
    {
        update(process);
    }
    for child in &mut node.children {
        change(child, pid, update);
    }
}

#[test]
fn sort_refresh_and_collapse_preserve_identity_and_tree() {
    let mut app = app();
    select(&mut app, "pid:10");
    let selected = app.rows[app.selected].key.clone();
    app.sort = Sort::Uptime;
    app.rebuild();
    assert_eq!(app.rows[app.selected].key, selected);
    let first = app
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, _)| app.node(index))
        .find(|node| node.process.is_some())
        .unwrap();
    assert_eq!(first.reference, "pid:10");
    let mut next = snapshot();
    change(&mut next.root, 10, |process| {
        process.resident_bytes = Some(999)
    });
    app.sort = Sort::Memory;
    app.update(Ok(next));
    assert_eq!(app.rows[app.selected].key, selected);
    app.collapse();
    let parent = app.rows[app.selected].key.clone();
    app.collapse();
    assert!(!app.rows.iter().any(|row| row.key.starts_with("pid:10")));
    app.update(Ok(snapshot()));
    assert_eq!(app.rows[app.selected].key, parent);
    assert!(!app.rows.iter().any(|row| row.key.starts_with("pid:10")));
    app.expand();
    assert!(app.rows.iter().any(|row| row.key.starts_with("pid:10")));
}

#[test]
fn pid_reuse_and_missing_scope_never_silently_retarget_selection() {
    let mut app = app();
    select(&mut app, "pid:10");
    let mut replaced = snapshot();
    change(&mut replaced.root, 10, |process| {
        process.started_unix_us = Some(900)
    });
    app.update(Ok(replaced.clone()));
    assert_ne!(app.node(app.selected).unwrap().reference, "pid:10");
    let mut scoped = App::new("pid:10".into(), 2);
    scoped.update(Ok(snapshot()));
    scoped.update(Ok(replaced));
    assert!(scoped.error.as_ref().unwrap().contains("identity changed"));
    assert_eq!(
        scoped
            .snapshot
            .as_ref()
            .unwrap()
            .root
            .process
            .as_ref()
            .unwrap()
            .started_unix_us,
        Some(100)
    );
    let mut missing = snapshot();
    missing.root.children.clear();
    scoped.update(Ok(missing));
    assert!(scoped.error.as_ref().unwrap().contains("target not found"));
    scoped.update(Err("permission denied".into()));
    assert!(scoped.snapshot.is_some());
    assert_eq!(scoped.error.as_deref(), Some("permission denied"));
}

#[test]
fn search_reveals_ancestors_without_losing_saved_expansion() {
    let mut app = App::new("group:all".into(), 0);
    app.update(Ok(snapshot()));
    let original: Vec<_> = app.rows.iter().map(|row| row.key.clone()).collect();
    assert!(!original.contains(&"group:all".to_string()));
    let saved = app.expanded.clone();
    app.query = "pid:10".into();
    app.rebuild();
    assert!(app.rows.iter().any(|row| row.key.starts_with("pid:10")));
    assert!(app.rows.iter().any(|row| row.key == "group:applications"));
    assert!(!app.rows.iter().any(|row| row.key.starts_with("pid:11")));
    app.query = "/workspace/demo".into();
    app.rebuild();
    assert!(app.rows.iter().any(|row| row.key == "group:live"));
    assert!(!app.rows.iter().any(|row| row.key == "group:applications"));
    app.query.clear();
    app.rebuild();
    assert_eq!(
        app.rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>(),
        original
    );
    assert_eq!(app.expanded, saved);
}

#[test]
fn active_filter_keeps_unknowns_and_unreadable_is_one_leaf() {
    let mut app = app();
    let before = serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap();
    app.active_only = true;
    app.rebuild();
    assert!(app.rows.iter().any(|row| row.key == "group:live"));
    assert!(app.rows.iter().any(|row| row.key == "group:unknown"));
    assert!(!app.rows.iter().any(|row| row.key == "group:stopped"));
    assert_eq!(
        serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap(),
        before
    );
    assert!(!app.rows.iter().any(|row| row.key.starts_with("pid:30")));
    let index = app.rows.iter().position(|row| row.unreadable == 2).unwrap();
    assert!(!app.rows[index].has_children);
    app.selected = index;
    app.expand();
    assert_eq!(app.selected, index);
    let parent = app.rows[index].path.clone();
    app.collapse();
    assert_eq!(app.rows[app.selected].path, parent);
    assert_eq!(app.rows[app.selected].unreadable, 0);
    select(&mut app, "group:runtime");
    assert_eq!(view::selected_paths(&app), "Path —");
}

#[test]
fn overview_skips_redundant_wrappers_without_changing_data_or_scoped_roots() {
    let mut source = snapshot();
    source.root = tree::build(vec![Process {
        pid: 20,
        name: "system-agent".into(),
        executable: Some("/usr/libexec/system-agent".into()),
        cpu_percent: Some(240.0),
        resident_bytes: Some(100),
        started_unix_us: Some(1),
        ..Process::default()
    }])
    .unwrap();
    source.system.cpu_percent = Some(30.0);
    let wrapper = source.root.children[0].children[0].reference.clone();
    source.project(None);
    let original = serde_json::to_value(&source.root).unwrap();
    let mut app = App::new("group:all".into(), 2);
    app.update(Ok(source.clone()));
    assert_eq!(app.rows.len(), 2);
    assert_eq!(app.node(0).unwrap().reference, "group:system");
    assert_eq!(app.rows[0].depth, 0);
    assert_eq!(app.node(1).unwrap().reference, "pid:20");
    assert_eq!(app.rows[1].depth, 1);
    assert_eq!(
        serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap(),
        original
    );
    app.selected = 1;
    app.collapse();
    assert_eq!(app.node(app.selected).unwrap().reference, "group:system");
    app.collapse();
    assert_eq!(app.rows.len(), 1);
    app.update(Ok(source.clone()));
    assert_eq!(app.rows.len(), 1);
    app.expand();
    assert_eq!(app.rows.len(), 2);
    let mut terminal = Terminal::new(TestBackend::new(150, 24)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Host CPU 30.0%"));
    assert!(text.contains("CPU/core"));
    assert!(text.contains("240.0%"));
    assert!(!text.contains("System services"));
    let mut scoped = App::new(wrapper.clone(), 1);
    scoped.update(Ok(source));
    assert_eq!(scoped.node(0).unwrap().reference, wrapper);
    assert_eq!(scoped.rows[0].depth, 0);
}

#[test]
fn natural_name_sort_compares_numeric_runs_and_preserves_selection_and_parenthood() {
    let names = [
        "11", "1", "10", "3", "02", "2", "node10", "Node2", "v2.11", "v2.3", "版10", "版2", "B1",
        "b2", "A10", "a2",
    ];
    let mut source = snapshot();
    source.root.children = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Node::group(
                format!("group:item{index}"),
                (*name).into(),
                vec![],
                Grouping::Category,
            )
        })
        .collect();
    let mut child = tree::select(&snapshot().root, "pid:10").unwrap();
    child.name = "000".into();
    source.root.children[0].children.push(child);
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(source.clone()));
    select(&mut app, "group:item0");
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('n')));
    assert_eq!(app.sort, Sort::Name);
    assert_eq!(app.node(app.selected).unwrap().reference, "group:item0");
    let order: Vec<_> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.depth == 0)
        .map(|(index, _)| app.node(index).unwrap().name.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "1", "2", "02", "3", "10", "11", "a2", "A10", "B1", "b2", "Node2", "node10", "v2.3",
            "v2.11", "版2", "版10"
        ]
    );
    assert_eq!(app.node(app.selected + 1).unwrap().name, "000");
    source.root.children.reverse();
    app.update(Ok(source));
    assert_eq!(app.node(app.selected).unwrap().reference, "group:item0");
    let mut source = snapshot();
    source.root.children = vec![
        Node::group(
            "group:b".into(),
            format!("n1{}", "0".repeat(80)),
            vec![],
            Grouping::Category,
        ),
        Node::group(
            "group:a".into(),
            format!("n{}", "9".repeat(80)),
            vec![],
            Grouping::Category,
        ),
    ];
    app.update(Ok(source));
    assert_eq!(app.node(0).unwrap().reference, "group:a");
}

#[test]
fn keys_keep_search_text_separate_from_shortcuts() {
    let mut app = app();
    for ch in ['/', 'c', 'q', '界'] {
        assert!(!handle_key(&mut app, KeyEvent::from(KeyCode::Char(ch))));
    }
    assert_eq!(app.query, "cq界");
    assert_eq!(app.sort, Sort::Memory);
    handle_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    assert_eq!(app.query, "cq");
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    assert!(app.query.is_empty());
    assert!(handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('a')));
    assert!(app.active_only);
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    assert!(app.paused);
}

#[test]
fn sorting_refresh_and_layout_keep_selected_screen_row_without_pinning_navigation() {
    let mut source = snapshot();
    let template = tree::select(&source.root, "pid:10").unwrap();
    source.root.children = (1..=80)
        .map(|pid| {
            let mut node = template.clone();
            node.reference = format!("pid:{pid}");
            node.name = format!("worker{pid}");
            let process = node.process.as_mut().unwrap();
            process.pid = pid;
            process.resident_bytes = Some(u64::from(pid));
            process.cpu_percent = Some(f64::from(81 - pid));
            node
        })
        .collect();
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(source.clone()));
    select(&mut app, "pid:30@");
    let mut table = TableState::default().with_offset(app.selected - 5);
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(app.selected - table.offset(), 5);
    let anchor = SelectionAnchor::capture(&app, &table).unwrap();
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('c')));
    anchor.restore(&app, &mut table);
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(app.node(app.selected).unwrap().reference, "pid:30");
    assert_eq!(app.selected - table.offset(), 5);

    change(&mut source.root, 30, |process| {
        process.cpu_percent = Some(20.5)
    });
    let anchor = SelectionAnchor::capture(&app, &table).unwrap();
    app.update(Ok(source));
    anchor.restore(&app, &mut table);
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(app.node(app.selected).unwrap().reference, "pid:30");
    assert_eq!(app.selected - table.offset(), 5);
    let before = table.offset();
    for key in ['?', '?', 'a'] {
        let anchor = SelectionAnchor::capture(&app, &table).unwrap();
        handle_key(&mut app, KeyEvent::from(KeyCode::Char(key)));
        anchor.restore(&app, &mut table);
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        assert_eq!(table.offset(), before);
    }
    let anchor = SelectionAnchor::capture(&app, &table).unwrap();
    handle_key(&mut app, KeyEvent::from(KeyCode::Down));
    anchor.restore(&app, &mut table);
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(table.offset(), before);
    assert_eq!(app.selected - table.offset(), 6);

    let selected_key = app.rows[app.selected].key.clone();
    terminal.backend_mut().resize(70, 14);
    terminal.autoresize().unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(app.rows[app.selected].key, selected_key);
    assert!(app.selected >= table.offset());
    assert!(app.selected - table.offset() < 6);

    let anchor = SelectionAnchor::capture(&app, &table).unwrap();
    handle_key(&mut app, KeyEvent::from(KeyCode::Home));
    anchor.restore(&app, &mut table);
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert_eq!(table.offset(), 0);
    assert_eq!(app.selected, 0);

    let anchor = SelectionAnchor::capture(&app, &table).unwrap();
    app.query = "no-such-resource".into();
    app.rebuild();
    anchor.restore(&app, &mut table);
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    assert!(app.rows.is_empty());
    assert_eq!(table.selected(), None);
    assert_eq!(table.offset(), 0);
}

#[test]
fn layouts_and_untrusted_strings_render_without_terminal_controls() {
    let mut app = app();
    let mut snapshot = snapshot();
    snapshot.root.children[0].name = "root\u{1b}[31m\n\u{202e}".into();
    app.update(Ok(snapshot));
    for (width, height) in [(150, 35), (80, 24), (36, 10), (20, 6)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut state = TableState::default();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut state))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains(['\u{1b}', '\u{202e}']));
        assert!(text.contains(if width >= 36 { "Host CPU" } else { "reswho" }));
        if width >= 80 {
            assert!(!text.contains("Details"));
            assert!(text.contains("Path —"));
            assert!(text.contains("Memory"));
        }
        app.help = true;
        terminal
            .draw(|frame| view::draw(frame, &app, &mut state))
            .unwrap();
        app.help = false;
    }
}

#[test]
fn path_status_tracks_selection_and_preserves_literal_paths() {
    let mut app = app();
    select(&mut app, "pid:10");
    let paths = view::selected_paths(&app);
    assert!(paths.contains("cwd /"));
    assert!(paths.contains("exe /Applications/Demo.app/Contents/MacOS/demo"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char(']')));
    assert_eq!(app.path_scroll, 16);
    app.move_selection(1);
    assert_eq!(app.path_scroll, 0);
    select(&mut app, "group:live");
    assert_eq!(view::selected_paths(&app), "candidate /workspace/demo");
    select(&mut app, "group:applications");
    assert_eq!(view::selected_paths(&app), "Path —");
    for evidence in [
        "Application bundle: /Applications/Some App.app",
        "Git root from working directory: /workspace/some repo",
        "project Cargo.toml /workspace/專案 one",
        "project /workspace/專案 one",
        "exe /custom/bin/tool",
        "compose /workspace/compose project",
    ] {
        let mut next = snapshot();
        next.root
            .children
            .iter_mut()
            .find(|node| node.reference == "group:applications")
            .unwrap()
            .evidence = vec![evidence.into()];
        app.update(Ok(next));
        let path = evidence.find('/').unwrap();
        assert_eq!(
            view::selected_paths(&app),
            format!("Path {}", &evidence[path..])
        );
    }
    let mut next = snapshot();
    change(&mut next.root, 10, |process| {
        process.cwd = Some("/tmp/line\n\u{1b}[31m\u{202e}".into())
    });
    app.update(Ok(next));
    select(&mut app, "pid:10");
    let paths = view::selected_paths(&app);
    assert!(!paths.contains(['\n', '\u{1b}', '\u{202e}']));
    assert!(paths.contains("\\u{1b}"));
}

#[test]
fn first_snapshot_wait_shows_loading_then_data_or_error() {
    let mut app = App::new("group:all".into(), 2);
    app.collecting = true;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut table = TableState::default();
    let render = |app: &App, terminal: &mut Terminal<TestBackend>, table: &mut TableState| {
        terminal
            .draw(|frame| view::draw(frame, app, table))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let first = render(&app, &mut terminal, &mut table);
    let heading_column = |terminal: &Terminal<TestBackend>| {
        (0..80).find(|&x| terminal.backend().buffer()[(x, 3)].symbol() == "R")
    };
    let loading_column = heading_column(&terminal);
    assert_eq!(loading_column, Some(3));
    assert!(first.contains("| Loading resources..."));
    assert!(first.contains("Resources"));
    assert!(first.contains("Memory"));
    assert!(first.contains("Sort RAM"));
    assert!(first.contains("Path —"));
    assert!(!first.contains("No selection"));
    app.loading_frame = 1;
    assert!(render(&app, &mut terminal, &mut table).contains("/ Loading resources..."));
    app.update(Err("collection failed".into()));
    let error = render(&app, &mut terminal, &mut table);
    assert_eq!(heading_column(&terminal), loading_column);
    assert!(error.contains("Unable to load resources"));
    assert!(error.contains("collection failed"));
    assert!(!error.contains("Loading resources..."));
    app.update(Ok(snapshot()));
    let loaded = render(&app, &mut terminal, &mut table);
    assert_eq!(
        loading_column,
        heading_column(&terminal),
        "column headings must keep their padding before and after loading"
    );
    let header: String = terminal.backend().buffer().content[..80]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    let toolbar: String = terminal.backend().buffer().content[80..160]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(header.starts_with("Host CPU"));
    assert!(header.contains("processes"));
    assert!(toolbar.starts_with("Sort RAM"));
    assert!(loaded.contains("Resources"));
    assert!(!loaded.contains("Loading resources..."));
    app.query = "no-matching-fixture".into();
    app.rebuild();
    render(&app, &mut terminal, &mut table);
    assert_eq!(heading_column(&terminal), loading_column);
    app.query.clear();
    app.rebuild();
    assert!(!loaded.contains("s ago"));
    assert!(!loaded.contains("| live"));
    app.collecting = true;
    assert_eq!(render(&app, &mut terminal, &mut table), loaded);
    app.paused = true;
    assert!(render(&app, &mut terminal, &mut table).contains("| paused"));
    app.update(Err("refresh failed".into()));
    let failed = render(&app, &mut terminal, &mut table);
    assert!(failed.contains("| error"));
    assert!(failed.contains("refresh failed"));
}

#[test]
fn worker_cancels_and_joins_owned_collection() {
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let start_flag = started.clone();
    let finish_flag = finished.clone();
    let worker = Worker::with_collector(move |cancel, _| {
        start_flag.store(true, Ordering::SeqCst);
        let result = cancel.wait(Duration::from_secs(30));
        finish_flag.store(true, Ordering::SeqCst);
        result
            .map(|_| snapshot())
            .map_err(|error| error.to_string())
    });
    assert!(worker.request(None));
    let start = Instant::now();
    while !started.load(Ordering::SeqCst) {
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(worker);
    assert!(finished.load(Ordering::SeqCst));
    assert!(start.elapsed() < Duration::from_secs(2));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(cancel.wait(Duration::ZERO).is_err());
}

#[test]
fn apple_runtime_renders_and_stopped_containers_follow_active_filter() {
    let mut snapshot = snapshot();
    let containers = snapshot
        .root
        .children
        .iter_mut()
        .find(|n| n.reference == "group:containers")
        .unwrap();
    let runtime = &mut containers.children[0];
    runtime.name = "apple (local)".into();
    for node in &mut runtime.children {
        let container = node.container.as_mut().unwrap();
        container.runtime = "apple".into();
        if container.state.as_deref() == Some("exited") {
            container.state = Some("stopped".into());
        }
    }
    let mut app = App::new("group:containers".into(), usize::MAX);
    app.update(Ok(snapshot));
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("apple (local)"));
    assert!(app.rows.iter().any(|row| row.key == "group:stopped"));
    app.active_only = true;
    app.rebuild();
    assert!(!app.rows.iter().any(|row| row.key == "group:stopped"));
    assert!(app.rows.iter().any(|row| row.key == "group:live"));
    assert!(app.rows.iter().any(|row| row.key == "group:unknown"));
}

#[test]
fn refresh_wait_pause_and_manual_requests_are_independent_of_sampling() {
    let mut app = App::new("group:all".into(), 2);
    let now = Instant::now();
    let next = now + Duration::from_millis(750);
    app.collecting = false;
    assert!(!super::should_collect(&app, next, now));
    assert!(!super::should_collect(
        &app,
        next,
        next - Duration::from_millis(1)
    ));
    assert!(super::should_collect(&app, next, next));
    app.paused = true;
    assert!(!super::should_collect(
        &app,
        next,
        next + Duration::from_secs(5)
    ));
    app.request_refresh();
    assert!(super::should_collect(&app, next, now));
    assert!(app.paused);
    app.collecting = true;
    assert!(!super::should_collect(
        &app,
        next,
        next + Duration::from_secs(5)
    ));
    app.update(Err("fixture failure".into()));
    assert!(app.paused);
    assert!(!super::should_collect(
        &app,
        next,
        next + Duration::from_secs(5)
    ));
}

#[test]
fn extension_command_details_render_on_demand_with_core_redaction() {
    let render = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    use crate::extensions::{
        self, Capability, Contribution, Descriptor, DetailProposal, Extension, ProcessIdentity,
        Request,
    };
    struct CommandFixture;
    impl Extension for CommandFixture {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: "fixture".into(),
                version: "1".into(),
                protocol_version: 1,
                source: "builtin",
                platforms: vec!["macos".into()],
                capabilities: vec![Capability::Details],
                enabled: true,
                availability: "unknown",
            }
        }
        fn execute(
            &self,
            _: &extensions::Context<'_>,
            request: Request<'_>,
        ) -> anyhow::Result<Contribution> {
            let Request::Details { process, .. } = request else {
                anyhow::bail!("unexpected fixture request");
            };
            Ok(Contribution::Details {
                proposal: DetailProposal {
                    identity: ProcessIdentity::observed(process).unwrap(),
                    command: crate::model::ProcessCommand {
                        argv: Some(vec![
                            "demo".into(),
                            "--token".into(),
                            "fixture-secret".into(),
                        ]),
                        ..Default::default()
                    },
                },
            })
        }
    }
    let settings = extensions::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let mut app = App::new("pid:10".into(), 2);
    app.update(Ok(snapshot()));
    app.open_details();
    assert_eq!(app.details.as_ref().unwrap().query_pid(), Some(10));
    assert!(render(&app).contains("Loading command"));
    let mut result = snapshot();
    fn attach(node: &mut Node, context: &extensions::Context<'_>) {
        if let Some(process) = &mut node.process
            && process.pid == 10
        {
            process.command =
                Some(extensions::details(&CommandFixture, context, process, 8).unwrap());
        }
        for child in &mut node.children {
            attach(child, context);
        }
    }
    attach(&mut result.root, &context);
    app.update(Ok(result));
    let text = render(&app);
    assert!(text.contains("[filtered]"));
    assert!(!text.contains("fixture-secret"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    assert!(app.details.is_none());
}

#[test]
fn installed_external_example_renders_parsed_details_without_command_payload() {
    use crate::extensions::{
        self, external,
        state::{Manifest, State},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let manifest = Manifest::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/extensions/example.json")
            .as_path(),
    )
    .unwrap();
    State::mutate(&path, |state| state.install(manifest)).unwrap();
    let state = State::load(&path).unwrap();
    let settings = state
        .registry()
        .unwrap()
        .resolve(
            &crate::settings::Config::default(),
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
    let cancel = Cancellation::default();
    let context = extensions::Context::new(&settings, &cancel);
    let providers = external::installed(&state, &settings);
    let mut app = app();
    select(&mut app, "pid:10");
    app.open_details();
    let mut source = snapshot();
    change(&mut source.root, 10, |process| {
        assert!(external::attach_details(&[&providers[0]], &context, process).is_empty());
    });
    app.update(Ok(source));
    let mut terminal = Terminal::new(TestBackend::new(150, 40)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Shared sample ms"));
    assert!(text.contains("[filtered]"));
    assert!(text.contains("Host CLI version"));
    assert!(!text.contains("fixture-secret-value"));
    assert!(!text.contains("Loading command"));
    dir.close().unwrap();
}

#[test]
fn extension_page_preserves_navigation_never_requests_collection_and_keeps_failed_report() {
    let mut app = app();
    select(&mut app, "pid:11");
    let selected = app.selected;
    app.paused = true;
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        Some("1.0.0"),
        "read",
        "failed",
        Some("invalid_manifest"),
        Some(Instant::now()),
    )];
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    let mut table = TableState::default();
    table.select(Some(selected));
    *table.offset_mut() = 1;
    let offset = table.offset();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Extension issues: 1 (e)"));
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('e')));
    terminal
        .draw(|frame| view::draw(frame, &app, &mut table))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Extensions") && text.contains("System diagnostics"));
    assert!(text.contains("invalid_manifest"));
    assert_eq!(table.offset(), offset);
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    assert_eq!(app.selected, selected);
    assert_eq!(table.offset(), offset);
    assert!(app.manual_refresh.is_none());
    assert!(!app.collecting);
    app.update(Err("synthetic worker failure".into()));
    assert_eq!(app.extensions.len(), 1);
    assert_eq!(
        app.extensions[0].reason.as_deref(),
        Some("invalid_manifest")
    );
}

#[test]
fn settings_popup_routes_drafts_masks_values_and_restores_focus() {
    let key = KeyEvent::from;
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    use std::collections::BTreeMap;
    let owner = tempfile::tempdir().unwrap();
    let path = owner.path().canonicalize().unwrap().join("config.json");
    let mut registry = Registry::core();
    let definitions = [
        json!({"key":"extensions.fixture.flag","type":"boolean","default":false,"description":"A flag","provider":"fixture","version":"1","sensitive":false}),
        json!({"key":"extensions.fixture.number","type":"number","options":[-1.25,2.5],"default":-1.25,"description":"A choice","provider":"fixture","version":"1","sensitive":false}),
        json!({"key":"extensions.fixture.secret","type":"string","options":["secret-option","another-secret"],"default":"secret-option","description":"Sensitive choice","provider":"fixture","version":"1","sensitive":true}),
        json!({"key":"extensions.fixture.text","type":"string","default":"initial","description":format!("{}Description end", "Long description ".repeat(50)),"provider":"fixture","version":"1","sensitive":false}),
    ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect();
    registry
        .register_extension("fixture", definitions, vec![])
        .unwrap();
    let baseline: Config = serde_json::from_value(
        json!({"version":1,"settings":{"extensions.fixture.flag":"invalid"}}),
    )
    .unwrap();
    std::fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    assert!(
        registry
            .resolve(&baseline, &BTreeMap::new())
            .unwrap()
            .get("extensions.fixture.flag")
            .is_none()
    );
    let context = Settings::new(
        registry,
        baseline,
        BTreeMap::from([("refresh.interval_ms".into(), json!(60000))]),
        Some(path.clone()),
    );
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(snapshot()));
    app.selected = 2;
    app.paused = true;
    app.settings = Some(settings::Popup::new(context));
    let mut table = TableState::default().with_offset(1);
    let selected = app.selected;
    let offset = table.offset();
    assert!(!handle_key(&mut app, key(KeyCode::Char('s'))));
    handle_key(&mut app, key(KeyCode::Char(' ')));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Enter));
    for (width, height) in [(100, 30), (36, 10), (20, 6)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains("secret-option") && !text.contains("another-secret"));
        assert!(text.contains("Settings"));
    }
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::PageDown));
    let mut pages = String::new();
    for _ in 0..8 {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        pages.extend(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol()),
        );
        handle_key(&mut app, key(KeyCode::PageDown));
    }
    assert!(pages.contains("Description end"));
    handle_key(&mut app, key(KeyCode::End));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    for ch in "qs text".chars() {
        assert!(!handle_key(&mut app, key(KeyCode::Char(ch))));
    }
    handle_key(&mut app, key(KeyCode::Enter));
    let popup = app.settings.as_ref().unwrap();
    assert_eq!(
        popup.changes["extensions.fixture.text"],
        Some(json!("qs text"))
    );
    assert_eq!(popup.changes["extensions.fixture.flag"], Some(json!(true)));
    assert_eq!(popup.changes["extensions.fixture.number"], Some(json!(2.5)));
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.settings.as_ref().unwrap().changes.is_empty());
    assert_eq!(app.selected, selected);
    assert_eq!(table.offset(), offset);
    assert!(app.paused);
    assert_eq!(
        Config::load(Some(&path)).unwrap().settings["extensions.fixture.flag"],
        json!("invalid")
    );
    handle_key(&mut app, key(KeyCode::Char('s')));
    handle_key(&mut app, key(KeyCode::Home));
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.settings.as_ref().unwrap().changes.is_empty());
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, key(KeyCode::Char('1')));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().applied.is_none());
    owner.close().unwrap();
}

#[test]
fn compact_settings_keeps_primary_controls_input_and_scrollable_info_visible() {
    use crate::settings::{Config, Registry, edit::Settings};
    use std::collections::BTreeMap;
    let mut app = App::new("group:all".into(), 2);
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        BTreeMap::new(),
        None,
    )));
    handle_key(&mut app, KeyCode::Char('s').into());
    handle_key(&mut app, KeyCode::End.into());
    let render = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let initial = render(&app);
    for label in [
        "Ctrl-S Save",
        "Ctrl-R Reset",
        "Esc discard",
        "Sampling interva",
        "Time between",
        "[250] ms",
    ] {
        assert!(initial.contains(label), "missing {label}: {initial}");
    }
    handle_key(&mut app, KeyCode::PageDown.into());
    let mut pages = String::new();
    for _ in 0..8 {
        pages.push_str(&render(&app));
        handle_key(&mut app, KeyCode::PageDown.into());
    }
    for label in [
        "Key:",
        "Default: 250",
        "Constraints:",
        "max=5000",
        "Value: 250",
    ] {
        assert!(pages.contains(label), "missing {label}: {pages}");
    }
    for _ in 0..8 {
        handle_key(&mut app, KeyCode::PageUp.into());
    }
    assert!(render(&app).contains("Time between"));
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, KeyCode::Char('1').into());
    handle_key(&mut app, KeyCode::Enter.into());
    let invalid = render(&app);
    for label in [
        "[1_]",
        "Invalid value",
        "Ctrl-S Save",
        "Ctrl-U Clear",
        "Esc Undo field",
    ] {
        assert!(invalid.contains(label), "missing {label}: {invalid}");
    }
}

#[test]
fn settings_panel_is_centered_and_content_sized() {
    use crate::settings::{Config, Registry, edit::Settings};
    let mut app = app();
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        Default::default(),
        None,
    )));
    handle_key(&mut app, KeyCode::Char('s').into());
    let mut terminal = Terminal::new(TestBackend::new(160, 45)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = (0..45)
        .map(|y| (0..160).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let title = rows
        .iter()
        .position(|row| row.contains("Settings"))
        .unwrap();
    assert!(title > 5, "panel should be vertically centered");
    assert!(rows[title].find('┌').unwrap() >= 40);
    assert!(rows.iter().any(|row| row.contains("Refresh interval")));
    assert!(!rows.join("\n").contains("18446744073709551615"));
}

#[test]
fn selected_settings_rows_stay_stable_and_extension_hints_wrap_only_when_needed() {
    use crate::settings::{Config, Registry, edit::Settings};
    let mut app = app();
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        Default::default(),
        None,
    )));
    let resolved = crate::extensions::default_settings().unwrap();
    let registry = crate::extensions::state::State::default();
    app.inventory = registry.list(&resolved, true);
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        None,
        "read",
        "failed",
        Some("invalid_manifest"),
        None,
    )];
    let dump = |app: &App, width, height, name: &str| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE") {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                std::path::Path::new(&path).join(format!("{name}-{width}x{height}.txt")),
                &text,
            )
            .unwrap();
        }
        text
    };
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "main");
        for hint in ["? Help", "q Quit", "r refresh"] {
            assert!(text.contains(hint), "missing {hint}: {text}");
        }
        assert!(!text.contains("j/k move"));
        if width == 100 {
            for hint in ["Enter Toggle", "/ Search", "s Settings", "e Extensions"] {
                assert!(text.contains(hint), "missing {hint}: {text}");
            }
        }
    }
    select(&mut app, "pid:11");
    app.open_details();
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "process-details");
        assert!(text.contains("Esc back") && text.contains("p pause"));
        assert!(!text.contains("j/k") && !text.contains("scroll"));
    }
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('s').into());
    for (width, height) in [(160, 45), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "settings");
        assert!(text.contains("[Save]") && text.contains("[Cancel]"));
        assert!(
            text.contains("Ctrl-S Save")
                && text.contains("Ctrl-R Reset")
                && text.contains("Esc discard")
        );
        assert!(!text.contains("Move"));
        if width >= 100 {
            assert_eq!(text.matches("General").count(), 1);
        }
    }
    let first = dump(&app, 100, 30, "settings-first");
    handle_key(&mut app, KeyCode::Down.into());
    let second = dump(&app, 100, 30, "settings-second");
    let row = |text: &str, label: &str| text.lines().position(|line| line.contains(label)).unwrap();
    for label in ["Refresh interval", "Sampling interval"] {
        assert_eq!(row(&first, label), row(&second, label));
    }
    for (text, description) in [
        (&first, "Wait between completed collections."),
        (&second, "Time between resource samples."),
    ] {
        assert!(row(text, "Sampling interval") < row(text, description));
        assert!(row(text, "[Save]") < row(text, description));
        assert_eq!(row(text, description) + 1, row(text, "Enter Edit"));
        assert_eq!(text.matches(description).count(), 1);
    }
    dump(&app, 36, 10, "settings-second");
    handle_key(&mut app, KeyCode::Home.into());
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, KeyCode::Char('0').into());
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "settings-edit");
        assert!(
            text.contains("[0_]")
                && text.contains("Enter Accept")
                && text.contains("Esc Undo field")
        );
        assert!(
            !text.contains("Edit:") && !text.contains("Default:") && !text.contains("Constraints:")
        );
        if width == 100 {
            assert_eq!(
                row(&first, "Refresh interval"),
                row(&text, "Refresh interval")
            );
        }
    }
    handle_key(&mut app, KeyCode::Enter.into());
    assert!(dump(&app, 100, 30, "settings-invalid").contains("Invalid value"));
    assert!(dump(&app, 36, 10, "settings-invalid").contains("Invalid value"));
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(app.settings.as_ref().unwrap().open);
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('s').into());
    handle_key(&mut app, KeyCode::Tab.into());
    handle_key(&mut app, KeyCode::Enter.into());
    assert!(!app.settings.as_ref().unwrap().open);
    handle_key(&mut app, KeyCode::Char('s').into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    handle_key(&mut app, KeyCode::Enter.into());
    assert!(!app.settings.as_ref().unwrap().open);
    handle_key(&mut app, KeyCode::Char('e').into());
    handle_key(&mut app, KeyCode::Char('j').into());
    let selected = app.extension_selected.clone();
    app.inventory.reverse();
    app.update(Ok(snapshot()));
    assert_eq!(app.extension_selected, selected);
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        None,
        "read",
        "failed",
        Some("invalid_manifest"),
        None,
    )];
    for (id, phase, result, duration) in [
        ("docker", "collect", "success", 561.0),
        ("app_bundle", "classify", "success", 1.2),
        ("command", "details", "failed", 8.4),
    ] {
        let mut record = crate::extensions::report::Record::new(
            id,
            Some("0.1.0"),
            phase,
            result,
            if result == "failed" {
                Some("invalid_data")
            } else {
                None
            },
            None,
        );
        record.duration_ms = Some(duration);
        app.extensions.push(record);
    }
    for (width, height) in [(160, 45), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "extensions");
        assert!(text.contains("System diagnostics") && text.contains("invalid_manifest"));
        assert!(text.contains("Name") && text.contains("State"));
        assert!(
            !text.contains("s Settings")
                && !text.contains("q Quit")
                && !text.contains("Enter for details")
        );
        assert!(text.contains("Shift") && text.contains("Save") && !text.contains("j/k"));
        for hint in ["Enter Details", "Esc discard"] {
            assert!(text.contains(hint), "missing {hint}: {text}");
        }
        if width >= 100 {
            assert!(
                text.lines()
                    .any(|line| line.contains("Enter Details") && line.contains("Esc discard"))
            );
        }
        if width >= 100 {
            for column in ["Version", "Collect", "Classify", "Details"] {
                assert!(text.contains(column));
            }
        }
    }
    app.extension_selected = Some("command".into());
    handle_key(&mut app, KeyCode::Enter.into());
    let text = dump(&app, 100, 30, "extension-details");
    assert!(text.contains("Availability: unknown") && text.contains("No invocation timing"));
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "extension-details");
        assert!(!text.contains("Scroll") && !text.contains("j/k"));
        assert_eq!(text.matches("Esc Back").count(), 1, "{text}");
        assert!(text.contains("Process command"));
    }
    handle_key(&mut app, KeyCode::Esc.into());
    app.inventory[0].explicit_enabled = false;
    assert!(dump(&app, 100, 30, "extensions-disabled").contains("Disabled"));
}

#[test]
fn invalid_setting_input_keeps_draft_while_controls_remain_reachable() {
    use crate::settings::{Config, Registry, edit::Settings};
    let mut app = app();
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        Default::default(),
        None,
    )));
    let send = |app: &mut App, code| {
        handle_key(app, KeyEvent::from(code));
    };
    send(&mut app, KeyCode::Char('s'));
    send(&mut app, KeyCode::Enter);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    send(&mut app, KeyCode::Char('0'));
    send(&mut app, KeyCode::Enter);
    send(&mut app, KeyCode::BackTab);
    send(&mut app, KeyCode::Enter);
    assert!(app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().applied.is_none());
    send(&mut app, KeyCode::Tab);
    send(&mut app, KeyCode::Char(' '));
    let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("[0 _]"), "{text}");
    send(&mut app, KeyCode::BackTab);
    send(&mut app, KeyCode::BackTab);
    send(&mut app, KeyCode::Enter);
    assert!(!app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().changes.is_empty());
    send(&mut app, KeyCode::Char('s'));
    send(&mut app, KeyCode::Enter);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    send(&mut app, KeyCode::Char('0'));
    send(&mut app, KeyCode::Tab);
    send(&mut app, KeyCode::Enter);
    assert!(!app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().applied.is_none());
}

#[test]
fn compact_explicit_info_exposes_current_long_label_boolean_and_masks_sensitive_value() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    for sensitive in [false, true] {
        let mut registry = Registry::core();
        let key = "extensions.fixture.a_very_long_boolean_setting_label";
        registry.register_extension("fixture",vec![serde_json::from_value(json!({"key":key,"type":"boolean","default":false,"description":"Fixture checkbox","provider":"fixture","version":"1","sensitive":sensitive})).unwrap()],vec![]).unwrap();
        let baseline: Config =
            serde_json::from_value(json!({"version":1,"settings":{key:true}})).unwrap();
        let mut app = app();
        app.settings = Some(settings::Popup::new(Settings::new(
            registry,
            baseline,
            Default::default(),
            None,
        )));
        handle_key(&mut app, KeyCode::Char('s').into());
        handle_key(&mut app, KeyCode::End.into());
        let mut initial = Terminal::new(TestBackend::new(36, 10)).unwrap();
        initial
            .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
            .unwrap();
        handle_key(&mut app, KeyCode::PageDown.into());
        handle_key(&mut app, KeyCode::PageUp.into());
        let mut pages = String::new();
        for _ in 0..10 {
            let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..10)
                .map(|y| (0..36).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            if text.contains("Value:")
                && let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE")
            {
                std::fs::write(
                    std::path::Path::new(&path)
                        .join(format!("settings-longboolean-{sensitive}-36x10.txt")),
                    &text,
                )
                .unwrap();
            }
            pages.push_str(&text);
            handle_key(&mut app, KeyCode::PageDown.into());
        }
        assert!(pages.contains("Key:"));
        if sensitive {
            assert!(
                pages.contains("Default: [masked]") && pages.contains("Value: [masked]"),
                "{pages}"
            );
            assert!(!pages.contains("Value: [x]") && !pages.contains("Default: [ ]"));
        } else {
            assert!(
                pages.contains("Default: [ ]") && pages.contains("Value: [x]"),
                "{pages}"
            );
        }
    }
}

#[test]
fn modal_keys_stay_local_and_search_keeps_text() {
    use crate::settings::{Config, Registry, edit::Settings};
    let mut app = app();
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        Default::default(),
        None,
    )));
    app.inventory = crate::extensions::state::State::default()
        .list(&crate::extensions::default_settings().unwrap(), true);
    select(&mut app, "pid:11");
    let selected = app.selected;
    app.paused = true;
    for modal in ['e', 'd', '?'] {
        handle_key(&mut app, KeyCode::Char(modal).into());
        for details in [false, true] {
            if modal == 'e' && details {
                handle_key(&mut app, KeyCode::Enter.into());
                assert!(app.extension_details);
            }
            for ch in ['s', 'e', 'q'] {
                assert!(!handle_key(&mut app, KeyCode::Char(ch).into()));
                assert!(!app.settings.as_ref().unwrap().open);
            }
            if modal == 'e' && details {
                handle_key(&mut app, KeyCode::Esc.into());
                assert!(app.extensions_page && !app.extension_details);
            }
        }
        handle_key(&mut app, KeyCode::Esc.into());
        assert!(!app.extensions_page && app.details.is_none() && !app.help);
        assert_eq!(app.selected, selected);
        assert!(app.paused && app.manual_refresh.is_none() && !app.collecting);
    }
    handle_key(&mut app, KeyCode::Char('/').into());
    for ch in "seq".chars() {
        assert!(!handle_key(&mut app, KeyCode::Char(ch).into()));
    }
    assert_eq!(app.query, "seq");
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(handle_key(&mut app, KeyCode::Char('q').into()));
    app.help = true;
    assert!(handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
}

#[test]
fn settings_escape_discards_only_active_field_before_popup() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    let mut app = app();
    app.settings = Some(settings::Popup::new(Settings::new(
        Registry::core(),
        Config::default(),
        Default::default(),
        None,
    )));
    for code in [KeyCode::Char('s'), KeyCode::Enter] {
        handle_key(&mut app, code.into());
    }
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    for ch in "3000".chars() {
        handle_key(&mut app, KeyCode::Char(ch).into());
    }
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(&mut app, KeyCode::Down.into());
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, KeyCode::Char('0').into());
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(&mut app, KeyCode::Esc.into());
    let popup = app.settings.as_ref().unwrap();
    assert!(popup.open);
    assert_eq!(popup.changes.len(), 1);
    assert_eq!(popup.changes["refresh.interval_ms"], Some(json!(3000)));
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(!app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().changes.is_empty());
}

#[test]
fn inline_input_keeps_unicode_tail_cursor_visible_and_masks_secrets() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    for sensitive in [false, true] {
        let mut registry = Registry::core();
        registry
            .register_extension(
                "fixture",
                vec![serde_json::from_value(json!({
            "key":"extensions.fixture.long_text", "type":"string", "default":"initial",
            "description":"Text", "provider":"fixture", "version":"1", "sensitive":sensitive
        })).unwrap()],
                vec![],
            )
            .unwrap();
        let mut app = app();
        app.settings = Some(settings::Popup::new(Settings::new(
            registry,
            Config::default(),
            Default::default(),
            None,
        )));
        for code in [KeyCode::Char('s'), KeyCode::End, KeyCode::Enter] {
            handle_key(&mut app, code.into());
        }
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        for ch in "專案內容很長很長很長尾端".chars() {
            handle_key(&mut app, KeyCode::Char(ch).into());
        }
        for (width, height) in [(100, 30), (36, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
                .unwrap();
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(cursor.x < width - 2 && cursor.y < height - 1);
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[cursor].symbol(), "_");
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            let visible = text.replace(' ', "");
            if sensitive {
                assert!(!visible.contains("尾端") && visible.contains("***_"));
            } else {
                assert!(visible.contains("尾端_]"), "{text}");
            }
        }
    }
}

#[test]
fn narrow_inline_input_reserves_tail_before_long_or_unicode_units() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    for (name, unit) in [
        ("core", "ms"),
        ("normal", "ms"),
        ("long", "milliseconds"),
        ("unicode", "秒鐘"),
        ("long-unicode", "毫秒刻度 milliseconds"),
    ] {
        for sensitive in [false, true] {
            if name == "core" && sensitive {
                continue;
            }
            let mut registry = Registry::core();
            if name != "core" {
                registry
                    .register_extension(
                        "fixture",
                        vec![
                            serde_json::from_value(json!({
                                "key":"extensions.fixture.interval", "type":"number", "default":250,
                                "unit":unit, "description":"Fixture interval", "provider":"fixture",
                                "version":"1", "sensitive":sensitive
                            }))
                            .unwrap(),
                        ],
                        vec![],
                    )
                    .unwrap();
            }
            let mut app = app();
            app.settings = Some(settings::Popup::new(Settings::new(
                registry,
                Config::default(),
                Default::default(),
                None,
            )));
            for code in [KeyCode::Char('s'), KeyCode::End, KeyCode::Enter] {
                handle_key(&mut app, code.into());
            }
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            );
            for ch in "6789".chars() {
                handle_key(&mut app, KeyCode::Char(ch).into());
            }
            let mut terminal = Terminal::new(TestBackend::new(37, 10)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
                .unwrap();
            let cursor = terminal.get_cursor_position().unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..10)
                .map(|y| (0..37).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(cursor.x < 35 && cursor.y < 9);
            assert_eq!(buffer[cursor].symbol(), "_");
            if sensitive {
                assert!(text.contains("[****_]"), "{name}: {text}");
                assert!(!text.contains("6789"));
            } else {
                assert!(text.contains("[6789_]"), "{name}: {text}");
            }
            if name == "normal" || name == "core" {
                assert!(text.contains(" ms"), "{text}");
            }
            if name == "unicode" {
                assert!(text.replace(' ', "").contains("秒鐘"), "{text}");
            }
            if let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE") {
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(
                    std::path::Path::new(&path)
                        .join(format!("settings-unit-{name}-masked-{sensitive}-37x10.txt")),
                    &text,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn generic_resource_list_and_details_show_units_without_process_queries() {
    let mut source = snapshot();
    let mut node = Node::group(
        "group:fixturevolume".into(),
        "Data volume".into(),
        vec![],
        Grouping::Category,
    );
    node.kind = "resource";
    node.grouping = None;
    node.resource = Some(crate::model::Resource {
        provider: "fixture".into(),
        version: "1.0.0".into(),
        id: "disk0".into(),
        kind: "storage.volume".into(),
        label: "Data volume".into(),
        path: Some("/Volumes/Data".into()),
        metrics: vec![
            crate::model::ResourceMetric {
                key: "capacity".into(),
                label: "Capacity".into(),
                unit: "bytes".into(),
                value: Some(9007199254740993u64.into()),
            },
            crate::model::ResourceMetric {
                key: "temperature".into(),
                label: "Temperature".into(),
                unit: "vendor.celsius".into(),
                value: None,
            },
        ],
    });
    source.root.children.push(node);
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(source));
    select(&mut app, "group:fixturevolume");
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..30)
        .map(|y| {
            (0..160)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Capacity: 9007199254740993 bytes"), "{text}");
    app.open_details();
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.query_pid(), None);
    assert!(!details.needs_query);
    let lines = details.lines().join("\n");
    assert!(lines.contains("Temperature: ? vendor.celsius"));
    assert!(lines.contains("/Volumes/Data"));
    assert!(lines.contains("fixture/1.0.0"));
}

#[test]
fn repair_empty_resource_provider_keeps_metric_scope_in_the_table() {
    let mut source = snapshot();
    source.root = Node::group(
        "group:resourceprovider".into(),
        "Volume provider".into(),
        vec![],
        Grouping::Resource,
    );
    source.scope = source.root.reference.clone();
    let mut app = App::new(source.scope.clone(), 0);
    app.update(Ok(source));
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..20)
        .map(|y| {
            (0..120)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Volume provider") && text.contains("resources"),
        "{text}"
    );
    let row = text
        .lines()
        .find(|line| line.contains("Volume provider"))
        .unwrap();
    assert!(
        !row.contains("0 proc") && !row.contains("0 B") && !row.contains("0.0%"),
        "{row}"
    );
}

#[test]
fn priority_dialog_reorders_builtins_saves_and_keeps_failures_as_draft() {
    use crate::settings::{Config, edit::Settings};
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let resolved = crate::extensions::default_settings().unwrap();
    let mut app = App::new("group:all".into(), 2);
    app.inventory = crate::extensions::state::State::default().list(&resolved, false);
    app.settings = Some(settings::Popup::new(Settings::new(
        crate::extensions::settings_registry().unwrap(),
        Config::default(),
        Default::default(),
        Some(path.clone()),
    )));
    app.extensions_page = true;
    app.extension_selected = Some("app_bundle".into());
    extensions::key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(
        &app.priority_draft.as_ref().unwrap()[..3],
        ["docker", "app_bundle", "apple"]
    );
    extensions::key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    extensions::key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.priority_draft.is_some());
    extensions::key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(app.extensions_page);
    extensions::key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(!app.extensions_page);
    let saved = Config::load_path(Some(&path), true).unwrap();
    assert_eq!(saved.source_priority[1], "app_bundle");
    assert!(app.settings.as_mut().unwrap().applied.take().is_some());
    app.extensions_page = true;
    extensions::key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    std::fs::write(&path, "malformed").unwrap();
    extensions::key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.extensions_page && app.priority_error.is_some() && app.priority_draft.is_some());
    assert!(app.settings.as_ref().unwrap().applied.is_none());
    for (width, height) in [(100, 30), (36, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| extensions::draw(frame, &app))
            .unwrap();
        if let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE") {
            std::fs::create_dir_all(&path).unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(
                std::path::Path::new(&path)
                    .join(format!("priority-save-error-{width}x{height}.txt")),
                text,
            )
            .unwrap();
        }
    }
    extensions::key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.extensions_page && app.priority_draft.is_none());
    root.close().unwrap();
}

#[test]
fn tag_payload_is_absent_from_tree_and_only_selected_pid_is_rendered_in_details() {
    use crate::extensions::tags::{Effective, Tag};
    let render = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(160, 80)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let mut source = snapshot();
    for pid in [10, 11] {
        change(&mut source.root, pid, |process| {
            let tag: Tag = serde_json::from_value(serde_json::json!({"dimension":"role","id":"role:test","label":format!("private-tag-{pid}"),"path":null,"basis":"observed"})).unwrap();
            process
                .tag_report
                .as_mut()
                .unwrap()
                .effective
                .push(Effective {
                    source: "fixture".into(),
                    tag,
                });
        });
    }
    let mut app = App::new("group:all".into(), 4);
    app.update(Ok(source.clone()));
    let text = render(&app);
    assert!(!text.contains("private-tag"));
    assert!(
        !serde_json::to_string(&app.snapshot.as_ref().unwrap().root)
            .unwrap()
            .contains("tag_report")
    );
    select(&mut app, "pid:10");
    app.open_details();
    app.update(Ok(source));
    let text = render(&app);
    assert!(text.contains("private-tag-10") && !text.contains("private-tag-11"));
}
