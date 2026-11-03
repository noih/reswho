use super::view::safe;
use crate::{
    model::{Node, Protocol, Snapshot},
    output, tree,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{cell::Cell, collections::HashSet};

enum Status {
    Loading,
    Ready,
    Failed(String),
    Unavailable(String),
}

pub(super) struct Details {
    resource_ref: Option<String>,
    pid: u32,
    started: Option<u64>,
    node: Option<Node>,
    status: Status,
    pub needs_query: bool,
    vertical: usize,
    height: Cell<usize>,
    width: Cell<usize>,
}

impl Details {
    pub fn new(mut node: Node) -> Self {
        if node.resource.is_some() {
            return Self {
                resource_ref: Some(node.reference.clone()),
                pid: 0,
                started: None,
                node: Some(node),
                status: Status::Ready,
                needs_query: false,
                vertical: 0,
                height: Cell::new(1),
                width: Cell::new(1),
            };
        }
        let process = node.process.as_mut().expect("process detail selection");
        let pid = process.pid;
        let started = process.started_unix_us;
        process.command = None;
        process.extension_details.clear();
        node.children.clear();
        Self {
            resource_ref: None,
            pid,
            started,
            node: Some(node),
            status: if started.is_some() {
                Status::Loading
            } else {
                Status::Unavailable("Process identity unavailable; command not queried".into())
            },
            needs_query: started.is_some(),
            vertical: 0,
            height: Cell::new(1),
            width: Cell::new(1),
        }
    }

    pub fn query_pid(&self) -> Option<u32> {
        (self.resource_ref.is_none() && !matches!(self.status, Status::Unavailable(_)))
            .then_some(self.pid)
    }

    pub fn retryable_error(&self) -> bool {
        matches!(self.status, Status::Failed(_))
            || self
                .node
                .as_ref()
                .and_then(|node| node.process.as_ref())
                .and_then(|process| process.command.as_ref())
                .is_some_and(|command| !command.unavailable.is_empty())
    }

    pub fn refresh(&mut self) {
        if self.query_pid().is_some() {
            self.needs_query = true;
            if matches!(self.status, Status::Failed(_)) {
                self.status = Status::Loading;
            }
        }
    }

    pub fn update(&mut self, result: &Result<Snapshot, String>) {
        if let Some(reference) = &self.resource_ref {
            match result {
                Ok(snapshot) => match tree::select(&snapshot.root, reference) {
                    Ok(node) => {
                        self.node = Some(node);
                        self.status = Status::Ready;
                    }
                    Err(_) => {
                        self.node = None;
                        self.status = Status::Unavailable("Resource no longer observed".into());
                    }
                },
                Err(_) => {
                    self.node = None;
                    self.status = Status::Failed("Resource refresh failed".into());
                }
            }
            return;
        }
        if matches!(self.status, Status::Unavailable(_)) {
            return;
        }
        self.needs_query = false;
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.node = None;
                self.status = Status::Failed(format!("Refresh failed: {error}"));
                return;
            }
        };
        let Ok(mut node) = tree::select(&snapshot.root, &format!("pid:{}", self.pid)) else {
            self.node = None;
            self.status =
                Status::Unavailable("Process no longer present (exited or unavailable)".into());
            return;
        };
        let process = node.process.as_ref().expect("selected PID process");
        if process.started_unix_us != self.started || process.started_unix_us.is_none() {
            self.node = None;
            self.status = Status::Unavailable(
                "Process identity changed or unavailable; close and select a current process"
                    .into(),
            );
            return;
        }
        self.needs_query = process.command.is_none() && process.extension_details.is_empty();
        self.status = if self.needs_query {
            Status::Loading
        } else {
            Status::Ready
        };
        node.children.clear();
        self.node = Some(node);
    }

    fn rows(&self) -> Vec<DetailRow> {
        let mut rows = Vec::new();
        match &self.status {
            Status::Loading => rows.push(DetailRow::notice("Loading command...")),
            Status::Failed(error) | Status::Unavailable(error) => {
                rows.push(DetailRow::notice(format!("! {error}")));
            }
            Status::Ready => {}
        }
        if let Some(node) = &self.node {
            if let Some(resource) = &node.resource {
                rows.push(DetailRow::heading(&resource.label));
                rows.push(DetailRow::field(
                    "Provider",
                    format!("{}/{}", resource.provider, resource.version),
                ));
                rows.push(DetailRow::field("Identity", &resource.id));
                rows.push(DetailRow::field("Kind", &resource.kind));
                if let Some(path) = &resource.path {
                    rows.push(DetailRow::field("Path", path.display().to_string()));
                }
                rows.extend(
                    resource
                        .metrics
                        .iter()
                        .map(|metric| DetailRow::text(output::metric_text(metric))),
                );
                return rows;
            }
            let process = node.process.as_ref().expect("detail process");
            rows.extend(
                process
                    .roles
                    .iter()
                    .map(|role| DetailRow::text(output::role_text(role))),
            );
            rows.push(DetailRow::heading(&node.name));
            rows.push(DetailRow::text(format!(
                "CPU/core {}  RAM {}  Uptime {}  PPID {}  UID {}",
                output::percent(process.cpu_percent),
                output::memory(process.resident_bytes),
                output::uptime(process.uptime_seconds),
                process.ppid.map_or("?".into(), |v| v.to_string()),
                process.uid.map_or("?".into(), |v| v.to_string())
            )));
            rows.push(DetailRow::text(""));
            if let Some(command) = &process.command {
                if let Some(argv) = &command.argv {
                    rows.push(DetailRow::field(
                        "Command",
                        if argv.is_empty() {
                            "[]".into()
                        } else {
                            output::command_text(argv)
                        },
                    ));
                }
                if let Some(entry) = &command.entrypoint {
                    rows.push(DetailRow::field("Entry", entry));
                }
                rows.extend(
                    command
                        .unavailable
                        .iter()
                        .map(|error| DetailRow::notice(format!("! {error}"))),
                );
            }
            for details in &process.extension_details {
                rows.push(DetailRow::heading(format!(
                    "Extension {}/{}",
                    details.provider, details.version
                )));
                for field in &details.fields {
                    rows.push(DetailRow::field(&field.label, field.value.to_string()));
                }
            }
            for (label, path) in [("Cwd", &process.cwd), ("Exe", &process.executable)] {
                if let Some(path) = path {
                    rows.push(DetailRow::field(label, path.display().to_string()));
                }
            }
            rows.push(DetailRow::text(""));
            rows.push(DetailRow::heading("Network"));
            if let Some(network) = &process.network {
                rows.push(DetailRow::text(format!(
                    "TCP/UDP sockets {}{}",
                    network.sockets.len(),
                    if network.complete { "" } else { " + ?" }
                )));
                for socket in &network.sockets {
                    let protocol = match socket.protocol {
                        Protocol::Tcp => "TCP",
                        Protocol::Udp => "UDP",
                    };
                    rows.push(DetailRow::text(format!(
                        "{protocol} {} → {}  {}  fd {}",
                        socket.local,
                        socket.remote.map_or("-".into(), |value| value.to_string()),
                        socket.tcp_state.as_deref().unwrap_or("-"),
                        socket.fd
                    )));
                }
                rows.extend(
                    network
                        .unavailable
                        .iter()
                        .map(|error| DetailRow::notice(format!("! {error}"))),
                );
            } else {
                rows.push(DetailRow::text("TCP/UDP sockets ?"));
            }
            if !node.evidence.is_empty() {
                rows.push(DetailRow::text(""));
                rows.push(DetailRow::heading("Attribution"));
                let mut groups = Vec::new();
                let mut evidence = Vec::new();
                let mut seen = HashSet::new();
                for item in &node.evidence {
                    if !seen.insert(item) {
                        continue;
                    }
                    if let Some((name, _)) = item
                        .strip_prefix("Grouped under: ")
                        .and_then(|value| value.strip_suffix(')'))
                        .and_then(|value| value.rsplit_once(" (group:"))
                    {
                        groups.push(name);
                    } else if let Some(path) = item.strip_prefix("Application bundle: ") {
                        evidence.push(DetailRow::field("App", path));
                    } else if let Some(bundle) = item.strip_prefix("bundle ") {
                        evidence.push(DetailRow::field("Bundle", bundle));
                    } else {
                        evidence.push(DetailRow::text(item));
                    }
                }
                if !groups.is_empty() {
                    rows.push(DetailRow::field("Group", groups.join(" → ")));
                }
                rows.extend(evidence);
            }
            rows.extend(
                process
                    .unavailable
                    .iter()
                    .chain(&node.diagnostics)
                    .map(|error| DetailRow::notice(format!("! {error}"))),
            );
        }
        rows
    }

    #[cfg(test)]
    pub fn lines(&self) -> Vec<String> {
        self.rows()
            .into_iter()
            .map(|row| {
                if row.label.is_empty() {
                    row.value
                } else {
                    format!("{}  {}", row.label, row.value)
                }
            })
            .collect()
    }

    fn wrapped_lines(&self) -> Vec<Line<'static>> {
        self.rows()
            .into_iter()
            .flat_map(|row| row.wrap(self.width.get()))
            .collect()
    }

    pub fn scroll(&mut self, delta: isize) {
        let max = self.wrapped_lines().len().saturating_sub(self.height.get());
        self.vertical = self.vertical.min(max).saturating_add_signed(delta).min(max);
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, paused: bool, refresh_hint: &str) {
        let [title, body, position, controls] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(self.resource_ref.as_ref().map_or_else(
                || format!("Process details · pid:{}", self.pid),
                |reference| format!("Resource details · {reference}"),
            ))
            .style(Style::default().fg(Color::Cyan)),
            title,
        );
        let body = Rect::new(
            body.x + 1,
            body.y,
            body.width.saturating_sub(2),
            body.height,
        );
        self.height.set(body.height as usize);
        self.width.set(body.width as usize);
        let lines = self.wrapped_lines();
        let count = lines.len();
        let vertical = self.vertical.min(count.saturating_sub(self.height.get()));
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(vertical)
                    .take(self.height.get())
                    .collect::<Vec<_>>(),
            ),
            body,
        );
        let status = if paused { " · paused" } else { "" };
        frame.render_widget(
            Paragraph::new(format!(
                "Lines {}-{}/{}{status}",
                vertical + 1,
                (vertical + self.height.get()).min(count),
                count
            ))
            .style(Style::default().fg(Color::DarkGray)),
            position,
        );
        let refresh = if refresh_hint.is_empty() {
            String::new()
        } else {
            format!("{refresh_hint}  ")
        };
        let keys = format!("{refresh}p pause  Esc back");
        frame.render_widget(Paragraph::new(keys), controls);
    }
}

struct DetailRow {
    label: String,
    value: String,
    style: Style,
}

impl DetailRow {
    fn text(value: impl AsRef<str>) -> Self {
        Self {
            label: String::new(),
            value: safe(value.as_ref()),
            style: Style::default(),
        }
    }

    fn field(label: &str, value: impl AsRef<str>) -> Self {
        Self {
            label: label.into(),
            ..Self::text(value)
        }
    }

    fn heading(value: impl AsRef<str>) -> Self {
        Self {
            style: Style::default().fg(Color::Cyan),
            ..Self::text(value)
        }
    }

    fn notice(value: impl AsRef<str>) -> Self {
        Self {
            style: Style::default().fg(Color::Yellow),
            ..Self::text(value)
        }
    }

    fn wrap(self, width: usize) -> Vec<Line<'static>> {
        let indent = if self.label.is_empty() { 2 } else { 9 };
        let prefix = if self.label.is_empty() {
            String::new()
        } else {
            format!("{:<9}", self.label)
        };
        let mut available = width.saturating_sub(prefix.len()).max(2);
        let mut prefix = Span::styled(prefix, Style::default().fg(Color::DarkGray));
        let mut lines = Vec::new();
        let mut chunk = String::new();
        let mut used = 0;
        let value = Span::raw(self.value);
        for grapheme in value.styled_graphemes(Style::default()) {
            let size = Span::raw(grapheme.symbol).width();
            if used + size > available && !chunk.is_empty() {
                lines.push(Line::from(vec![
                    prefix,
                    Span::styled(std::mem::take(&mut chunk), self.style),
                ]));
                prefix = Span::raw(" ".repeat(indent));
                available = width.saturating_sub(indent).max(2);
                used = 0;
            }
            chunk.push_str(grapheme.symbol);
            used += size;
        }
        lines.push(Line::from(vec![prefix, Span::styled(chunk, self.style)]));
        lines
    }
}
