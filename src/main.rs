mod fuzzy;
mod model;
mod parser;
mod ui;

use std::io::{self, Read, Write};
use std::process;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
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
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app);

    // Terminal teardown
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result?;

    // Output confirmed hunks as unified diff
    let output = parser::format_confirmed_diff(&app.files);
    if !output.is_empty() {
        io::stdout().write_all(output.as_bytes())?;
    }

    Ok(())
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

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    app.should_exit = true;
                }

                KeyCode::Enter => app.toggle_and_advance(),

                KeyCode::Left => app.fold_current(),
                KeyCode::Right => app.unfold_current(),

                KeyCode::Down => app.cursor_down(),
                KeyCode::Up => app.cursor_up(),

                KeyCode::Char('j') => app.next_file(),
                KeyCode::Char('k') => app.prev_file(),

                KeyCode::Char(' ') => app.toggle_current(),
                KeyCode::Char('a') => app.invert_confirmation(),

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
