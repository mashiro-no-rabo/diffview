use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};

use crate::model::{App, FileViewLine, VisibleKind};
use crate::parser::{FileStatus, HunkLine};

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

    let confirmed = app.total_confirmed_hunks();
    let total = app.total_hunks();
    let header_title = format!(" Diffview  {}/{} confirmed ", confirmed, total);
    let header_block = Block::default()
        .borders(Borders::ALL)
        .title(header_title)
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Cyan));
    let header_inner = header_block.inner(main_area);
    frame.render_widget(header_block, main_area);

    if app.file_view.is_some() {
        draw_file_view(frame, app, header_inner);
    } else {
        draw_main_view(frame, app, header_inner);
    }
    draw_status_bar(frame, app, status_area);

    if app.show_help {
        draw_help_dialog(frame);
    }

    if app.show_file_list {
        draw_file_list_popup(frame, app);
    }
}

// ── Segment tree ──

enum Segment<'a> {
    Folder {
        path: String,        // display name (may be compressed like "src/app")
        full_path: String,   // actual full path for state lookups
        children: Vec<Segment<'a>>,
    },
    File {
        file_idx: usize,
        children: Vec<Segment<'a>>,
    },
    Line(Line<'a>),
}

/// Build segment tree directly from the app's visible items.
fn build_segment_tree<'a>(app: &'a App, cursor: usize) -> Vec<Segment<'a>> {
    let visible = app.visible_items();

    // First, build a raw nested structure from visible items.
    // Group by folder stack, then by file.
    // We iterate visible items and track folder/file context.

    struct FileData<'b> {
        folder_stack: Vec<String>,
        file_idx: usize,
        lines: Vec<Line<'b>>,
    }

    let mut file_datas: Vec<FileData> = Vec::new();
    let mut current_file: Option<FileData> = None;
    let mut current_folder_stack: Vec<String> = Vec::new();

    for (vis_idx, vi) in visible.iter().enumerate() {
        match &vi.kind {
            VisibleKind::Folder(path) => {
                // Update folder stack to this depth
                let depth = vi.depth;
                current_folder_stack.truncate(depth);
                if current_folder_stack.len() == depth {
                    current_folder_stack.push(path.clone());
                }
            }
            VisibleKind::File(file_idx) => {
                // Flush previous file
                if let Some(fd) = current_file.take() {
                    file_datas.push(fd);
                }
                let mut lines = Vec::new();
                let file = &app.files[*file_idx];

                // For binary or no-hunk files, add an info line
                if file.hunks.is_empty() && !app.folded_files.contains(file_idx) && !file.all_confirmed() {
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
                    lines.push(Line::from(Span::styled(
                        msg,
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                current_file = Some(FileData {
                    folder_stack: current_folder_stack.clone(),
                    file_idx: *file_idx,
                    lines,
                });
            }
            VisibleKind::HunkHeader(file_idx, hunk_idx) => {
                if let Some(ref mut fd) = current_file {
                    let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                    let is_focused = vis_idx == cursor;
                    let check = if hunk.confirmed { "✓" } else { " " };

                    let marker_color = if is_focused {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    };

                    fd.lines.push(Line::from(vec![
                        Span::styled(HUNK_MARKER_TOP, Style::default().fg(marker_color)),
                        Span::styled(
                            format!(" [{}] ", check),
                            if is_focused {
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
                    ]));
                }
            }
            VisibleKind::HunkLine(file_idx, hunk_idx, line_idx) => {
                if let Some(ref mut fd) = current_file {
                    let hunk = &app.files[*file_idx].hunks[*hunk_idx];
                    let is_last = *line_idx + 1 == hunk.lines.len();
                    let marker = if is_last {
                        HUNK_MARKER_BOT
                    } else {
                        HUNK_MARKER_MID
                    };

                    let hunk_header_focused = visible.iter().enumerate().any(|(vi, item)| {
                        vi == cursor
                            && matches!(&item.kind, VisibleKind::HunkHeader(fi, hi) if *fi == *file_idx && *hi == *hunk_idx)
                    });
                    let marker_color = if hunk_header_focused {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    };

                    // Compute line numbers from hunk header
                    let (old_start, new_start) = parse_hunk_start(&hunk.header);
                    let mut old_line = old_start;
                    let mut new_line = new_start;
                    // Walk lines up to line_idx to compute current line numbers
                    for l in &hunk.lines[..*line_idx] {
                        match l {
                            HunkLine::Context(_) => { old_line += 1; new_line += 1; }
                            HunkLine::Addition(_) => { new_line += 1; }
                            HunkLine::Deletion(_) => { old_line += 1; }
                        }
                    }

                    let hunk_line = &hunk.lines[*line_idx];
                    let (prefix, text, style, line_num_str) = match hunk_line {
                        HunkLine::Context(s) => {
                            (" ", s.as_str(), Style::default().fg(Color::Cyan),
                             format!("{:>4}", old_line))
                        }
                        HunkLine::Addition(s) => {
                            ("+", s.as_str(), Style::default().fg(Color::Green),
                             format!("{:>4}", new_line))
                        }
                        HunkLine::Deletion(s) => {
                            ("-", s.as_str(), Style::default().fg(Color::Red),
                             format!("{:>4}", old_line))
                        }
                    };

                    fd.lines.push(Line::from(vec![
                        Span::styled(marker, Style::default().fg(marker_color)),
                        Span::styled(
                            format!(" {} ", line_num_str),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(format!("{} ", prefix), style),
                        Span::styled(text, style),
                    ]));
                }
            }
        }
    }
    // Flush last file
    if let Some(fd) = current_file.take() {
        file_datas.push(fd);
    }

    // Now build segments from file_datas, nesting by folder_stack
    fn nest_files<'b>(files: &mut [FileData<'b>], depth: usize) -> Vec<Segment<'b>> {
        let mut segments: Vec<Segment<'b>> = Vec::new();
        let mut i = 0;

        while i < files.len() {
            if depth < files[i].folder_stack.len() {
                let folder_path = files[i].folder_stack[depth].clone();
                let start = i;
                while i < files.len()
                    && depth < files[i].folder_stack.len()
                    && files[i].folder_stack[depth] == folder_path
                {
                    i += 1;
                }
                let children = nest_files(&mut files[start..i], depth + 1);
                // Display name: relative to parent merged folder
                let display_name = if depth > 0 {
                    let parent = &files[start].folder_stack[depth - 1];
                    folder_path
                        .strip_prefix(parent)
                        .and_then(|s| s.strip_prefix('/'))
                        .unwrap_or(&folder_path)
                        .to_string()
                } else {
                    folder_path.clone()
                };

                segments.push(Segment::Folder {
                    path: display_name,
                    full_path: folder_path,
                    children,
                });
            } else {
                let fd = &mut files[i];
                let lines = std::mem::take(&mut fd.lines);
                let children: Vec<Segment<'b>> =
                    lines.into_iter().map(Segment::Line).collect();
                segments.push(Segment::File {
                    file_idx: fd.file_idx,
                    children,
                });
                i += 1;
            }
        }

        segments
    }

    nest_files(&mut file_datas, 0)
}

fn segment_height(seg: &Segment) -> u16 {
    match seg {
        Segment::Line(_) => 1,
        Segment::Folder { children, .. } | Segment::File { children, .. } => {
            let inner: u16 = children.iter().map(|c| segment_height(c)).sum();
            inner + 2 // +2 for top and bottom border
        }
    }
}

fn render_segments(
    frame: &mut Frame,
    area: Rect,
    segments: &[Segment],
    app: &App,
    scroll: &mut u16,
    focused_folder: Option<&str>,
    focused_file: Option<usize>,
) {
    let mut y = area.y;
    let bottom = area.y + area.height;

    for seg in segments {
        let h = segment_height(seg);

        if *scroll >= h {
            *scroll -= h;
            continue;
        }

        if y >= bottom {
            break;
        }

        match seg {
            Segment::Line(line) => {
                if *scroll > 0 {
                    *scroll -= 1;
                    continue;
                }
                if y < bottom {
                    let line_area = Rect::new(area.x, y, area.width, 1);
                    frame.render_widget(Paragraph::new(line.clone()), line_area);
                    y += 1;
                }
            }
            Segment::Folder {
                path,
                full_path,
                children,
            } => {
                let available = bottom.saturating_sub(y);
                let render_h = h.saturating_sub(*scroll).min(available);
                if render_h == 0 {
                    continue;
                }

                let block_area = Rect::new(area.x, y, area.width, render_h);

                let check = if app.folder_all_confirmed(full_path) {
                    "✓"
                } else {
                    " "
                };
                let fold_icon = if app.folded.contains(full_path) {
                    "▶"
                } else {
                    "▼"
                };
                let title = format!(" [{}] {} {}/ ", check, fold_icon, path);

                let is_focused = focused_folder == Some(full_path.as_str());
                let border_color = if is_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                let title_style = if is_focused {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(title_style)
                    .border_style(Style::default().fg(border_color));

                let inner = block.inner(block_area);
                frame.render_widget(block, block_area);

                let mut child_scroll = if *scroll > 0 {
                    let s = scroll.saturating_sub(1);
                    *scroll = 0;
                    s
                } else {
                    0
                };

                render_segments(
                    frame,
                    inner,
                    children,
                    app,
                    &mut child_scroll,
                    focused_folder,
                    focused_file,
                );

                y += render_h;
            }
            Segment::File { file_idx, children } => {
                let available = bottom.saturating_sub(y);
                let render_h = h.saturating_sub(*scroll).min(available);
                if render_h == 0 {
                    continue;
                }

                let block_area = Rect::new(area.x, y, area.width, render_h);

                let file = &app.files[*file_idx];
                let check = if file.all_confirmed() { "✓" } else { " " };
                let fold_icon = if app.folded_files.contains(file_idx) || file.all_confirmed() {
                    "▶"
                } else {
                    "▼"
                };
                let name = file.rel_path.rsplit('/').next().unwrap_or(&file.rel_path);

                let status_char = match file.status {
                    FileStatus::Modified => "M",
                    FileStatus::Added => "A",
                    FileStatus::Deleted => "D",
                    FileStatus::Renamed => "R",
                    FileStatus::Copied => "C",
                };

                let title = if file.binary {
                    let size_info = match (file.binary_old_size, file.binary_new_size) {
                        (Some(_), Some(new)) if file.status == FileStatus::Added => {
                            format!("  {} bytes", new)
                        }
                        (Some(old), Some(_)) if file.status == FileStatus::Deleted => {
                            format!("  was {} bytes", old)
                        }
                        (Some(old), Some(new)) => format!("  {} → {} bytes", old, new),
                        _ => String::new(),
                    };
                    format!(
                        " [{}] {} {}  {} BIN{} ",
                        check, fold_icon, name, status_char, size_info
                    )
                } else {
                    format!(
                        " [{}] {} {}  {}  +{} -{} ",
                        check, fold_icon, name, status_char, file.additions, file.deletions
                    )
                };

                let is_focused = focused_file == Some(*file_idx);
                let border_color = if is_focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                };
                let title_style = if is_focused {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                // Color the +/- in title via separate title spans isn't easy with
                // Block::title taking a single string. Use the status color for the
                // whole title when focused, white otherwise.
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(title_style)
                    .border_style(Style::default().fg(border_color));

                let inner = block.inner(block_area);
                frame.render_widget(block, block_area);

                let mut child_scroll = if *scroll > 0 {
                    let s = scroll.saturating_sub(1);
                    *scroll = 0;
                    s
                } else {
                    0
                };

                render_segments(
                    frame,
                    inner,
                    children,
                    app,
                    &mut child_scroll,
                    focused_folder,
                    focused_file,
                );

                y += render_h;
            }
        }
    }
}

fn draw_main_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_items();
    if visible.is_empty() {
        let msg = Paragraph::new("  All hunks confirmed! Press q to exit.");
        frame.render_widget(msg, area);
        return;
    }

    let cursor = app.cursor;

    // Determine focused folder/file
    let focused_folder = visible.get(cursor).and_then(|vi| {
        if let VisibleKind::Folder(path) = &vi.kind {
            Some(path.clone())
        } else {
            None
        }
    });
    let focused_file = visible.get(cursor).and_then(|vi| match &vi.kind {
        VisibleKind::File(idx) => Some(*idx),
        _ => None,
    });

    let segments = build_segment_tree(app, cursor);
    let total_height: u16 = segments.iter().map(|s| segment_height(s)).sum();

    let scroll = compute_scroll_nested(
        cursor,
        &visible,
        &segments,
        area.height,
        app.scroll_offset as u16,
    );

    let mut scroll_remaining = scroll;
    render_segments(
        frame,
        area,
        &segments,
        app,
        &mut scroll_remaining,
        focused_folder.as_deref(),
        focused_file,
    );

    // Scrollbar — evenly divided by hunk/unit count
    let total_units: usize = app.files.iter().map(|f| f.total_units()).sum();
    if total_height > area.height && total_units > 0 {
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

    app.scroll_offset = scroll as usize;
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

fn compute_scroll_nested(
    cursor: usize,
    visible: &[crate::model::VisibleItem],
    segments: &[Segment],
    visible_height: u16,
    current_scroll: u16,
) -> u16 {
    // Find which entry index corresponds to the cursor position
    // Entries skip Folder and File items (they're blocks, not lines).
    // But we need to account for folder/file block borders in the y offset.
    // Use a different approach: walk the segment tree and find which segment
    // contains the cursor's visible item.

    let cursor_kind = visible.get(cursor).map(|vi| &vi.kind);

    let search = match cursor_kind {
        Some(VisibleKind::Folder(path)) => Some(SearchTarget::Folder(path)),
        Some(VisibleKind::File(idx)) => Some(SearchTarget::File(*idx)),
        Some(VisibleKind::HunkHeader(_, _) | VisibleKind::HunkLine(_, _, _)) => {
            let mut entry_idx = 0;
            let mut target = None;
            for (vis_idx, vi) in visible.iter().enumerate() {
                if matches!(vi.kind, VisibleKind::Folder(_) | VisibleKind::File(_)) {
                    continue;
                }
                if vis_idx == cursor {
                    target = Some(entry_idx);
                    break;
                }
                entry_idx += 1;
            }
            target.map(SearchTarget::Line)
        }
        None => None,
    };

    let offset = search.and_then(|s| {
        let mut state = FindState { y: 0, line_counter: 0 };
        find_y_offset(segments, &s, &mut state)
    });

    let offset = offset.unwrap_or(0) as u16;

    // For files and hunks: anchor scroll to parent folder so siblings are out of view.
    if matches!(
        cursor_kind,
        Some(
            VisibleKind::File(_)
                | VisibleKind::HunkHeader(_, _)
                | VisibleKind::HunkLine(_, _, _)
        )
    ) {
        let parent_y = parent_folder_y(cursor, visible, segments);
        if offset >= parent_y && offset - parent_y < visible_height {
            // Parent folder and cursor both fit — anchor at parent
            return parent_y;
        }
        // Cursor too far from parent folder — center on cursor
        return offset.saturating_sub(visible_height / 2);
    }

    // For folders: use margin-based approach
    let margin = visible_height / 4;

    if offset < current_scroll + margin {
        return offset.saturating_sub(margin);
    }

    if offset + margin >= current_scroll + visible_height {
        return offset.saturating_sub(visible_height / 2);
    }

    current_scroll
}

/// Walk backwards from cursor to find the nearest parent folder's y offset.
fn parent_folder_y(
    cursor: usize,
    visible: &[crate::model::VisibleItem],
    segments: &[Segment],
) -> u16 {
    for i in (0..cursor).rev() {
        if let VisibleKind::Folder(path) = &visible[i].kind {
            let mut state = FindState {
                y: 0,
                line_counter: 0,
            };
            if let Some(y) = find_y_offset(segments, &SearchTarget::Folder(path), &mut state) {
                return y as u16;
            }
        }
    }
    0
}

enum SearchTarget<'a> {
    Folder(&'a str),
    File(usize),
    Line(usize), // nth content line
}

/// Find y offset of a target within the segment tree.
fn find_y_offset(segments: &[Segment], target: &SearchTarget, state: &mut FindState) -> Option<usize> {
    for seg in segments {
        match seg {
            Segment::Line(_) => {
                if let SearchTarget::Line(n) = target {
                    if state.line_counter == *n {
                        return Some(state.y);
                    }
                    state.line_counter += 1;
                }
                state.y += 1;
            }
            Segment::Folder { full_path, children, .. } => {
                if let SearchTarget::Folder(p) = target
                    && full_path.as_str() == *p
                {
                    return Some(state.y);
                }
                state.y += 1; // top border
                if let Some(r) = find_y_offset(children, target, state) {
                    return Some(r);
                }
                state.y += 1; // bottom border
            }
            Segment::File { file_idx, children, .. } => {
                if let SearchTarget::File(idx) = target
                    && *file_idx == *idx
                {
                    return Some(state.y);
                }
                state.y += 1; // top border
                if let Some(r) = find_y_offset(children, target, state) {
                    return Some(r);
                }
                state.y += 1; // bottom border
            }
        }
    }
    None
}

struct FindState {
    y: usize,
    line_counter: usize,
}

fn draw_file_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let fv = app.file_view.as_mut().unwrap();
    let file_idx = fv.file_idx;
    let file = &app.files[file_idx];

    let status_char = match file.status {
        FileStatus::Modified => "M",
        FileStatus::Added => "A",
        FileStatus::Deleted => "D",
        FileStatus::Renamed => "R",
        FileStatus::Copied => "C",
    };

    let title = format!(
        " {}  {}  +{} -{} ",
        file.rel_path, status_char, file.additions, file.deletions
    );

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
    let fv = app.file_view.as_mut().unwrap();
    fv.viewport_height = inner.height;

    let total = lines.len();
    if total == 0 {
        return;
    }

    // Clamp cursor
    if fv.line_cursor >= total {
        fv.line_cursor = total.saturating_sub(1);
    }

    // Compute scroll with margin
    let cursor = fv.line_cursor;
    let vh = inner.height as usize;
    let margin = vh / 4;
    let mut scroll = fv.scroll_offset;

    if cursor < scroll + margin {
        scroll = cursor.saturating_sub(margin);
    }
    if cursor + margin >= scroll + vh {
        scroll = (cursor + margin + 1).saturating_sub(vh);
    }
    // Clamp
    let max_scroll = total.saturating_sub(vh);
    scroll = scroll.min(max_scroll);
    fv.scroll_offset = scroll;

    let line_cursor = cursor;

    // Render visible lines
    let end = (scroll + vh).min(total);
    for (i, line_item) in lines[scroll..end].iter().enumerate() {
        let abs_idx = scroll + i;
        let is_cursor = abs_idx == line_cursor;
        let y = inner.y + i as u16;
        let line_area = Rect::new(inner.x, y, inner.width, 1);

        let rendered = match line_item {
            FileViewLine::HunkHeader(hunk_idx) => {
                let hunk = &app.files[file_idx].hunks[*hunk_idx];
                let check = if hunk.confirmed { "✓" } else { " " };
                let marker_color = if is_cursor { Color::Cyan } else { Color::DarkGray };

                Line::from(vec![
                    Span::styled(HUNK_MARKER_TOP, Style::default().fg(marker_color)),
                    Span::styled(
                        format!(" [{}] ", check),
                        if is_cursor {
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
                ])
            }
            FileViewLine::HunkLine(hunk_idx, line_idx) => {
                let hunk = &app.files[file_idx].hunks[*hunk_idx];
                let is_last = *line_idx + 1 == hunk.lines.len();

                let marker = if is_cursor {
                    "→"
                } else if is_last {
                    HUNK_MARKER_BOT
                } else {
                    HUNK_MARKER_MID
                };

                let marker_color = if is_cursor { Color::Cyan } else { Color::DarkGray };

                let (old_start, new_start) = parse_hunk_start(&hunk.header);
                let mut old_line = old_start;
                let mut new_line = new_start;
                for l in &hunk.lines[..*line_idx] {
                    match l {
                        HunkLine::Context(_) => { old_line += 1; new_line += 1; }
                        HunkLine::Addition(_) => { new_line += 1; }
                        HunkLine::Deletion(_) => { old_line += 1; }
                    }
                }

                let hunk_line = &hunk.lines[*line_idx];
                let (prefix, text, style, line_num_str) = match hunk_line {
                    HunkLine::Context(s) => (
                        " ", s.as_str(), Style::default().fg(Color::DarkGray),
                        format!("{:>4}", old_line),
                    ),
                    HunkLine::Addition(s) => (
                        "+", s.as_str(), Style::default().fg(Color::Green),
                        format!("{:>4}", new_line),
                    ),
                    HunkLine::Deletion(s) => (
                        "-", s.as_str(), Style::default().fg(Color::Red),
                        format!("{:>4}", old_line),
                    ),
                };

                Line::from(vec![
                    Span::styled(marker, Style::default().fg(marker_color)),
                    Span::styled(
                        format!(" {} ", line_num_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!("{} ", prefix), style),
                    Span::styled(text, style),
                ])
            }
        };

        if is_cursor {
            // Render with highlight background
            let bg_style = Style::default().bg(Color::Rgb(40, 40, 50));
            // Fill background first
            let bg_line = Line::from(Span::styled(
                " ".repeat(inner.width as usize),
                bg_style,
            ));
            frame.render_widget(Paragraph::new(bg_line), line_area);
            // Render the styled content on top with background
            let highlighted: Line = Line::from(
                rendered.spans.into_iter().map(|mut span| {
                    span.style = span.style.bg(Color::Rgb(40, 40, 50));
                    span
                }).collect::<Vec<_>>()
            );
            frame.render_widget(Paragraph::new(highlighted), line_area);
        } else {
            frame.render_widget(Paragraph::new(rendered), line_area);
        }
    }

    // Scrollbar
    if total > vh {
        let mut scrollbar_state = ScrollbarState::new(total)
            .position(line_cursor)
            .viewport_content_length(vh);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::DarkGray)),
            inner,
            &mut scrollbar_state,
        );
    }
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
            Span::raw("Fold folder/file"),
        ]),
        Line::from(vec![
            Span::styled("  →          ", Style::default().fg(Color::Yellow)),
            Span::raw("Unfold folder/file"),
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