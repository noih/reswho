use super::{
    settings::readable,
    state::App,
    view::{panel, safe},
};
use crate::extensions::state::Listing;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Row, Table, TableState, Wrap},
};

fn providers(app: &App) -> Vec<&Listing> {
    app.inventory
        .iter()
        .filter(|entry| !entry.descriptor.capabilities.is_empty())
        .collect()
}

fn selected(app: &App, entries: &[&Listing]) -> usize {
    entries
        .iter()
        .position(|entry| Some(&entry.descriptor.id) == app.extension_selected.as_ref())
        .unwrap_or(0)
}

pub(super) fn key(app: &mut App, key: KeyEvent) -> bool {
    let entries = providers(app);
    let index = selected(app, &entries);
    if app.extension_details {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.extension_details = false;
                app.extensions_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.extensions_scroll = app.extensions_scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.extensions_scroll = app.extensions_scroll.saturating_sub(1)
            }
            KeyCode::PageDown => app.extensions_scroll = app.extensions_scroll.saturating_add(4),
            KeyCode::PageUp => app.extensions_scroll = app.extensions_scroll.saturating_sub(4),
            KeyCode::Home => app.extensions_scroll = 0,
            _ => {}
        }
        return false;
    }
    let next = match key.code {
        KeyCode::Esc => {
            app.extensions_page = false;
            return false;
        }
        KeyCode::Down | KeyCode::Char('j') => (index + 1).min(entries.len().saturating_sub(1)),
        KeyCode::Up | KeyCode::Char('k') => index.saturating_sub(1),
        KeyCode::Home => 0,
        KeyCode::End => entries.len().saturating_sub(1),
        KeyCode::Enter => {
            app.extension_details = true;
            app.extensions_scroll = 0;
            return false;
        }
        KeyCode::PageDown => {
            app.extensions_scroll = app.extensions_scroll.saturating_add(2);
            return false;
        }
        KeyCode::PageUp => {
            app.extensions_scroll = app.extensions_scroll.saturating_sub(2);
            return false;
        }
        _ => return false,
    };
    app.extension_selected = entries.get(next).map(|entry| entry.descriptor.id.clone());
    false
}

fn name(id: &str) -> String {
    match id {
        "app_bundle" => "App bundles".into(),
        "apple" => "Apple Container".into(),
        "command" => "Process command".into(),
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

fn state(app: &App, entry: &Listing) -> String {
    let failed = app
        .extensions
        .iter()
        .any(|record| record.provider == entry.descriptor.id && record.issue());
    let enabled = if !entry.explicit_enabled {
        "Disabled"
    } else if !entry.descriptor.enabled {
        "Blocked"
    } else {
        "Enabled"
    };
    format!("{enabled}{}", if failed { " / error" } else { "" })
}

fn details(app: &App, entry: &Listing) -> Vec<Line<'static>> {
    let descriptor = &entry.descriptor;
    let mut lines = vec![
        format!("{}  {}", name(&descriptor.id), descriptor.version),
        format!("ID: {}", descriptor.id),
        format!(
            "State: {} | Availability: {}",
            state(app, entry),
            descriptor.availability
        ),
        format!(
            "Source: {} | Compatible: {} | Requires: {}",
            if descriptor.source == "builtin" {
                "Built-in"
            } else {
                "External"
            },
            if entry.compatible { "Yes" } else { "No" },
            entry.requires
        ),
    ];
    if let Some(installed) = app.extension_registry.as_ref().and_then(|registry| {
        registry
            .installed
            .iter()
            .find(|installed| installed.manifest.id == descriptor.id)
    }) {
        lines.push(format!("Path: {}", installed.manifest.executable.display()));
        lines.push(format!(
            "Origin: {}",
            serde_json::to_string(&installed.origin).unwrap_or_default()
        ));
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
    let entries = providers(app);
    let index = selected(app, &entries);
    let system: Vec<_> = app
        .extensions
        .iter()
        .filter(|record| {
            record.issue()
                && !entries
                    .iter()
                    .any(|entry| entry.descriptor.id == record.provider)
        })
        .collect();
    let diagnostic_height = if system.is_empty() {
        0
    } else {
        (system.len() as u16 + 1).min(3)
    };
    let footer = Paragraph::new(if app.extension_details {
        "Esc Back"
    } else {
        "Enter Details  Esc Back"
    })
    .wrap(Wrap { trim: false })
    .style(Style::default().fg(Color::Gray));
    let footer_height = footer
        .line_count(frame.area().width.min(98).saturating_sub(4))
        .min(usize::from(u16::MAX)) as u16;
    let desired = if app.extension_details {
        entries.get(index).map_or(6, |entry| {
            Paragraph::new(details(app, entry))
                .wrap(Wrap { trim: false })
                .line_count(frame.area().width.min(98).saturating_sub(4))
                .min(usize::from(u16::MAX - 2 - footer_height)) as u16
                + 2
                + footer_height
        })
    } else {
        (entries.len() as u16).saturating_add(4 + footer_height + diagnostic_height)
    };
    let popup = panel(frame.area(), 98, desired.max(10));
    frame.render_widget(Clear, popup);
    let block = Block::bordered().title(if app.extension_details {
        " Extension details "
    } else {
        " Extensions "
    });
    let block = block.padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let [body, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(footer_height)]).areas(inner);
    if app.extension_details {
        if let Some(entry) = entries.get(index) {
            let paragraph = Paragraph::new(details(app, entry)).wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(body.width)
                .saturating_sub(usize::from(body.height))
                .min(usize::from(u16::MAX)) as u16;
            frame.render_widget(
                paragraph.scroll((app.extensions_scroll.min(max_scroll), 0)),
                body,
            );
        }
    } else {
        let diagnostic_height = if system.is_empty() {
            0
        } else {
            (system.len() as u16 + 1).min(3)
        };
        let [table, summary, diagnostics] = Layout::vertical([
            Constraint::Min(2),
            Constraint::Length(1),
            Constraint::Length(diagnostic_height),
        ])
        .areas(body);
        let narrow = inner.width < 65;
        let rows = entries.iter().map(|entry| {
            let descriptor = &entry.descriptor;
            let cells = if narrow {
                vec![name(&descriptor.id), state(app, entry)]
            } else {
                vec![
                    name(&descriptor.id),
                    descriptor.version.clone(),
                    state(app, entry),
                    timing(app, &descriptor.id, "collect"),
                    timing(app, &descriptor.id, "classify"),
                    timing(app, &descriptor.id, "details"),
                ]
            };
            Row::new(cells).style(
                if app
                    .extensions
                    .iter()
                    .any(|record| record.provider == descriptor.id && record.issue())
                {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            )
        });
        let widths = if narrow {
            vec![Constraint::Percentage(52), Constraint::Percentage(48)]
        } else {
            vec![
                Constraint::Min(17),
                Constraint::Length(9),
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(10),
            ]
        };
        let header = Row::new(if narrow {
            vec!["Name", "State"]
        } else {
            vec!["Name", "Version", "State", "Collect", "Classify", "Details"]
        })
        .style(Style::default().fg(Color::Yellow));
        frame.render_stateful_widget(
            Table::new(rows, widths)
                .header(header)
                .row_highlight_style(Style::default().fg(Color::White).bg(Color::DarkGray))
                .highlight_symbol("> "),
            table,
            &mut TableState::default().with_selected(Some(index)),
        );
        frame.render_widget(
            Paragraph::new(entries.get(index).map_or("No providers".into(), |entry| {
                format!(
                    "{} · {}",
                    name(&entry.descriptor.id),
                    entry.descriptor.source
                )
            }))
            .style(Style::default().fg(Color::Gray)),
            summary,
        );
        if !system.is_empty() {
            let mut lines = vec![Line::styled(
                format!("System diagnostics ({}) · PgDn", system.len()),
                Style::default().fg(Color::Yellow),
            )];
            lines.extend(system.iter().map(|record| {
                Line::raw(safe(&format!(
                    "{}: {} {}",
                    record.provider,
                    record.phase,
                    record.reason.as_deref().unwrap_or(&record.result)
                )))
            }));
            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(diagnostics.width)
                .saturating_sub(usize::from(diagnostics.height))
                .min(usize::from(u16::MAX)) as u16;
            frame.render_widget(
                paragraph.scroll((app.extensions_scroll.min(max_scroll), 0)),
                diagnostics,
            );
        }
    }
    frame.render_widget(footer, footer_area);
}
