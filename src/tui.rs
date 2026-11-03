mod details;
mod extensions;
mod help;
mod settings;
mod state;
#[cfg(test)]
mod tests;
mod view;
mod worker;

use crate::snapshot::Options;
use crate::sort::Sort;
use anyhow::{Context, Result, bail};
use crossterm::{
    SynchronizedUpdate,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::EndSynchronizedUpdate,
};
use ratatui::widgets::TableState;
use state::App;
use std::{
    io::{self, IsTerminal},
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};
use worker::Worker;

struct TerminalRestore(bool);

struct SelectionAnchor {
    key: String,
    screen_row: usize,
}

impl SelectionAnchor {
    fn capture(app: &App, table: &TableState) -> Option<Self> {
        app.rows.get(app.selected).map(|row| Self {
            key: row.key.clone(),
            screen_row: app.selected.saturating_sub(table.offset()),
        })
    }

    fn restore(self, app: &App, table: &mut TableState) {
        if app
            .rows
            .get(app.selected)
            .is_some_and(|row| row.key == self.key)
        {
            *table.offset_mut() = app.selected.saturating_sub(self.screen_row);
        }
    }
}

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        if self.0 {
            let _ = ratatui::try_restore();
            let _ = crossterm::execute!(io::stdout(), EndSynchronizedUpdate);
        }
    }
}

pub fn run(
    options: Options,
    scope: String,
    depth: usize,
    sort: Sort,
    refresh_interval: Duration,
    settings: crate::settings::edit::Settings,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("TUI requires terminal stdin and stdout; use --md or --json");
    }
    let inventory = options
        .extensions
        .list(&options.settings, options.containers);
    let registry = options.extensions.clone();
    let containers = options.containers;
    let worker = Worker::start(options);
    let mut restore = TerminalRestore(true);
    let mut app = App::new(scope, depth);
    app.extensions = worker.report.lock().expect("report lock").clone();
    app.inventory = inventory;
    app.extension_registry = Some(registry);
    app.containers = containers;
    app.sort = sort;
    app.settings = Some(settings::Popup::new(settings));
    request_snapshot(&worker, &mut app);
    let mut table = TableState::default();
    let mut terminal = io::stdout()
        .sync_update(|_| -> Result<_> {
            let mut terminal = ratatui::try_init().context("initialize terminal")?;
            terminal.draw(|frame| view::draw(frame, &app, &mut table))?;
            Ok(terminal)
        })
        .context("present initial frame")??;
    let result = event_loop(&mut terminal, &worker, app, table, refresh_interval);
    drop(terminal);
    let restored = ratatui::try_restore().context("restore terminal");
    restore.0 = restored.is_err();
    drop(restore);
    drop(worker);
    result.and(restored)
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    worker: &Worker,
    mut app: App,
    mut table: TableState,
    mut refresh_interval: Duration,
) -> Result<()> {
    let mut next_refresh = Instant::now();
    let mut dirty = false;
    let animation_start = Instant::now();
    loop {
        match worker.results.try_recv() {
            Ok(result) => {
                app.extensions = worker.report.lock().expect("report lock").clone();
                let anchor = SelectionAnchor::capture(&app, &table);
                app.update(result);
                if let Some(anchor) = anchor {
                    anchor.restore(&app, &mut table);
                }
                dirty = true;
                next_refresh = Instant::now()
                    .checked_add(refresh_interval)
                    .context("refresh interval cannot be safely scheduled")?;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => bail!("resource collector stopped unexpectedly"),
        }
        if should_collect(&app, next_refresh, Instant::now()) {
            request_snapshot(worker, &mut app);
            dirty |= app.snapshot.is_none();
        }
        let loading_frame = if app.snapshot.is_none() && app.collecting {
            (animation_start.elapsed().as_millis() / 250 % 4) as usize
        } else {
            0
        };
        if app.loading_frame != loading_frame {
            app.loading_frame = loading_frame;
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| view::draw(frame, &app, &mut table))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    dirty = true;
                    let anchor = SelectionAnchor::capture(&app, &table);
                    if handle_key(&mut app, key) {
                        return Ok(());
                    }
                    if let Some(settings) =
                        app.settings.as_mut().and_then(|popup| popup.applied.take())
                    {
                        refresh_interval = settings.interval("refresh.interval_ms");
                        if let Some(registry) = &app.extension_registry {
                            app.inventory = registry.list(&settings, app.containers);
                        }
                        worker.set_settings(settings);
                        next_refresh = Instant::now()
                            .checked_add(refresh_interval)
                            .context("refresh interval cannot be safely scheduled")?;
                    }
                    if let Some(anchor) = anchor {
                        anchor.restore(&app, &mut table);
                    }
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
    }
}

fn should_collect(app: &App, next_refresh: Instant, now: Instant) -> bool {
    !app.collecting
        && (app.manual_refresh.is_some()
            || app
                .details
                .as_ref()
                .is_some_and(|details| details.needs_query)
            || (!app.paused && now >= next_refresh))
}

fn request_snapshot(worker: &Worker, app: &mut App) {
    let pid = app.details.as_ref().and_then(details::Details::query_pid);
    app.collecting = worker.request(pid);
    if app.collecting
        && let Some(details) = &mut app.details
    {
        details.needs_query = false;
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }
    if let Some(popup) = &mut app.settings
        && popup.open
    {
        popup.key(key);
        return false;
    }
    if app.extensions_page {
        return extensions::key(app, key);
    }
    if let Some(details) = &mut app.details {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('d') => {
                app.details = None
            }
            KeyCode::Down | KeyCode::Char('j') => details.scroll(1),
            KeyCode::Up | KeyCode::Char('k') => details.scroll(-1),
            KeyCode::PageDown => details.scroll(10),
            KeyCode::PageUp => details.scroll(-10),
            KeyCode::Home => details.scroll(isize::MIN),
            KeyCode::End => details.scroll(isize::MAX),
            KeyCode::Char('r') => app.request_refresh(),
            KeyCode::Char('p') => app.paused = !app.paused,
            _ => {}
        }
        return false;
    }
    if app.help {
        if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
            app.help = false;
        } else {
            app.help_view.key(key);
        }
        return false;
    }
    if app.editing {
        match key.code {
            KeyCode::Esc => {
                app.query.clear();
                app.editing = false;
                app.rebuild();
            }
            KeyCode::Enter => app.editing = false,
            KeyCode::Backspace => {
                app.query.pop();
                app.rebuild();
            }
            KeyCode::Char(ch)
                if !ch.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && app.query.len() < 1024 =>
            {
                app.query.push(ch);
                app.rebuild();
            }
            _ => {}
        }
        return false;
    }
    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('s') => {
            if let Some(popup) = &mut app.settings {
                popup.show();
            }
        }
        KeyCode::Char('e') => app.extensions_page = true,
        KeyCode::Char('d') => app.open_details(),
        KeyCode::Char('r') => app.request_refresh(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
        KeyCode::Home => app.move_selection(isize::MIN),
        KeyCode::End => app.move_selection(isize::MAX),
        KeyCode::Right | KeyCode::Char('l') => {
            if app
                .rows
                .get(app.selected)
                .is_some_and(|row| row.has_children)
            {
                app.expand();
            } else {
                app.open_details();
            }
        }
        KeyCode::Left | KeyCode::Char('h') => app.collapse(),
        KeyCode::Enter | KeyCode::Char(' ') => app.toggle(),
        KeyCode::Char('c') => {
            app.sort = Sort::Cpu;
            app.rebuild();
        }
        KeyCode::Char('n') => {
            app.sort = Sort::Name;
            app.rebuild();
        }
        KeyCode::Char('m') => {
            app.sort = Sort::Memory;
            app.rebuild();
        }
        KeyCode::Char('u') => {
            app.sort = Sort::Uptime;
            app.rebuild();
        }
        KeyCode::Char('/') => app.editing = true,
        KeyCode::Esc => {
            app.query.clear();
            app.rebuild();
        }
        KeyCode::Char('p') => app.paused = !app.paused,
        KeyCode::Char(']') => {
            let width = ratatui::text::Line::from(view::selected_paths(app)).width();
            app.path_scroll = app
                .path_scroll
                .saturating_add(16)
                .min(width.saturating_sub(1).min(u16::MAX as usize) as u16);
        }
        KeyCode::Char('[') => app.path_scroll = app.path_scroll.saturating_sub(16),
        KeyCode::Char('?') => {
            app.help = true;
            app.help_view = help::Help::default();
        }
        _ => {}
    }
    false
}
