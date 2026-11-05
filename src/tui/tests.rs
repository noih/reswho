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
fn settings_and_extensions_keep_navigation_in_their_own_context() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    let mut app = app();
    let mut registry = Registry::core();
    registry
        .register_extension(
            "fixture",
            vec![
                serde_json::from_value(json!({
                    "key":"extensions.fixture.choice", "type":"string", "options":["a","b","c"],
                    "default":"a", "description":"Choice", "provider":"fixture", "sensitive":false
                }))
                .unwrap(),
                serde_json::from_value(json!({
                    "key":"extensions.fixture.text", "type":"string", "default":"initial",
                    "description":"Text", "provider":"fixture", "sensitive":false
                }))
                .unwrap(),
            ],
            vec![],
        )
        .unwrap();
    app.settings = Some(settings::Popup::new(Settings::new(
        registry,
        Config::default(),
        Default::default(),
        None,
    )));
    handle_key(&mut app, KeyCode::Char('s').into());
    handle_key(&mut app, KeyCode::End.into());
    handle_key(&mut app, KeyCode::Up.into());
    handle_key(&mut app, KeyCode::Enter.into());
    assert_eq!(
        app.settings.as_ref().unwrap().changes["extensions.fixture.choice"],
        Some(json!("b"))
    );
    handle_key(&mut app, KeyCode::Left.into());
    assert_eq!(
        app.settings.as_ref().unwrap().changes["extensions.fixture.choice"],
        Some(json!("b"))
    );
    handle_key(&mut app, KeyCode::End.into());
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    for key in [
        KeyCode::Char('h'),
        KeyCode::Left,
        KeyCode::Char('l'),
        KeyCode::Right,
    ] {
        handle_key(&mut app, key.into());
        assert!(app.settings.as_ref().unwrap().open);
    }
    handle_key(&mut app, KeyCode::Enter.into());
    assert_eq!(
        app.settings.as_ref().unwrap().changes["extensions.fixture.text"],
        Some(json!("hl"))
    );
    assert!(app.settings.as_ref().unwrap().applied.is_none());
    assert!(app.details.is_none() && !app.extensions_page);
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Esc.into());
    app.inventory =
        crate::extensions::inventory::list(&crate::extensions::default_settings().unwrap(), true);
    for (right, left) in [
        (KeyCode::Right, KeyCode::Left),
        (KeyCode::Char('l'), KeyCode::Char('h')),
        (KeyCode::Char('d'), KeyCode::Esc),
    ] {
        handle_key(&mut app, KeyCode::Char('e').into());
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        let draft = app.priority_draft.clone();
        let selected = app.extension_selected.clone();
        app.extensions_scroll = 2;
        handle_key(&mut app, right.into());
        assert!(app.extension_details);
        handle_key(&mut app, left.into());
        assert!(app.extensions_page && !app.extension_details);
        assert_eq!(app.priority_draft, draft);
        assert_eq!(app.extension_selected, selected);
        assert_eq!(app.extensions_scroll, 2);
        handle_key(&mut app, left.into());
        if left != KeyCode::Esc {
            assert!(app.extensions_page);
            assert_eq!(app.priority_draft, draft);
            assert_eq!(app.extension_selected, selected);
            assert_eq!(app.extensions_scroll, 2);
            handle_key(&mut app, KeyCode::Esc.into());
        }
        assert!(!app.extensions_page);
        assert!(app.priority_draft.is_none() && app.manual_refresh.is_none());
    }
}

#[test]
fn extensions_root_back_keys_preserve_both_tabs_and_all_drafts() {
    for mode in [0, 1] {
        let mut app = app();
        app.inventory = crate::extensions::inventory::list(
            &crate::extensions::default_settings().unwrap(),
            true,
        );
        handle_key(&mut app, KeyCode::Char('e').into());
        if mode == 1 {
            handle_key(&mut app, KeyCode::Tab.into());
        }
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        app.builtin_draft.insert("node".into(), false);
        app.builtin_draft.insert("fixture".into(), false);
        app.extensions_scroll = 2;
        let source = app.priority_draft.clone();
        let ownership = app.ownership_draft.clone();
        let builtin = app.builtin_draft.clone();
        let toggles = app.builtin_draft.clone();
        let selected = app.extension_selected.clone();
        let main_selected = app.selected;
        let expanded = app.expanded.clone();
        for back in [KeyCode::Left, KeyCode::Char('h'), KeyCode::Esc] {
            handle_key(&mut app, KeyCode::Right.into());
            assert!(app.extension_details);
            handle_key(&mut app, back.into());
            assert!(app.extensions_page && !app.extension_details);
            for code in [KeyCode::Left, KeyCode::Char('h'), KeyCode::Enter] {
                assert!(!handle_key(&mut app, code.into()));
                assert!(app.extensions_page && !app.extension_details);
                assert_eq!(app.priority_mode, mode);
                assert_eq!(app.priority_draft, source);
                assert_eq!(app.ownership_draft, ownership);
                assert_eq!(app.builtin_draft, builtin);
                assert_eq!(app.builtin_draft, toggles);
                assert_eq!(app.extension_selected, selected);
                assert_eq!(app.extensions_scroll, 2);
                assert_eq!(app.selected, main_selected);
                assert_eq!(app.expanded, expanded);
                assert!(app.manual_refresh.is_none());
            }
        }
        handle_key(&mut app, KeyCode::Esc.into());
        assert!(!app.extensions_page);
        assert!(app.priority_draft.is_none() && app.ownership_draft.is_none());
        assert!(app.builtin_draft.is_empty());
    }
}

#[test]
fn hierarchy_keys_open_leaves_and_restore_the_tree() {
    for (right, left) in [
        (KeyCode::Right, KeyCode::Left),
        (KeyCode::Char('l'), KeyCode::Char('h')),
    ] {
        let mut app = app();
        select(&mut app, "pid:11");
        let selected = app.selected;
        app.path_scroll = 16;
        let mut table = TableState::default().with_offset(1);
        handle_key(&mut app, right.into());
        assert!(app.details.is_some());
        handle_key(&mut app, right.into());
        assert!(app.details.is_some());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut table))
            .unwrap();
        assert_eq!(table.offset(), 1);
        handle_key(&mut app, left.into());
        assert!(app.details.is_none());
        assert_eq!(app.selected, selected);
        assert_eq!(table.offset(), 1);
        assert_eq!(app.path_scroll, 16);
        handle_key(&mut app, KeyCode::Char(']').into());
        assert_eq!(app.path_scroll, 32);
        handle_key(&mut app, KeyCode::Char('[').into());
        assert_eq!(app.path_scroll, 16);
        handle_key(&mut app, left.into());
        let parent = app.selected;
        assert!(app.rows[parent].has_children);
        handle_key(&mut app, left.into());
        assert!(!app.is_open(&app.rows[parent]));
        handle_key(&mut app, right.into());
        assert_eq!(app.selected, parent);
        assert!(app.is_open(&app.rows[parent]));
        handle_key(&mut app, right.into());
        assert_eq!(app.selected, parent + 1);
        assert!(app.details.is_none());
    }
    let mut source = snapshot();
    source.root = tree::select(&source.root, "pid:11").unwrap();
    source
        .root
        .children
        .push(tree::select(&snapshot().root, "pid:10").unwrap());
    let mut app = App::new("pid:11".into(), usize::MAX);
    app.update(Ok(source));
    handle_key(&mut app, KeyCode::Right.into());
    assert_eq!(app.node(app.selected).unwrap().reference, "pid:10");
    assert!(app.details.is_none());
    handle_key(&mut app, KeyCode::Left.into());
    assert_eq!(app.node(app.selected).unwrap().reference, "pid:11");
    handle_key(&mut app, KeyCode::Char('d').into());
    assert_eq!(app.details.as_ref().unwrap().query_pid(), Some(11));
    handle_key(&mut app, KeyCode::Left.into());
    handle_key(&mut app, KeyCode::Left.into());
    handle_key(&mut app, KeyCode::Left.into());
    assert_eq!(app.selected, 0);
    assert!(app.details.is_none());
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
    assert!(render(&app).contains("Refresh once; keep pause"));
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
    assert!(text.contains("Esc Back"));
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
    assert!(text.contains("Esc Back"));
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
            assert!(text.contains("Esc Back"));
        }
    }
}

#[test]
fn runtime_failure_reason_and_hint_show_in_status_bar() {
    let mut source = snapshot();
    let runtime = &mut source.root.children.last_mut().unwrap().children[0];
    runtime.runtime = Some(Box::new(crate::containers::Runtime {
        metrics_scope: None,
        host_scope: None,
        configuration: None,
        name: "docker".into(),
        context: Some("test".into()),
        endpoint_ref: None,
        daemon_id: None,
        status: "unavailable".into(),
        inventory_complete: false,
        collection_ms: 1,
        unavailable: vec![crate::model::Diagnostic::failure(
            "collection_failed",
            "docker",
            "info failed",
            crate::model::FailureReason::ConnectionRefused,
            None,
        )],
    }));
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(source));
    select(&mut app, "group:runtime");
    let mut terminal = Terminal::new(TestBackend::new(200, 24)).unwrap();
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
    assert!(
        text.contains(
            "info failed · reason connection_refused: endpoint is not accepting connections"
        ),
        "{text}"
    );
}

fn screen(app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, app, &mut TableState::default()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn versions_show_on_consistent_bin_rows_named_rows_details_and_search() {
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(crate::tree::versions::fixtures::snapshot("group:all")));
    for (query, expected, absent) in [
        ("24.18", "node 24.18.0", None),
        ("24.22.1", "node 24.22.1", None),
        ("1.0.0", "run pid:50", Some("run 1.0.0")),
        ("2.1.316", "claude 2.1.316 pid:20", None),
        ("claude", "claude 2.1.316 pid:20", None),
        (
            "1.99.1",
            "rust-analyzer-proc-macro-srv pid:30",
            Some("srv 1.99.1"),
        ),
    ] {
        app.query = query.into();
        app.rebuild();
        let text = screen(&app, 160);
        assert!(text.contains(expected), "{query}: {text}");
        if let Some(absent) = absent {
            assert!(!text.contains(absent), "{query}: {text}");
        }
    }
    app.query.clear();
    app.rebuild();
    select(&mut app, "pid:20");
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    let text = screen(&app, 160);
    assert!(
        text.contains("2.1.316 · subject claude · from versioned_install (executable)"),
        "{text}"
    );
    assert!(
        text.contains("2.1.316 · display name from versioned_install (executable)"),
        "{text}"
    );
}

#[test]
fn details_label_name_and_version_tags_and_show_rejected_subjects() {
    for (pid, tags, trace) in [
        (
            20,
            "Tags  #executable · name claude · version claude 2.1.316",
            "add version 2.1.316 · subject claude",
        ),
        (30, "Tags  #executable", "add version 1.99.1 · subject rust"),
    ] {
        let mut source = crate::tree::versions::fixtures::snapshot("group:all");
        change(&mut source.root, pid, |process| {
            process.command = Some(Default::default());
        });
        let mut app = App::new("group:all".into(), usize::MAX);
        app.update(Ok(source.clone()));
        select(&mut app, &format!("pid:{pid}"));
        handle_key(&mut app, KeyCode::Char('d').into());
        app.update(Ok(source));
        let lines = app.details.as_ref().unwrap().lines();
        assert!(lines.iter().any(|line| line.trim() == tags), "{lines:?}");
        let text = lines.join("\n");
        assert!(text.contains(trace), "{text}");
        if pid == 30 {
            assert!(
                text.contains("rejected: version subject mismatch"),
                "{text}"
            );
        }
    }
}

#[test]
fn work_rows_show_work_labels_and_work_groups() {
    let (temp, root) = crate::extensions::builtin::node::tests::work_tree();
    let mut snapshot = crate::tree::versions::fixtures::snapshot("group:all");
    snapshot.root = root;
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(snapshot));
    let text = screen(&app, 160);
    let group = text
        .lines()
        .find(|line| line.contains("v firecrawl-mcp "))
        .unwrap();
    assert!(group.contains("2 proc"), "{text}");
    assert!(text.contains("codegraph pid:15"), "{text}");
    assert!(text.contains("chrome-devtools-mcp pid:12"), "{text}");
    assert!(!text.contains("firecrawl-mcp 22.1.0"), "{text}");
    assert!(!text.contains("index.js"), "{text}");
    temp.close().unwrap();
}

#[test]
fn python_work_rows_show_script_and_module_labels() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("sync_mail.py"), "").unwrap();
    let user = unsafe { libc::geteuid() };
    let python = |pid: u32, argv: &[&str]| Process {
        pid,
        ppid: Some(1),
        uid: Some(user),
        started_unix_us: Some(u64::from(pid)),
        name: "python3.12".into(),
        executable: Some(root.join("bin/python3.12")),
        cwd: Some(root.clone()),
        resident_bytes: Some(1),
        command: Some(crate::model::ProcessCommand {
            argv: Some(argv.iter().map(|a| (*a).to_owned()).collect()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut snapshot = crate::tree::versions::fixtures::snapshot("group:all");
    snapshot.root = crate::tree::build(vec![
        python(
            70,
            &["python3", "-u", "sync_mail.py", "--password", "hunter2"],
        ),
        python(71, &["python3", "-m", "mcp_server_fetch"]),
        python(72, &["python3", "-c", "pass"]),
    ])
    .unwrap();
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(snapshot));
    let text = screen(&app, 160);
    for expected in [
        "sync_mail pid:70",
        "mcp_server_fetch pid:71",
        "python3.12 pid:72",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert!(!text.contains("hunter2"));
    temp.close().unwrap();
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
fn all_container_states_remain_visible_and_unreadable_is_one_leaf() {
    let mut app = app();
    let before = serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap();
    let rows: Vec<_> = app.rows.iter().map(|row| row.key.clone()).collect();
    select(&mut app, "group:stopped");
    let selected = app.selected;
    let expanded = app.expanded.clone();
    handle_key(&mut app, KeyCode::Char('a').into());
    assert_eq!(
        app.rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>(),
        rows
    );
    assert_eq!(app.selected, selected);
    assert_eq!(app.expanded, expanded);
    assert!(app.rows.iter().any(|row| row.key == "group:live"));
    assert!(app.rows.iter().any(|row| row.key == "group:unknown"));
    assert!(app.rows.iter().any(|row| row.key == "group:stopped"));
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
    source.project(&[]);
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
    assert!(app.rows.iter().any(|row| row.key == "group:stopped"));
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
            assert!(!text.contains("Process details"));
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
fn table_headers_mark_only_the_active_sort_in_ready_and_loading_views() {
    for width in [100, 80, 36] {
        for (sort, heading) in [
            (Sort::Name, "Resource ↑"),
            (Sort::Cpu, "CPU/core ↓"),
            (Sort::Memory, "Memory ↓"),
            (Sort::Uptime, "Age ↓"),
        ] {
            for loading in [false, true] {
                let mut app = App::new("group:all".into(), 2);
                app.sort = sort;
                if loading {
                    app.collecting = true;
                } else {
                    app.update(Ok(snapshot()));
                }
                let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
                terminal
                    .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
                    .unwrap();
                let lines: Vec<String> = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                    .collect();
                println!(
                    "FRAME {width}x30 {sort:?} loading={loading}\n{}",
                    lines.join("\n")
                );
                assert_eq!(lines[1].trim_end(), "/ search");
                assert!(!lines.join("\n").contains("Sort "));
                assert!(lines[3].contains(heading), "{}", lines[3]);
                assert_eq!(lines[3].matches(['↑', '↓']).count(), 1);
                assert_eq!(lines[3].contains("State / Ports"), width >= 85);
                assert_eq!(lines.join("\n").contains("Loading resources..."), loading);
            }
        }
    }
}

#[test]
fn resource_table_keeps_gutters_and_disclosure_at_boundary_sizes() {
    use ratatui::style::{Color, Modifier};
    for (width, height) in [
        (36, 10),
        (46, 12),
        (50, 12),
        (51, 12),
        (88, 16),
        (89, 16),
        (100, 24),
    ] {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for (collapsed, query) in [(false, ""), (true, ""), (false, "pid:11")] {
            if collapsed {
                app.expanded.clear();
                app.expanded.insert(app.rows[0].key.clone(), false);
                app.rebuild();
            }
            app.query = query.into();
            if !query.is_empty() {
                app.rebuild();
            }
            terminal
                .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
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
            assert_eq!(buffer[(0, 2)].symbol(), "┌", "{text}");
            assert_eq!(buffer[(width - 1, 2)].symbol(), "┐", "{text}");
            for x in 1..width - 1 {
                assert_eq!(buffer[(x, 2)].symbol(), "─", "{text}");
            }
            assert_eq!(buffer[(2, 3)].symbol(), "R", "{text}");
            let border_y = (3..height)
                .find(|&y| buffer[(0, y)].symbol() == "└")
                .unwrap();
            for y in 3..border_y {
                for x in [1, width - 2] {
                    assert_eq!(buffer[(x, y)].symbol(), " ", "{text}");
                    assert_eq!(buffer[(x, y)].bg, Color::Reset, "{text}");
                }
                assert_eq!(buffer[(0, y)].symbol(), "│", "{text}");
                assert_eq!(buffer[(width - 1, y)].symbol(), "│", "{text}");
            }
            let marker = if collapsed { ">" } else { "v" };
            assert_eq!(buffer[(2, 4)].symbol(), marker, "{text}");
            assert_eq!(buffer[(3, 4)].symbol(), " ", "{text}");
            assert_eq!(buffer[(2, 4)].bg, Color::DarkGray);
            assert!(buffer[(2, 4)].modifier.contains(Modifier::BOLD));
            assert!(!text.contains('»'));
            if !collapsed {
                for (index, row) in app
                    .rows
                    .iter()
                    .take(usize::from(border_y.saturating_sub(4)))
                    .enumerate()
                {
                    let x = 2 + 2 * row.depth.min(12) as u16;
                    if row.has_children {
                        assert_eq!(buffer[(x, 4 + index as u16)].symbol(), "v", "{text}");
                    }
                }
            }
            if width >= 89 {
                assert!(text.contains("State / Ports"), "{text}");
            }
            if !collapsed
                && [36, 100].contains(&width)
                && let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE")
            {
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(
                    std::path::Path::new(&path)
                        .join(format!("pl1-{width}x{height}-{collapsed}.txt")),
                    &text,
                )
                .unwrap();
            }
        }
    }
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
        let buffer = terminal.backend().buffer();
        for y in 3..20 {
            for x in [1, 78] {
                assert_eq!(buffer[(x, y)].symbol(), " ");
                assert_eq!(buffer[(x, y)].bg, ratatui::style::Color::Reset);
            }
        }
        if let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE") {
            let state = if app.snapshot.is_some() {
                if app.rows.is_empty() {
                    "empty"
                } else {
                    "ready"
                }
            } else if app.error.is_some() {
                "error"
            } else {
                "loading"
            };
            let text = (0..24)
                .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                std::path::Path::new(&path).join(format!("pl1-{state}-80x24.txt")),
                text,
            )
            .unwrap();
        }
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
    assert_eq!(loading_column, Some(2));
    assert!(first.contains("| Loading resources..."));
    assert!(first.contains("Resource"));
    assert!(first.contains("Memory"));
    assert!(first.contains("Memory ↓"));
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
    assert!(toolbar.starts_with("/ search"));
    assert!(loaded.contains("Resource"));
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
fn apple_runtime_renders_all_containers_after_a() {
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
    let mut stopped = runtime.children[1].clone();
    stopped.reference = "group:stopped-only-child".into();
    stopped.name = "stopped-only-child".into();
    let mut totals = ContainerTotals {
        inventory_complete: true,
        ..ContainerTotals::default()
    };
    totals.add(stopped.container.as_ref().unwrap());
    let mut branch = Node::group(
        "group:stopped-only".into(),
        "stopped-only".into(),
        vec![],
        Grouping::Runtime,
    );
    branch.children.push(stopped);
    branch.container_totals = Some(totals);

    containers
        .container_totals
        .as_mut()
        .unwrap()
        .add(branch.children[0].container.as_ref().unwrap());
    containers.children.push(branch);
    let mut app = App::new("group:containers".into(), usize::MAX);
    app.update(Ok(snapshot));
    handle_key(&mut app, KeyCode::Char('a').into());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
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
    for name in ["stopped", "live", "unknown", "stopped-only-child"] {
        assert!(text.contains(name), "{text}");
    }
    assert!(!text.contains("Containers all") && !text.contains("Containers active"));
    assert!(text.contains("/ search"));
    if let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE") {
        std::fs::create_dir_all(&path).unwrap();
        let lines = terminal
            .backend()
            .buffer()
            .content
            .chunks(100)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            std::path::Path::new(&path).join("main-all-containers-100x30.txt"),
            lines,
        )
        .unwrap();
    }
    select(&mut app, "group:stopped-only");
    handle_key(&mut app, KeyCode::Left.into());
    assert!(
        !app.rows
            .iter()
            .any(|row| row.key == "group:stopped-only-child")
    );
    handle_key(&mut app, KeyCode::Right.into());
    assert!(
        app.rows
            .iter()
            .any(|row| row.key == "group:stopped-only-child")
    );
    let before = serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap();
    app.query = "stopped-only-child".into();
    app.rebuild();
    assert!(app.rows.iter().any(|row| row.key == "group:stopped-only"));
    assert!(!app.rows.iter().any(|row| row.key == "group:live"));
    assert_eq!(
        serde_json::to_value(&app.snapshot.as_ref().unwrap().root).unwrap(),
        before
    );
    app.query.clear();
    app.rebuild();
    assert!(app.rows.iter().any(|row| row.key == "group:stopped"));
    handle_key(&mut app, KeyCode::Char('a').into());
    assert!(app.rows.iter().any(|row| row.key == "group:stopped"));
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
fn extension_page_preserves_navigation_never_requests_collection_and_keeps_failed_report() {
    let mut app = app();
    select(&mut app, "pid:11");
    let selected = app.selected;
    app.paused = true;
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        "read",
        "failed",
        Some("invalid_data"),
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
    assert!(text.contains("invalid_data"));
    assert_eq!(table.offset(), offset);
    handle_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    handle_key(&mut app, KeyEvent::from(KeyCode::Esc));
    assert_eq!(app.selected, selected);
    assert_eq!(table.offset(), offset);
    assert!(app.manual_refresh.is_none());
    assert!(!app.collecting);
    app.update(Err("synthetic worker failure".into()));
    assert_eq!(app.extensions.len(), 1);
    assert_eq!(app.extensions[0].reason.as_deref(), Some("invalid_data"));
}

#[test]
fn settings_left_never_starts_editing_or_changes_a_draft() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    for (value_type, default, options) in [
        ("integer", json!(42), None),
        ("number", json!(1.25), None),
        ("boolean", json!(false), None),
        ("string", json!("initial"), None),
        ("string", json!("a"), Some(json!(["a", "b", "c"]))),
        ("number", json!(1.25), Some(json!([1.25, 2.5, 3.75]))),
    ] {
        for (width, height) in [(100, 30), (36, 10)] {
            let mut registry = Registry::core();
            let mut definition = json!({
                "key":"extensions.fixture.value", "type":if value_type == "integer" { "number" } else { value_type }, "default":default,
                "description":"Value", "provider":"fixture", "sensitive":false
            });
            if value_type == "integer" {
                definition["integer"] = json!(true);
            }
            if let Some(options) = &options {
                definition["options"] = options.clone();
            }
            registry
                .register_extension(
                    "fixture",
                    vec![serde_json::from_value(definition).unwrap()],
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
            for code in [KeyCode::Char('s'), KeyCode::End] {
                assert!(!handle_key(&mut app, code.into()));
            }
            let render = |app: &App| {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| view::draw(frame, app, &mut TableState::default()))
                    .unwrap();
                terminal.backend().buffer().clone()
            };
            let before = render(&app);
            handle_key(&mut app, KeyCode::Left.into());
            assert_eq!(
                render(&app),
                before,
                "passive Left must do nothing: {value_type}"
            );
            assert!(app.settings.as_ref().unwrap().changes.is_empty());
            handle_key(&mut app, KeyCode::Enter.into());
            let editing = render(&app);
            assert_ne!(editing, before, "Enter must edit or toggle");
            let input = options.is_none() && value_type != "boolean";
            if input {
                for code in [KeyCode::Left, KeyCode::Right] {
                    handle_key(&mut app, code.into());
                    assert_eq!(render(&app), editing, "arrows must preserve active input");
                }
                handle_key(&mut app, KeyCode::Enter.into());
            }
            let draft = app.settings.as_ref().unwrap().changes.clone();
            assert!(!draft.is_empty());
            let list = render(&app);
            for back in [KeyCode::Left, KeyCode::Esc] {
                handle_key(&mut app, KeyCode::Right.into());
                assert_ne!(render(&app), list);
                handle_key(&mut app, back.into());
                assert_eq!(render(&app), list);
                handle_key(&mut app, KeyCode::Left.into());
                assert_eq!(render(&app), list);
                assert_eq!(app.settings.as_ref().unwrap().changes, draft);
                assert!(app.settings.as_ref().unwrap().open);
            }
            handle_key(&mut app, KeyCode::Esc.into());
            assert!(!app.settings.as_ref().unwrap().open);
            assert!(app.settings.as_ref().unwrap().changes.is_empty());
        }
    }
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
        json!({"key":"extensions.fixture.flag","type":"boolean","default":false,"description":"A flag","provider":"fixture","sensitive":false}),
        json!({"key":"extensions.fixture.number","type":"number","options":[-1.25,2.5],"default":-1.25,"description":"A choice","provider":"fixture","sensitive":false}),
        json!({"key":"extensions.fixture.secret","type":"string","options":["secret-option","another-secret"],"default":"secret-option","description":"Sensitive choice","provider":"fixture","sensitive":true}),
        json!({"key":"extensions.fixture.text","type":"string","default":"initial","description":format!("{}Description end", "Long description ".repeat(50)),"provider":"fixture","sensitive":false}),
    ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect();
    registry
        .register_extension("fixture", definitions, vec![])
        .unwrap();
    let baseline: Config = serde_json::from_value(
        json!({"version":1,"settings":{"extensions.fixture.flag":"invalid"}}),
    )
    .unwrap();
    std::fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    assert_eq!(
        registry
            .resolve(&baseline, &BTreeMap::new())
            .unwrap()
            .get("extensions.fixture.flag"),
        Some(&json!(false))
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
    handle_key(&mut app, key(KeyCode::Down));
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
    handle_key(&mut app, key(KeyCode::Right));
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
    handle_key(&mut app, key(KeyCode::Left));
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
    handle_key(&mut app, key(KeyCode::Down));
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
        "Sampling inter",
        "Time between",
        "[250] ms",
    ] {
        assert!(initial.contains(label), "missing {label}: {initial}");
    }
    handle_key(&mut app, KeyCode::Right.into());
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
    handle_key(&mut app, KeyCode::Left.into());
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
fn settings_children_indent_without_moving_values() {
    use crate::settings::{Config, Registry, edit::Settings};
    use ratatui::style::{Color, Modifier};
    let mut registry = Registry::core();
    for provider in ["node", "python", "zfixture"] {
        registry
            .register_extension(
                provider,
                vec![serde_json::from_value(serde_json::json!({
            "key": format!("extensions.{provider}.entry_argv"), "type":"boolean",
            "default":true, "description":"Entry", "provider":provider, "sensitive":false
        })).unwrap()],
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
    handle_key(&mut app, KeyCode::Char('s').into());
    for index in 0..6 {
        for (width, height) in [(100, 30), (60, 30), (36, 10)] {
            for changed in [false, true] {
                let popup = app.settings.as_mut().unwrap();
                popup.changes = if changed {
                    popup
                        .context
                        .registry
                        .definitions()
                        .map(|d| (d.key.clone(), Some(d.default.clone())))
                        .collect()
                } else {
                    Default::default()
                };
                for tab in [KeyCode::Tab, KeyCode::BackTab] {
                    handle_key(&mut app, tab.into());
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| app.settings.as_ref().unwrap().draw(frame))
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
                    let group_x = (width - width.min(88)) / 2 + 2;
                    let value_x = group_x + if width == 36 { 17 } else { 33 };
                    let selected_y = (0..height)
                        .find(|&y| buffer[(group_x, y)].bg == Color::DarkGray)
                        .unwrap();
                    assert_eq!(
                        buffer[(group_x, selected_y)].symbol(),
                        if changed { "*" } else { " " },
                        "{text}"
                    );
                    assert_eq!(buffer[(group_x + 1, selected_y)].symbol(), " ", "{text}");
                    assert_ne!(buffer[(group_x + 2, selected_y)].symbol(), " ", "{text}");
                    assert_eq!(buffer[(value_x, selected_y)].symbol(), "[", "{text}");
                    assert_ne!(buffer[(group_x - 1, selected_y)].bg, Color::DarkGray);
                    assert_eq!(buffer[(value_x, selected_y)].bg, Color::DarkGray);
                    assert!(
                        !buffer[(group_x + 1, selected_y)]
                            .modifier
                            .contains(Modifier::DIM)
                    );
                    assert!(!text.contains('»'));
                    if width >= 60 && index == 0 {
                        for group in ["General", "node", "python", "zfixture"] {
                            let y =
                                text.lines().position(|line| line.contains(group)).unwrap() as u16;
                            assert_eq!(buffer[(group_x, y)].symbol(), &group[..1], "{text}");
                        }
                    }
                    if [0, 5].contains(&index)
                        && [36, 100].contains(&width)
                        && let Some(path) = std::env::var_os("RESWHO_UI_EVIDENCE")
                    {
                        std::fs::create_dir_all(&path).unwrap();
                        std::fs::write(
                            std::path::Path::new(&path)
                                .join(format!("pl2-{width}x{height}-{index}.txt")),
                            &text,
                        )
                        .unwrap();
                    }
                }
            }
        }
        handle_key(&mut app, KeyCode::Down.into());
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
    let left = rows[title].chars().position(|ch| ch == '┌').unwrap();
    let right = rows[title].chars().rev().position(|ch| ch == '┐').unwrap();
    assert_eq!(left, right);
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
    app.inventory = crate::extensions::inventory::list(&resolved, true);
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        "read",
        "failed",
        Some("invalid_data"),
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
            for hint in ["→ Open", "/ Search", "s Settings", "e Extensions"] {
                assert!(text.contains(hint), "missing {hint}: {text}");
            }
        }
    }
    app.help = true;
    let text = dump(&app, 100, 30, "help");
    assert!(text.contains("[Controls]") && text.contains("Navigation"));
    assert!(!text.contains("reswho controls"));
    app.help = false;
    select(&mut app, "pid:11");
    app.open_details();
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "process-details");
        assert!(text.contains("Esc Back") && text.contains("p pause"));
        assert!(!text.contains("j/k") && !text.contains("scroll"));
    }
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('s').into());
    handle_key(&mut app, KeyCode::Down.into());
    for (width, height) in [(160, 45), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "settings");
        assert!(!text.contains("[Save]") && !text.contains("[Cancel]"));
        assert!(!text.contains("Tab Controls"));
        assert!(text.contains("Enter Edit · → Details"));
        assert!(
            text.contains("Ctrl-S Save")
                && text.contains("Ctrl-R Reset")
                && text.contains("Esc discard")
        );
        assert!(!text.contains("Move"));
        let last = text
            .lines()
            .position(|line| line.contains("Esc discard"))
            .unwrap();
        assert!(text.lines().nth(last + 1).unwrap().contains('└'), "{text}");
        if width >= 100 {
            assert_eq!(text.matches("General").count(), 1);
            let hints = text
                .lines()
                .find(|line| line.contains("Enter Edit"))
                .unwrap();
            assert!(hints.contains("Esc discard"), "footer should fit: {text}");
            let hint_row = text
                .lines()
                .position(|line| line.contains("Enter Edit"))
                .unwrap();
            assert!(text.lines().nth(hint_row + 1).unwrap().contains('└'));
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
        assert_eq!(row(text, description) + 1, row(text, "Enter Edit"));
        assert_eq!(text.matches(description).count(), 1);
    }
    dump(&app, 36, 10, "settings-second");
    handle_key(&mut app, KeyCode::Home.into());
    handle_key(&mut app, KeyCode::Down.into());
    handle_key(&mut app, KeyCode::Enter.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    handle_key(&mut app, KeyCode::Char('0').into());
    for (width, height) in [(160, 45), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "settings-edit");
        assert!(
            text.contains("[0_]")
                && text.contains("Enter Accept")
                && text.contains("Esc Undo field")
        );
        assert!(
            !text.contains("Edit:") && !text.contains("Default:") && !text.contains("Constraints:")
        );
        let last = text
            .lines()
            .position(|line| line.contains("Ctrl-R Reset"))
            .unwrap();
        assert!(text.lines().nth(last + 1).unwrap().contains('└'), "{text}");
        if width >= 100 {
            let hints = text
                .lines()
                .find(|line| line.contains("Enter Accept"))
                .unwrap();
            assert!(hints.contains("Ctrl-R Reset"), "footer should fit: {text}");
        }
        if width == 100 {
            assert_eq!(
                row(&first, "Refresh interval"),
                row(&text, "Refresh interval")
            );
        }
    }
    handle_key(&mut app, KeyCode::Enter.into());
    for (width, height) in [(160, 45), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "settings-invalid");
        for label in ["Invalid value", "Enter Accept", "Ctrl-S Save"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        assert!(!text.contains("PgDn Info"));
    }
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(app.settings.as_ref().unwrap().open);
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('s').into());
    for code in [KeyCode::Tab, KeyCode::BackTab] {
        let before = dump(&app, 100, 30, "settings-before-tab");
        handle_key(&mut app, code.into());
        assert_eq!(dump(&app, 100, 30, "settings-after-tab"), before);
    }
    handle_key(&mut app, KeyCode::Enter.into());
    assert!(app.settings.as_ref().unwrap().open);
    assert!(dump(&app, 100, 30, "settings-edit-after-tab").contains("Enter Accept"));
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('e').into());
    handle_key(&mut app, KeyCode::Char('j').into());
    let selected = app.extension_selected.clone();
    app.inventory.reverse();
    app.update(Ok(snapshot()));
    assert_eq!(app.extension_selected, selected);
    app.extensions = vec![crate::extensions::report::Record::new(
        "broken",
        "read",
        "failed",
        Some("invalid_data"),
        None,
    )];
    for (id, phase, result, duration) in [
        ("docker", "collect", "success", 561.0),
        ("app_bundle", "classify", "success", 1.2),
        ("command", "details", "failed", 8.4),
    ] {
        let mut record = crate::extensions::report::Record::new(
            id,
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
    app.extension_selected = Some("docker".into());
    for (width, height) in [(160, 45), (160, 10), (100, 30), (36, 10)] {
        let text = dump(&app, width, height, "extensions");
        assert!(
            (width == 36 || text.contains("System diagnostics")) && text.contains("invalid_data"),
            "{width}x{height}: {text}"
        );
        assert!(text.contains("ID") && text.contains("State") && !text.contains("Type"));
        assert!(text.contains("Ctrl-S Save") && text.contains("Esc Cancel"));
        for line in text.lines() {
            assert!(!line.trim_end_matches([' ', '│']).ends_with('·'), "{text}");
        }
        assert!(
            text.lines()
                .any(|line| line.contains("docker") && line.contains("Enabled")),
            "selected row must remain visible: {text}"
        );
        assert!(
            !text.contains("s Settings")
                && !text.contains("q Quit")
                && !text.contains("Enter for details")
        );
        assert!(text.contains("Space Disable") && text.contains("Save") && !text.contains("j/k"));
        let hints = ["Space Disable", "Esc Cancel", "Ctrl-S Save"];
        for hint in hints {
            assert!(text.contains(hint), "missing {hint}: {text}");
        }
        if width >= 160 {
            for column in ["Collect", "Classify", "Details"] {
                assert!(text.contains(column));
            }
        }
    }
    app.extension_selected = Some("python".into());
    handle_key(&mut app, KeyCode::Char('d').into());
    let text = dump(&app, 100, 30, "extension-details");
    assert!(text.contains("Availability: unknown") && text.contains("No invocation timing"));
    for (width, height) in [(100, 30), (36, 10)] {
        let text = dump(&app, width, height, "extension-details");
        assert_eq!(text.matches("↑↓ Scroll").count(), 1);
        assert!(!text.contains("j/k") && !text.contains("Pg"));
        assert_eq!(text.matches("←/Esc Back").count(), 1, "{text}");
        assert!(text.contains("Python"));
    }
    handle_key(&mut app, KeyCode::Esc.into());
    app.inventory[0].explicit_enabled = false;
    app.settings
        .as_mut()
        .unwrap()
        .context
        .baseline
        .disabled_builtins
        .push(app.inventory[0].descriptor.id.clone());
    app.extension_selected = Some(app.inventory[0].descriptor.id.clone());
    assert!(dump(&app, 100, 30, "extensions-disabled").contains("Disabled"));
}

#[test]
fn invalid_setting_input_ignores_tabs_and_keeps_shortcuts_reachable() {
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
    send(&mut app, KeyCode::Down);
    send(&mut app, KeyCode::Enter);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    send(&mut app, KeyCode::Char('0'));
    send(&mut app, KeyCode::Enter);
    for code in [KeyCode::Tab, KeyCode::BackTab] {
        send(&mut app, code);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        );
        assert!(app.settings.as_ref().unwrap().open);
        assert!(app.settings.as_ref().unwrap().applied.is_none());
        let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
        terminal
            .draw(|frame| app.settings.as_ref().unwrap().draw(frame))
            .unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[cursor].symbol(), "_");
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("[0_]") && text.contains("Invalid value"),
            "{text}"
        );
    }
    send(&mut app, KeyCode::Char(' '));
    let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
    terminal
        .draw(|frame| app.settings.as_ref().unwrap().draw(frame))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("[0 _]"), "{text}");
    send(&mut app, KeyCode::Enter);
    assert!(app.settings.as_ref().unwrap().open);
    send(&mut app, KeyCode::Esc);
    assert!(app.settings.as_ref().unwrap().open);
    send(&mut app, KeyCode::Tab);
    send(&mut app, KeyCode::BackTab);
    send(&mut app, KeyCode::Esc);
    assert!(!app.settings.as_ref().unwrap().open);
    assert!(app.settings.as_ref().unwrap().changes.is_empty());
    assert!(app.settings.as_ref().unwrap().applied.is_none());
}

#[test]
fn compact_explicit_info_exposes_current_long_label_boolean_and_masks_sensitive_value() {
    use crate::settings::{Config, Registry, edit::Settings};
    use serde_json::json;
    for sensitive in [false, true] {
        let mut registry = Registry::core();
        let key = "extensions.fixture.a_very_long_boolean_setting_label";
        registry.register_extension("fixture",vec![serde_json::from_value(json!({"key":key,"type":"boolean","default":false,"description":"Fixture checkbox","provider":"fixture","sensitive":sensitive})).unwrap()],vec![]).unwrap();
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
        handle_key(&mut app, KeyCode::Right.into());
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
    app.inventory =
        crate::extensions::inventory::list(&crate::extensions::default_settings().unwrap(), true);
    select(&mut app, "pid:11");
    let selected = app.selected;
    app.paused = true;
    for modal in ['e', 'd', '?'] {
        handle_key(&mut app, KeyCode::Char(modal).into());
        for details in [false, true] {
            if modal == 'e' && details {
                handle_key(&mut app, KeyCode::Enter.into());
                assert!(!app.extension_details);
                handle_key(&mut app, KeyCode::Char('d').into());
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
    for ch in "seqdhl".chars() {
        assert!(!handle_key(&mut app, KeyCode::Char(ch).into()));
    }
    handle_key(&mut app, KeyCode::Left.into());
    handle_key(&mut app, KeyCode::Right.into());
    assert_eq!(app.query, "seqdhl");
    assert!(app.details.is_none());
    handle_key(&mut app, KeyCode::Esc.into());
    let selected = app.selected;
    for _ in 0..3 {
        assert!(!handle_key(&mut app, KeyCode::Esc.into()));
        assert!(!app.editing && app.query.is_empty());
        assert!(app.details.is_none() && !app.extensions_page && !app.help);
        assert_eq!(app.selected, selected);
    }
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
    for code in [KeyCode::Char('s'), KeyCode::Down, KeyCode::Enter] {
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
                vec![
                    serde_json::from_value(json!({
                        "key":"extensions.fixture.long_text", "type":"string", "default":"initial",
                        "description":"Text", "provider":"fixture", "sensitive":sensitive
                    }))
                    .unwrap(),
                ],
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
        for ch in "專案內容很長很長很長尾端d".chars() {
            handle_key(&mut app, KeyCode::Char(ch).into());
        }
        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('\t'),
            KeyCode::Left,
            KeyCode::Right,
        ] {
            handle_key(&mut app, code.into());
        }
        for (width, height) in [(100, 30), (36, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
                .unwrap();
            assert!(app.details.is_none() && app.settings.as_ref().unwrap().open);
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(cursor.x < width - 2 && cursor.y < height - 1);
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[cursor].symbol(), "_");
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            let visible = text.replace(' ', "");
            if sensitive {
                assert!(!visible.contains("尾端") && visible.contains("***_"));
            } else {
                assert!(visible.contains("尾端d_]"), "{text}");
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
                                "unit":unit, "description":"Fixture interval", "provider":"fixture", "sensitive":sensitive
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
    app.path_scroll = 16;
    handle_key(&mut app, KeyCode::Char('?').into());
    let now = std::time::Instant::now();
    assert!(should_collect(&app, now, now));
    app.update(Ok(app.snapshot.clone().unwrap()));
    for code in [
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Down,
        KeyCode::Char('q'),
        KeyCode::Char('s'),
        KeyCode::Char('e'),
    ] {
        assert!(!handle_key(&mut app, code.into()));
    }
    assert!(app.help && app.details.is_none() && !app.extensions_page);
    handle_key(&mut app, KeyCode::Esc.into());
    assert_eq!(
        app.node(app.selected).unwrap().reference,
        "group:fixturevolume"
    );
    assert_eq!(app.path_scroll, 16);
    assert!(app.manual_refresh.is_none());
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
    handle_key(&mut app, KeyCode::Right.into());
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.query_pid(), None);
    assert!(!details.needs_query);
    let lines = details.lines().join("\n");
    assert!(lines.contains("Temperature: ? vendor.celsius"));
    assert!(lines.contains("/Volumes/Data"));
    assert!(lines.contains("fixture") && !lines.contains("fixture/1.0.0"));
    handle_key(&mut app, KeyCode::Left.into());
    assert!(app.details.is_none());
    assert_eq!(
        app.node(app.selected).unwrap().reference,
        "group:fixturevolume"
    );
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

fn priority_reset_app(path: &std::path::Path) -> App {
    let resolved = crate::extensions::default_settings().unwrap();
    let mut app = App::new("group:all".into(), 2);
    app.inventory = crate::extensions::inventory::list(&resolved, false);
    for id in ["z_module", "a_module"] {
        let mut entry = crate::extensions::inventory::list(&resolved, false).remove(0);
        entry.descriptor.id = id.into();
        app.inventory.push(entry);
    }
    app.settings = Some(settings::Popup::new(crate::settings::edit::Settings::new(
        crate::extensions::settings_registry().unwrap(),
        crate::settings::Config::load_path(Some(path), false).unwrap(),
        Default::default(),
        Some(path.to_path_buf()),
    )));
    handle_key(&mut app, KeyCode::Char('e').into());
    app
}

fn priority_reset_frame(app: &App, width: u16, height: u16, name: &str) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| extensions::draw(frame, app)).unwrap();
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
}

#[test]
fn priority_reset_discards_saves_defaults_and_moves_module_sources() {
    use crate::settings::Config;
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let path = root_path.join("config.json");
    let saved = Config {
        recovery: None,
        disabled_builtins: vec![],
        ownership_priority: vec![],
        version: 1,
        source_priority: vec!["a_module".into(), "missing_source".into(), "project".into()],
        settings: [("sampling.interval_ms".into(), serde_json::json!(250))].into(),
    };
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut app = priority_reset_app(&path);
    app.extension_selected = Some("project".into());
    let text = priority_reset_frame(&app, 100, 30, "priority-caption-saved");
    assert!(text.contains("Projects · → Details"), "{text}");
    assert!(
        text.contains(" Extensions ") && !text.contains("Extensions *"),
        "{text}"
    );
    assert!(
        !text.contains("· saved") && !text.contains("· draft"),
        "{text}"
    );
    app.extension_selected = Some("a_module".into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert!(
        app.priority_draft
            .as_ref()
            .unwrap()
            .contains(&"missing_source".into())
    );
    let moved = app.priority_draft.clone();
    handle_key(&mut app, KeyCode::Right.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.priority_draft, moved);
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('r').into());
    assert_eq!(app.priority_draft, moved);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert!(app.priority_draft.as_ref().unwrap().is_empty());
    assert_eq!(app.extension_selected.as_deref(), Some("a_module"));
    for (width, height) in [(100, 30), (71, 30), (36, 10)] {
        let text = priority_reset_frame(&app, width, height, "priority-reset-draft");
        for hint in ["Ctrl-R Reset", "Ctrl-S Save", "a_module"] {
            assert!(text.contains(hint), "missing {hint}: {text}");
        }
        assert!(
            !text.contains("Extensions *") && text.contains('*'),
            "{text}"
        );
        assert!(
            !text.contains("· saved") && !text.contains("· draft"),
            "{text}"
        );
        if width == 100 {
            assert!(text.contains("Sources"), "{text}");
            assert!(text.contains("A module · → Details"), "{text}");
        }
        if width == 36 {
            assert!(text.contains("Shift↑↓ Move"), "{text}");
        }
        if width >= 71 {
            let footer = text
                .lines()
                .find(|line| line.contains("Ctrl-R Reset order"))
                .unwrap();
            assert!(
                !footer.trim_end_matches([' ', '│']).ends_with('·'),
                "{text}"
            );
            assert!(text.contains("Esc Cancel"), "{text}");
        }
    }
    handle_key(&mut app, KeyCode::Esc.into());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    handle_key(&mut app, KeyCode::Char('e').into());
    assert!(
        !priority_reset_frame(&app, 100, 30, "priority-caption-discarded").contains("Extensions *")
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.extensions_page && app.priority_error.is_none());
    let config = Config::load_path(Some(&path), true).unwrap();
    assert!(config.source_priority.is_empty());
    assert_eq!(config.settings, saved.settings);
    let mut restarted = priority_reset_app(&path);
    let text = priority_reset_frame(&restarted, 100, 30, "priority-caption-saved-default");
    assert!(!text.contains("Extensions *"), "{text}");
    handle_key(
        &mut restarted,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert!(
        !priority_reset_frame(&restarted, 100, 30, "priority-caption-noop-reset")
            .contains("Extensions *")
    );
    handle_key(&mut restarted, KeyCode::Esc.into());
    handle_key(&mut restarted, KeyCode::Char('e').into());
    restarted.extension_selected = Some("a_module".into());
    handle_key(&mut restarted, KeyCode::Char('r').into());
    let order = extensions::order(&restarted, false);
    assert_eq!(
        &order[..13],
        [
            "app_bundle",
            "bin",
            "command",
            "project",
            "ancestor",
            "versioned_install",
            "node",
            "python",
            "launched_by",
            "docker",
            "podman",
            "apple",
            "vm",
        ]
    );
    assert_eq!(&order[13..], ["z_module", "a_module"]);
    handle_key(
        &mut restarted,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut restarted,
        KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
    );
    let order = restarted.priority_draft.as_ref().unwrap();
    assert_eq!(&order[13..], ["a_module", "z_module"]);
    for _ in 0..13 {
        handle_key(
            &mut restarted,
            KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
        );
    }
    assert_eq!(restarted.priority_draft.as_ref().unwrap()[0], "a_module");
    assert!(restarted.manual_refresh.is_none() && restarted.extensions.is_empty());
    root.close().unwrap();
    assert!(!root_path.exists());
}

#[test]
fn priority_reset_default_is_idempotent_and_preserves_conflict_safe_save() {
    use crate::settings::Config;
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let path = root_path.join("config.json");
    let mut app = priority_reset_app(&path);
    for _ in 0..2 {
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
    }
    assert!(!priority_reset_frame(&app, 100, 30, "priority-reset-default").contains("draft"));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(!path.exists());
    let default = Config {
        version: 1,
        ..Config::default()
    };
    let default_bytes = serde_json::to_vec(&default).unwrap();
    std::fs::write(&path, &default_bytes).unwrap();
    let before = std::fs::metadata(&path).unwrap();
    let mut app = priority_reset_app(&path);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert_eq!(std::fs::read(&path).unwrap(), default_bytes);
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&before),
        std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&path).unwrap())
    );
    let saved = Config {
        version: 1,
        source_priority: vec!["bin".into()],
        ..Config::default()
    };
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut app = priority_reset_app(&path);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    let fresh = Config {
        version: 1,
        source_priority: vec!["project".into()],
        ..Config::default()
    };
    let bytes = serde_json::to_vec(&fresh).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(
        app.extensions_page
            && app
                .priority_error
                .as_ref()
                .unwrap()
                .contains("Priority changed on disk")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    root.close().unwrap();
    assert!(!root_path.exists());
}

#[test]
fn priority_dialog_reorders_builtins_saves_and_keeps_failures_as_draft() {
    use crate::settings::{Config, edit::Settings};
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let resolved = crate::extensions::default_settings().unwrap();
    let mut app = App::new("group:all".into(), 2);
    app.inventory = crate::extensions::inventory::list(&resolved, false);
    app.settings = Some(settings::Popup::new(Settings::new(
        crate::extensions::settings_registry().unwrap(),
        Config::default(),
        Default::default(),
        Some(path.clone()),
    )));
    app.extensions_page = true;
    app.priority_mode = 0;
    app.extension_selected = Some("project".into());
    extensions::key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(
        &app.priority_draft.as_ref().unwrap()[..6],
        [
            "app_bundle",
            "bin",
            "command",
            "ancestor",
            "project",
            "versioned_install"
        ]
    );
    app.extensions_scroll = 2;
    let draft = app.priority_draft.clone();
    extensions::key(&mut app, KeyCode::Enter.into());
    assert!(!app.extension_details);
    handle_key(&mut app, KeyCode::Right.into());
    assert!(app.extension_details);
    handle_key(&mut app, KeyCode::Right.into());
    assert!(app.extension_details);
    extensions::key(&mut app, KeyCode::Down.into());
    extensions::key(&mut app, KeyCode::Enter.into());
    assert!(app.extension_details);
    handle_key(&mut app, KeyCode::Left.into());
    assert_eq!(app.priority_draft, draft);
    assert_eq!(app.extension_selected.as_deref(), Some("project"));
    assert_eq!(app.extensions_scroll, 2);
    extensions::key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(app.extensions_page);
    extensions::key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.extensions_page);
    let saved = Config::load_path(Some(&path), true).unwrap();
    assert_eq!(saved.source_priority[4], "project");
    assert!(app.settings.as_mut().unwrap().applied.take().is_some());
    app.extensions_page = true;
    app.priority_mode = 0;
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

#[test]
fn ui10_effective_tags_and_classification_trace_are_separate_and_wrapped() {
    use crate::extensions::tags::Report;
    use serde_json::json;
    for conflict in [false, true] {
        let tag = json!({"dimension":"bin","id":"bin:hash","label":"deleted","path":"/usr/local/bin/demo","basis":"executable"});
        let mut report = json!({
            "effective":[{"source":"bin","tag":tag}],
            "proposals":[{"source":"bin","proposal":{"id":"bin-add","identity":{"pid":10,"started_unix_us":100},"change":{"operation":"add","tag":tag}}}],
            "decisions":[{"source":"bin","proposal_id":"bin-add","status":"effective","by_source":null,"reason":"source_priority"}],
            "diagnostics":[]
        });
        if conflict {
            report["effective"] = json!([]);
            report["proposals"].as_array_mut().unwrap().push(json!({
                "source":"fixture",
                "proposal":{"id":"exclude-bin","identity":{"pid":10,"started_unix_us":100},
                    "change":{"operation":"exclude","selector":{"dimension":"bin","tag_id":null,"source":"bin"}}}
            }));
            report["decisions"][0]["status"] = json!("excluded");
            report["decisions"][0]["reason"] = json!("higher_priority_exclusion");
            report["decisions"][0]["by_source"] = json!("fixture");
        }
        let report: Report = serde_json::from_value(report).unwrap();
        let mut source = snapshot();
        change(&mut source.root, 10, |process| {
            process.tag_report = Some(report.clone());
            process.command = Some(Default::default());
        });
        let mut app = App::new("group:all".into(), 4);
        app.update(Ok(source.clone()));
        select(&mut app, "pid:10");
        handle_key(&mut app, KeyCode::Char('d').into());
        app.update(Ok(source));
        let lines = app.details.as_ref().unwrap().lines();
        let tags = lines.iter().find(|line| line.starts_with("Tags ")).unwrap();
        assert_eq!(
            tags.trim(),
            if conflict {
                "Tags  None"
            } else {
                "Tags  #executable"
            }
        );
        let network = lines.iter().position(|line| line == "Network").unwrap();
        let classification = lines
            .iter()
            .position(|line| line == "Classification")
            .unwrap();
        assert!(network < classification);
        assert!(!lines[..classification].join("\n").contains("bin:hash"));
        let trace = lines[classification..].join("\n");
        for fact in ["bin:hash", "bin-add", "/usr/local/bin/demo", "deleted"] {
            assert!(trace.contains(fact), "{trace}");
        }
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("Tags "))
                .count(),
            1
        );
        for (width, height) in [(100, 30), (36, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for page in 0..3 {
                terminal
                    .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
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
                        std::path::Path::new(&path).join(format!(
                            "ui10-{}-{width}x{height}-page{page}.txt",
                            if conflict { "conflict" } else { "normal" }
                        )),
                        &text,
                    )
                    .unwrap();
                }
                if width == 100 {
                    assert!(
                        text.contains("Classification") && text.contains("bin:hash"),
                        "{text}"
                    );
                }
                handle_key(&mut app, KeyCode::PageDown.into());
            }
        }
        let mut node = tree::select(&app.snapshot.as_ref().unwrap().root, "pid:10").unwrap();
        node.process.as_mut().unwrap().tag_report = None;
        let details = details::Details::new(node.clone());
        assert!(
            details
                .lines()
                .iter()
                .any(|line| line == "Tags  Unavailable")
        );
        node.process.as_mut().unwrap().tag_report = Some(Default::default());
        let details = details::Details::new(node);
        assert!(details.lines().iter().any(|line| line == "Tags  None"));
    }
}

#[test]
fn help_pages_wrap_scroll_clamp_and_restore_main_state() {
    let mut app = app();
    select(&mut app, "pid:11");
    app.path_scroll = 16;
    app.paused = true;
    app.collecting = true;
    let selected = app.rows[app.selected].key.clone();
    let expanded = app.expanded.clone();
    let mut table = TableState::default();
    let rendered_offset = std::cell::Cell::new(0);
    let mut dump = |app: &App, width, height, name: &str| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut table))
            .unwrap();
        rendered_offset.set(table.offset());
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
                std::path::Path::new(&path).join(format!("help-{name}-{width}x{height}.txt")),
                &text,
            )
            .unwrap();
        }
        text
    };
    dump(&app, 100, 30, "before");
    let offset = rendered_offset.get();
    handle_key(&mut app, KeyCode::Char('?').into());
    for (page, name, last) in [
        (0, "Controls", "quit anywhere"),
        (1, "Panels", "List: 4 rows; details: visible height"),
        (2, "Metrics", "one query at a time"),
    ] {
        assert_eq!(app.help_view.page, page);
        let scroll = app.help_view.scroll.get();
        for code in [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
            KeyCode::Char('h'),
        ] {
            assert!(!handle_key(&mut app, code.into()));
            assert!(app.help);
            assert_eq!(app.help_view.page, page);
            assert_eq!(app.help_view.scroll.get(), scroll);
            assert_eq!(app.expanded, expanded);
        }
        for (width, height) in [(160, 45), (100, 30), (36, 10)] {
            app.help_view.scroll.set(0);
            let text = dump(&app, width, height, name);
            assert_eq!(rendered_offset.get(), offset);
            assert!(text.contains(&format!("[{name}]")), "{text}");
            for group in ["Tab/Shift-Tab page", "Esc/? close"] {
                assert!(text.contains(group), "{text}");
            }
            let hint = if text.contains("↑↓ Scroll") {
                "↑↓ Scroll"
            } else {
                "Tab/Shift-Tab page"
            };
            let check_gap = |text: &str| {
                let lines: Vec<_> = text.lines().collect();
                let footer = lines.iter().position(|line| line.contains(hint)).unwrap();
                let top = lines.iter().find(|line| line.contains("┌ Help ")).unwrap();
                let left = top[..top.find("┌ Help ").unwrap()].chars().count();
                assert!(
                    lines[footer - 1]
                        .chars()
                        .skip(left + 1)
                        .take(usize::from(width.min(96) - 2))
                        .all(|ch| ch == ' '),
                    "missing body/footer gap: {text}"
                );
            };
            check_gap(&text);
            if width == 160 {
                assert_eq!(app.help_view.scroll.get(), 0);
                assert!(text.contains(last), "wide content should fit: {text}");
                if page == 1 {
                    assert!(text.contains("PgUp / PgDn"));
                    assert!(text.contains("Page by visible content height"));
                    assert!(text.contains("Scroll lines / page by visible height"));
                }
                if page == 2 {
                    assert!(text.contains(
                        "Host RSS and container memory have separate scopes; do not add them"
                    ));
                    assert!(text.contains("Filter visible rows, not aggregate totals"));
                }
            }
            handle_key(&mut app, KeyCode::End.into());
            let bottom = dump(&app, width, height, &format!("{name}-bottom"));
            assert!(
                bottom
                    .replace('│', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains(last),
                "{bottom}"
            );
            check_gap(&bottom);
            let lines: Vec<_> = bottom.lines().collect();
            let footer = lines.iter().position(|line| line.contains(hint)).unwrap();
            let top = lines.iter().find(|line| line.contains("┌ Help ")).unwrap();
            let left = top[..top.find("┌ Help ").unwrap()].chars().count();
            assert!(
                lines[footer - 2]
                    .chars()
                    .skip(left + 1)
                    .take(usize::from(width.min(96) - 2))
                    .any(|ch| ch != ' '),
                "expected exactly one blank row before footer: {bottom}"
            );
            let end = app.help_view.scroll.get();
            handle_key(&mut app, KeyCode::Down.into());
            assert_eq!(app.help_view.scroll.get(), end);
            handle_key(&mut app, KeyCode::PageUp.into());
            assert!(app.help_view.scroll.get() <= end);
            handle_key(&mut app, KeyCode::Home.into());
            assert_eq!(app.help_view.scroll.get(), 0);
            handle_key(&mut app, KeyCode::PageDown.into());
            assert!(app.help_view.scroll.get() > 0 || end == 0);
        }
        app.help_view.scroll.set(usize::MAX);
        dump(&app, 160, 45, &format!("{name}-resized"));
        assert!(app.help_view.scroll.get() < usize::MAX);
        handle_key(&mut app, KeyCode::Tab.into());
        assert_eq!(app.help_view.scroll.get(), 0);
    }
    assert_eq!(app.help_view.page, 0);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert_eq!(app.help_view.page, 2);
    handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
    assert_eq!(app.help_view.page, 1);
    for ch in ['q', 's', 'e', 'r', 'p', '/', 'c', 'd'] {
        assert!(!handle_key(&mut app, KeyCode::Char(ch).into()));
    }
    assert!(app.help && app.details.is_none() && !app.extensions_page);
    assert!(app.manual_refresh.is_none() && app.paused && app.collecting);
    assert_eq!(app.rows[app.selected].key, selected);
    assert_eq!(app.expanded, expanded);
    assert_eq!(app.path_scroll, 16);
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(!app.help);
    dump(&app, 100, 30, "restored");
    assert_eq!(rendered_offset.get(), offset);

    assert_eq!(app.rows[app.selected].key, selected);
    handle_key(&mut app, KeyCode::Char('?').into());
    assert_eq!(app.help_view.page, 0);
    assert_eq!(app.help_view.scroll.get(), 0);
    handle_key(&mut app, KeyCode::Char('?').into());
    assert!(!app.help);
    assert!(handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
}

#[test]
fn inventory_layout_indents_ids_and_keeps_wide_columns_compact() {
    use ratatui::style::Color;
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let mut app = priority_reset_app(&root.path().join("config.json"));
    app.priority_mode = 0;
    app.extension_selected = Some("project".into());
    for (width, height) in [(160, 30), (240, 30), (122, 30), (123, 30), (36, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| extensions::draw(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let lines: Vec<String> = (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let locate = |text: &str| {
            lines
                .iter()
                .enumerate()
                .find_map(|(y, line)| {
                    line.find(text)
                        .map(|x| (line[..x].chars().count() as u16, y as u16))
                })
                .unwrap_or_else(|| panic!("missing {text}: {lines:?}"))
        };
        let (header_x, header_y) = locate("ID");
        let (id_x, id_y) = locate("project");
        assert_eq!(id_x, header_x + 2);
        assert_eq!(buffer[(header_x, id_y)].symbol(), " ");
        assert_eq!(buffer[(id_x, id_y)].bg, Color::DarkGray);
        assert!(!lines[id_y as usize].contains(['>', '»']));
        let (state_x, _) = locate("State");
        let state: String = (state_x..state_x + 7)
            .map(|x| buffer[(x, id_y)].symbol())
            .collect();
        assert_eq!(state, "Enabled");
        if width >= 160 {
            assert_eq!(locate("a_module").0, header_x + 2);
            for (header, value) in [
                ("Capabilities", "classify"),
                ("State", "Enabled"),
                ("Rank", "4"),
                ("Collect", "—"),
                ("Classify", "—"),
                ("Details", "—"),
            ] {
                let (x, y) = locate(header);
                assert_eq!(y, header_y);
                let actual: String = (x..x + value.chars().count() as u16)
                    .map(|x| buffer[(x, id_y)].symbol())
                    .collect();
                assert_eq!(actual, value, "{header} at width {width}");
            }
            assert_eq!(locate("Capabilities").0, header_x + 43);
            let (last_x, _) = locate("Details");
            assert!(last_x < 150);
            assert!(
                lines[header_y as usize]
                    .chars()
                    .skip(last_x as usize + 7)
                    .take((width - last_x - 9) as usize)
                    .all(|c| c == ' ')
            );
        }
        assert_eq!(
            lines[header_y as usize].contains("Capabilities"),
            width >= 123
        );
        if width == 160 || width == 36 {
            priority_reset_frame(&app, width, height, "pl4-pl5-inventory");
        }
    }
    let long_id = "module_with_a_very_long_identifier_that_exceeds_the_inventory_column";
    app.inventory.last_mut().unwrap().descriptor.id = long_id.into();
    app.extension_selected = Some(long_id.into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    for (width, height) in [(160, 30), (36, 10)] {
        let text = priority_reset_frame(&app, width, height, "sources-long-id-marker");
        let row = text.lines().find(|line| line.contains("module_w")).unwrap();
        assert!(
            row.contains("* module_w") && row.contains("Disabled"),
            "{text}"
        );
    }
    let text = priority_reset_frame(&app, 160, 30, "pl4-pl5-long-id");
    assert!(text.contains(" module_with_a_very"));
    assert!(!text.contains(long_id));
    handle_key(&mut app, KeyCode::Right.into());
    assert!(
        priority_reset_frame(&app, 160, 30, "pl4-pl5-details").contains(&format!("ID: {long_id}"))
    );
    let mut pages = String::new();
    for _ in 0..10 {
        pages.push_str(&priority_reset_frame(&app, 36, 10, "pl4-pl5-long-details"));
        handle_key(&mut app, KeyCode::Down.into());
    }
    let unwrapped: String = pages
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '│')
        .collect();
    assert!(unwrapped.contains(long_id));
    handle_key(&mut app, KeyCode::Left.into());
    {
        app.priority_mode = 1;
        app.extension_selected = Some("project".into());
        let text = priority_reset_frame(&app, 160, 30, "pl4-pl5-priority");
        let header = text
            .lines()
            .find(|line| line.contains("Rank / Name"))
            .unwrap();
        let row = text.lines().find(|line| line.contains("Projects")).unwrap();
        let x = header.find("Rank / Name").unwrap();
        assert_eq!(&row[x..x + 2], "  ");
        assert!(row.as_bytes()[x + 2].is_ascii_digit());
        let id_x = header.find("ID").unwrap();
        assert_eq!(&row[id_x..id_x + 7], "project");
    }
    root.close().unwrap();
}

#[test]
fn extension_actions_follow_bundled_draft_intent() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let mut app = priority_reset_app(&path);
    app.extension_selected = Some("docker".into());
    app.extensions.push(crate::extensions::report::Record::new(
        "docker",
        "collect",
        "failed",
        Some("provider_failed"),
        None,
    ));
    for (width, height) in [(160, 30), (36, 10)] {
        let text = priority_reset_frame(&app, width, height, "pl6-blocked-action");
        for hint in ["Space Disable", "Ctrl-S Save", "Esc Cancel", "Blocked"] {
            assert!(text.contains(hint), "{hint}: {text}");
        }
        if width == 160 {
            assert!(text.contains("Blocked / error"));
        }
        assert!(text.contains("Shift↑↓ Move") && text.contains("Ctrl-R Reset"));
        if width == 160 {
            assert!(text.contains("Ctrl-R Reset order"));
        }
    }
    app.extensions.push(crate::extensions::report::Record::new(
        "broken",
        "read",
        "failed",
        Some("invalid_data"),
        None,
    ));
    app.priority_error = Some("Config changed on disk".into());
    let error = priority_reset_frame(&app, 36, 10, "sources-error-row");
    assert!(error.contains("Config changed on disk"));
    assert!(
        error
            .lines()
            .any(|line| line.contains("docker") && line.contains("Blocked")),
        "{error}"
    );
    app.priority_error = None;
    handle_key(&mut app, KeyCode::Char(' ').into());
    for (width, height) in [(160, 30), (36, 10)] {
        let text = priority_reset_frame(&app, width, height, "pl6-disabled-action");
        for hint in [
            "Space Enable",
            "* docker",
            "Ctrl-S Save",
            "Esc Cancel",
            "Disabled",
        ] {
            assert!(text.contains(hint), "{hint}: {text}");
        }
    }
    handle_key(&mut app, KeyCode::Char(' ').into());
    let text = priority_reset_frame(&app, 36, 10, "pl6-restored-action");
    assert!(text.contains("Space Disable") && !text.contains("* docker"));
    assert!(!text.contains("Unsaved draft") && !text.contains("Extensions *"));
    app.extension_selected = Some("a_module".into());
    for (width, height) in [(160, 30), (36, 10)] {
        let text = priority_reset_frame(&app, width, height, "pl6-module-cli");
        assert!(text.contains("Space Disable"), "{text}");
        assert!(!text.contains("enable|disable ID") && !text.contains("Toggle"));
    }
    {
        handle_key(&mut app, KeyCode::Tab.into());
        assert_eq!(app.priority_mode, 1);
        for (width, height) in [(160, 30), (36, 10)] {
            let text = priority_reset_frame(&app, width, height, "pl6-priority-controls");
            for hint in ["Shift↑↓ Move", "Ctrl-R Reset", "Ctrl-S Save", "Esc Cancel"] {
                assert!(text.contains(hint), "{hint}: {text}");
            }
            assert!(!text.contains("Space ") && !text.contains("enable|disable"));
        }
    }
    let owned_path = root.path().to_path_buf();
    root.close().unwrap();
    assert!(!owned_path.exists());
}

#[test]
fn sources_and_ownership_keep_drafts_and_highlight_without_prefixes() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    std::fs::write(&path, serde_json::json!({"version":1,"settings":{"future.key":1},"source_priority":["unknown_source"],"ownership_priority":["unknown_dimension"]}).to_string()).unwrap();
    let mut app = priority_reset_app(&path);
    assert_eq!(app.priority_mode, 0);
    app.extension_selected = Some("project".into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    assert_eq!(app.builtin_draft.get("project"), Some(&false));
    for (width, height) in [(160, 30), (36, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| extensions::draw(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cells: Vec<_> = buffer
            .content
            .iter()
            .filter(|c| c.bg == ratatui::style::Color::DarkGray)
            .collect();
        assert!(!cells.is_empty());
        assert!(cells.iter().all(|c| !matches!(c.symbol(), ">" | "»")));

        let selected_y = (0..height)
            .find(|&y| buffer[(3, y)].symbol() == "*")
            .unwrap();
        assert_eq!(buffer[(3, selected_y)].bg, ratatui::style::Color::DarkGray);
        assert_eq!(buffer[(4, selected_y)].symbol(), " ");
        assert_eq!(buffer[(5, selected_y)].symbol(), "p");
        assert_ne!(buffer[(2, selected_y)].bg, ratatui::style::Color::DarkGray);
    }
    handle_key(&mut app, KeyCode::Home.into());
    let inventory = priority_reset_frame(&app, 160, 30, "rw49-groups");
    app.extension_selected = Some("project".into());
    assert!(!inventory.contains("Type") && !inventory.contains("Version"));
    assert!(inventory.contains("Capabilities") && inventory.contains("Rank"));
    assert!(
        inventory.contains("app_bundle")
            && inventory.contains("Fixed")
            && !inventory.contains("Required")
    );
    handle_key(&mut app, KeyCode::Right.into());
    let mut pages = String::new();
    for _ in 0..30 {
        pages.push_str(&priority_reset_frame(&app, 36, 10, "rw49-narrow-details"));
        handle_key(&mut app, KeyCode::Down.into());
    }
    for label in [
        "ID: project",
        "Capabilities:",
        "classify",
        "Source rank:",
        "Disabled",
        "Timings",
    ] {
        assert!(pages.contains(label), "{label}: {pages}");
    }
    handle_key(&mut app, KeyCode::Esc.into());
    assert_eq!(app.priority_mode, 0);
    app.extension_selected = Some("a_module".into());
    for _ in 0..20 {
        handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    }
    assert_eq!(app.priority_draft.as_ref().unwrap()[0], "a_module");
    assert!(
        app.priority_draft
            .as_ref()
            .unwrap()
            .contains(&"app_bundle".into())
    );
    let sources = app.priority_draft.clone();
    handle_key(&mut app, KeyCode::Tab.into());
    assert_eq!(app.priority_mode, 1);
    app.extension_selected = Some("project".into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.priority_draft, sources);
    assert!(
        app.ownership_draft
            .as_ref()
            .unwrap()
            .contains(&"unknown_dimension".into())
    );
    assert!(priority_reset_frame(&app, 100, 30, "rw49-ownership").contains("Ownership"));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    let saved = crate::settings::Config::load_path(Some(&path), true).unwrap();
    assert_eq!(saved.source_priority[0], "a_module");
    assert!(
        saved
            .ownership_priority
            .contains(&"unknown_dimension".into())
    );
    assert_eq!(saved.disabled_builtins, ["project"]);
    assert_eq!(saved.settings["future.key"], 1);
    assert!(app.extensions_page && app.priority_mode == 1);
    handle_key(&mut app, KeyCode::Esc.into());
    handle_key(&mut app, KeyCode::Char('e').into());
    app.extension_selected = Some("project".into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    handle_key(&mut app, KeyCode::Esc.into());
    assert_eq!(
        crate::settings::Config::load_path(Some(&path), true)
            .unwrap()
            .disabled_builtins,
        ["project"]
    );
    assert!(app.manual_refresh.is_none());
    root.close().unwrap();
}

#[test]
fn settings_tabs_keep_selection_highlight_and_main_disclosure_markers() {
    use ratatui::style::{Color, Modifier};
    let mut app = app();
    app.settings = Some(settings::Popup::new(crate::settings::edit::Settings::new(
        crate::settings::Registry::core(),
        Default::default(),
        Default::default(),
        None,
    )));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!text.contains('»'));
    assert!(text.contains('v') || text.contains('>'));
    handle_key(&mut app, KeyCode::Char('s').into());
    for tab in [KeyCode::Tab, KeyCode::BackTab] {
        handle_key(&mut app, tab.into());
        terminal
            .draw(|frame| app.settings.as_ref().unwrap().draw(frame))
            .unwrap();
        let selected: Vec<_> = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|c| c.bg == Color::DarkGray)
            .collect();
        assert!(!selected.is_empty());
        assert!(selected.iter().all(|c| !matches!(c.symbol(), ">" | "»")));
        assert!(selected.iter().all(|c| !c.modifier.contains(Modifier::DIM)));
    }
}

#[test]
fn review_priority_noop_and_return_to_baseline_do_not_materialize_defaults() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let baseline = serde_json::json!({"version":1,"settings":{"unknown.key":1},"source_priority":["unknown_source"],"ownership_priority":["unknown_dimension"]});
    for mode in [0, 1] {
        for movement in [vec![KeyCode::Up], vec![KeyCode::Down, KeyCode::Up], vec![]] {
            std::fs::write(&path, baseline.to_string()).unwrap();
            let before = std::fs::read(&path).unwrap();
            let mut app = priority_reset_app(&path);
            app.priority_mode = mode;
            handle_key(&mut app, KeyCode::Home.into());
            for key in movement {
                handle_key(&mut app, KeyEvent::new(key, KeyModifiers::SHIFT));
            }
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal
                .draw(|frame| extensions::draw(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let highlighted: Vec<_> = buffer
                .content
                .iter()
                .filter(|c| c.bg == ratatui::style::Color::DarkGray)
                .collect();
            let selected: String = highlighted.iter().map(|c| c.symbol()).collect();
            assert!(selected.contains("unknown"), "{selected}");
            assert!(highlighted.iter().all(|c| !matches!(c.symbol(), ">" | "»")));
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            );
            assert_eq!(std::fs::read(&path).unwrap(), before, "mode {mode}");
        }
    }
    root.close().unwrap();
}

#[test]
fn review_ownership_ties_match_core_and_main_selected_style_is_visible() {
    use crate::extensions::{
        placement::{Ownership, Registry},
        tags::{Effective, Report},
    };
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let mut app = priority_reset_app(&root.path().join("config.json"));
    app.ownership_draft = Some(vec!["extensions.fixture.owner".into(), "work".into()]);
    app.priority_mode = 1;
    let ids = extensions::order(&app, true);
    assert!(
        ids.iter()
            .position(|id| id == "extensions.fixture.owner")
            .unwrap()
            < ids.iter().position(|id| id == "work").unwrap()
    );
    let registry = Registry {
        categories: Default::default(),
        ownership: [
            Ownership::new("work", "unclassified", 300),
            Ownership::new("extensions.fixture.owner", "extensions.fixture.jobs", 300),
        ]
        .into_iter()
        .map(|o| (o.dimension.clone(), o))
        .collect(),
    };
    let report = Report { effective: ["work","extensions.fixture.owner"].map(|dimension| Effective { source:"fixture".into(), tag:serde_json::from_value(serde_json::json!({"dimension":dimension,"id":format!("{dimension}:job"),"label":"Job","path":null,"basis":"observed"})).unwrap() }).into(), ..Default::default() };
    assert_eq!(
        registry
            .selected(&report, &crate::extensions::default_settings().unwrap())
            .unwrap()
            .tag
            .dimension,
        "extensions.fixture.owner"
    );
    let mut main = super::tests::app();
    select(&mut main, "pid:11");
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &main, &mut TableState::default()))
        .unwrap();
    let cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .filter(|c| c.bg == ratatui::style::Color::DarkGray)
        .collect();
    let selected: String = cells.iter().map(|c| c.symbol()).collect();
    assert!(selected.contains("pid:11"), "{selected}");
    assert!(
        cells
            .iter()
            .all(|c| c.modifier.contains(ratatui::style::Modifier::BOLD))
    );
    assert!(!selected.contains('»'));
    root.close().unwrap();
}

#[test]
fn launched_by_details_and_search_survive_snapshot_projection_and_disable() {
    use crate::extensions::builtin::launched_by::tests::snapshot;
    let mut app = App::new("group:all".into(), usize::MAX);
    let mut initial = snapshot(true, "group:all");
    initial.project(&[]);
    app.update(Ok(initial));
    app.query = "lumina".into();
    app.rebuild();
    let text = screen(&app, 160);
    assert!(text.contains("pid:40"), "{text}");
    assert!(
        !serde_json::to_string(&app.snapshot.as_ref().unwrap().root)
            .unwrap()
            .contains("evidence_chain")
    );
    select(&mut app, "pid:40");
    app.open_details();
    app.update(Ok(snapshot(true, "group:all")));
    let text = screen(&app, 160);
    assert!(text.contains("Launched by Lumina"), "{text}");
    for pid in [40, 30, 20] {
        assert!(text.contains(&format!("via pid:{pid}")), "{text}");
    }
    assert!(
        text.contains("/Applications/Lumina.app/Contents/Helpers/agent"),
        "{text}"
    );
    app.details = None;
    app.update(Ok(snapshot(false, "group:all")));
    let text = screen(&app, 160);
    assert!(!text.contains("pid:40"), "{text}");
}

#[test]
fn vm_host_and_in_vm_metrics_have_separate_details_without_command_queries() {
    for apple in [false, true] {
        let source = crate::extensions::builtin::vm::tests::fixture_snapshot(apple, true);
        let mut app = App::new("group:all".into(), usize::MAX);
        app.update(Ok(source));
        let mut pending = vec![&app.snapshot.as_ref().unwrap().root];
        let mut reference = None;
        while let Some(node) = pending.pop() {
            if !node.host_resources.is_empty() {
                reference = Some(node.reference.clone());
                break;
            }
            pending.extend(&node.children);
        }
        select(&mut app, &reference.unwrap());
        app.open_details();
        assert!(app.details.as_ref().unwrap().query_pid().is_none());
        let lines = app.details.as_ref().unwrap().lines().join("\n");
        assert!(lines.contains("VM host pid:40"), "{lines}");
        assert!(lines.contains("separate from in-VM usage"), "{lines}");
        if !apple {
            assert!(lines.contains("in-VM usage: CPU"), "{lines}");
        }
        assert!(lines.contains("Config ("), "{lines}");
        if apple {
            assert!(lines.contains("VM total RAM unknown"), "{lines}");
            assert!(lines.contains("in-VM"), "{lines}");
        }
        let text = screen(&app, 160);
        assert!(text.contains("VM host pid:40"), "{text}");
    }
}

pub(crate) fn assert_aggregate_metrics_scope(snapshot: Snapshot, expected_in_vm: bool) {
    let mut app = App::new(snapshot.scope.clone(), 0);
    app.update(Ok(snapshot));
    let text = screen(&app, 160);
    assert_eq!(text.contains("in-VM"), expected_in_vm, "{text}");
}

#[test]
fn priority_markers_follow_changed_displayed_ranks_and_reset() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let mut app = priority_reset_app(&path);
    for mode in [0, 1] {
        app.priority_mode = mode;
        let ids = extensions::order(&app, mode == 1);
        app.extension_selected = Some(ids[0].clone());
        let initial = priority_reset_frame(&app, 160, 30, "pl6-priority-clean");
        let initial_narrow = priority_reset_frame(&app, 36, 10, "priority-clean-narrow");
        assert!(!initial.contains('*'));
        handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert!(!priority_reset_frame(&app, 160, 30, "pl6-priority-noop").contains('*'));
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        let changed = priority_reset_frame(&app, 160, 30, "pl6-priority-changed");
        assert_eq!(changed.matches('*').count(), 2);
        let narrow = priority_reset_frame(&app, 36, 10, "priority-narrow-marker");
        assert!(
            narrow.contains('*') && !narrow.contains("Extensions *"),
            "{narrow}"
        );
        assert!(!changed.contains("Extensions *") && !changed.contains("Unsaved draft"));
        for (text, clean) in [(&changed, &initial), (&narrow, &initial_narrow)] {
            let header = text
                .lines()
                .find(|line| line.contains(if mode == 0 { "ID" } else { "Rank / Name" }))
                .unwrap();
            let x = header
                .find(if mode == 0 { "ID" } else { "Rank / Name" })
                .unwrap();
            for row in text.lines().filter(|line| line.contains('*')) {
                assert_eq!(&row[x..x + 2], "* ", "{row}");
                assert_ne!(&row[x + 2..x + 3], " ", "{row}");
            }
            if mode == 0 {
                let id = &ids[0];
                let before = clean.lines().find(|line| line.contains(id)).unwrap();
                let after = text.lines().find(|line| line.contains(id)).unwrap();
                assert_eq!(before.find(id), after.find(id));
            }
        }
        handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert!(!priority_reset_frame(&app, 160, 30, "pl6-priority-undo").contains('*'));
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert!(!priority_reset_frame(&app, 160, 30, "pl6-priority-reset").contains('*'));
    }
    root.close().unwrap();
}

#[test]
fn extension_tabs_stay_visible_and_highlight_active_mode_at_both_widths() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let mut app = priority_reset_app(&root.path().join("config.json"));
    for (expected, key) in [
        (0, None),
        (1, Some(KeyCode::Tab)),
        (0, Some(KeyCode::Tab)),
        (1, Some(KeyCode::BackTab)),
    ] {
        if let Some(key) = key {
            handle_key(&mut app, key.into());
        }
        assert_eq!(app.priority_mode, expected);
        for (width, height, labels) in [
            (160, 30, ["Sources", "Ownership"]),
            (36, 10, ["Sources", "Ownership"]),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| extensions::draw(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let (y, line) = (0..height)
                .map(|y| {
                    (
                        y,
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>(),
                    )
                })
                .find(|(_, line)| labels.iter().all(|label| line.contains(label)))
                .expect("all tabs must share a visible strip");
            for (mode, label) in labels.iter().enumerate() {
                let x = line[..line.find(label).unwrap()].chars().count() as u16;
                assert_eq!(
                    buffer[(x, y)].bg == ratatui::style::Color::DarkGray,
                    mode == usize::from(expected),
                    "active tab {mode} at {width}: {line}"
                );
            }
            assert!(line.contains("Source") && line.contains("Ownership"));
        }
    }
    root.close().unwrap();
}

#[test]
fn sources_mix_core_inventory_and_unavailable_ids_without_extension_side_effects() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    std::fs::write(&path, serde_json::json!({"version":1,"settings":{},"source_priority":["missing_source","project","a_module","command","invalid_package"]}).to_string()).unwrap();
    let mut app = priority_reset_app(&path);
    let inventory_ids: Vec<_> = app
        .inventory
        .iter()
        .map(|e| e.descriptor.id.clone())
        .collect();
    assert!(
        !inventory_ids
            .iter()
            .any(|id| matches!(id.as_str(), "app_bundle" | "bin" | "command"))
    );
    let baseline = extensions::order(&app, false);
    let initial = priority_reset_frame(&app, 160, 30, "sources-mixed");
    for (id, kind, state) in [
        ("command", "Core", "Fixed"),
        ("project", "Builtin", "Enabled"),
        ("a_module", "Module", "Enabled"),
        ("missing_source", "Unknown", "Unavailable"),
        ("invalid_package", "Unknown", "Unavailable"),
    ] {
        let row = initial
            .lines()
            .find(|line| line.contains(&format!(" {id} ")))
            .unwrap();
        assert!(row.contains(state), "{row}");
        if kind == "Core" || kind == "Unknown" {
            assert!(
                !row.contains("classify")
                    && !row.contains("1.1.0")
                    && !row.contains("invented-version")
            );
        }
    }
    assert!(!initial.contains('*'));
    for id in ["command", "missing_source", "invalid_package"] {
        app.extension_selected = Some(id.into());
        handle_key(&mut app, KeyCode::Char(' ').into());
        assert!(app.builtin_draft.is_empty());
        for (width, height) in [(160, 30), (36, 10)] {
            let text = priority_reset_frame(&app, width, height, "sources-fixed-action");
            assert!(
                text.contains("State")
                    && !text.contains("Space Enable")
                    && !text.contains("Space Disable"),
                "{text}"
            );
        }
        handle_key(&mut app, KeyCode::Right.into());
        let details = priority_reset_frame(&app, 160, 30, "sources-fixed-details");
        assert!(details.contains(&format!("ID: {id}")) && details.contains("Rank:"));
        assert!(
            !details.contains("Capabilities:")
                && !details.contains("Timings")
                && !details.contains("invented-version")
        );
        assert_eq!(details.contains("fixed enabled"), id == "command");
        handle_key(&mut app, KeyCode::Left.into());
    }
    app.extension_selected = Some("a_module".into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(
        &extensions::order(&app, false)[..4],
        ["missing_source", "project", "command", "a_module"]
    );
    assert_eq!(app.extension_selected.as_deref(), Some("a_module"));
    app.extension_selected = Some("command".into());
    let core_frame = priority_reset_frame(&app, 160, 30, "sources-core-prefix");
    assert!(core_frame.contains("* command"), "{core_frame}");
    app.extension_selected = Some("a_module".into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(extensions::order(&app, false), baseline);
    assert!(!priority_reset_frame(&app, 160, 30, "sources-undo-core").contains('*'));
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(
        &extensions::order(&app, false)[..3],
        ["missing_source", "a_module", "project"]
    );
    assert_eq!(
        priority_reset_frame(&app, 160, 30, "sources-cross-type")
            .matches('*')
            .count(),
        2
    );
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(extensions::order(&app, false)[0], "a_module");
    app.extension_selected = Some("missing_source".into());
    assert!(
        priority_reset_frame(&app, 160, 30, "sources-unknown-prefix").contains("* missing_source")
    );
    assert_eq!(
        inventory_ids,
        app.inventory
            .iter()
            .map(|e| e.descriptor.id.clone())
            .collect::<Vec<_>>()
    );
    assert!(app.manual_refresh.is_none() && app.extensions.is_empty());
    root.close().unwrap();
}

#[test]
fn sources_reset_only_order_and_tabs_retain_identity_and_scroll() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let mut app = priority_reset_app(&path);
    app.extension_selected = Some("project".into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    app.extension_selected = Some("a_module".into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    let ownership = app.ownership_draft.clone();
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert!(app.priority_draft.as_ref().unwrap().is_empty());
    assert_eq!(app.ownership_draft, ownership);
    assert_eq!(app.builtin_draft.get("project"), Some(&false));
    assert_eq!(app.builtin_draft.get("a_module"), Some(&false));
    priority_reset_frame(&app, 36, 10, "sources-scroll-before");
    let offset = app.extension_tables.borrow()[0].offset();
    assert!(offset > 0);
    handle_key(&mut app, KeyCode::Tab.into());
    app.extension_selected = Some("project".into());
    handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    let sources = app.priority_draft.clone();
    handle_key(&mut app, KeyCode::Char(' ').into());
    assert_eq!(app.builtin_draft.len(), 2);
    handle_key(&mut app, KeyCode::Right.into());
    let details = priority_reset_frame(&app, 160, 30, "ownership-dimension-details");
    assert!(details.contains("Ownership: main placement"));
    assert!(!details.contains("Capabilities:") && !details.contains("Timings"));
    handle_key(&mut app, KeyCode::Left.into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.priority_draft, sources);
    assert!(app.ownership_draft.as_ref().unwrap().is_empty());
    priority_reset_frame(&app, 36, 10, "ownership-scroll");
    let owner_offset = app.extension_tables.borrow()[1].offset();
    handle_key(&mut app, KeyCode::BackTab.into());
    assert_eq!(app.extension_selected.as_deref(), Some("a_module"));
    let text = priority_reset_frame(&app, 36, 10, "sources-scroll-restored");
    assert_eq!(app.extension_tables.borrow()[0].offset(), offset);
    assert!(text.contains("* a_module") && text.contains("State"));
    handle_key(&mut app, KeyCode::BackTab.into());
    assert_eq!(app.extension_selected.as_deref(), Some("project"));
    priority_reset_frame(&app, 36, 10, "ownership-scroll-restored");
    assert_eq!(app.extension_tables.borrow()[1].offset(), owner_offset);
    handle_key(&mut app, KeyCode::Tab.into());
    handle_key(&mut app, KeyCode::Esc.into());
    assert!(app.builtin_draft.is_empty() && !path.exists());
    root.close().unwrap();
}

#[test]
fn settings_details_keep_drafts_and_expose_complete_descriptions_at_both_sizes() {
    use crate::settings::{Config, edit::Settings};
    for (width, height) in [(100, 30), (36, 10)] {
        let mut registry = crate::extensions::settings_registry().unwrap();
        registry
            .register_extension(
                "zzz",
                vec![
                    serde_json::from_value(serde_json::json!({
                        "key":"extensions.zzz.secret", "type":"string", "default":"hidden-default",
                        "options":["hidden-default", "hidden-choice"], "sensitive":true,
                        "description":format!("{}Description end", "Long description ".repeat(100)),
                        "provider":"zzz"
                    }))
                    .unwrap(),
                ],
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
        let render = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, app, &mut TableState::default()))
                .unwrap();
            (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        handle_key(&mut app, KeyCode::Char('s').into());
        handle_key(&mut app, KeyCode::End.into());
        handle_key(&mut app, KeyCode::Enter.into());
        let draft = app.settings.as_ref().unwrap().changes.clone();
        let before = render(&app);
        handle_key(&mut app, KeyCode::PageDown.into());
        assert_eq!(render(&app), before, "list PageDown must not open details");
        for back in [KeyCode::Left, KeyCode::Esc] {
            handle_key(&mut app, KeyCode::Right.into());
            let top = render(&app);
            let rows: Vec<_> = top.lines().collect();
            let border = rows
                .iter()
                .position(|line| line.contains("┌ Settings details"))
                .unwrap();
            let footer = rows
                .iter()
                .position(|line| line.contains("↑↓ Scroll"))
                .unwrap();
            let body_height = footer - border - 1;
            assert!(!rows[footer..].iter().any(|line| line.contains("Pg")));
            handle_key(&mut app, KeyCode::PageDown.into());
            let page = render(&app);
            handle_key(&mut app, KeyCode::Left.into());
            handle_key(&mut app, KeyCode::Right.into());
            render(&app);
            for _ in 0..body_height {
                handle_key(&mut app, KeyCode::Down.into());
            }
            assert_eq!(render(&app), page);
            handle_key(&mut app, KeyCode::PageUp.into());
            assert_eq!(render(&app), top);
            let mut pages = String::new();
            for _ in 0..20 {
                pages.push_str(&render(&app));
                handle_key(&mut app, KeyCode::PageDown.into());
            }
            for expected in [
                "Key:",
                "Default: [masked]",
                "Constraints: [masked]",
                "Value: [masked]",
                "Description end",
                "Back",
                "Scroll",
            ] {
                assert!(pages.contains(expected), "missing {expected}: {pages}");
            }
            assert!(!pages.contains("hidden-default") && !pages.contains("hidden-choice"));
            let bottom = render(&app);
            handle_key(&mut app, KeyCode::Up.into());
            assert_ne!(render(&app), bottom);
            for _ in 0..20 {
                handle_key(&mut app, KeyCode::PageUp.into());
            }
            assert!(render(&app).contains("Label:"), "PageUp stays in details");
            handle_key(&mut app, back.into());
            assert_eq!(render(&app), before);
            assert_eq!(app.settings.as_ref().unwrap().changes, draft);
        }
        handle_key(&mut app, KeyCode::Home.into());
        for _ in 0..3 {
            handle_key(&mut app, KeyCode::Down.into());
        }
        let summary = render(&app);
        assert!(summary.contains("Entry argv"));
        assert!(!summary.contains("once per round"));
        if width == 100 {
            assert!(summary.contains("Identify entry work from your processes' launch arguments."));
        }
        handle_key(&mut app, KeyCode::Right.into());
        let mut pages = String::new();
        for _ in 0..25 {
            pages.push_str(&render(&app));
            handle_key(&mut app, KeyCode::Down.into());
        }
        let words = pages
            .replace('│', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(words.contains("once per round"), "{pages}");
        handle_key(&mut app, KeyCode::Left.into());
        handle_key(&mut app, KeyCode::Down.into());
        if width == 100 {
            assert!(
                render(&app).contains(
                    "Identify scripts and modules from your processes' launch arguments."
                )
            );
        }
        handle_key(&mut app, KeyCode::Right.into());
        let mut pages = String::new();
        for _ in 0..20 {
            pages.push_str(&render(&app));
            handle_key(&mut app, KeyCode::PageDown.into());
        }
        assert!(
            pages
                .replace('│', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("own python processes once per round")
        );
    }
}

#[test]
fn main_footer_ends_at_last_row_and_separates_actions_without_clipping() {
    for (width, height) in [(36, 10), (36, 30), (79, 30), (80, 30), (100, 30), (160, 30)] {
        let app = app();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
            .unwrap();
        let rows: Vec<String> = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect()
            })
            .collect();
        assert!(
            !rows[usize::from(height - 1)].trim().is_empty(),
            "unused bottom row: {rows:?}"
        );
        let footer = rows.join("\n");
        for group in ["→ Open", "s Settings", "? Help", "q Quit", "r refresh"] {
            assert_eq!(footer.matches(group).count(), 1, "{footer}");
        }
        assert!(footer.contains("Path") && footer.contains("group:applications"));
        for line in rows.iter().filter(|line| line.contains(" · ")) {
            assert!(!line.trim().ends_with('·'), "{footer}");
        }
        if width >= 80 {
            assert!(
                footer.contains("→ Open · / Search · s Settings · e Extensions"),
                "{footer}"
            );
        }
    }
}

#[test]
fn hint_groups_pack_atomically_by_cell_width_without_dangling_separators() {
    use ratatui::text::Line;
    let groups = ["Tab Switch", "鍵 動作", "e\u{301} 👩‍💻", "Esc Back"];
    assert_eq!(Line::raw(groups[1]).width(), 7);
    assert_eq!(Line::raw(groups[2]).width(), 4);
    let joined = groups.join(" · ");
    let boundary = Line::raw(&joined).width() as u16;
    assert_eq!(view::hint_lines(&groups, boundary), vec![Line::raw(joined)]);
    assert_eq!(view::hint_lines(&groups, boundary - 1).len(), 2);
    for width in [0, 1, 7, 10, 20, boundary] {
        let lines = view::hint_lines(&groups, width);
        assert!(lines.len() <= groups.len());
        let actual: Vec<_> = lines.iter().map(ToString::to_string).collect();
        assert_eq!(
            actual
                .iter()
                .flat_map(|line| line.split(" · "))
                .collect::<Vec<_>>(),
            groups
        );
        for line in &lines {
            let text = line.to_string();
            assert!(!text.starts_with('·') && !text.ends_with('·'));
            assert!(line.width() <= usize::from(width) || groups.contains(&text.as_str()));
        }
    }
    assert!(view::hint_lines(&["", ""], 10).is_empty());
    let mut terminal = Terminal::new(TestBackend::new(3, 2)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                ratatui::widgets::Paragraph::new(view::hint_lines(&["Tab Switch", "Esc Back"], 3)),
                frame.area(),
            );
        })
        .unwrap();
    let clipped: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(clipped, "TabEsc");
}

#[test]
fn extension_footer_uses_exact_inner_width_and_finishes_above_border() {
    use ratatui::text::Line;
    let mut app = app();
    app.inventory =
        crate::extensions::inventory::list(&crate::extensions::default_settings().unwrap(), true);
    extensions::open(&mut app);
    app.extension_selected = Some("project".into());
    let full = "Tab Switch · Space Disable · Shift↑↓ Move · → Details · Ctrl-S Save · Ctrl-R Reset order · Esc Cancel";
    let boundary = Line::raw(full).width() as u16 + 6;
    for (width, height, expected_rows) in [
        (36, 10, 4),
        (boundary - 1, 20, 2),
        (boundary, 20, 1),
        (160, 20, 1),
    ] {
        let text = priority_reset_frame(&app, width, height, "hint-boundary");
        let rows: Vec<_> = text.lines().collect();
        let first = rows
            .iter()
            .position(|row| row.contains("Tab Switch"))
            .unwrap();
        let last = rows
            .iter()
            .position(|row| row.contains("Esc Cancel"))
            .unwrap();
        assert_eq!(last - first + 1, expected_rows, "{text}");
        assert!(rows[last + 1].contains('└'), "{text}");
        for group in [
            "Tab Switch",
            "Space Disable",
            "Shift↑↓ Move",
            "Ctrl-S Save",
            "Ctrl-R Reset",
            "Esc Cancel",
        ] {
            assert_eq!(text.matches(group).count(), 1, "{text}");
        }
        assert!(
            rows.iter()
                .any(|row| row.contains("project") && row.contains("Enabled")),
            "{text}"
        );
        if expected_rows == 1 {
            assert!(rows[first].contains(full), "{text}");
        }
    }
}

#[test]
fn search_and_loading_hints_keep_context_and_last_terminal_row() {
    let mut app = App::new("group:all".into(), 2);
    for editing in [false, true] {
        app.editing = editing;
        app.snapshot = None;
        app.collecting = true;
        let mut terminal = Terminal::new(TestBackend::new(36, 10)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..10)
            .map(|y| (0..36).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let text = rows.join("\n");
        assert!(!rows[9].trim().is_empty(), "{text}");
        assert!(text.contains("Path"), "{text}");
        if editing {
            assert!(rows[9].contains("Enter Accept · Esc Clear"), "{text}");
            assert!(
                !text.contains("q Quit") && !text.contains("r refresh"),
                "{text}"
            );
        } else {
            for group in [
                "→ Open",
                "/ Find",
                "s Settings",
                "e Exts",
                "? Help",
                "q Quit",
                "r refresh",
            ] {
                assert_eq!(text.matches(group).count(), 1, "{text}");
            }
        }
    }
}

#[test]
fn detail_pages_follow_rendered_height_and_clamp_after_resize() {
    fn render(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, app, &mut TableState::default()))
            .unwrap();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }
    for family in 0..3 {
        let mut app = app();
        if family == 0 {
            select(&mut app, "pid:11");
            app.open_details();
            let mut source = command_snapshot(11);
            change(&mut source.root, 11, |process| {
                process.cwd = Some(format!("/{}END", "long-path/".repeat(300)).into());
            });
            app.update(Ok(source));
        } else if family == 1 {
            let mut node = Node::group(
                "group:resource".into(),
                "Resource".into(),
                vec![],
                Grouping::Category,
            );
            node.kind = "resource";
            node.resource = Some(crate::model::Resource {
                provider: "fixture".into(),
                id: "resource".into(),
                kind: "storage.volume".into(),
                label: "Resource".into(),
                path: Some(format!("/{}END", "long-path/".repeat(300)).into()),
                metrics: vec![],
            });
            app.details = Some(details::Details::new(node));
        } else {
            app.inventory = crate::extensions::inventory::list(
                &crate::extensions::default_settings().unwrap(),
                true,
            );
            handle_key(&mut app, KeyCode::Char('e').into());
            app.extension_selected = Some("node".into());
            app.priority_error = Some(format!("{}END", "long-error ".repeat(300)));
            handle_key(&mut app, KeyCode::Right.into());
        }
        assert!(!handle_key(&mut app, KeyCode::PageDown.into()));
        for (width, height) in [(100, 30), (36, 10), (80, 18)] {
            let lines = render(&app, width, height);
            let footer = lines
                .iter()
                .position(|line| line.contains("↑↓ Scroll"))
                .unwrap();
            assert!(lines[footer..].iter().all(|line| !line.contains("Pg")));
            let body_height = if family == 2 {
                let caption = lines
                    .iter()
                    .position(|line| line.contains("Sources"))
                    .unwrap();
                footer - caption - 1
            } else {
                let title = lines
                    .iter()
                    .position(|line| line.contains("details ·"))
                    .unwrap();
                let position = lines
                    .iter()
                    .position(|line| line.contains("Lines "))
                    .unwrap();
                position - title - 2
            };
            handle_key(&mut app, KeyCode::Home.into());
            render(&app, width, height);
            handle_key(&mut app, KeyCode::PageDown.into());
            let page = render(&app, width, height);
            handle_key(&mut app, KeyCode::Home.into());
            for _ in 0..body_height {
                handle_key(&mut app, KeyCode::Down.into());
            }
            assert_eq!(
                render(&app, width, height),
                page,
                "family {family}, {width}x{height}"
            );
            handle_key(&mut app, KeyCode::PageUp.into());
            let top = render(&app, width, height);
            handle_key(&mut app, KeyCode::Up.into());
            assert_eq!(render(&app, width, height), top);
            handle_key(&mut app, KeyCode::End.into());
            let bottom = render(&app, width, height);
            handle_key(&mut app, KeyCode::Home.into());
            let mut pages = String::new();
            for _ in 0..200 {
                let current = render(&app, width, height);
                pages.push_str(&current.join("\n"));
                if current == bottom {
                    break;
                }
                handle_key(&mut app, KeyCode::PageDown.into());
            }
            assert_eq!(render(&app, width, height), bottom);
            assert!(pages.contains("END"), "family {family}, {width}x{height}");
            handle_key(&mut app, KeyCode::Down.into());
            assert_eq!(render(&app, width, height), bottom);
            handle_key(&mut app, KeyCode::PageUp.into());
            let previous = render(&app, width, height);
            handle_key(&mut app, KeyCode::End.into());
            for _ in 0..body_height {
                handle_key(&mut app, KeyCode::Up.into());
            }
            assert_eq!(render(&app, width, height), previous);
        }
        render(&app, 36, 10);
        handle_key(&mut app, KeyCode::End.into());
        render(&app, 36, 10);
        let bottom = render(&app, 100, 30);
        handle_key(&mut app, KeyCode::Up.into());
        assert_ne!(render(&app, 100, 30), bottom);
    }
}

#[test]
fn extension_core_unknown_and_ownership_details_clamp_at_document_end() {
    let mut app = app();
    handle_key(&mut app, KeyCode::Char('e').into());
    for (mode, id) in [(0, "app_bundle"), (0, "unknown-fixture"), (1, "project")] {
        app.priority_mode = mode;
        if mode == 0 {
            app.priority_draft = Some(vec![id.into()]);
        }
        app.extension_selected = Some(id.into());
        handle_key(&mut app, KeyCode::Right.into());
        for (width, height) in [(36, 10), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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
            assert!(text.contains(&format!("ID: {id}")), "{text}");
            assert!(
                text.contains("↑↓ Scroll") && text.contains("←/Esc Back") && !text.contains("Pg")
            );
            handle_key(&mut app, KeyCode::End.into());
            let end = app.extension_details_scroll.get();
            handle_key(&mut app, KeyCode::PageDown.into());
            assert_eq!(app.extension_details_scroll.get(), end);
            handle_key(&mut app, KeyCode::Home.into());
            handle_key(&mut app, KeyCode::PageUp.into());
            assert_eq!(app.extension_details_scroll.get(), 0);
        }
        handle_key(&mut app, KeyCode::Left.into());
        assert!(!app.extension_details && app.extensions_page);
    }
}

#[test]
fn bundled_source_details_show_metadata_without_forcing_columns() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let mut app = priority_reset_app(&root.path().join("config.json"));
    app.extension_selected = Some("project".into());
    for (width, height) in [(160, 30), (36, 10)] {
        let list = priority_reset_frame(&app, width, height, "bundled-list");
        assert!(
            !list.contains("Type") && !list.contains("Version") && !list.contains("Author"),
            "{list}"
        );
    }
    handle_key(&mut app, KeyCode::Right.into());
    let detail = priority_reset_frame(&app, 160, 30, "bundled-detail");
    assert!(detail.contains("ID: project") && detail.contains("Capabilities: classify"));
    for removed in [
        "Source:",
        "Origin:",
        "Requires:",
        "Compatible:",
        "Path:",
        "Author:",
        "Version:",
        "Provenance:",
    ] {
        assert!(!detail.contains(removed), "{detail}");
    }
    root.close().unwrap();
}

pub(crate) fn assert_extension_category(snapshot: Snapshot, category: &str, owner: &str) -> String {
    let mut app = App::new("group:all".into(), usize::MAX);
    app.update(Ok(snapshot));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, &app, &mut TableState::default()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..30)
        .map(|y| {
            (0..120)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains(category), "{text}");
    assert!(text.contains(owner), "{text}");
    assert_eq!(app.snapshot.as_ref().unwrap().root.totals.processes, 1);
    text
}

#[test]
fn review_r2_linked_enablement_ignores_unknown_native_intent_key() {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    let path = root.path().join("config.json");
    let original = br#"{"version":1,"settings":{"extensions.project.enabled":false}}"#;
    std::fs::write(&path, original).unwrap();
    let mut app = priority_reset_app(&path);
    app.extension_selected = Some("project".into());
    assert!(
        app.inventory
            .iter()
            .find(|e| e.descriptor.id == "project")
            .unwrap()
            .explicit_enabled
    );
    handle_key(&mut app, KeyCode::Right.into());
    assert!(
        priority_reset_frame(&app, 160, 40, "review-r2-linked-enabled").contains("State: Enabled")
    );
    handle_key(&mut app, KeyCode::Left.into());
    handle_key(&mut app, KeyCode::Char(' ').into());
    assert_eq!(app.builtin_draft.get("project"), Some(&false));
    handle_key(&mut app, KeyCode::Char(' ').into());
    assert!(app.builtin_draft.is_empty());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.settings.as_ref().unwrap().applied.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    handle_key(&mut app, KeyCode::Char(' ').into());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(app.extensions_page && app.priority_error.is_none() && app.builtin_draft.is_empty());
    let popup = app.settings.as_ref().unwrap();
    let applied = popup.applied.as_ref().unwrap();
    assert!(applied.is_disabled("project"));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["settings"]["extensions.project.enabled"], false);
    assert_eq!(saved["disabled_builtins"], serde_json::json!(["project"]));
    let cancel = crate::snapshot::Cancellation::default();
    let context = crate::extensions::Context::new(applied, &cancel);
    let facts = crate::extensions::host::Facts::new(&[]);
    assert!(
        crate::extensions::dispatch(
            &crate::extensions::Builtin::Project,
            &context,
            crate::extensions::Request::Classify(&facts)
        )
        .is_err()
    );
    assert!(
        context
            .report
            .lock()
            .unwrap()
            .iter()
            .any(|record| record.provider == "project" && record.result == "skipped")
    );
    root.close().unwrap();
}

#[test]
fn sources_save_all_disabled_restarts_with_core_categories_and_totals() {
    let owner = tempfile::tempdir().unwrap();
    let path = owner.path().canonicalize().unwrap().join("config.json");
    let mut app = priority_reset_app(&path);
    app.inventory =
        crate::extensions::inventory::list(&crate::extensions::default_settings().unwrap(), true);
    let ids: Vec<_> = app
        .inventory
        .iter()
        .map(|e| e.descriptor.id.clone())
        .collect();
    for id in &ids {
        app.extension_selected = Some(id.clone());
        handle_key(&mut app, KeyCode::Char(' ').into());
    }
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(
        app.priority_error.is_none() && app.builtin_draft.is_empty(),
        "{:?}",
        app.priority_error
    );
    assert!(app.settings.as_ref().unwrap().applied.is_some());
    let config = crate::settings::Config::load_path(Some(&path), true).unwrap();
    let settings = crate::extensions::settings_registry()
        .unwrap()
        .resolve(&config, &Default::default())
        .unwrap();
    assert!(ids.iter().all(|id| settings.is_disabled(id)));
    let mut restarted = priority_reset_app(&path);
    restarted.inventory = crate::extensions::inventory::list(&settings, true);
    assert!(
        restarted
            .inventory
            .iter()
            .all(|e| !e.explicit_enabled && !e.descriptor.enabled)
    );
    for id in &ids {
        restarted.extension_selected = Some(id.clone());
        handle_key(&mut restarted, KeyCode::Right.into());
        assert!(
            priority_reset_frame(&restarted, 120, 30, "all-disabled").contains("State: Disabled")
        );
        handle_key(&mut restarted, KeyCode::Left.into());
    }
    let processes = [
        "/Applications/Demo.app/Contents/MacOS/demo",
        "/usr/libexec/demo",
        "/opt/other/demo",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, executable)| Process {
        pid: i as u32 + 1,
        started_unix_us: Some(i as u64 + 1),
        name: "demo".into(),
        executable: Some(executable.into()),
        resident_bytes: Some(100),
        ..Default::default()
    })
    .collect();
    let cancel = Cancellation::default();
    let context = crate::extensions::Context::new(&settings, &cancel);
    let root = tree::build_with_providers(processes, &context, &[]).unwrap();
    assert_eq!(root.totals.processes, 3);
    assert_eq!(root.totals.resident_bytes, 300);
    for category in ["applications", "system", "unclassified"] {
        assert_eq!(
            tree::select(&root, &format!("group:{category}"))
                .unwrap()
                .totals
                .processes,
            1
        );
    }
    tree::validate_references(&root).unwrap();
    assert_eq!(root.children.len(), 3);
    let owned = owner.path().to_path_buf();
    owner.close().unwrap();
    assert!(!owned.exists());
}
