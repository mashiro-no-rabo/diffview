mod fuzzy;
mod model;
mod parser;
mod ui;

use std::io::{self, Read};
use std::process;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::model::App;

fn main() {
    if let Err(e) = run() {
        eprintln!("diffview: {e:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    // Read diff from stdin
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if input.is_empty() {
        eprintln!("No diff to review.");
        return Ok(());
    }

    let files = parser::parse_diff(&input);
    if files.is_empty() {
        panic!(
            "parsed 0 files from {} bytes of input.\nFirst 500 chars of input:\n{}",
            input.len(),
            &input[..input.len().min(500)]
        );
    }

    let mut app = App::new(files);

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app);

    // Terminal teardown. Disable mouse capture first so the terminal stops
    // emitting scroll sequences, then drain anything still buffered (trackpad
    // momentum keeps delivering mouse events for a while after a scroll) before
    // leaving raw mode — otherwise those leftover bytes get echoed to the shell.
    crossterm::execute!(terminal.backend_mut(), DisableMouseCapture)?;
    drain_pending_input();
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Consume any buffered input events until the stream is quiet for a short
/// window. Used on teardown to swallow trailing trackpad-momentum scroll
/// sequences that would otherwise leak to the shell prompt.
fn drain_pending_input() {
    while let Ok(true) = event::poll(Duration::from_millis(50)) {
        if event::read().is_err() {
            break;
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        if app.should_exit {
            return Ok(());
        }

        let ev = event::read()?;

        if let Event::Mouse(mouse) = ev {
            if app.show_help || app.show_file_list {
                continue;
            }
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if app.file_view.is_some() {
                        app.file_view_up();
                    } else {
                        app.scroll_line_up();
                    }
                }
                MouseEventKind::ScrollDown => {
                    if app.file_view.is_some() {
                        app.file_view_down();
                    } else {
                        app.scroll_line_down();
                    }
                }
                _ => {}
            }
            continue;
        }

        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Help dialog intercepts all keys
            if app.show_help {
                match key.code {
                    KeyCode::Char('?')
                    | KeyCode::Esc
                    | KeyCode::Char(' ')
                    | KeyCode::Enter
                    | KeyCode::Char('q') => {
                        app.show_help = false;
                    }
                    _ => {}
                }
                continue;
            }

            // File list popup intercepts keys
            if app.show_file_list {
                handle_file_list_key(app, key.code, key.modifiers);
                continue;
            }

            // Ctrl+C force quit
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                app.should_exit = true;
                continue;
            }

            // File view mode intercepts keys
            if app.file_view.is_some() {
                handle_file_view_key(app, key.code, key.modifiers);
                continue;
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    app.should_exit = true;
                }

                KeyCode::Enter => app.confirm_and_advance(),

                KeyCode::Left => app.fold_current(),
                KeyCode::Right => app.unfold_current(),

                KeyCode::Down => app.cursor_down(),
                KeyCode::Up => app.cursor_up(),

                KeyCode::Char('j') => app.next_file(),
                KeyCode::Char('k') => app.prev_file(),

                KeyCode::Char(' ') => app.confirm_and_advance(),
                KeyCode::Char('a') => app.invert_confirmation(),

                KeyCode::Tab => app.enter_file_view(),

                KeyCode::Char('?') => app.show_help = true,
                KeyCode::Char('f') => {
                    app.show_file_list = true;
                    app.file_list_query.clear();
                    app.file_list_cursor = 0;
                }

                _ => {}
            }
        }
    }
}

fn handle_file_view_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.should_exit = true;
        return;
    }

    match code {
        KeyCode::Up => app.file_view_up(),
        KeyCode::Down => app.file_view_down(),
        KeyCode::Char('k') => app.file_view_half_page_up(),
        KeyCode::Char('j') => app.file_view_half_page_down(),
        KeyCode::Char(' ') => app.file_view_toggle(),
        KeyCode::Enter => app.file_view_toggle_and_advance(),
        KeyCode::Tab | KeyCode::Esc => app.exit_file_view(),
        KeyCode::Char('q') => app.should_exit = true,
        _ => {}
    }
}

fn handle_file_list_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.should_exit = true;
        return;
    }

    match code {
        KeyCode::Esc => {
            app.show_file_list = false;
        }
        KeyCode::Enter => {
            let filtered = app.filtered_files();
            if let Some((file_idx, _)) = filtered.get(app.file_list_cursor) {
                let file_idx = *file_idx;
                app.show_file_list = false;
                app.jump_to_file(file_idx);
            }
        }
        KeyCode::Up => {
            if app.file_list_cursor > 0 {
                app.file_list_cursor -= 1;
            }
        }
        KeyCode::Down => {
            let filtered = app.filtered_files();
            if app.file_list_cursor + 1 < filtered.len() {
                app.file_list_cursor += 1;
            }
        }
        KeyCode::Backspace => {
            app.file_list_query.pop();
            app.file_list_cursor = 0;
        }
        KeyCode::Char(c) => {
            app.file_list_query.push(c);
            app.file_list_cursor = 0;
        }
        _ => {}
    }
}
