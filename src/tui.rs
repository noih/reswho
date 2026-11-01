mod state;
#[cfg(test)]
mod tests;
mod view;
mod worker;

use crate::snapshot::Options;
use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use state::{App, Sort};
use std::{
    io::{self, IsTerminal},
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};
use worker::Worker;

struct TerminalRestore(bool);

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        if self.0 {
            let _ = ratatui::try_restore();
        }
    }
}

pub fn run(options: Options, scope: String, depth: usize) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("TUI requires terminal stdin and stdout; use --md or --json");
    }
    let worker = Worker::start(options);
    let mut restore = TerminalRestore(true);
    let mut terminal = ratatui::try_init().context("initialize terminal")?;
    let result = event_loop(&mut terminal, &worker, App::new(scope, depth));
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
) -> Result<()> {
    let mut table = TableState::default();
    let mut next_refresh = Instant::now();
    let mut dirty = true;
    let mut displayed_age = None;
    let animation_start = Instant::now();
    loop {
        match worker.results.try_recv() {
            Ok(result) => {
                app.update(result);
                dirty = true;
                next_refresh = Instant::now() + Duration::from_secs(2);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => bail!("resource collector stopped unexpectedly"),
        }
        if !app.collecting && !app.paused && Instant::now() >= next_refresh {
            app.collecting = worker.request();
            dirty = true;
        }
        let age = app.updated.map(|time| time.elapsed().as_secs());
        let loading_frame = if app.snapshot.is_none() && app.collecting {
            (animation_start.elapsed().as_millis() / 250 % 4) as usize
        } else {
            0
        };
        if app.loading_frame != loading_frame {
            app.loading_frame = loading_frame;
            dirty = true;
        }
        if dirty || displayed_age != age {
            terminal.draw(|frame| view::draw(frame, &app, &mut table))?;
            displayed_age = age;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    dirty = true;
                    let browsing = !app.editing && !app.help;
                    if handle_key(&mut app, key) {
                        return Ok(());
                    }
                    if browsing
                        && key.code == KeyCode::Char('r')
                        && !app.editing
                        && !app.help
                        && !app.collecting
                    {
                        app.collecting = worker.request();
                    }
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }
    if app.help {
        if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
            app.help = false;
        }
        return key.code == KeyCode::Char('q');
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
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
        KeyCode::Home => app.move_selection(isize::MIN),
        KeyCode::End => app.move_selection(isize::MAX),
        KeyCode::Right | KeyCode::Char('l') => app.expand(),
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
        KeyCode::Char('a') => {
            app.active_only = !app.active_only;
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
        KeyCode::Char('?') => app.help = true,
        _ => {}
    }
    false
}
