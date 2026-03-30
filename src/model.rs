use std::collections::{HashMap, HashSet};

use crate::fuzzy::{ArinaeMatcher, CaseMatching};
use crate::parser::FileEntry;

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
    pub should_exit: bool,
    pub show_help: bool,
    pub show_file_list: bool,
    pub file_list_query: String,
    pub file_list_cursor: usize,
    matcher: ArinaeMatcher,
}

/// Precompute merged folder stacks: single-child folder chains are collapsed
/// so navigation skips through them in one step.
fn compute_merged_folder_stacks(files: &[FileEntry]) -> Vec<Vec<String>> {
    struct Node {
        children: HashMap<String, Node>,
        has_files: bool,
    }

    let mut root = Node {
        children: HashMap::new(),
        has_files: false,
    };

    for file in files {
        let parts: Vec<&str> = file.rel_path.split('/').collect();
        let mut node = &mut root;
        for &part in &parts[..parts.len().saturating_sub(1)] {
            node = node
                .children
                .entry(part.to_string())
                .or_insert_with(|| Node {
                    children: HashMap::new(),
                    has_files: false,
                });
        }
        node.has_files = true;
    }

    files
        .iter()
        .map(|file| {
            let parts: Vec<&str> = file.rel_path.split('/').collect();
            let folder_parts = &parts[..parts.len().saturating_sub(1)];

            let mut stack = Vec::new();
            let mut node = &root;
            let mut accumulated = String::new();

            for &part in folder_parts {
                if !accumulated.is_empty() {
                    accumulated.push('/');
                }
                accumulated.push_str(part);

                let child = &node.children[part];

                // Emit unless this folder has exactly one subfolder child and no direct files
                if child.children.len() != 1 || child.has_files {
                    stack.push(accumulated.clone());
                }

                node = child;
            }

            stack
        })
        .collect()
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
            should_exit: false,
            show_help: false,
            show_file_list: false,
            file_list_query: String::new(),
            file_list_cursor: 0,
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

    pub fn total_confirmed_hunks(&self) -> usize {
        self.files.iter().map(|f| f.confirmed_count()).sum()
    }

    pub fn total_hunks(&self) -> usize {
        self.files.iter().map(|f| f.total_units()).sum()
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

    pub fn cursor_up(&mut self) {
        let targets = self.cursor_targets();
        if targets.is_empty() {
            return;
        }
        let current_target_idx = targets
            .iter()
            .rposition(|&t| t <= self.cursor)
            .unwrap_or(targets.len() - 1);
        let new_idx = if current_target_idx == 0 {
            targets.len() - 1
        } else {
            current_target_idx - 1
        };
        self.cursor = targets[new_idx];
    }

    pub fn cursor_down(&mut self) {
        let targets = self.cursor_targets();
        if targets.is_empty() {
            return;
        }
        let current_target_idx = targets
            .iter()
            .position(|&t| t >= self.cursor)
            .unwrap_or(0);
        let new_idx = if current_target_idx + 1 >= targets.len() {
            0
        } else {
            current_target_idx + 1
        };
        self.cursor = targets[new_idx];
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
                    if let Some(parent) = self.parent_merged_folder(path) {
                        if let Some(pos) = items.iter().position(|i| {
                            matches!(&i.kind, VisibleKind::Folder(p) if *p == parent)
                        }) {
                            self.cursor = pos;
                        }
                    }
                } else {
                    self.folded.insert(path.clone());
                }
            }
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                let visually_folded =
                    self.folded_files.contains(&idx) || self.files[idx].all_confirmed();
                if visually_folded {
                    // Already folded — move cursor to parent folder (don't fold it)
                    if let Some(parent) = self.parent_folder(idx) {
                        let new_items = self.visible_items();
                        if let Some(pos) = new_items.iter().position(|i| {
                            matches!(&i.kind, VisibleKind::Folder(p) if *p == parent)
                        }) {
                            self.cursor = pos;
                        }
                    }
                } else {
                    self.folded_files.insert(idx);
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

    pub fn toggle_current(&mut self) {
        let items = self.visible_items();
        match items.get(self.cursor).map(|i| &i.kind) {
            Some(VisibleKind::File(idx)) => {
                let idx = *idx;
                let new_state = !self.files[idx].all_confirmed();
                self.toggle_file(idx, new_state);
            }
            Some(VisibleKind::Folder(path)) => {
                let indices = self.files_under_folder(path);
                let all_confirmed = indices.iter().all(|&i| self.files[i].all_confirmed());
                let new_state = !all_confirmed;
                for &i in &indices {
                    self.toggle_file(i, new_state);
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
    }

    pub fn toggle_and_advance(&mut self) {
        self.toggle_current();
        self.cursor_down();
    }

    // ── File list popup ──

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
    }
}
