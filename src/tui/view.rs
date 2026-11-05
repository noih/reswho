use super::state::App;
use crate::{model::Node, output, sort::Sort};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Padding, Paragraph, Row, Table, TableState, Wrap,
    },
};

pub(super) fn safe(value: &str) -> String {
    value.chars().flat_map(|ch| {
        if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}') {
            format!("\\u{{{:x}}}", ch as u32).chars().collect::<Vec<_>>()
        } else { vec![ch] }
    }).collect()
}

pub(super) fn hint_lines(groups: &[&str], width: u16) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::text::Line;
    let mut lines = Vec::new();
    let mut line = String::new();
    for group in groups.iter().filter(|group| !group.is_empty()) {
        if !line.is_empty()
            && Line::raw(&line).width() + 3 + Line::raw(*group).width() > usize::from(width)
        {
            lines.push(Line::raw(std::mem::take(&mut line)));
        }
        if !line.is_empty() {
            line.push_str(" · ");
        }
        line.push_str(group);
    }
    if !line.is_empty() {
        lines.push(Line::raw(line));
    }
    lines
}

fn clipped_summary(value: &str, width: usize) -> String {
    use ratatui::text::Span;
    if Span::raw(value).width() <= width {
        return value.to_owned();
    }
    let mut clipped = String::new();
    let mut used = 0;
    for grapheme in Span::raw(value).styled_graphemes(Style::default()) {
        used += Span::raw(grapheme.symbol).width();
        if used >= width {
            break;
        }
        clipped.push_str(grapheme.symbol);
    }
    format!("{clipped}…")
}

fn metrics(node: &Node) -> [String; 3] {
    if matches!(node.grouping, Some(crate::model::Grouping::Resource)) || node.resource.is_some() {
        return ["-".into(), "-".into(), "-".into()];
    }
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
        if node.totals.processes > 0 {
            return ["-".into(), "-".into(), "-".into()];
        }
        return output::containers::summary_metrics(totals);
    }
    [
        output::group_cpu(&node.totals),
        output::group_memory(&node.totals),
        output::group_uptime(&node.totals),
    ]
}

fn state_label(node: &Node) -> String {
    if matches!(node.grouping, Some(crate::model::Grouping::Resource)) {
        return "resources".into();
    }
    if let Some(resource) = &node.resource {
        return resource
            .metrics
            .iter()
            .take(2)
            .map(output::metric_text)
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if let Some(container) = &node.container {
        return format!(
            "{}{}",
            if container.metrics_scope.as_deref() == Some("in_vm") {
                "in-VM "
            } else {
                ""
            },
            container.state.as_deref().unwrap_or("?")
        );
    }
    if let Some(process) = &node.process {
        let ports = output::ports(process.network.as_ref());
        return if process.roles.is_empty() {
            ports
        } else {
            format!(
                "{} · {}",
                ports,
                process
                    .roles
                    .iter()
                    .map(|role| format!("{}:{}", role.provider, role.label))
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        };
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
        let scope = if output::metrics_scope(node) == Some("in_vm") {
            "in-VM "
        } else {
            ""
        };
        return if node.totals.processes > 0 {
            format!("{} proc {scope}{active}/{count} ctr", node.totals.processes)
        } else {
            format!("{scope}{active} / {count} ctr")
        };
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
    } else if let Some(resource) = &node.resource {
        paths.push(format!(
            "{} · {} · {}",
            resource.provider,
            resource.id,
            resource
                .metrics
                .iter()
                .map(output::metric_text)
                .collect::<Vec<_>>()
                .join(" · ")
        ));
        if let Some(path) = &resource.path {
            paths.push(format!("path {}", path.display()));
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

pub(super) fn panel(area: Rect, width: u16, height: u16) -> Rect {
    let width = area.width.min(width);
    let height = area.height.min(height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub(super) fn draw(frame: &mut Frame, app: &App, table: &mut TableState) {
    let overlay =
        app.help || app.extensions_page || app.settings.as_ref().is_some_and(|popup| popup.open);
    if overlay {
        draw_base(frame, app, &mut table.clone());
        let area = frame.area();
        frame.buffer_mut().set_style(
            area,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        );
    } else {
        draw_base(frame, app, table);
    }
    if app.help {
        help(frame, frame.area(), app);
    }
    if app.extensions_page {
        super::extensions::draw(frame, app);
    }
    if let Some(popup) = &app.settings
        && popup.open
    {
        popup.draw(frame);
    }
}

fn draw_base(frame: &mut Frame, app: &App, table: &mut TableState) {
    let area = frame.area();
    if area.width < 36 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("reswho: enlarge terminal (36x10 minimum). q quits.")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let refresh = app.refresh_hint();
    let groups =
        if app.help || app.extensions_page || app.settings.as_ref().is_some_and(|popup| popup.open)
        {
            vec![]
        } else if app.editing {
            vec!["Enter Accept", "Esc Clear"]
        } else {
            vec![
                "→ Open",
                if area.width < 80 {
                    "/ Find"
                } else {
                    "/ Search"
                },
                "s Settings",
                if area.width < 80 {
                    "e Exts"
                } else {
                    "e Extensions"
                },
                "? Help",
                "q Quit",
                refresh,
            ]
        };
    let keys = hint_lines(&groups, area.width);
    let keys_height = keys.len() as u16;
    let keys = Paragraph::new(keys);
    let [header, toolbar, tree_area, path_area, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1 + keys_height),
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
    let search = if app.editing {
        format!("/{}_", safe(&app.query))
    } else if app.query.is_empty() {
        "/ search".into()
    } else {
        format!("/{}", safe(&app.query))
    };
    frame.render_widget(
        Paragraph::new(format!("{search}{status}")).style(Style::default().fg(Color::Yellow)),
        toolbar,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1));
    let tree_inner = block.inner(tree_area);
    let wide = tree_inner.width >= 85;
    let mut shown: Vec<(&[usize], &crate::extensions::Summary)> = Vec::new();
    let rows = app.rows.iter().enumerate().map(|(index, row)| {
        shown.retain(|(path, _)| row.path.starts_with(path) && row.path.len() > path.len());
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
        let name = format!("{indent}{arrow} {}{suffix}", safe(&node.display_name()));
        let name_width = usize::from(tree_inner.width).saturating_sub(if wide { 46 } else { 31 });
        let mut lines = vec![ratatui::text::Line::raw(name)];
        if let Some(summary) = &node.summary {
            let redundant =
                summary.inherited
                    && shown.iter().any(|(_, ancestor)| {
                        summary.evidence.len() == ancestor.evidence.len()
                            && summary.evidence.iter().zip(&ancestor.evidence).all(
                                |(hint, parent)| {
                                    hint.source == parent.source
                                        && hint.kind == parent.kind
                                        && hint.value == parent.value
                                        && hint.title == parent.title
                                        && hint.path == parent.path
                                },
                            )
                    });
            let available = name_width.saturating_sub(indent.len() + 2);
            let label_width = ratatui::text::Line::raw(&summary.label).width();
            if !redundant && available > label_width + 3 {
                shown.push((&row.path, summary));
                let text = safe(&format!("{}: {}", summary.label, summary.value));
                lines.push(ratatui::text::Line::styled(
                    format!("{indent}  {}", clipped_summary(&text, available)),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
            }
        }
        let height = lines.len() as u16;
        let [cpu, ram, age] = metrics(node);
        let mut cells = vec![
            Cell::from(ratatui::text::Text::from(lines)),
            Cell::from(cpu.split(" (").next().unwrap_or(&cpu).to_owned()),
            Cell::from(ram.split(" (").next().unwrap_or(&ram).to_owned()),
            Cell::from(age),
        ];
        if wide {
            cells.push(Cell::from(safe(&state_label(node))));
        }
        Row::new(cells).height(height).style(
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
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
    ];
    let mut headings = match app.sort {
        Sort::Name => vec!["Resource ↑", "CPU/core", "Memory", "Age"],
        Sort::Cpu => vec!["Resource", "CPU/core ↓", "Memory", "Age"],
        Sort::Memory => vec!["Resource", "CPU/core", "Memory ↓", "Age"],
        Sort::Uptime => vec!["Resource", "CPU/core", "Memory", "Age ↓"],
    };
    if tree_inner.width < 47 {
        let active = match app.sort {
            Sort::Name => 0,
            Sort::Cpu => 1,
            Sort::Memory => 2,
            Sort::Uptime => 3,
        };
        widths[active] = Constraint::Min(headings[active].chars().count() as u16);
    }
    if wide {
        widths.push(Constraint::Length(14));
        headings.push("State / Ports");
    }
    let widget = Table::new(rows, widths)
        .header(Row::new(headings).style(Style::default().fg(Color::Cyan)))
        .block(block)
        .row_highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .remove_modifier(Modifier::DIM)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);
    table.select((!app.rows.is_empty()).then_some(app.selected));
    frame.render_stateful_widget(widget, tree_area, table);
    if app.rows.is_empty() && app.snapshot.is_some() {
        frame.render_widget(
            Paragraph::new("No matches"),
            Rect::new(
                tree_inner.x,
                tree_inner.y + 1,
                tree_inner.width,
                tree_inner.height.saturating_sub(1),
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
                tree_inner.x,
                tree_inner.y + 1 + tree_inner.height.saturating_sub(2) / 2,
                tree_inner.width,
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
    let issues = app
        .extensions
        .iter()
        .filter(|record| record.issue())
        .count();
    let diagnostic = if issues > 0 {
        format!("Extension issues: {issues} (e)  {diagnostic}")
    } else {
        diagnostic
    };
    let [diagnostic_area, keys_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(keys_height)]).areas(footer);
    frame.render_widget(
        Paragraph::new(diagnostic).style(Style::default().fg(Color::Yellow)),
        diagnostic_area,
    );
    frame.render_widget(keys, keys_area);
}

fn help(frame: &mut Frame, area: Rect, app: &App) {
    app.help_view.draw(frame, area, app);
}
