use crate::settings::{Definition, Resolved, ValueType, edit::Settings};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Padding, Paragraph, Wrap},
};
use serde_json::Value;
use std::{cell::Cell, collections::BTreeMap, sync::Arc};

pub(super) struct Popup {
    pub context: Settings,
    pub open: bool,
    pub changes: BTreeMap<String, Option<Value>>,
    selected: usize,
    scroll: u16,
    max_scroll: Cell<u16>,
    page_height: Cell<u16>,
    info_open: bool,
    focus: u8,
    input: Option<String>,
    error: Option<String>,
    pub applied: Option<Arc<Resolved>>,
}

impl Popup {
    pub fn new(context: Settings) -> Self {
        Self {
            context,
            open: false,
            changes: BTreeMap::new(),
            selected: 0,
            scroll: 0,
            max_scroll: Cell::new(0),
            page_height: Cell::new(1),
            info_open: false,
            focus: 0,
            input: None,
            error: None,
            applied: None,
        }
    }

    pub fn show(&mut self) {
        self.changes.clear();
        self.focus = 0;
        self.info_open = false;
        self.input = None;
        self.error = self
            .context
            .reload()
            .err()
            .map(|_| "Cannot read config safely; repair the file before saving".into());
        self.open = true;
    }

    fn definitions(&self) -> Vec<&Definition> {
        let mut definitions: Vec<_> = self.context.registry.definitions().collect();
        definitions.sort_by_key(|definition| {
            (
                if definition.provider == "core" {
                    0
                } else if definition.key.starts_with("shared.") {
                    1
                } else {
                    2
                },
                &definition.provider,
                &definition.key,
            )
        });
        definitions
    }

    fn definition(&self) -> Option<&Definition> {
        self.definitions().get(self.selected).copied()
    }

    fn value<'a>(&'a self, definition: &'a Definition) -> (&'a Value, &'static str) {
        if let Some(value) = self.context.cli.get(&definition.key) {
            return (value, "cli locked");
        }
        if let Some(change) = self.changes.get(&definition.key) {
            return match change {
                Some(value) => (value, "user draft"),
                None => (&definition.default, "default draft"),
            };
        }
        self.context
            .baseline
            .settings
            .get(&definition.key)
            .map_or((&definition.default, "default"), |value| (value, "user"))
    }

    fn set(&mut self, key: String, value: Option<Value>) {
        if value.as_ref() == self.context.baseline.settings.get(&key) {
            self.changes.remove(&key);
        } else {
            self.changes.insert(key, value);
        }
        self.error = None;
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            if self.input.take().is_some() {
                self.focus = 0;
                self.error = None;
            } else {
                self.open = false;
                self.changes.clear();
            }
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => {
                    if self.input.is_some() && !self.accept_input() {
                        return;
                    }
                    match self.context.save(&self.changes) {
                        Ok(applied) => {
                            self.applied = applied;
                            self.changes.clear();
                            self.open = false;
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                KeyCode::Char('r') => {
                    if let Some(definition) = self.definition() {
                        let key = definition.key.clone();
                        if self.context.cli.contains_key(&key) {
                            self.error = Some("CLI override is locked".into());
                        } else {
                            self.set(key, None);
                            self.input = None;
                        }
                    }
                }
                KeyCode::Char('u') => {
                    if let Some(input) = &mut self.input {
                        input.clear();
                    }
                }
                _ => {}
            }
            return;
        }
        if matches!(key.code, KeyCode::PageDown | KeyCode::PageUp) && self.page_height.get() == 0 {
            self.info_open = true;
            self.scroll = 0;
            return;
        }
        if key.code == KeyCode::PageDown && !self.info_open {
            self.info_open = true;
            self.scroll = 0;
            return;
        }
        if key.code == KeyCode::PageUp && self.scroll == 0 {
            self.info_open = false;
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            self.focus = if key.code == KeyCode::BackTab {
                (self.focus + 2) % 3
            } else {
                (self.focus + 1) % 3
            };
            return;
        }
        if self.focus != 0 {
            if key.code == KeyCode::Enter || key.code == KeyCode::Char(' ') {
                if self.focus == 1 {
                    self.open = false;
                    self.changes.clear();
                    self.input = None;
                } else {
                    self.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
                }
            }
            return;
        }
        if let Some(input) = &mut self.input {
            match key.code {
                KeyCode::Enter => {
                    self.accept_input();
                }
                KeyCode::PageDown => {
                    self.scroll = self
                        .scroll
                        .saturating_add(self.page_height.get())
                        .min(self.max_scroll.get())
                }
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page_height.get()),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(ch)
                    if !ch.is_control()
                        && !key.modifiers.contains(KeyModifiers::ALT)
                        && input.len() < 1_048_576 =>
                {
                    input.push(ch)
                }
                _ => {}
            }
            return;
        }
        let count = self.context.registry.definitions().count();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
                self.scroll = 0;
                self.info_open = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
                self.info_open = false;
            }
            KeyCode::PageDown => {
                self.scroll = self
                    .scroll
                    .saturating_add(self.page_height.get())
                    .min(self.max_scroll.get())
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page_height.get()),
            KeyCode::Home => {
                self.selected = 0;
                self.scroll = 0;
                self.info_open = false;
            }
            KeyCode::End => {
                self.selected = count.saturating_sub(1);
                self.scroll = 0;
                self.info_open = false;
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left => {
                let Some(definition) = self.definition().cloned() else {
                    return;
                };
                if self.context.cli.contains_key(&definition.key) {
                    self.error =
                        Some("CLI override is locked; restart without that override".into());
                    return;
                }
                let (value, _) = self.value(&definition);
                if let Some(options) = &definition.contract.limits.allowed {
                    let current = options.iter().position(|option| option == value);
                    let index = if key.code == KeyCode::Left {
                        current
                            .unwrap_or(0)
                            .checked_sub(1)
                            .unwrap_or(options.len() - 1)
                    } else {
                        current.map_or(0, |index| (index + 1) % options.len())
                    };
                    self.set(definition.key.clone(), Some(options[index].clone()));
                } else if definition.contract.value_type == ValueType::Boolean {
                    self.set(
                        definition.key.clone(),
                        Some(Value::Bool(!value.as_bool().unwrap_or(false))),
                    );
                } else {
                    self.input = Some(if definition.sensitive {
                        String::new()
                    } else {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    });
                    self.error = None;
                }
            }
            _ => {}
        }
    }

    fn accept_input(&mut self) -> bool {
        let Some(definition) = self.definition() else {
            return false;
        };
        let Some(input) = &self.input else {
            return true;
        };
        let value = if definition.contract.value_type == ValueType::String {
            Ok(Value::String(input.clone()))
        } else {
            serde_json::from_str::<Value>(input)
        };
        match value {
            Ok(value)
                if definition.contract.validate(&value).is_ok()
                    && !(definition.sensitive && value.as_str() == Some("[redacted]")) =>
            {
                let key = definition.key.clone();
                self.set(key, Some(value));
                self.input = None;
                true
            }
            _ => {
                self.error = Some("Invalid value; check type, bounds and options".into());
                false
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let definitions = self.definitions();
        let compact = frame.area().width < 60 || frame.area().height < 16;
        let width = frame.area().width.min(88);
        let mut lines = Vec::new();
        let mut cursor_column = None;
        let mut group = String::new();
        for (index, definition) in definitions.iter().enumerate() {
            if compact && index != self.selected {
                continue;
            }
            let next_group = if definition.key.starts_with("shared.") {
                "Shared"
            } else if definition.provider == "core" {
                "General"
            } else {
                &definition.provider
            };
            if next_group != group && !compact {
                lines.push(Line::styled(
                    super::view::safe(next_group),
                    Style::default().fg(Color::Yellow),
                ));
                group = next_group.into();
            }
            let selected = index == self.selected;
            let (value, source) = self.value(definition);
            let label = label(definition);
            let editing = selected && self.input.is_some();
            let value = if let Some(input) = self.input.as_ref().filter(|_| selected) {
                if definition.sensitive {
                    "*".repeat(input.chars().count().min(80))
                } else {
                    super::view::safe(input)
                }
            } else {
                super::view::safe(&preview(definition, value))
            };
            let unit = super::view::safe(definition.contract.unit.as_deref().unwrap_or(""));
            let marker = if source == "cli locked" {
                " locked"
            } else if self.changes.contains_key(&definition.key) {
                " *"
            } else {
                ""
            };
            let inner_width = usize::from(width.saturating_sub(4));
            let minimum_value_width = 8;
            let marker_width = Line::raw(marker).width();
            let label_width = inner_width
                .saturating_sub(16)
                .clamp(1, 32)
                .min(inner_width.saturating_sub(3 + minimum_value_width + marker_width));
            let label = clipped(&super::view::safe(&label), label_width, false);
            let remaining_width = inner_width.saturating_sub(label_width + 3);
            let unit = clipped(
                &format!(" {unit}"),
                remaining_width.saturating_sub(minimum_value_width + marker_width),
                false,
            );
            let suffix = format!("{unit}{marker}");
            let value_width = remaining_width.saturating_sub(Line::raw(&suffix).width());
            let value = if editing {
                format!(
                    "[{}_]",
                    clipped(&value, value_width.saturating_sub(3), true)
                )
            } else if definition.contract.value_type == ValueType::Boolean {
                clipped(&value, value_width, false)
            } else {
                let (left, right) = if definition.contract.limits.allowed.is_some() {
                    ('‹', '›')
                } else {
                    ('[', ']')
                };
                format!(
                    "{left}{}{right}",
                    clipped(&value, value_width.saturating_sub(2), false)
                )
            };
            let padding = " ".repeat(label_width.saturating_sub(Line::raw(&label).width()));
            let prefix = format!("{} {label}{padding} ", if selected { ">" } else { " " });
            if editing && self.focus == 0 {
                cursor_column = Some(
                    Line::raw(format!("{prefix}{value}"))
                        .width()
                        .saturating_sub(2),
                );
            }
            lines.push(Line::styled(
                format!("{prefix}{value}{suffix}"),
                if selected && self.focus == 0 {
                    Style::default().fg(Color::White).bg(Color::DarkGray)
                } else {
                    Style::default()
                },
            ));
        }
        let info = self
            .definition()
            .map(|definition| {
                let limits = &definition.contract.limits;
                let constraints = if definition.sensitive {
                    "Constraints: [masked]".into()
                } else {
                    let max = limits.max.as_ref().map(ToString::to_string);
                    let max = match max.as_deref() {
                        Some("18446744073709551615") => "platform integer limit",
                        Some(max) => max,
                        None => "none",
                    };
                    format!(
                        "Constraints: min={} max={}{}",
                        limits
                            .min
                            .as_ref()
                            .map_or("none".into(), ToString::to_string),
                        max,
                        limits
                            .allowed
                            .as_ref()
                            .map_or(String::new(), |options| format!(
                                " options={}",
                                serde_json::to_string(options).unwrap_or_default()
                            ))
                    )
                };
                format!(
                    "Key: {}\nDefault: {} {}\n{}\nValue: {}\n{}{}",
                    definition.key,
                    preview(definition, &definition.default),
                    definition.contract.unit.as_deref().unwrap_or(""),
                    constraints,
                    preview(definition, self.value(definition).0),
                    description(definition),
                    self.error
                        .as_ref()
                        .map_or(String::new(), |error| format!("\nError: {error}")),
                )
            })
            .unwrap_or_default();
        let info_paragraph = Paragraph::new(
            info.lines()
                .map(|line| Line::raw(super::view::safe(line)))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false });
        let info_height = if self.info_open {
            if compact { 1 } else { 4 }
        } else {
            0
        };
        let status_height = 1;
        let description_height = if compact { 1 } else { 2 };
        let description = Paragraph::new(self.definition().map_or(String::new(), |definition| {
            super::view::safe(description(definition))
        }))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Gray));
        let controls = if self.focus != 0 {
            "Enter Choose  Tab/Shift-Tab Focus  Ctrl-S Save  Esc Back"
        } else if self.input.is_some() {
            if width < 60 {
                "Enter Accept  Esc Undo field\nTab Controls  Ctrl-U Clear\nCtrl-S Save"
            } else {
                "Enter Accept  Esc Undo field  Ctrl-U Clear  Tab Controls  Ctrl-S Save  Ctrl-R Reset"
            }
        } else if width < 60 {
            "Enter Edit\nCtrl-S Save  Ctrl-R Reset\nEsc discard  Tab Controls"
        } else {
            "Enter Edit  Tab Controls  Ctrl-S Save  Ctrl-R Reset  Esc discard"
        };
        let footer = Paragraph::new(controls)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Gray));
        let footer_height = footer
            .line_count(width.saturating_sub(4))
            .min(usize::from(u16::MAX)) as u16;
        let desired = lines.len() as u16
            + info_height
            + status_height
            + description_height
            + footer_height
            + 3;
        let popup = super::view::panel(frame.area(), width, desired);
        frame.render_widget(Clear, popup);
        let title = if self.changes.is_empty() {
            " Settings ".into()
        } else {
            format!(" Settings ({} changes) ", self.changes.len())
        };
        let block = Block::bordered()
            .title(title)
            .padding(Padding::horizontal(1));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let [list, detail, status, buttons, explanation, footer_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(info_height),
            Constraint::Length(status_height),
            Constraint::Length(1),
            Constraint::Length(description_height),
            Constraint::Length(footer_height),
        ])
        .areas(inner);
        let selected_line = lines
            .iter()
            .position(|line| line.to_string().starts_with('>'))
            .unwrap_or(0);
        let offset = selected_line.saturating_sub(usize::from(list.height).saturating_sub(1));
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), list);
        let max_scroll = info_paragraph
            .line_count(detail.width)
            .saturating_sub(usize::from(detail.height))
            .min(usize::from(u16::MAX)) as u16;
        self.max_scroll.set(max_scroll);
        self.page_height.set(detail.height);
        frame.render_widget(
            info_paragraph.scroll((self.scroll.min(max_scroll), 0)),
            detail,
        );
        if let Some(column) = cursor_column {
            let row = selected_line.saturating_sub(offset) as u16;
            if row < list.height && column < usize::from(list.width) {
                frame.set_cursor_position((list.x + column as u16, list.y + row));
            }
        }
        let status_text = self.error.as_deref().map_or_else(
            || {
                if self.changes.is_empty() {
                    ""
                } else {
                    "Unsaved changes"
                }
            },
            |error| {
                if compact {
                    error.split(';').next().unwrap_or(error)
                } else {
                    error
                }
            },
        );
        frame.render_widget(
            Paragraph::new(status_text)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Yellow)),
            status,
        );
        let button = |label: &'static str, focus| {
            Span::styled(
                label,
                if self.focus == focus {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                },
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                button("[Cancel]", 1),
                Span::raw("  "),
                button("[Save]", 2),
            ]))
            .alignment(ratatui::layout::Alignment::Right),
            buttons,
        );
        let height = description
            .line_count(explanation.width)
            .min(usize::from(explanation.height)) as u16;
        frame.render_widget(
            description,
            Rect::new(
                explanation.x,
                explanation.bottom() - height,
                explanation.width,
                height,
            ),
        );
        frame.render_widget(footer, footer_area);
    }
}

fn preview(definition: &Definition, value: &Value) -> String {
    if definition.sensitive {
        return "[masked]".into();
    }
    match value {
        Value::Bool(value) => if *value { "[x]" } else { "[ ]" }.into(),
        _ => value.to_string(),
    }
}

fn label(definition: &Definition) -> String {
    match definition.key.as_str() {
        "refresh.interval_ms" => "Refresh interval".into(),
        "sampling.interval_ms" => "Sampling interval".into(),
        _ => readable(
            definition
                .key
                .strip_prefix(&format!("extensions.{}.", definition.provider))
                .unwrap_or(
                    definition
                        .key
                        .strip_prefix("shared.")
                        .unwrap_or(&definition.key),
                ),
        ),
    }
}

pub(super) fn readable(value: &str) -> String {
    let mut text = value.replace(['.', '_', '-'], " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

fn description(definition: &Definition) -> &str {
    match definition.key.as_str() {
        "refresh.interval_ms" => "Wait between completed collections.",
        "sampling.interval_ms" => "Time between resource samples.",
        _ => &definition.description,
    }
}

fn clipped(value: &str, width: usize, tail: bool) -> String {
    let mut used = 0;
    let mut chars = Vec::new();
    let mut add = |ch: char| {
        used += Span::raw(ch.to_string()).width();
        if used > width {
            return false;
        }
        chars.push(ch);
        true
    };
    if tail {
        for ch in value.chars().rev() {
            if !add(ch) {
                break;
            }
        }
        chars.reverse();
    } else {
        for ch in value.chars() {
            if !add(ch) {
                break;
            }
        }
    }
    chars.into_iter().collect()
}
