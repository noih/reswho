use super::state::App;
use crate::{model::Node, output};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{
        Block, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap,
    },
};

pub(super) fn safe(value: &str) -> String {
    value.chars().flat_map(|ch| {
        if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}') {
            format!("\\u{{{:x}}}", ch as u32).chars().collect::<Vec<_>>()
        } else { vec![ch] }
    }).collect()
}

fn metrics(node: &Node) -> [String; 3] {
    if let Some(process) = &node.process {
        return [
            output::percent(process.cpu_percent),
            output::memory(process.resident_bytes),
            output::uptime(process.uptime_seconds),
        ];
    }
    if let Some(container) = &node.container {
        if container.is_inactive() {
            return ["-".into(), "-".into(), "-".into()];
        }
        return [
            output::percent(container.cpu_percent),
            output::memory(container.memory_bytes),
            output::uptime(container.uptime_seconds),
        ];
    }
    if let Some(totals) = &node.container_totals {
        return output::containers::summary_metrics(totals);
    }
    [
        output::group_cpu(&node.totals),
        output::group_memory(&node.totals),
        output::group_uptime(&node.totals),
    ]
}

fn state_label(node: &Node) -> String {
    if let Some(container) = &node.container {
        return container.state.clone().unwrap_or("?".into());
    }
    if let Some(process) = &node.process {
        return output::ports(process.network.as_ref());
    }
    if let Some(totals) = &node.container_totals {
        let active = if totals.activity_unknown > 0 || !totals.inventory_complete {
            if totals.active_containers == 0 {
                "?".into()
            } else {
                format!(">={}", totals.active_containers)
            }
        } else {
            totals.active_containers.to_string()
        };
        let count = if totals.inventory_complete {
            totals.containers.to_string()
        } else if totals.containers == 0 {
            "?".into()
        } else {
            format!(">={}", totals.containers)
        };
        return format!("{active} / {count} ctr");
    }
    format!("{} proc", node.totals.processes)
}

fn evidence_path(value: &str) -> Option<&str> {
    for prefix in [
        "Application bundle: ",
        "Git root from working directory: ",
        "repo ",
        "exe ",
        "container project path ",
        "compose ",
    ] {
        if let Some(path) = value.strip_prefix(prefix) {
            return Some(path);
        }
    }
    let project = value.strip_prefix("project ")?;
    if project.starts_with('/') {
        Some(project)
    } else {
        project.split_once(' ').map(|(_, path)| path)
    }
}

pub(super) fn selected_paths(app: &App) -> String {
    let Some(node) = app.node(app.selected) else {
        return "Path —".into();
    };
    let mut paths = Vec::new();
    if let Some(process) = &node.process {
        for (label, path) in [("cwd", &process.cwd), ("exe", &process.executable)] {
            if let Some(path) = path {
                paths.push(format!("{label} {}", path.display()));
            }
        }
    } else if let Some(container) = &node.container {
        if let Some(path) = container
            .project
            .as_ref()
            .and_then(|project| project.path.as_ref())
        {
            paths.push(format!("project {}", path.display()));
        }
        for path in &container.candidates {
            paths.push(format!("candidate {}", path.display()));
        }
        for mount in container.mounts.iter().filter(|mount| mount.kind == "bind") {
            paths.push(format!(
                "bind {} → {}",
                mount.source.display(),
                mount.destination.display()
            ));
        }
    } else {
        let mut evidence: Vec<_> = node.evidence.iter().collect();
        if matches!(node.grouping, Some(crate::model::Grouping::App)) {
            evidence.extend(
                node.children
                    .iter()
                    .filter(|child| child.process.is_none())
                    .flat_map(|child| &child.evidence),
            );
        }
        for path in evidence
            .into_iter()
            .filter_map(|value| evidence_path(value))
        {
            if std::path::Path::new(path).is_absolute() {
                let value = format!("Path {path}");
                if !paths.contains(&value) {
                    paths.push(value);
                }
            }
        }
    }
    if paths.is_empty() {
        "Path —".into()
    } else {
        safe(&paths.join("  ·  "))
    }
}

fn selected_issue(app: &App) -> Option<&str> {
    let node = app.node(app.selected)?;
    node.process
        .as_ref()
        .and_then(|process| {
            process.unavailable.first().or_else(|| {
                process
                    .network
                    .as_ref()
                    .and_then(|network| network.unavailable.first())
            })
        })
        .or_else(|| {
            node.container
                .as_ref()
                .and_then(|container| container.unavailable.first())
        })
        .or_else(|| {
            node.runtime
                .as_ref()
                .and_then(|runtime| runtime.unavailable.first())
        })
        .map(crate::model::Diagnostic::as_str)
        .or_else(|| {
            node.evidence
                .iter()
                .find(|value| {
                    value.starts_with("Runtime unavailable: ")
                        || value.starts_with("Repository metadata unavailable: ")
                })
                .map(String::as_str)
        })
}

pub(super) fn draw(frame: &mut Frame, app: &App, table: &mut TableState) {
    let area = frame.area();
    if area.width < 36 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("reswho: enlarge terminal (36x10 minimum). q quits.")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let [header, toolbar, tree_area, path_area, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(area);
    let status = if app.error.is_some() {
        " | error"
    } else if app.paused {
        " | paused"
    } else {
        ""
    };
    let summary =
        app.snapshot
            .as_ref()
            .map_or("Host CPU —  RAM —  swap —".into(), |snapshot| {
                let processes = if snapshot.scope == "group:all" {
                    format!("  |  {} processes", snapshot.root.totals.processes)
                } else {
                    String::new()
                };
                format!(
                    "Host CPU {}  RAM {}/{}  swap {}{processes}  |  sample {}ms  collection {}ms",
                    output::percent(snapshot.system.cpu_percent),
                    output::memory(snapshot.system.memory_used_bytes),
                    output::memory(snapshot.system.memory_total_bytes),
                    output::memory(snapshot.system.swap_used_bytes),
                    snapshot.sample_ms,
                    snapshot.collection_ms
                )
            });
    frame.render_widget(Paragraph::new(summary), header);
    if let Some(details) = &app.details {
        details.draw(
            frame,
            Rect::new(
                area.x,
                header.bottom(),
                area.width,
                area.height.saturating_sub(header.height),
            ),
            app.paused,
            app.refresh_hint(),
        );
        return;
    }
    let filter = if app.active_only {
        "active (+ unknown)"
    } else {
        "all"
    };
    let search = if app.editing {
        format!("/{}_", safe(&app.query))
    } else if app.query.is_empty() {
        "/ search".into()
    } else {
        format!("/{}", safe(&app.query))
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Sort {}{status} | Containers {filter} | {search}",
            app.sort.label()
        ))
        .style(Style::default().fg(Color::Yellow)),
        toolbar,
    );
    let wide = tree_area.width >= 85;
    let rows = app.rows.iter().enumerate().map(|(index, row)| {
        let indent = "  ".repeat(row.depth.min(12));
        let arrow = if row.has_children {
            if app.is_open(row) { "v" } else { ">" }
        } else {
            " "
        };
        if row.unreadable > 0 {
            return Row::new(vec![Cell::from(format!(
                "{indent}  Unreadable ({} processes)",
                row.unreadable
            ))]);
        }
        let node = app.node(index).expect("row path in snapshot");
        let suffix = if node.process.is_some() {
            format!(" {}", node.reference)
        } else {
            String::new()
        };
        let name = format!("{indent}{arrow} {}{suffix}", safe(&node.name));
        let [cpu, ram, age] = metrics(node);
        let mut cells = vec![
            Cell::from(name),
            Cell::from(cpu.split(" (").next().unwrap_or(&cpu).to_owned()),
            Cell::from(ram.split(" (").next().unwrap_or(&ram).to_owned()),
            Cell::from(age),
        ];
        if wide {
            cells.push(Cell::from(safe(&state_label(node))));
        }
        Row::new(cells).style(
            if node
                .container
                .as_ref()
                .is_some_and(|container| container.is_inactive())
            {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            },
        )
    });
    let mut widths = vec![
        Constraint::Min(12),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(8),
    ];
    let mut headings = vec!["Resource", "CPU/core", "Memory", "Age"];
    if wide {
        widths.push(Constraint::Length(14));
        headings.push("State / Ports");
    }
    let title = if app.active_only || !app.query.is_empty() {
        " Resources (totals include hidden rows) "
    } else {
        " Resources (host / containers separate) "
    };
    let widget = Table::new(rows, widths)
        .header(Row::new(headings).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ")
        .highlight_spacing(HighlightSpacing::Always);
    table.select((!app.rows.is_empty()).then_some(app.selected));
    frame.render_stateful_widget(widget, tree_area, table);
    if app.rows.is_empty() && app.snapshot.is_some() {
        frame.render_widget(
            Paragraph::new("No matches"),
            Rect::new(
                tree_area.x + 1,
                tree_area.y + 2,
                tree_area.width.saturating_sub(2),
                tree_area.height.saturating_sub(3),
            ),
        );
    }
    if app.snapshot.is_none() {
        let message = if app.collecting {
            format!(
                "{} Loading resources...",
                ["|", "/", "-", "\\"][app.loading_frame % 4]
            )
        } else if app.error.is_some() {
            "Unable to load resources".into()
        } else {
            "Loading paused".into()
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Cyan)),
            Rect::new(
                tree_area.x + 1,
                tree_area.y + 2 + tree_area.height.saturating_sub(4) / 2,
                tree_area.width.saturating_sub(2),
                1,
            ),
        );
    }
    frame.render_widget(
        Paragraph::new(selected_paths(app))
            .scroll((0, app.path_scroll))
            .style(Style::default().fg(Color::Cyan)),
        path_area,
    );
    let diagnostic = app
        .error
        .as_ref()
        .map(|error| {
            format!(
                "{}: {}",
                if app.snapshot.is_some() {
                    "STALE"
                } else {
                    "Error"
                },
                safe(error)
            )
        })
        .or_else(|| selected_issue(app).map(|issue| format!("! {}", safe(issue))))
        .or_else(|| {
            app.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .warnings
                    .iter()
                    .find(|warning| !crate::collect::COLLECTION_NOTES.contains(&warning.as_str()))
                    .map(|warning| format!("! {}", safe(warning.as_str())))
            })
        })
        .unwrap_or_else(|| {
            app.node(app.selected).map_or(String::new(), |node| {
                node.grouping
                    .as_ref()
                    .map(|group| group.label())
                    .filter(|label| !label.is_empty())
                    .map_or_else(
                        || node.reference.clone(),
                        |label| format!("{label} · {}", node.reference),
                    )
            })
        });
    let refresh = if app.refresh_hint().is_empty() {
        String::new()
    } else {
        format!("{}  ", app.refresh_hint())
    };
    frame.render_widget(Paragraph::new(vec![Line::styled(diagnostic, Style::default().fg(Color::Yellow)), Line::raw(format!("j/k move  {refresh}enter toggle  d details  c/m/u/n sort  / search  a active  p pause  ? help  q quit"))]), footer);
    if app.help {
        help(frame, area, app);
    }
}

fn help(frame: &mut Frame, area: Rect, app: &App) {
    let popup = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    frame.render_widget(Clear, popup);
    let mut text = "reswho controls\n\nUp/Down or k/j: move    Left/h: collapse or parent\nRight/l: expand or child    Enter/Space: toggle\nd: process details    Esc: back to list\nDetails: j/k/PgUp/PgDn scroll, r refresh, p pause\nc: CPU    m: memory    u: uptime (descending within siblings)\nn: name (natural order, case-insensitive; node2 before node10)\n/: search name, PID, ref or path    Enter: accept    Esc: clear\na: all / active containers (unknown retained)\n[ / ]: scroll path    PgUp/PgDn: move 10 rows\nr: refresh once (keeps pause); reuse pending collection\np: pause automatic refresh\nq / Ctrl-C: quit    ? / Esc: close help\n\nRefresh: 2 seconds after each completed collection; one query at a time.\nHost CPU: whole-machine usage (0-100%). CPU/core: 100% = one core.\nRows can exceed 100%; >= means a partial total with unreadable values.\nAge is process uptime or oldest group member, including idle time.\nHost RSS and container memory are separate scopes; never add both.\nFilters affect visible rows, not aggregate totals. Unknown is not zero.\nSearch reveals matching branches without changing saved expansion.\n".to_owned();
    if let Some(error) = &app.error {
        text.push_str(&format!("\nSTALE: {}\n", safe(error)));
    }
    if let Some(snapshot) = &app.snapshot {
        for warning in snapshot
            .warnings
            .iter()
            .filter(|warning| !crate::collect::COLLECTION_NOTES.contains(&warning.as_str()))
        {
            text.push_str(&format!("\n! {}", safe(warning.as_str())));
        }
    }
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title(" Help "))
            .wrap(Wrap { trim: false }),
        popup,
    );
}
