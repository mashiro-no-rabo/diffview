use std::collections::HashSet;

use crate::fuzzy::{ArinaeMatcher, CaseMatching};
use crate::parser::FileEntry;

/// State for the single-file view mode.
pub struct FileViewState {
    pub file_idx: usize,
    pub line_cursor: usize,
    pub scroll_offset: usize,
    pub viewport_height: u16,
}

/// An item in the file view's flat line list.
#[derive(Debug, Clone)]
pub enum FileViewLine {
    HunkHeader(usize), // hunk_idx
    HunkLine(usize, usize), // (hunk_idx, line_idx)
}

#[derive(Debug, Clone)]
pub enum VisibleKind {
    Folder(String),
    File(usize),
    HunkHeader(usize, usize), // (file_idx, hunk_idx)
    HunkLine(usize, usize, usize), // (file_idx, hunk_idx, line_idx)
}

#[derive(Debug, Clone)]
pub struct VisibleItem {
    pub kind: VisibleKind,
    pub depth: usize,
}

pub struct App {
    pub files: Vec<FileEntry>,
    merged_folder_stacks: Vec<Vec<String>>, // per-file merged folder paths
    pub folded: HashSet<String>,            // folded folder paths
    pub folded_files: HashSet<usize>,       // folded file indices (hunks hidden)
    pub cursor: usize,
    pub scroll_offset: usize,
    pub viewport_height: u16, // set by renderer each frame
    pub wrap: bool,           // word-wrap long diff lines
    /// Row of each visible item in the last rendered frame, and that frame's
    /// total height. Both are published by the renderer, which is the only
    /// place that knows how wrapping expanded lines into rows.
    pub item_ys: Vec<usize>,
    pub rendered_height: usize,
    /// Visible-item index the next frame should place at the top of the
    /// viewport, so a re-layout (wrap toggle) keeps the same code in view.
    pub scroll_anchor: Option<usize>,
    pub should_exit: bool,
    pub show_help: bool,
    pub show_file_list: bool,
    pub file_list_query: String,
    pub file_list_cursor: usize,
    pub file_view: Option<FileViewState>,
    matcher: ArinaeMatcher,
}

/// Scroll offset that keeps row `cursor_y` inside a `vh`-tall viewport,
/// with a quarter-viewport margin above and below.
pub fn scroll_for_cursor(cursor_y: usize, scroll: usize, vh: usize) -> usize {
    let margin = vh / 4;
    if cursor_y < scroll + margin {
        cursor_y.saturating_sub(margin)
    } else if cursor_y + margin >= scroll + vh {
        (cursor_y + margin + 1).saturating_sub(vh)
    } else {
        scroll
    }
}

/// Folder grouping is disabled — the default view is a flat file list.
fn compute_merged_folder_stacks(files: &[FileEntry]) -> Vec<Vec<String>> {
    files.iter().map(|_| Vec::new()).collect()
}

impl App {
    pub fn new(files: Vec<FileEntry>) -> Self {
        // Default fold .lock files, deleted files, and binary files
        let mut folded_files = HashSet::new();
        for (idx, file) in files.iter().enumerate() {
            if file.rel_path.ends_with(".lock")
                || file.status == crate::parser::FileStatus::Deleted
                || file.binary
            {
                folded_files.insert(idx);
            }
        }

        let merged_folder_stacks = compute_merged_folder_stacks(&files);

        Self {
            files,
            merged_folder_stacks,
            folded: HashSet::new(),
            folded_files,
            cursor: 0,
            scroll_offset: 0,
            viewport_height: 0,
            wrap: true,
            item_ys: Vec::new(),
            rendered_height: 0,
            scroll_anchor: None,
            should_exit: false,
            show_help: false,
            show_file_list: false,
            file_list_query: String::new(),
            file_list_cursor: 0,
            file_view: None,
            matcher: ArinaeMatcher::new(CaseMatching::Smart),
        }
    }

    /// Build flat visible-items list: folders, files, hunk headers, hunk lines.
    /// Confirmed items and their children are hidden.
    /// Folded folders hide their children.
    pub fn visible_items(&self) -> Vec<VisibleItem> {
        let mut items = Vec::new();
        let mut emitted_folders: HashSet<String> = HashSet::new();

        for (file_idx, file) in self.files.iter().enumerate() {
            let folder_stack = &self.merged_folder_stacks[file_idx];
            let mut hidden = false;

            // Emit merged folder nodes
            for (depth, folder_path) in folder_stack.iter().enumerate() {
                if !emitted_folders.contains(folder_path) {
                    emitted_folders.insert(folder_path.clone());
                    items.push(VisibleItem {
                        kind: VisibleKind::Folder(folder_path.clone()),
                        depth,
                    });
                }

                if self.folded.contains(folder_path) {
                    hidden = true;
                    break;
                }
            }

            if hidden {
                continue;
            }

            let file_depth = folder_stack.len();

            // File header (always shown)
            items.push(VisibleItem {
                kind: VisibleKind::File(file_idx),
                depth: file_depth,
            });

            // Hunk headers and lines
            let file_folded = self.folded_files.contains(&file_idx) || file.all_confirmed();
            if !file_folded {
                for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                    items.push(VisibleItem {
                        kind: VisibleKind::HunkHeader(file_idx, hunk_idx),
                        depth: file_depth + 1,
                    });

                    if !hunk.confirmed {
                        for (line_idx, _) in hunk.lines.iter().enumerate() {
                            items.push(VisibleItem {
                                kind: VisibleKind::HunkLine(file_idx, hunk_idx, line_idx),
                                depth: file_depth + 1,
                            });
                        }
                    }
                }
            }
        }

        items
    }

    /// Items that the cursor can land on (folders, files, hunk headers — not hunk lines).
    pub fn cursor_targets(&self) -> Vec<usize> {
        self.visible_items()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                matches!(
                    item.kind,
                    VisibleKind::Folder(_) | VisibleKind::File(_) | VisibleKind::HunkHeader(_, _)
                )
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn files_under_folder(&self, folder_path: &str) -> Vec<usize> {
        let prefix = format!("{}/", folder_path);
        self.files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.rel_path.starts_with(&prefix))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn folder_all_confirmed(&self, folder_path: &str) -> bool {
        let indices = self.files_under_folder(folder_path);
        !indices.is_empty() && indices.iter().all(|&i| self.files[i].all_confirmed())
    }

    // ── Rendered geometry ──

    /// True when the cached geometry still describes the current item list.
    fn geometry_valid(&self, item_count: usize) -> bool {
        !self.item_ys.is_empty() && self.item_ys.len() == item_count
    }

    /// Row of each visible item in the rendered output. Uses the geometry the
    /// renderer published last frame; before the first frame (and right after
    /// a state change that adds or removes items) it falls back to an estimate
    /// that assumes one row per line: each file contributes a top-border row
    /// (the file row itself), an optional info line for binary / mode-only
    /// files, and a bottom-border row after its children.
    pub fn item_y_positions(&self) -> Vec<usize> {
        let items = self.visible_items();
        if self.geometry_valid(items.len()) {
            return self.item_ys.clone();
        }

        let mut ys = Vec::with_capacity(items.len());
        let mut y: usize = 0;
        let mut last_was_file = false;

        for item in &items {
            match &item.kind {
                VisibleKind::File(idx) => {
                    if last_was_file {
                        y += 1; // previous file's bottom border
                    }
                    ys.push(y);
                    y += 1; // top border (= file row)
                    let file = &self.files[*idx];
                    if file.hunks.is_empty()
                        && !self.folded_files.contains(idx)
                        && !file.all_confirmed()
                    {
                        y += 1; // info line
                    }
                    last_was_file = true;
                }
                VisibleKind::HunkHeader(_, _) | VisibleKind::HunkLine(_, _, _) => {
                    ys.push(y);
                    y += 1;
                }
                VisibleKind::Folder(_) => {
                    if last_was_file {
                        y += 1;
                    }
                    ys.push(y);
                    y += 1;
                    last_was_file = false;
                }
            }
        }

        ys
    }

    /// Total height of the rendered main view in rows.
    pub fn total_rendered_height(&self) -> usize {
        if self.rendered_height > 0 && self.geometry_valid(self.visible_items().len()) {
            return self.rendered_height;
        }
        let ys = self.item_y_positions();
        match ys.last() {
            Some(&last) => last + 2, // last item's row + closing bottom border
            None => 0,
        }
    }

    fn max_scroll(&self) -> usize {
        self.total_rendered_height()
            .saturating_sub(self.viewport_height as usize)
    }

    /// Turn word wrapping on/off. Row positions all change, so drop the cached
    /// geometry and pin whatever is at the top of the viewport — the next frame
    /// re-derives the scroll offset from it.
    pub fn toggle_wrap(&mut self) {
        let count = self.visible_items().len();
        self.scroll_anchor = if self.geometry_valid(count) {
            self.item_ys
                .iter()
                .rposition(|&y| y <= self.scroll_offset)
                .or(Some(self.cursor))
        } else {
            Some(self.cursor)
        };
        self.wrap = !self.wrap;
        self.item_ys.clear();
        self.rendered_height = 0;
    }

    // ── Navigation ──

    /// Clamp cursor to valid target after state changes.
    fn clamp_cursor(&mut self) {
        let targets = self.cursor_targets();
        if targets.is_empty() {
            self.cursor = 0;
            return;
        }
        // Find nearest target
        if let Some(&nearest) = targets.iter().min_by_key(|&&t| {
            (t as isize - self.cursor as isize).unsigned_abs()
        }) {
            self.cursor = nearest;
        }
    }

    /// Adjust scroll_offset so the cursor sits within the viewport.
    pub fn ensure_cursor_visible(&mut self) {
        if self.viewport_height == 0 {
            return;
        }
        let ys = self.item_y_positions();
        let Some(&cursor_y) = ys.get(self.cursor) else {
            return;
        };
        self.scroll_offset = scroll_for_cursor(
            cursor_y,
            self.scroll_offset,
            self.viewport_height as usize,
        );
        let max = self.max_scroll();
        if self.scroll_offset > max {
            self.scroll_offset = max;
        }
    }

    /// Up arrow: if previous cursor target is visible, move cursor to it;
    /// otherwise scroll the viewport up by a half-page (cursor unchanged).
    pub fn cursor_up(&mut self) {
        let targets = self.cursor_targets();
        if targets.is_empty() {
            return;
        }
        if self.viewport_height == 0 {
            // Pre-render fallback: walk targets without wrap-around.
            let idx = targets
                .iter()
                .rposition(|&t| t <= self.cursor)
                .unwrap_or(0);
            if idx > 0 {
                self.cursor = targets[idx - 1];
            }
            return;
        }

        let ys = self.item_y_positions();
        let cursor_idx = targets.iter().position(|&t| t == self.cursor);
        let prev_idx = match cursor_idx {
            Some(i) if i > 0 => Some(i - 1),
            None => None, // cursor not on a target — recover via clamp on next op
            Some(_) => None,
        };

        if let Some(pi) = prev_idx {
            let prev_target = targets[pi];
            let prev_y = ys[prev_target];
            let scroll = self.scroll_offset;
            if prev_y >= scroll && prev_y < scroll + self.viewport_height as usize {
                self.cursor = prev_target;
                return;
            }
        }

        let step = (self.viewport_height / 2).max(1) as usize;
        self.scroll_offset = self.scroll_offset.saturating_sub(step);
    }

    /// Down arrow: if next cursor target is visible, move cursor to it;
    /// otherwise scroll the viewport down by a half-page (cursor unchanged).
    pub fn cursor_down(&mut self) {
        let targets = self.cursor_targets();
        if targets.is_empty() {
            return;
        }
        if self.viewport_height == 0 {
            let idx = targets
                .iter()
                .position(|&t| t >= self.cursor)
                .unwrap_or(targets.len() - 1);
            if idx + 1 < targets.len() {
                self.cursor = targets[idx + 1];
            }
            return;
        }

        let ys = self.item_y_positions();
        let cursor_idx = targets.iter().position(|&t| t == self.cursor);
        let next_idx = match cursor_idx {
            Some(i) if i + 1 < targets.len() => Some(i + 1),
            _ => None,
        };

        if let Some(ni) = next_idx {
            let next_target = targets[ni];
            let next_y = ys[next_target];
            let scroll = self.scroll_offset;
            if next_y >= scroll && next_y < scroll + self.viewport_height as usize {
                self.cursor = next_target;
                return;
            }
        }

        let step = (self.viewport_height / 2).max(1) as usize;
        let max = self.max_scroll();
        self.scroll_offset = (self.scroll_offset + step).min(max);
    }

    /// Mouse wheel: scroll one line up. Cursor unchanged.
    pub fn scroll_line_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Mouse wheel: scroll one line down. Cursor unchanged.
    pub fn scroll_line_down(&mut self) {
        if self.viewport_height == 0 {
            return;
        }
        let max = self.max_scroll();
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    /// Jump to previous file header.
    pub fn prev_file(&mut self) {
        let items = self.visible_items();
        let file_positions: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.kind, VisibleKind::File(_)))
            .map(|(i, _)| i)
            .collect();
        if file_positions.is_empty() {
            return;
        }
        let current = file_positions
            .iter()
            .rposition(|&p| p < self.cursor)
            .unwrap_or(file_positions.len() - 1);
        self.cursor = file_positions[current];
        self.ensure_cursor_visible();
    }

    /// Jump to next file header.
    pub fn next_file(&mut self) {
        let items = self.visible_items();
        let file_positions: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.kind, VisibleKind::File(_)))
            .map(|(i, _)| i)
            .collect();
        if file_positions.is_empty() {
            return;
        }
        let current = file_positions
            .iter()
            .position(|&p| p > self.cursor)
            .unwrap_or(0);
        self.cursor = file_positions[current];
        self.ensure_cursor_visible();
    }

    /// Find the innermost merged folder for a given file.
    fn parent_folder(&self, file_idx: usize) -> Option<String> {
        self.merged_folder_stacks[file_idx].last().cloned()
    }

    /// Find the parent merged folder of a given folder path.
    fn parent_merged_folder(&self, folder_path: &str) -> Option<String> {
        for stack in &self.merged_folder_stacks {
            if let Some(pos) = stack.iter().position(|p| p == folder_path) {
                return if pos > 0 {
                    Some(stack[pos - 1].clone())
                } else {
                    None
                };
            }
        }
        None
    }

    pub fn fold_current(&mut self) {
        let items = self.visible_items();
        match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::Folder(path)) => {
                if self.folded.contains(path) {
                    // Already folded — move to parent folder
                    if let Some(parent) = self.parent_merged_folder(path)
                        && let Some(pos) = items.iter().position(|i| {
                            matches!(&i.kind, VisibleKind::Folder(p) if *p == parent)
                        })
                    {
                        self.cursor = pos;
                    }
                } else {
                    self.folded.insert(path.clone());
                }
            }
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                // Always move to parent folder — never fold
                if let Some(parent) = self.parent_folder(idx)
                    && let Some(pos) = items.iter().position(|i| {
                        matches!(&i.kind, VisibleKind::Folder(p) if *p == parent)
                    })
                {
                    self.cursor = pos;
                }
            }
            Some(VisibleKind::HunkHeader(file_idx, _)) => {
                let file_idx = *file_idx;
                // Fold the file this hunk belongs to
                self.folded_files.insert(file_idx);
                // Move cursor to the file header
                let new_items = self.visible_items();
                if let Some(pos) = new_items
                    .iter()
                    .position(|i| matches!(&i.kind, VisibleKind::File(fi) if *fi == file_idx))
                {
                    self.cursor = pos;
                }
            }
            _ => {}
        }
        self.ensure_cursor_visible();
    }

    pub fn unfold_current(&mut self) {
        let items = self.visible_items();
        match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::Folder(path)) => {
                if self.folded.contains(path) {
                    self.folded.remove(path);
                } else {
                    // Move to first child
                    let targets = self.cursor_targets();
                    if let Some(&next) = targets.iter().find(|&&t| t > self.cursor) {
                        self.cursor = next;
                    }
                }
            }
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                if self.folded_files.contains(&idx) {
                    self.folded_files.remove(&idx);
                } else {
                    // Move to first hunk
                    let targets = self.cursor_targets();
                    if let Some(&next) = targets.iter().find(|&&t| t > self.cursor) {
                        self.cursor = next;
                    }
                }
            }
            _ => {}
        }
        self.ensure_cursor_visible();
    }

    // ── Selection/Confirmation ──

    fn toggle_file(&mut self, idx: usize, state: bool) {
        let file = &mut self.files[idx];
        if file.hunks.is_empty() {
            file.confirmed = state;
        } else {
            for hunk in &mut file.hunks {
                hunk.confirmed = state;
            }
        }
    }

    fn invert_file(&mut self, idx: usize) {
        let file = &mut self.files[idx];
        if file.hunks.is_empty() {
            file.confirmed = !file.confirmed;
        } else {
            for hunk in &mut file.hunks {
                hunk.confirmed = !hunk.confirmed;
            }
        }
    }

    pub fn invert_confirmation(&mut self) {
        let items = self.visible_items();
        match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                self.invert_file(idx);
            }
            Some(VisibleKind::Folder(path)) => {
                let indices = self.files_under_folder(path);
                for &i in &indices {
                    self.invert_file(i);
                }
            }
            Some(VisibleKind::HunkHeader(file_idx, hunk_idx)) => {
                let file_idx = *file_idx;
                let hunk_idx = *hunk_idx;
                if let Some(hunk) = self.files[file_idx].hunks.get_mut(hunk_idx) {
                    hunk.confirmed = !hunk.confirmed;
                }
            }
            _ => {}
        }
        self.clamp_cursor();
        self.ensure_cursor_visible();
    }

    pub fn confirm_and_advance(&mut self) {
        let items = self.visible_items();
        let targets = self.cursor_targets();
        // Find current position in targets list for "advance without wrap"
        let current_target_idx = targets.iter().position(|&t| t >= self.cursor);

        match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                self.toggle_file(idx, true);
                self.folded_files.insert(idx);
            }
            Some(VisibleKind::Folder(path)) => {
                let path = path.clone();
                let indices = self.files_under_folder(&path);
                for &i in &indices {
                    self.toggle_file(i, true);
                }
                self.folded.insert(path);
            }
            Some(VisibleKind::HunkHeader(file_idx, hunk_idx)) => {
                let file_idx = *file_idx;
                let hunk_idx = *hunk_idx;
                if let Some(hunk) = self.files[file_idx].hunks.get_mut(hunk_idx) {
                    hunk.confirmed = true;
                }
            }
            _ => {}
        }

        // Move to next item without wrapping
        let new_targets = self.cursor_targets();
        if new_targets.is_empty() {
            self.cursor = 0;
            return;
        }
        if let Some(ct_idx) = current_target_idx {
            // Try to land on the same index in the new targets list,
            // which is effectively the "next" item since the current one
            // may have collapsed. If we're at/past the end, stay on last.
            if ct_idx < new_targets.len() {
                self.cursor = new_targets[ct_idx];
            } else {
                self.cursor = *new_targets.last().unwrap();
            }
        } else {
            self.clamp_cursor();
        }
        self.ensure_cursor_visible();
    }

    // ── File list popup ──

    #[allow(clippy::type_complexity)]
    pub fn filtered_files(&self) -> Vec<(usize, Option<(i64, Vec<usize>)>)> {
        if self.file_list_query.is_empty() {
            return self
                .files
                .iter()
                .enumerate()
                .map(|(i, _)| (i, None))
                .collect();
        }

        let mut results: Vec<(usize, Option<(i64, Vec<usize>)>)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                self.matcher
                    .fuzzy_indices(&f.rel_path, &self.file_list_query)
                    .map(|(score, indices)| (i, Some((score, indices))))
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| {
            let sa = a.1.as_ref().map(|x| x.0).unwrap_or(0);
            let sb = b.1.as_ref().map(|x| x.0).unwrap_or(0);
            sb.cmp(&sa)
        });

        results
    }

    pub fn jump_to_file(&mut self, file_idx: usize) {
        let items = self.visible_items();
        if let Some(pos) = items
            .iter()
            .position(|item| matches!(&item.kind, VisibleKind::File(idx) if *idx == file_idx))
        {
            self.cursor = pos;
        }
        self.ensure_cursor_visible();
    }

    // ── File View ──

    /// Build flat list of renderable lines for a file in file view.
    pub fn file_view_lines(&self, file_idx: usize) -> Vec<FileViewLine> {
        let file = &self.files[file_idx];
        let mut lines = Vec::new();
        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            lines.push(FileViewLine::HunkHeader(hunk_idx));
            if !hunk.confirmed {
                for (line_idx, _) in hunk.lines.iter().enumerate() {
                    lines.push(FileViewLine::HunkLine(hunk_idx, line_idx));
                }
            }
        }
        lines
    }

    pub fn enter_file_view(&mut self) {
        let items = self.visible_items();
        let file_idx = match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::File(idx)) => *idx,
            Some(VisibleKind::HunkHeader(file_idx, _)) => *file_idx,
            Some(VisibleKind::HunkLine(file_idx, _, _)) => *file_idx,
            _ => return,
        };
        if self.files[file_idx].hunks.is_empty() {
            return;
        }
        self.file_view = Some(FileViewState {
            file_idx,
            line_cursor: 0,
            scroll_offset: 0,
            viewport_height: 0,
        });
    }

    pub fn exit_file_view(&mut self) {
        if let Some(fv) = self.file_view.take() {
            self.jump_to_file(fv.file_idx);
        }
    }

    pub fn file_view_up(&mut self) {
        if let Some(fv) = &mut self.file_view {
            fv.line_cursor = fv.line_cursor.saturating_sub(1);
        }
    }

    pub fn file_view_down(&mut self) {
        let total = self.file_view.as_ref()
            .map(|fv| self.file_view_lines(fv.file_idx).len())
            .unwrap_or(0);
        if let Some(fv) = &mut self.file_view
            && fv.line_cursor + 1 < total
        {
            fv.line_cursor += 1;
        }
    }

    pub fn file_view_half_page_up(&mut self) {
        if let Some(fv) = &mut self.file_view {
            let half = (fv.viewport_height / 2).max(1) as usize;
            fv.line_cursor = fv.line_cursor.saturating_sub(half);
        }
    }

    pub fn file_view_half_page_down(&mut self) {
        let (total, half) = self.file_view.as_ref()
            .map(|fv| {
                let total = self.file_view_lines(fv.file_idx).len();
                let half = (fv.viewport_height / 2).max(1) as usize;
                (total, half)
            })
            .unwrap_or((0, 1));
        if let Some(fv) = &mut self.file_view {
            fv.line_cursor = (fv.line_cursor + half).min(total.saturating_sub(1));
        }
    }

    pub fn file_view_toggle(&mut self) {
        if let Some(fv) = &self.file_view {
            let file_idx = fv.file_idx;
            let lines = self.file_view_lines(file_idx);
            let hunk_idx = match lines.get(fv.line_cursor) {
                Some(FileViewLine::HunkHeader(hi)) => *hi,
                Some(FileViewLine::HunkLine(hi, _)) => *hi,
                None => return,
            };
            self.files[file_idx].hunks[hunk_idx].confirmed =
                !self.files[file_idx].hunks[hunk_idx].confirmed;
            // Reclamp cursor since confirmed hunks collapse their lines
            let total = self.file_view_lines(file_idx).len();
            if let Some(fv) = &mut self.file_view
                && fv.line_cursor >= total
            {
                fv.line_cursor = total.saturating_sub(1);
            }
        }
    }

    pub fn file_view_toggle_and_advance(&mut self) {
        self.file_view_toggle();
        self.file_view_down();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_diff;

    const FLAT_TEST_DIFF: &str = "\
diff --git a/zoo/mammals/cat.txt b/zoo/mammals/cat.txt
--- a/zoo/mammals/cat.txt
+++ b/zoo/mammals/cat.txt
@@ -1,3 +1,4 @@
 meow
+purr
 whiskers
 paws
diff --git a/zoo/mammals/dog.txt b/zoo/mammals/dog.txt
--- a/zoo/mammals/dog.txt
+++ b/zoo/mammals/dog.txt
@@ -1,3 +1,4 @@
 woof
+bark
 tail
 ears
diff --git a/zoo/birds/parrot.txt b/zoo/birds/parrot.txt
--- a/zoo/birds/parrot.txt
+++ b/zoo/birds/parrot.txt
@@ -1,3 +1,4 @@
 squawk
+mimic
 feathers
 beak
";

    #[test]
    fn default_view_emits_no_folders() {
        let app = App::new(parse_diff(FLAT_TEST_DIFF));
        let items = app.visible_items();
        let folders = items
            .iter()
            .filter(|i| matches!(i.kind, VisibleKind::Folder(_)))
            .count();
        assert_eq!(folders, 0, "flat view must have no folder items");

        let files = items
            .iter()
            .filter(|i| matches!(i.kind, VisibleKind::File(_)))
            .count();
        assert_eq!(files, 3);

        // All files should be at depth 0
        for item in &items {
            if matches!(item.kind, VisibleKind::File(_)) {
                assert_eq!(item.depth, 0);
            }
        }
    }
}
