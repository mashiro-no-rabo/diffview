use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::{App, FileViewLine, VisibleKind};
use crate::parser::{FileStatus, Hunk, HunkLine};

/// Parse "@@ -old_start,count +new_start,count @@" to extract starting line numbers.
fn parse_hunk_start(header: &str) -> (usize, usize) {
    // header looks like "@@ -36,8 +36,8 @@ optional context"
    let mut old_start = 1;
    let mut new_start = 1;
    if let Some(rest) = header.strip_prefix("@@ -") {
        if let Some(comma_or_space) = rest.find([',', ' ']) {
            old_start = rest[..comma_or_space].parse().unwrap_or(1);
        }
        if let Some(plus) = rest.find('+') {
            let after_plus = &rest[plus + 1..];
            if let Some(comma_or_space) = after_plus.find([',', ' ']) {
                new_start = after_plus[..comma_or_space].parse().unwrap_or(1);
            }
        }
    }
    (old_start, new_start)
}

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

    if app.file_view.is_some() {
        draw_file_view(frame, app, main_area);
    } else {
        draw_main_view(frame, app, main_area);
    }
    draw_status_bar(frame, app, status_area);

    if app.show_help {
        draw_help_dialog(frame);
    }

    if app.show_file_list {
        draw_file_list_popup(frame, app);
    }
}

// ── Row model ──
//
// The main view is rendered as a flat list of full-width terminal rows, each
// carrying its own box-drawing characters. Scrolling is then a plain slice of
// that list, so every row — including the last one — is reachable, and word
// wrapping is just a line producing more than one row.

/// Columns consumed before the diff text: marker + " NNNN " + "P ".
const GUTTER_WIDTH: usize = 9;

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Truncate spans to at most `max` display columns.
fn clip_spans<'a>(spans: Vec<Span<'a>>, max: usize) -> Vec<Span<'a>> {
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0usize;
    for span in spans {
        if used >= max {
            break;
        }
        let w = span.content.width();
        if used + w <= max {
            used += w;
            out.push(span);
            continue;
        }
        let budget = max - used;
        let mut acc = 0usize;
        let mut end = span.content.len();
        for (i, ch) in span.content.char_indices() {
            let cw = ch.width().unwrap_or(0);
            if acc + cw > budget {
                end = i;
                break;
            }
            acc += cw;
        }
        let style = span.style;
        out.push(Span::styled(span.content[..end].to_string(), style));
        used = max;
    }
    out
}

/// Split `text` into chunks that each fit in `max` display columns, breaking
/// at whitespace when possible and hard-breaking long tokens.
fn wrap_text(text: &str, max: usize) -> Vec<&str> {
    if max == 0 || text.width() <= max {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut rest = text;
    loop {
        if rest.width() <= max {
            chunks.push(rest);
            return chunks;
        }
        let mut width = 0usize;
        let mut hard_end = rest.len();
        let mut soft_end: Option<usize> = None;
        for (i, ch) in rest.char_indices() {
            let cw = ch.width().unwrap_or(0);
            if width + cw > max {
                hard_end = i;
                break;
            }
            width += cw;
            if ch == ' ' || ch == '\t' {
                soft_end = Some(i + ch.len_utf8());
            }
        }
        if hard_end == 0 {
            // A single character wider than the budget — emit it alone.
            hard_end = rest.chars().next().map(char::len_utf8).unwrap_or(rest.len());
        }
        let split = soft_end.filter(|&s| s > 0).unwrap_or(hard_end);
        chunks.push(&rest[..split]);
        rest = &rest[split..];
    }
}

/// Wrap a full row: enclosing box borders, content padded to the inner width.
fn frame_row<'a>(borders: &[Color], content: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let inner_w = (width as usize).saturating_sub(2 * borders.len());
    let mut spans: Vec<Span<'a>> = borders
        .iter()
        .map(|c| Span::styled(HUNK_MARKER_MID, Style::default().fg(*c)))
        .collect();
    let content = clip_spans(content, inner_w);
    let pad = inner_w - spans_width(&content);
    spans.extend(content);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    for c in borders.iter().rev() {
        spans.push(Span::styled(HUNK_MARKER_MID, Style::default().fg(*c)));
    }
    Line::from(spans)
}

fn box_top<'a>(borders: &[Color], color: Color, title: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let inner_w = (width as usize).saturating_sub(2 * borders.len());
    let avail = inner_w.saturating_sub(2);
    let style = Style::default().fg(color);
    let title = clip_spans(title, avail);
    let fill = avail - spans_width(&title);
    let mut content = vec![Span::styled("┌", style)];
    content.extend(title);
    content.push(Span::styled("─".repeat(fill), style));
    content.push(Span::styled("┐", style));
    frame_row(borders, content, width)
}

fn box_bottom(borders: &[Color], color: Color, width: u16) -> Line<'static> {
    let inner_w = (width as usize).saturating_sub(2 * borders.len());
    let style = Style::default().fg(color);
    let content = vec![Span::styled(
        format!("└{}┘", "─".repeat(inner_w.saturating_sub(2))),
        style,
    )];
    frame_row(borders, content, width)
}

fn hunk_header_spans(hunk: &Hunk, focused: bool) -> Vec<Span<'_>> {
    let check = if hunk.confirmed { "✓" } else { " " };
    let marker_color = if focused { Color::Cyan } else { Color::DarkGray };
    vec![
        Span::styled(HUNK_MARKER_TOP, Style::default().fg(marker_color)),
        Span::styled(
            format!(" [{}] ", check),
            if focused {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ),
        Span::styled(
            format!("+{}", hunk.additions),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", hunk.deletions),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!("  {}", hunk.header),
            Style::default().fg(Color::DarkGray),
        ),
    ]
}

/// Old/new line numbers at `line_idx`, walking the hunk from its header.
fn line_numbers(hunk: &Hunk, line_idx: usize) -> (usize, usize) {
    let (mut old_line, mut new_line) = parse_hunk_start(&hunk.header);
    for l in &hunk.lines[..line_idx] {
        match l {
            HunkLine::Context(_) => {
                old_line += 1;
                new_line += 1;
            }
            HunkLine::Addition(_) => new_line += 1,
            HunkLine::Deletion(_) => old_line += 1,
        }
    }
    (old_line, new_line)
}

/// Render one diff line as the rows it occupies — one row unless wrapping
/// splits it. Continuation rows keep the gutter blank so text stays aligned.
fn hunk_line_rows<'a>(
    hunk: &'a Hunk,
    line_idx: usize,
    marker_color: Color,
    first_marker: Option<&'static str>,
    context_color: Color,
    inner_width: usize,
    wrap: bool,
) -> Vec<Vec<Span<'a>>> {
    let (old_line, new_line) = line_numbers(hunk, line_idx);
    let (prefix, text, style, line_num) = match &hunk.lines[line_idx] {
        HunkLine::Context(s) => (
            " ",
            s.as_str(),
            Style::default().fg(context_color),
            format!("{:>4}", old_line),
        ),
        HunkLine::Addition(s) => (
            "+",
            s.as_str(),
            Style::default().fg(Color::Green),
            format!("{:>4}", new_line),
        ),
        HunkLine::Deletion(s) => (
            "-",
            s.as_str(),
            Style::default().fg(Color::Red),
            format!("{:>4}", old_line),
        ),
    };

    let is_last = line_idx + 1 == hunk.lines.len();
    let chunks = if wrap {
        wrap_text(text, inner_width.saturating_sub(GUTTER_WIDTH))
    } else {
        vec![text]
    };
    let marker_style = Style::default().fg(marker_color);

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let last_row = i + 1 == chunks.len();
            let marker = match (i, first_marker) {
                (0, Some(m)) => m,
                _ if is_last && last_row => HUNK_MARKER_BOT,
                _ => HUNK_MARKER_MID,
            };
            let mut spans = vec![Span::styled(marker, marker_style)];
            if i == 0 {
                spans.push(Span::styled(
                    format!(" {} ", line_num),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(format!("{} ", prefix), style));
            } else {
                spans.push(Span::raw(" ".repeat(GUTTER_WIDTH - 1)));
            }
            spans.push(Span::styled(*chunk, style));
            spans
        })
        .collect()
}

fn file_title_spans(app: &App, file_idx: usize, focused: bool) -> Vec<Span<'_>> {
    let file = &app.files[file_idx];
    let check = if file.all_confirmed() { "✓" } else { " " };
    let fold_icon = if app.folded_files.contains(&file_idx) || file.all_confirmed() {
        "▶"
    } else {
        "▼"
    };
    let status_char = match file.status {
        FileStatus::Modified => "M",
        FileStatus::Added => "A",
        FileStatus::Deleted => "D",
        FileStatus::Renamed => "R",
        FileStatus::Copied => "C",
    };

    let title = if file.binary {
        let size_info = match (file.binary_old_size, file.binary_new_size) {
            (Some(_), Some(new)) if file.status == FileStatus::Added => format!("  {} bytes", new),
            (Some(old), Some(_)) if file.status == FileStatus::Deleted => {
                format!("  was {} bytes", old)
            }
            (Some(old), Some(new)) => format!("  {} → {} bytes", old, new),
            _ => String::new(),
        };
        format!(
            " [{}] {} {}  {} BIN{} ",
            check, fold_icon, file.rel_path, status_char, size_info
        )
    } else {
        format!(
            " [{}] {} {}  {}  +{} -{} ",
            check, fold_icon, file.rel_path, status_char, file.additions, file.deletions
        )
    };

    let style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    vec![Span::styled(title, style)]
}

fn folder_title_spans(app: &App, path: &str, focused: bool) -> Vec<Span<'static>> {
    let check = if app.folder_all_confirmed(path) {
        "✓"
    } else {
        " "
    };
    let fold_icon = if app.folded.contains(path) { "▶" } else { "▼" };
    let style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    vec![Span::styled(
        format!(" [{}] {} {}/ ", check, fold_icon, path),
        style,
    )]
}

/// The whole main view as rows, plus the row each visible item starts on.
struct Doc<'a> {
    rows: Vec<Line<'a>>,
    item_ys: Vec<usize>,
}

fn build_doc(app: &App, cursor: usize, width: u16) -> Doc<'_> {
    let visible = app.visible_items();
    let mut rows: Vec<Line> = Vec::new();
    let mut item_ys: Vec<usize> = Vec::with_capacity(visible.len());
    // Border colour of each currently open box, outermost first.
    let mut open: Vec<Color> = Vec::new();

    fn close_box(rows: &mut Vec<Line<'_>>, open: &mut Vec<Color>, width: u16) {
        if let Some(color) = open.pop() {
            rows.push(box_bottom(open, color, width));
        }
    }

    for (vis_idx, item) in visible.iter().enumerate() {
        let focused = vis_idx == cursor;
        let color = if focused { Color::Cyan } else { Color::DarkGray };

        match &item.kind {
            VisibleKind::Folder(path) => {
                while open.len() > item.depth {
                    close_box(&mut rows, &mut open, width);
                }
                item_ys.push(rows.len());
                let title = folder_title_spans(app, path, focused);
                rows.push(box_top(&open, color, title, width));
                open.push(color);
            }
            VisibleKind::File(file_idx) => {
                while open.len() > item.depth {
                    close_box(&mut rows, &mut open, width);
                }
                item_ys.push(rows.len());
                let title = file_title_spans(app, *file_idx, focused);
                rows.push(box_top(&open, color, title, width));
                open.push(color);

                // Binary / mode-only files have no hunks — say why.
                let file = &app.files[*file_idx];
                if file.hunks.is_empty()
                    && !app.folded_files.contains(file_idx)
                    && !file.all_confirmed()
                {
                    let msg = if file.binary {
                        match (file.binary_old_size, file.binary_new_size) {
                            (Some(_), Some(new)) if file.status == FileStatus::Added => {
                                format!("  Binary file ({} bytes)", new)
                            }
                            (Some(old), Some(_)) if file.status == FileStatus::Deleted => {
                                format!("  Binary file (was {} bytes)", old)
                            }
                            (Some(old), Some(new)) => {
                                format!("  Binary file ({} → {} bytes)", old, new)
                            }
                            _ => "  Binary file".to_string(),
                        }
                    } else {
                        "  Mode change only".to_string()
                    };
                    let span = Span::styled(msg, Style::default().fg(Color::DarkGray));
                    rows.push(frame_row(&open, vec![span], width));
                }
            }
            VisibleKind::HunkHeader(file_idx, hunk_idx) => {
                item_ys.push(rows.len());
                let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                rows.push(frame_row(&open, hunk_header_spans(hunk, focused), width));
            }
            VisibleKind::HunkLine(file_idx, hunk_idx, line_idx) => {
                item_ys.push(rows.len());
                let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                let header_focused = matches!(
                    visible.get(cursor).map(|i| &i.kind),
                    Some(VisibleKind::HunkHeader(fi, hi)) if fi == file_idx && hi == hunk_idx
                );
                let marker_color = if header_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                let inner_width = (width as usize).saturating_sub(2 * open.len());
                for spans in hunk_line_rows(
                    hunk,
                    *line_idx,
                    marker_color,
                    None,
                    Color::Cyan,
                    inner_width,
                    app.wrap,
                ) {
                    rows.push(frame_row(&open, spans, width));
                }
            }
        }
    }

    while !open.is_empty() {
        close_box(&mut rows, &mut open, width);
    }

    Doc { rows, item_ys }
}

fn draw_main_view(frame: &mut Frame, app: &mut App, area: Rect) {
    app.viewport_height = area.height;

    let visible = app.visible_items();
    if visible.is_empty() {
        app.item_ys.clear();
        app.rendered_height = 0;
        let msg = Paragraph::new("  All hunks confirmed! Press q to exit.");
        frame.render_widget(msg, area);
        return;
    }

    let cursor = app.cursor;
    let anchor = app.scroll_anchor;
    let doc = build_doc(app, cursor, area.width);
    let total_height = doc.rows.len();
    let item_ys = doc.item_ys.clone();
    let vh = area.height as usize;

    let mut scroll = app.scroll_offset;
    if let Some(&anchor_y) = anchor.and_then(|a| item_ys.get(a)) {
        scroll = anchor_y;
    }
    scroll = scroll.min(total_height.saturating_sub(vh));

    let end = (scroll + vh).min(total_height);
    let rows: Vec<Line> = doc.rows[scroll..end].to_vec();
    frame.render_widget(Paragraph::new(Text::from(rows)), area);

    // Scrollbar — evenly divided by hunk/unit count
    let total_units: usize = app.files.iter().map(|f| f.total_units()).sum();
    if total_height > area.height as usize && total_units > 0 {
        let unit_pos = cursor_unit_position(app, cursor, &visible);
        let mut scrollbar_state = ScrollbarState::new(total_units)
            .position(unit_pos)
            .viewport_content_length(1);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::DarkGray)),
            area,
            &mut scrollbar_state,
        );
    }

    // Publish this frame's geometry so navigation works on real row positions.
    app.item_ys = item_ys;
    app.rendered_height = total_height;
    app.scroll_offset = scroll;
    app.scroll_anchor = None;
}

/// Map cursor position to a sequential hunk/unit index for the scrollbar.
/// Folders and files map to the first unit of the first file they contain.
fn cursor_unit_position(
    app: &App,
    cursor: usize,
    visible: &[crate::model::VisibleItem],
) -> usize {
    let cursor_item = match visible.get(cursor) {
        Some(vi) => vi,
        None => return 0,
    };

    // Cumulative unit offset for a file index
    let file_unit_offset = |file_idx: usize| -> usize {
        app.files[..file_idx]
            .iter()
            .map(|f| f.total_units())
            .sum()
    };

    match &cursor_item.kind {
        VisibleKind::HunkHeader(file_idx, hunk_idx) => file_unit_offset(*file_idx) + hunk_idx,
        VisibleKind::File(file_idx) => file_unit_offset(*file_idx),
        VisibleKind::Folder(path) => {
            let prefix = format!("{}/", path);
            for (i, file) in app.files.iter().enumerate() {
                if file.rel_path.starts_with(&prefix) {
                    return file_unit_offset(i);
                }
            }
            0
        }
        VisibleKind::HunkLine(file_idx, hunk_idx, _) => file_unit_offset(*file_idx) + hunk_idx,
    }
}

fn draw_file_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let file_idx = app.file_view.as_ref().unwrap().file_idx;

    let title = {
        let file = &app.files[file_idx];
        let status_char = match file.status {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
        };
        format!(
            " {}  {}  +{} -{} ",
            file.rel_path, status_char, file.additions, file.deletions
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = app.file_view_lines(file_idx);
    let total = lines.len();

    let (line_cursor, prev_scroll) = {
        let fv = app.file_view.as_mut().unwrap();
        fv.viewport_height = inner.height;
        if fv.line_cursor >= total {
            fv.line_cursor = total.saturating_sub(1);
        }
        (fv.line_cursor, fv.scroll_offset)
    };

    if total == 0 {
        return;
    }

    // Build every row of the file, remembering which logical line each row
    // belongs to — with wrapping, one line can span several rows.
    let inner_width = inner.width as usize;
    let mut rows: Vec<(Line, usize)> = Vec::new();
    let mut row_start: Vec<usize> = Vec::with_capacity(total);
    for (idx, item) in lines.iter().enumerate() {
        row_start.push(rows.len());
        let is_cursor = idx == line_cursor;
        match item {
            FileViewLine::HunkHeader(hunk_idx) => {
                let hunk = &app.files[file_idx].hunks[*hunk_idx];
                rows.push((Line::from(hunk_header_spans(hunk, is_cursor)), idx));
            }
            FileViewLine::HunkLine(hunk_idx, line_idx) => {
                let hunk = &app.files[file_idx].hunks[*hunk_idx];
                let marker_color = if is_cursor {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                for spans in hunk_line_rows(
                    hunk,
                    *line_idx,
                    marker_color,
                    if is_cursor { Some("→") } else { None },
                    Color::DarkGray,
                    inner_width,
                    app.wrap,
                ) {
                    rows.push((Line::from(spans), idx));
                }
            }
        }
    }

    let total_rows = rows.len();
    let vh = inner.height as usize;
    let margin = vh / 4;

    // Keep the cursor line — all of its rows — inside the viewport.
    let cursor_start = row_start[line_cursor];
    let cursor_end = row_start
        .get(line_cursor + 1)
        .copied()
        .unwrap_or(total_rows);
    let mut scroll = prev_scroll;
    if cursor_start < scroll + margin {
        scroll = cursor_start.saturating_sub(margin);
    }
    if cursor_end + margin > scroll + vh {
        scroll = (cursor_end + margin).saturating_sub(vh);
    }
    scroll = scroll.min(total_rows.saturating_sub(vh));

    let end = (scroll + vh).min(total_rows);
    let bg = Color::Rgb(40, 40, 50);
    for (i, (line, line_idx)) in rows[scroll..end].iter().enumerate() {
        let line_area = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        if *line_idx == line_cursor {
            let mut spans: Vec<Span> = line
                .spans
                .iter()
                .cloned()
                .map(|mut s| {
                    s.style = s.style.bg(bg);
                    s
                })
                .collect();
            let used = spans_width(&spans);
            if used < inner_width {
                spans.push(Span::styled(
                    " ".repeat(inner_width - used),
                    Style::default().bg(bg),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), line_area);
        } else {
            frame.render_widget(Paragraph::new(line.clone()), line_area);
        }
    }

    if total_rows > vh {
        let mut scrollbar_state = ScrollbarState::new(total_rows)
            .position(cursor_start)
            .viewport_content_length(vh);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::DarkGray)),
            inner,
            &mut scrollbar_state,
        );
    }

    app.file_view.as_mut().unwrap().scroll_offset = scroll;
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let confirmed_files = app.files.iter().filter(|f| f.all_confirmed()).count();
    let total_files = app.files.len();

    let status = Line::from(Span::raw(format!(
        " {}/{} files confirmed",
        confirmed_files, total_files
    )));

    frame.render_widget(Paragraph::new(status), area);
}

fn draw_help_dialog(frame: &mut Frame) {
    let area = frame.area();
    let w = 55.min(area.width.saturating_sub(4));
    let h = 23.min(area.height.saturating_sub(4));
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
            Span::raw("Move cursor / scroll if off-screen"),
        ]),
        Line::from(vec![
            Span::styled("  Wheel      ", Style::default().fg(Color::Yellow)),
            Span::raw("Scroll viewport one line"),
        ]),
        Line::from(vec![
            Span::styled("  j/k        ", Style::default().fg(Color::Yellow)),
            Span::raw("Jump to next/prev file"),
        ]),
        Line::from(vec![
            Span::styled("  ←/→        ", Style::default().fg(Color::Yellow)),
            Span::raw("Fold/unfold current file"),
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
            Span::styled("  w          ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle word wrap (on by default)"),
        ]),
        Line::from(vec![
            Span::styled("  Tab        ", Style::default().fg(Color::Yellow)),
            Span::raw("Enter file view"),
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let search_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(Color::Yellow)),
        Span::raw(&app.file_list_query),
        Span::styled("_", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(search_line), chunks[0]);

    let filtered = app.filtered_files();
    let list_area = chunks[1];

    let mut items: Vec<ListItem> = Vec::new();
    for (file_idx, match_info) in &filtered {
        let file = &app.files[*file_idx];
        let confirmed_marker = if file.all_confirmed() {
            "[✓] "
        } else {
            "[ ] "
        };

        let (status_char, status_color) = match file.status {
            FileStatus::Modified => ("M", Color::Yellow),
            FileStatus::Added => ("A", Color::Green),
            FileStatus::Deleted => ("D", Color::Red),
            FileStatus::Renamed => ("R", Color::Blue),
            FileStatus::Copied => ("C", Color::Blue),
        };

        let path = &file.rel_path;
        let mut spans = vec![Span::raw(confirmed_marker.to_string())];

        if let Some((_, indices)) = match_info {
            let mut last = 0;
            for &idx in indices {
                if idx > last {
                    spans.push(Span::raw(&path[last..idx]));
                }
                if idx < path.len() {
                    let char_len = path[idx..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(1);
                    spans.push(Span::styled(
                        &path[idx..idx + char_len],
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                    last = idx + char_len;
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

    let list =
        List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, list_area, &mut list_state);
}