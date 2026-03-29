use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::model::{App, VisibleKind};
use crate::parser::{FileStatus, HunkLine};

const HUNK_MARKER_TOP: &str = "┌";
const HUNK_MARKER_MID: &str = "│";
const HUNK_MARKER_BOT: &str = "└";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let main_area = chunks[0];
    let status_area = chunks[1];

    draw_main_view(frame, app, main_area);
    draw_status_bar(frame, app, status_area);

    if app.show_help {
        draw_help_dialog(frame);
    }

    if app.show_file_list {
        draw_file_list_popup(frame, app);
    }
}

fn draw_main_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Diff Review ")
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = app.visible_items();
    if visible.is_empty() {
        let msg = Paragraph::new("  All hunks confirmed! Press q to exit.");
        frame.render_widget(msg, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let cursor = app.cursor;

    for (vis_idx, vi) in visible.iter().enumerate() {
        match &vi.kind {
            VisibleKind::Folder(path) => {
                let is_focused = vis_idx == cursor;
                let all_conf = app.folder_all_confirmed(path);
                let none_conf = app.folder_none_confirmed(path);
                let indicator = if all_conf {
                    "x"
                } else if none_conf {
                    " "
                } else {
                    "~"
                };
                let (open, close) = if is_focused { ("(", ")") } else { ("[", "]") };
                let fold_icon = if app.folded.contains(path) {
                    "▶"
                } else {
                    "▼"
                };
                let name = path.rsplit('/').next().unwrap_or(path);
                let indent = "──".repeat(vi.depth + 1);

                let line_style = if is_focused {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("├{} {}{}{} {} {}/", indent, open, indicator, close, fold_icon, name),
                        line_style.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ───", Style::default().fg(Color::DarkGray)),
                ]));
            }
            VisibleKind::File(file_idx) => {
                let file = &app.files[*file_idx];
                let is_focused = vis_idx == cursor;
                let all_conf = file.all_confirmed();
                let none_conf = file.none_confirmed();
                let indicator = if all_conf {
                    "x"
                } else if none_conf {
                    " "
                } else {
                    "~"
                };
                let (open, close) = if is_focused { ("(", ")") } else { ("[", "]") };
                let name = file.rel_path.rsplit('/').next().unwrap_or(&file.rel_path);
                let indent = "──".repeat(vi.depth + 1);

                let (status_char, status_color) = match file.status {
                    FileStatus::Modified => ("M", Color::Yellow),
                    FileStatus::Added => ("A", Color::Green),
                };

                let line_style = if is_focused {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("├{} {}{}{} {}", indent, open, indicator, close, name),
                        line_style,
                    ),
                    Span::raw("  "),
                    Span::styled(status_char, Style::default().fg(status_color)),
                    Span::raw("  "),
                    Span::styled(
                        format!("+{}", file.additions),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("-{}", file.deletions),
                        Style::default().fg(Color::Red),
                    ),
                ]));
            }
            VisibleKind::HunkHeader(file_idx, hunk_idx) => {
                let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                let is_focused = vis_idx == cursor;
                let indicator = if hunk.confirmed { "x" } else { " " };
                let (open, close) = if is_focused { ("(", ")") } else { ("[", "]") };

                let marker_color = if is_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                let header_style = if is_focused {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(vec![
                    Span::styled(HUNK_MARKER_TOP, Style::default().fg(marker_color)),
                    Span::styled(
                        format!(" {}{}{} {}", open, indicator, close, hunk.header),
                        header_style,
                    ),
                    Span::raw("  "),
                    Span::styled(format!("+{}", hunk.additions), Style::default().fg(Color::Green)),
                    Span::raw(" "),
                    Span::styled(format!("-{}", hunk.deletions), Style::default().fg(Color::Red)),
                ]));
            }
            VisibleKind::HunkLine(file_idx, hunk_idx, line_idx) => {
                let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                let is_last = *line_idx + 1 == hunk.lines.len();
                let marker = if is_last {
                    HUNK_MARKER_BOT
                } else {
                    HUNK_MARKER_MID
                };

                // Use same marker color as the hunk header
                let hunk_header_focused = visible.iter().enumerate().any(|(vi, item)| {
                    vi == cursor
                        && matches!(&item.kind, VisibleKind::HunkHeader(fi, hi) if *fi == *file_idx && *hi == *hunk_idx)
                });
                let marker_color = if hunk_header_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };

                let hunk_line = &hunk.lines[*line_idx];
                let (prefix, text, style) = match hunk_line {
                    HunkLine::Context(s) => (" ", s.as_str(), Style::default().fg(Color::DarkGray)),
                    HunkLine::Addition(s) => ("+", s.as_str(), Style::default().fg(Color::Green)),
                    HunkLine::Deletion(s) => ("-", s.as_str(), Style::default().fg(Color::Red)),
                };

                lines.push(Line::from(vec![
                    Span::styled(marker, Style::default().fg(marker_color)),
                    Span::styled(format!(" {} ", prefix), style),
                    Span::styled(text, style),
                ]));
            }
        }
    }

    // Apply scroll offset to keep cursor visible
    let visible_height = inner.height as usize;
    let scroll = compute_scroll(cursor, &visible, visible_height, app.scroll_offset);
    app.scroll_offset = scroll;

    let visible_lines: Vec<Line> = lines.into_iter().skip(scroll).take(visible_height).collect();
    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, inner);
}

fn compute_scroll(
    cursor: usize,
    _visible: &[crate::model::VisibleItem],
    visible_height: usize,
    current_scroll: usize,
) -> usize {
    // Find line offset of cursor item
    let line_offset = cursor;

    if line_offset < current_scroll {
        return line_offset;
    }

    if line_offset >= current_scroll + visible_height {
        return line_offset.saturating_sub(visible_height / 2);
    }

    current_scroll
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let confirmed = app.total_confirmed_hunks();
    let total = app.total_hunks();

    let status = Line::from(vec![
        Span::styled(" ?", Style::default().fg(Color::Yellow)),
        Span::raw(":help "),
        Span::styled("Space", Style::default().fg(Color::Yellow)),
        Span::raw(":confirm "),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(":confirm+next "),
        Span::styled("f", Style::default().fg(Color::Yellow)),
        Span::raw(":files "),
        Span::styled("←→", Style::default().fg(Color::Yellow)),
        Span::raw(":fold "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(":quit "),
        Span::raw(format!(" {}/{} confirmed", confirmed, total)),
    ]);

    frame.render_widget(Paragraph::new(status), area);
}

fn draw_help_dialog(frame: &mut Frame) {
    let area = frame.area();
    let w = 55.min(area.width.saturating_sub(4));
    let h = 22.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let dialog = Rect::new(x, y, w, h);

    frame.render_widget(Clear, dialog);

    let help_text = vec![
        Line::from(Span::styled(
            "Keybindings",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  ↑/↓        ", Style::default().fg(Color::Yellow)),
            Span::raw("Navigate items"),
        ]),
        Line::from(vec![
            Span::styled("  j/k        ", Style::default().fg(Color::Yellow)),
            Span::raw("Jump to prev/next file"),
        ]),
        Line::from(vec![
            Span::styled("  ←          ", Style::default().fg(Color::Yellow)),
            Span::raw("Fold folder"),
        ]),
        Line::from(vec![
            Span::styled("  →          ", Style::default().fg(Color::Yellow)),
            Span::raw("Unfold folder"),
        ]),
        Line::from(vec![
            Span::styled("  Space      ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle confirmed"),
        ]),
        Line::from(vec![
            Span::styled("  Enter      ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle and advance"),
        ]),
        Line::from(vec![
            Span::styled("  a          ", Style::default().fg(Color::Yellow)),
            Span::raw("Invert confirmation"),
        ]),
        Line::from(vec![
            Span::styled("  f          ", Style::default().fg(Color::Yellow)),
            Span::raw("File list with fuzzy search"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  q / Esc    ", Style::default().fg(Color::Red)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C     ", Style::default().fg(Color::Red)),
            Span::raw("Force quit"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  Press ? / Esc / Space to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, dialog);
}

fn draw_file_list_popup(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let w = 70.min(area.width.saturating_sub(6));
    let h = (area.height as usize * 3 / 4).min(area.height.saturating_sub(4) as usize) as u16;
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let dialog = Rect::new(x, y, w, h);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Files (type to search, Enter to jump, Esc to close) ")
        .title_style(Style::default().fg(Color::Cyan))
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Split inner into search bar + file list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Search bar
    let search_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(Color::Yellow)),
        Span::raw(&app.file_list_query),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(search_line), chunks[0]);

    // File list
    let filtered = app.filtered_files();
    let list_area = chunks[1];

    let mut items: Vec<ListItem> = Vec::new();
    for (file_idx, match_info) in &filtered {
        let file = &app.files[*file_idx];
        let confirmed_marker = if file.all_confirmed() {
            "[x] "
        } else if file.none_confirmed() {
            "[ ] "
        } else {
            "[~] "
        };

        let (status_char, status_color) = match file.status {
            FileStatus::Modified => ("M", Color::Yellow),
            FileStatus::Added => ("A", Color::Green),
        };

        // Build path with highlighted match indices
        let path = &file.rel_path;
        let mut spans = vec![Span::raw(confirmed_marker.to_string())];

        if let Some((_, indices)) = match_info {
            let mut last = 0;
            for &idx in indices {
                if idx > last {
                    spans.push(Span::raw(&path[last..idx]));
                }
                if idx < path.len() {
                    spans.push(Span::styled(
                        &path[idx..idx + path[idx..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)],
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                    last = idx + path[idx..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                }
            }
            if last < path.len() {
                spans.push(Span::raw(&path[last..]));
            }
        } else {
            spans.push(Span::raw(path.as_str()));
        }

        spans.push(Span::raw("  "));
        spans.push(Span::styled(status_char, Style::default().fg(status_color)));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("+{}", file.additions),
            Style::default().fg(Color::Green),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("-{}", file.deletions),
            Style::default().fg(Color::Red),
        ));

        items.push(ListItem::new(Line::from(spans)));
    }

    let mut list_state = ListState::default();
    if !filtered.is_empty() {
        let clamped = app.file_list_cursor.min(filtered.len().saturating_sub(1));
        app.file_list_cursor = clamped;
        list_state.select(Some(clamped));
    }

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, list_area, &mut list_state);
}
