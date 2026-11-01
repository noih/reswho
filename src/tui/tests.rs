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
        assert!(text.contains("reswho"));
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
    assert!(error.contains("Unable to load resources"));
    assert!(error.contains("collection failed"));
    assert!(!error.contains("Loading resources..."));
    app.update(Ok(snapshot()));
    let loaded = render(&app, &mut terminal, &mut table);
    assert!(loaded.contains("Resources"));
    assert!(!loaded.contains("Loading resources..."));
}

#[test]
fn worker_cancels_and_joins_owned_collection() {
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let start_flag = started.clone();
    let finish_flag = finished.clone();
    let worker = Worker::with_collector(move |cancel| {
        start_flag.store(true, Ordering::SeqCst);
        let result = cancel.wait(Duration::from_secs(30));
        finish_flag.store(true, Ordering::SeqCst);
        result
            .map(|_| snapshot())
            .map_err(|error| error.to_string())
    });
    assert!(worker.request());
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
