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
    assert_eq!(text.matches("q quit").count(), 1);
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
