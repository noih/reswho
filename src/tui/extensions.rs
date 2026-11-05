mod save;

use super::{settings::readable, state::App, view::safe};
use crate::extensions::inventory::Listing;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Row, Table, Tabs, Wrap},
};

fn core(id: &str) -> bool {
    matches!(id, "app_bundle" | "bin" | "command")
}

fn toggleable(entry: &Listing) -> bool {
    !core(&entry.descriptor.id)
}

fn defaults(app: &App, ownership: bool) -> Vec<String> {
    if !ownership {
        return ["app_bundle", "bin", "command"]
            .into_iter()
            .map(String::from)
            .chain(app.inventory.iter().map(|e| e.descriptor.id.clone()))
            .collect();
    }
    use crate::extensions::Extension;
    let mut registrations = vec![crate::core::registration()];
    registrations.extend(crate::extensions::BUILTINS.iter().map(|e| e.registration()));
    registrations.extend(app.inventory.iter().map(|e| e.registration.clone()));
    let mut dimensions: Vec<_> = registrations
        .into_iter()
        .flat_map(|r| r.ownership)
        .collect();
    dimensions.sort_by(|a, b| (a.priority, &a.dimension).cmp(&(b.priority, &b.dimension)));
    let mut ids = Vec::new();
    for dimension in dimensions {
        if !ids.contains(&dimension.dimension) {
            ids.push(dimension.dimension);
        }
    }
    ids
}

pub(super) fn order(app: &App, ownership: bool) -> Vec<String> {
    let draft = if ownership {
        &app.ownership_draft
    } else {
        &app.priority_draft
    };
    complete_order(app, ownership, draft.as_ref())
}

fn complete_order(app: &App, ownership: bool, draft: Option<&Vec<String>>) -> Vec<String> {
    let mut ids = draft.cloned().unwrap_or_else(|| {
        app.settings
            .as_ref()
            .map(|p| {
                if ownership {
                    p.context.baseline.ownership_priority.clone()
                } else {
                    p.context.baseline.source_priority.clone()
                }
            })
            .unwrap_or_default()
    });
    for id in defaults(app, ownership) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

fn selected(app: &App, ids: &[String]) -> usize {
    ids.iter()
        .position(|id| Some(id) == app.extension_selected.as_ref())
        .unwrap_or(0)
}

pub(super) fn open(app: &mut App) {
    app.extensions_page = true;
    key(app, KeyCode::Null.into());
}

fn clear(app: &mut App) {
    app.priority_draft = None;
    app.ownership_draft = None;
    app.builtin_draft.clear();
    app.priority_error = None;
    app.extensions_page = false;
    app.priority_mode = 0;
    app.extension_selections = Default::default();
    *app.extension_tables.borrow_mut() = Default::default();
}

pub(super) fn key(app: &mut App, key: KeyEvent) -> bool {
    if app.priority_draft.is_none() {
        if let Some(popup) = app.settings.as_mut()
            && popup.context.reload().is_err()
        {
            app.priority_error =
                Some("Cannot read config safely; repair the file before saving".into());
        }
        app.priority_draft = Some(
            app.settings
                .as_ref()
                .map(|p| p.context.baseline.source_priority.clone())
                .unwrap_or_default(),
        );
        app.ownership_draft = Some(
            app.settings
                .as_ref()
                .map(|p| p.context.baseline.ownership_priority.clone())
                .unwrap_or_default(),
        );
    }
    if app.extension_details {
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                app.extension_details = false;
                return false;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown => app.extension_details_height.get().max(1) as isize,
            KeyCode::PageUp => -(app.extension_details_height.get().max(1) as isize),
            KeyCode::Home => isize::MIN,
            KeyCode::End => isize::MAX,
            _ => return false,
        };
        app.extension_details_scroll.set(
            usize::from(app.extension_details_scroll.get())
                .saturating_add_signed(delta)
                .min(usize::from(app.extension_details_max.get())) as u16,
        );
        return false;
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        let ids = order(app, app.priority_mode == 1);
        app.extension_selections[usize::from(app.priority_mode)] =
            ids.get(selected(app, &ids)).cloned();
        app.priority_mode = (app.priority_mode + 1) % 2;
        app.extension_selected = app.extension_selections[usize::from(app.priority_mode)].clone();
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        save::save(app);
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
        if app.priority_mode == 1 {
            app.ownership_draft = Some(Vec::new());
        } else {
            app.priority_draft = Some(Vec::new());
        }
        app.priority_error = None;
        return false;
    }
    let ids = order(app, app.priority_mode == 1);
    let index = selected(app, &ids);
    if key.code == KeyCode::Char(' ') && app.priority_mode == 0 {
        if let Some(id) = ids.get(index)
            && let Some(entry) = app.inventory.iter().find(|e| &e.descriptor.id == id)
            && toggleable(entry)
        {
            let enabled = enabled(app, entry);
            let baseline = baseline_enabled(app, entry);
            let draft = &mut app.builtin_draft;
            if !enabled == baseline {
                draft.remove(id);
            } else {
                draft.insert(id.clone(), !enabled);
            }
            app.priority_error = None;
        }
        return false;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Up | KeyCode::Down)
    {
        let other = if key.code == KeyCode::Up {
            index.saturating_sub(1)
        } else {
            (index + 1).min(ids.len().saturating_sub(1))
        };
        if index == other {
            return false;
        }
        if let Some(id) = ids.get(index) {
            let mut moved = ids.clone();
            moved.swap(index, other);
            let baseline = app
                .settings
                .as_ref()
                .map(|p| {
                    if app.priority_mode == 1 {
                        p.context.baseline.ownership_priority.clone()
                    } else {
                        p.context.baseline.source_priority.clone()
                    }
                })
                .unwrap_or_default();
            let mut effective = baseline.clone();
            for id in defaults(app, app.priority_mode == 1) {
                if !effective.contains(&id) {
                    effective.push(id);
                }
            }
            if moved == effective {
                moved = baseline;
            }
            if app.priority_mode == 1 {
                app.ownership_draft = Some(moved);
            } else {
                app.priority_draft = Some(moved);
            }
            app.extension_selected = Some(id.clone());
            app.priority_error = None;
        }
        return false;
    }
    let next = match key.code {
        KeyCode::Esc => {
            clear(app);
            return false;
        }
        KeyCode::Down | KeyCode::Char('j') => (index + 1).min(ids.len().saturating_sub(1)),
        KeyCode::Up | KeyCode::Char('k') => index.saturating_sub(1),
        KeyCode::Home => 0,
        KeyCode::End => ids.len().saturating_sub(1),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('d') if !ids.is_empty() => {
            app.extension_selected = ids.get(index).cloned();
            app.extension_details = true;
            app.extension_details_scroll.set(0);
            app.extension_details_height.set(1);
            app.extension_details_max.set(u16::MAX);
            return false;
        }
        KeyCode::PageDown => {
            app.extensions_scroll = app.extensions_scroll.saturating_add(1);
            (index + 4).min(ids.len().saturating_sub(1))
        }
        KeyCode::PageUp => {
            app.extensions_scroll = app.extensions_scroll.saturating_sub(1);
            index.saturating_sub(4)
        }
        _ => return false,
    };
    app.extension_selected = ids.get(next).cloned();
    false
}

fn name(id: &str) -> String {
    match id {
        "app_bundle" => "App bundles".into(),
        "apple" => "Apple Container".into(),
        "command" => "Process command".into(),
        "project" => "Projects".into(),
        "ancestor" => "Ancestor ownership".into(),
        "bin" => "Executable fallback".into(),
        _ => readable(id),
    }
}

fn timing(app: &App, id: &str, phase: &str) -> String {
    app.extensions
        .iter()
        .rev()
        .find(|record| record.provider == id && record.phase == phase)
        .and_then(|record| record.duration_ms)
        .map_or("—".into(), |ms| format!("{ms:.1}ms"))
}

fn baseline_enabled(app: &App, entry: &Listing) -> bool {
    let Some(popup) = &app.settings else {
        return entry.explicit_enabled;
    };
    let id = &entry.descriptor.id;
    !popup.context.baseline.disabled_builtins.contains(id)
}

fn enabled(app: &App, entry: &Listing) -> bool {
    app.builtin_draft
        .get(&entry.descriptor.id)
        .copied()
        .unwrap_or_else(|| baseline_enabled(app, entry))
}

fn state(app: &App, entry: &Listing) -> String {
    let failed = app
        .extensions
        .iter()
        .any(|record| record.provider == entry.descriptor.id && record.issue());
    let enabled = if !enabled(app, entry) {
        "Disabled"
    } else if entry.reason.is_some() {
        "Blocked"
    } else {
        "Enabled"
    };
    format!("{enabled}{}", if failed { " / error" } else { "" })
}

fn details(app: &App, entry: &Listing) -> Vec<Line<'static>> {
    let descriptor = &entry.descriptor;
    let mut lines = vec![
        name(&descriptor.id),
        format!("ID: {}", descriptor.id),
        format!(
            "Capabilities: {}",
            descriptor
                .capabilities
                .iter()
                .map(|c| format!("{c:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "Source rank: {}",
            order(app, false)
                .iter()
                .position(|id| id == &descriptor.id)
                .map_or(0, |i| i + 1)
        ),
        format!(
            "State: {} | Availability: {}",
            state(app, entry),
            descriptor.availability
        ),
    ];
    if let Some(error) = &app.priority_error {
        lines.push(format!("Save: {error}"));
    }
    if let Some(reason) = &entry.reason {
        lines.push(format!("Reason: {reason}"));
    }
    lines.push("Timings".into());
    let records: Vec<_> = app
        .extensions
        .iter()
        .filter(|record| record.provider == descriptor.id)
        .collect();
    if records.is_empty() {
        lines.push("No phase reports yet.".into());
    }
    for record in records {
        lines.push(format!(
            "{}: {}  {}  {}",
            record.phase,
            match record.result.as_str() {
                "never_invoked" if record.reason.as_deref() == Some("not_requested") =>
                    "Not requested",
                "never_invoked" => "No sample",
                "success" => "Success",
                "failed" => "Failed",
                "skipped" => "Skipped",
                _ => &record.result,
            },
            record
                .duration_ms
                .map_or("—".into(), |ms| format!("{ms:.3}ms")),
            record.reason.as_deref().unwrap_or("")
        ));
    }
    lines.push("— No invocation timing · phases are not additive".into());
    lines
        .into_iter()
        .map(|line| Line::raw(safe(&line)))
        .collect()
}

pub(super) fn draw(frame: &mut Frame, app: &App) {
    let entries: Vec<_> = app.inventory.iter().collect();
    let ids = order(app, app.priority_mode == 1);
    let index = selected(app, &ids);
    let popup = frame
        .area()
        .inner(Margin::new(1, if frame.area().height < 14 { 0 } else { 1 }));
    frame.render_widget(Clear, popup);
    let title = if app.extension_details && app.priority_mode == 1 {
        " Ownership details "
    } else if app.extension_details {
        " Source details "
    } else {
        " Extensions "
    };
    let block = Block::bordered()
        .title(title)
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let groups = if app.extension_details {
        vec!["↑↓ Scroll", "←/Esc Back"]
    } else {
        let action = if app.priority_mode == 1 {
            ""
        } else {
            ids.get(index)
                .and_then(|id| entries.iter().find(|entry| entry.descriptor.id == *id))
                .filter(|entry| toggleable(entry))
                .map_or("", |entry| {
                    if enabled(app, entry) {
                        "Space Disable"
                    } else {
                        "Space Enable"
                    }
                })
        };
        vec![
            "Tab Switch",
            action,
            "Shift↑↓ Move",
            "→ Details",
            "Ctrl-S Save",
            if inner.width < 65 {
                "Ctrl-R Reset"
            } else {
                "Ctrl-R Reset order"
            },
            "Esc Cancel",
        ]
    };
    let footer = super::view::hint_lines(&groups, inner.width);
    let footer_height = footer
        .len()
        .min(usize::from(inner.height.saturating_sub(3))) as u16;
    let footer = Paragraph::new(footer).style(Style::default().fg(Color::Gray));
    let system: Vec<_> = app
        .extensions
        .iter()
        .filter(|r| r.issue() && !entries.iter().any(|e| e.descriptor.id == r.provider))
        .collect();
    let compact = inner.height < 9;
    let diagnostic_height = if app.extension_details
        || system.is_empty()
        || (compact && app.priority_error.is_some())
    {
        0
    } else if compact && footer_height > 3 {
        1
    } else {
        2
    };
    let [caption, body, diagnostics, status, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(diagnostic_height),
        Constraint::Length(
            if app.extension_details || (compact && diagnostic_height > 0) {
                0
            } else {
                1
            },
        ),
        Constraint::Length(footer_height),
    ])
    .areas(inner);
    let tabs = ["Sources", "Ownership"];
    frame.render_widget(
        Tabs::new(tabs)
            .select(usize::from(app.priority_mode))
            .style(Style::default().fg(Color::Gray))
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        caption,
    );
    if app.extension_details {
        if let Some(id) = ids.get(index) {
            let lines = if app.priority_mode == 0
                && !core(id)
                && let Some(entry) = entries
                    .iter()
                    .find(|e| &e.descriptor.id == id)
                    .filter(|e| toggleable(e))
            {
                details(app, entry)
            } else {
                vec![
                    Line::raw(safe(&format!("ID: {id}"))),
                    Line::raw(format!("Rank: {} (highest first)", index + 1)),
                    Line::raw(if app.priority_mode == 1 {
                        "Ownership: main placement across dimensions"
                    } else if core(id) {
                        "Core · fixed enabled; source priority is editable"
                    } else {
                        "Unknown · unavailable; enable/disable unavailable"
                    }),
                ]
            };
            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            let max = paragraph
                .line_count(body.width)
                .saturating_sub(usize::from(body.height))
                .min(usize::from(u16::MAX)) as u16;
            app.extension_details_height.set(body.height);
            app.extension_details_max.set(max);
            let scroll = app.extension_details_scroll.get().min(max);
            app.extension_details_scroll.set(scroll);
            frame.render_widget(paragraph.scroll((scroll, 0)), body);
        }
    } else {
        let narrow = inner.width < 117;
        let baseline_order = complete_order(app, app.priority_mode == 1, None);
        let rank_width = inner.width.saturating_sub(1) * 55 / 100;
        let mut rows = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let entry = entries
                .iter()
                .find(|e| &e.descriptor.id == id)
                .filter(|e| toggleable(e));
            let marker = if baseline_order.get(i) != Some(id)
                || (app.priority_mode == 0 && app.builtin_draft.contains_key(id))
            {
                "* "
            } else {
                "  "
            };
            let cells = if app.priority_mode == 1 {
                let prefix = format!("{marker}{}  ", i + 1);
                let label: String = safe(&name(id))
                    .chars()
                    .take(usize::from(rank_width).saturating_sub(prefix.len()))
                    .collect();
                vec![format!("{prefix}{label}"), safe(id)]
            } else {
                let state = if core(id) {
                    "Fixed".into()
                } else {
                    entry.map_or("Unavailable".into(), |entry| state(app, entry))
                };
                let id_width = if narrow {
                    inner.width.saturating_sub(10)
                } else {
                    24
                };
                let label: String = safe(id)
                    .chars()
                    .take(usize::from(id_width).saturating_sub(marker.len()))
                    .collect();
                let mut cells = vec![format!("{marker}{label}"), state];
                if !narrow {
                    let entry = entry.filter(|_| !core(id));
                    cells.extend([
                        entry.map_or("—".into(), |e| {
                            e.descriptor
                                .capabilities
                                .iter()
                                .map(|c| format!("{c:?}").to_lowercase())
                                .collect::<Vec<_>>()
                                .join(",")
                        }),
                        (i + 1).to_string(),
                        entry.map_or("—".into(), |_| timing(app, id, "collect")),
                        entry.map_or("—".into(), |_| timing(app, id, "classify")),
                        entry.map_or("—".into(), |_| timing(app, id, "details")),
                    ]);
                }
                cells
            };
            rows.push(Row::new(cells));
        }
        let (widths, headers) = if app.priority_mode == 1 {
            (
                vec![Constraint::Length(rank_width), Constraint::Min(0)],
                vec!["Rank / Name", "ID"],
            )
        } else if narrow {
            (
                vec![Constraint::Min(10), Constraint::Length(8)],
                vec!["ID", "State"],
            )
        } else {
            (
                vec![
                    Constraint::Length(24),
                    Constraint::Length(17),
                    Constraint::Length(17),
                    Constraint::Length(4),
                    Constraint::Length(10),
                    Constraint::Length(10),
                    Constraint::Length(10),
                ],
                vec![
                    "ID",
                    "State",
                    "Capabilities",
                    "Rank",
                    "Collect",
                    "Classify",
                    "Details",
                ],
            )
        };
        if ids.is_empty() {
            let mut diagnostics = vec![Line::raw("System diagnostics")];
            diagnostics.extend(app.extensions.iter().filter(|r| r.issue()).map(|r| {
                Line::raw(safe(&format!(
                    "{}: {}",
                    r.provider,
                    r.reason.as_deref().unwrap_or(&r.result)
                )))
            }));
            frame.render_widget(Paragraph::new(diagnostics).wrap(Wrap { trim: false }), body);
        } else {
            let mut tables = app.extension_tables.borrow_mut();
            let table = &mut tables[usize::from(app.priority_mode)];
            table.select(Some(index));
            frame.render_stateful_widget(
                Table::new(rows, widths)
                    .header(Row::new(headers).style(Style::default().fg(Color::Yellow)))
                    .row_highlight_style(Style::default().fg(Color::White).bg(Color::DarkGray)),
                body,
                table,
            );
        }
        let summary = app.priority_error.clone().unwrap_or_else(|| {
            ids.get(index).map_or("No sources".into(), |id| {
                if app.priority_mode == 1 {
                    format!("{} · dimension · highest first", name(id))
                } else if core(id) {
                    "Core · fixed enabled · → Details".into()
                } else {
                    entries
                        .iter()
                        .find(|e| &e.descriptor.id == id)
                        .filter(|e| toggleable(e))
                        .map_or_else(
                            || "Unknown · unavailable · → Details".into(),
                            |_| format!("{} · → Details", name(id)),
                        )
                }
            })
        });
        frame.render_widget(
            Paragraph::new(safe(&summary)).style(Style::default().fg(
                if app.priority_error.is_some() {
                    Color::Yellow
                } else {
                    Color::Gray
                },
            )),
            status,
        );
    }
    if diagnostic_height > 0 {
        let [heading, content] = Layout::vertical([
            Constraint::Length(u16::from(diagnostic_height > 1)),
            Constraint::Min(0),
        ])
        .areas(diagnostics);
        frame.render_widget(
            Paragraph::new("System diagnostics").style(Style::default().fg(Color::Yellow)),
            heading,
        );
        let lines: Vec<_> = system
            .iter()
            .map(|r| {
                Line::raw(safe(&format!(
                    "{}: {}",
                    r.provider,
                    r.reason.as_deref().unwrap_or(&r.result)
                )))
            })
            .collect();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let max_scroll = paragraph
            .line_count(content.width)
            .saturating_sub(usize::from(content.height))
            .min(usize::from(u16::MAX)) as u16;
        frame.render_widget(
            paragraph.scroll((app.extensions_scroll.min(max_scroll), 0)),
            content,
        );
    }
    frame.render_widget(footer, footer_area);
}
