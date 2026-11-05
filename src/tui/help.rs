use super::{
    state::App,
    view::{panel, safe},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
};
use std::cell::Cell;

#[derive(Default)]
pub(super) struct Help {
    pub page: usize,
    pub scroll: Cell<usize>,
    max_scroll: Cell<usize>,
    height: Cell<usize>,
}

impl Help {
    pub fn key(&mut self, key: KeyEvent) {
        let delta = match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let previous =
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                self.page = (self.page + if previous { 2 } else { 1 }) % 3;
                self.scroll.set(0);
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown => self.height.get() as isize,
            KeyCode::PageUp => -(self.height.get() as isize),
            KeyCode::Home => isize::MIN,
            KeyCode::End => isize::MAX,
            _ => return,
        };
        self.scroll.set(
            self.scroll
                .get()
                .saturating_add_signed(delta)
                .min(self.max_scroll.get()),
        );
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, app: &App) {
        let width = area.width.min(96);
        let lines = self.lines(width.saturating_sub(2), app);
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let count = paragraph.line_count(width.saturating_sub(2));
        let tab_height = if width < 60 { 1 } else { 2 };
        let mut groups = vec!["↑↓ Scroll", "Tab/Shift-Tab page", "Esc/? close"];
        let footer_height = super::view::hint_lines(&groups, width.saturating_sub(2)).len();
        let limit = area.height.saturating_sub(2).max(10).min(area.height);
        if count + tab_height + 3 + footer_height <= usize::from(limit) {
            groups.remove(0);
        }
        let footer_lines = super::view::hint_lines(&groups, width.saturating_sub(2));
        let footer_height = footer_lines.len() as u16;
        let height =
            (count + tab_height + 3 + usize::from(footer_height)).min(u16::MAX as usize) as u16;
        let popup = panel(
            area,
            width,
            height.min(area.height.saturating_sub(2).max(10)),
        );
        frame.render_widget(Clear, popup);
        let block = Block::bordered().title(" Help ");
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let [tabs, body, _, footer] = Layout::vertical([
            Constraint::Length(tab_height as u16),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(footer_height),
        ])
        .areas(inner);
        let mut labels = Vec::new();
        for (index, name) in ["Controls", "Panels", "Metrics"].iter().enumerate() {
            if index > 0 {
                labels.push(Span::raw("  "));
            }
            labels.push(Span::styled(
                if self.page == index {
                    format!("[{name}]")
                } else {
                    (*name).to_owned()
                },
                if self.page == index {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(labels)), tabs);
        let max = count.saturating_sub(body.height as usize);
        let scroll = self.scroll.get().min(max);
        self.scroll.set(scroll);
        self.max_scroll.set(max);
        self.height.set(body.height as usize);
        frame.render_widget(paragraph.scroll((scroll as u16, 0)), body);
        let mut hint = footer_lines;
        if max > 0 && inner.width >= 80 {
            hint.last_mut()
                .expect("help controls")
                .spans
                .push(Span::raw(format!(
                    " · {}–{}/{}",
                    scroll + 1,
                    (scroll + body.height as usize).min(count),
                    count
                )));
        }
        frame.render_widget(Paragraph::new(hint), footer);
    }

    fn lines(&self, width: u16, app: &App) -> Vec<Line<'static>> {
        let sections: &[(&str, &[(&str, &str)])] = match self.page {
            0 => &[
                (
                    "Navigation",
                    &[
                        ("↑↓ / k j", "Move selection"),
                        ("← / h", "Collapse or select parent"),
                        ("→ / l", "Expand, select child or open leaf details"),
                        ("Enter / Space", "Toggle branch"),
                        ("Home / End", "First / last row"),
                        ("PgUp / PgDn", "Move 10 rows"),
                        ("[ / ]", "Scroll selected path"),
                    ],
                ),
                (
                    "Find / sort",
                    &[
                        ("/", "Search name, version, PID, ref or path"),
                        ("Enter / Esc", "Keep / clear search"),
                        (
                            "c / m / u",
                            "CPU / memory / uptime: descending within siblings",
                        ),
                        ("n", "Name: natural order, case-insensitive"),
                    ],
                ),
                (
                    "Refresh / actions",
                    &[
                        ("r", "Refresh once; keep pause, reuse pending query"),
                        ("p", "Pause / resume automatic refresh"),
                        ("s / e", "Open Settings / Extensions from main"),
                        (
                            "d",
                            "Process / resource details, including parent processes",
                        ),
                        ("?", "Open Help from main"),
                        ("q / Ctrl-C", "Quit from main / quit anywhere"),
                    ],
                ),
            ],
            1 => &[
                (
                    "Details",
                    &[
                        (
                            "← / h / Esc",
                            "Back; d also closes process / resource details",
                        ),
                        ("↑↓ / k j", "Scroll content"),
                        ("PgUp / PgDn", "Page by visible content height"),
                        (
                            "Home / End",
                            "Process / resource details: first / last line",
                        ),
                        ("r / p", "Process / resource details: refresh / pause"),
                    ],
                ),
                (
                    "Settings · s from main",
                    &[
                        ("↑↓ / k j", "Select setting"),
                        ("Enter / Space", "Edit value; Enter accepts input"),
                        ("→ / ←", "Open details / back; Space toggles booleans"),
                        ("↑↓ / PgUp PgDn", "Scroll lines / page by visible height"),
                        ("Ctrl-U", "Clear current input"),
                        ("Ctrl-S", "Save draft"),
                        ("Ctrl-R", "Reset selected setting to default"),
                        ("Esc", "Back from details; undo input; discard list"),
                        ("CLI lock", "Restart without the CLI override to edit"),
                    ],
                ),
                (
                    "Extensions · e from main",
                    &[
                        ("Tab / Shift-Tab", "Sources / Ownership"),
                        ("Space", "Enable/disable selected extension draft"),
                        ("↑↓ / k j", "Select row; priority is highest first"),
                        ("Shift-↑↓", "Move within the selected priority mode"),
                        ("Ctrl-S", "Save for next capture; stay in this tab"),
                        ("Ctrl-R", "Reset order only; keep enable/disable drafts"),
                        ("→ / l / d", "Open source details"),
                        ("← / h", "Back from details; no action in list"),
                        ("Esc", "Back from details; list cancels unsaved changes"),
                        ("PgUp / PgDn", "List: 4 rows; details: visible height"),
                    ],
                ),
            ],
            _ => &[
                (
                    "CPU",
                    &[
                        ("Host CPU", "Whole-machine usage, 0–100%"),
                        ("CPU/core", "100% = one core; rows can exceed 100%"),
                    ],
                ),
                (
                    "Memory / age",
                    &[
                        (
                            "Memory",
                            "Host RSS and container memory have separate scopes; do not add them",
                        ),
                        (
                            "Age",
                            "Process uptime or oldest group member; includes idle time",
                        ),
                    ],
                ),
                (
                    "Reading values",
                    &[
                        (">=", "Known lower bound; some values are unreadable"),
                        ("? / -", "Unknown / not applicable; unknown is not zero"),
                        ("Search", "Filter visible rows, not aggregate totals"),
                        ("Expansion", "Search reveals matches; keep saved expansion"),
                        (
                            "Refresh",
                            "Configured wait after each result; one query at a time",
                        ),
                    ],
                ),
            ],
        };
        let mut lines = Vec::new();
        for (heading, rows) in sections {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::styled(
                *heading,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            for (key, action) in *rows {
                if width < 60 {
                    lines.push(Line::styled(
                        (*key).to_owned(),
                        Style::default().fg(Color::Gray),
                    ));
                    lines.push(Line::raw(format!("  {action}")));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{key:16}"), Style::default().fg(Color::Gray)),
                        Span::raw((*action).to_owned()),
                    ]));
                }
            }
        }
        if self.page == 2 {
            if let Some(error) = &app.error {
                lines.push(Line::raw(""));
                lines.push(Line::raw(format!("STALE: {}", safe(error))));
            }
            if let Some(snapshot) = &app.snapshot {
                for warning in snapshot
                    .warnings
                    .iter()
                    .filter(|warning| !crate::collect::COLLECTION_NOTES.contains(&warning.as_str()))
                {
                    lines.push(Line::raw(format!("! {}", safe(warning.as_str()))));
                }
            }
        }
        lines
    }
}
