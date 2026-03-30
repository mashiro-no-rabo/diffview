//! Parse unified (git) diff format from a string into a list of file entries.

/// Strip ANSI escape sequences from input.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... (final byte is 0x40-0x7E)
            if let Some(next) = chars.next()
                && next == '['
            {
                for c2 in chars.by_ref() {
                    if c2.is_ascii() && (0x40..=0x7E).contains(&(c2 as u8)) {
                        break;
                    }
                }
            }
            // else: skip the single char after ESC
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Debug, Clone)]
pub enum HunkLine {
    Context(String),
    Addition(String),
    Deletion(String),
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<HunkLine>,
    pub confirmed: bool,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileEntry {
    pub rel_path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub binary_new_size: Option<usize>,
    pub binary_old_size: Option<usize>,
    /// File-level confirmation for binary/mode-change files (no hunks).
    pub confirmed: bool,
    /// Metadata lines between `diff --git` and content (index, mode, rename, etc.)
    pub header_lines: Vec<String>,
    /// Raw binary patch data for output (includes `GIT binary patch`, `literal`, base85, etc.)
    pub binary_content: String,
}

impl FileEntry {
    pub fn all_confirmed(&self) -> bool {
        if self.hunks.is_empty() {
            return self.confirmed;
        }
        self.hunks.iter().all(|h| h.confirmed)
    }

    pub fn confirmed_count(&self) -> usize {
        if self.hunks.is_empty() {
            return if self.confirmed { 1 } else { 0 };
        }
        self.hunks.iter().filter(|h| h.confirmed).count()
    }

    pub fn total_units(&self) -> usize {
        if self.hunks.is_empty() {
            1
        } else {
            self.hunks.len()
        }
    }
}

enum State {
    Init,
    FileHeader,
    HunkContent,
    BinaryContent,
}

struct PendingFile {
    path: String,
    old_path: Option<String>,
    status: Option<FileStatus>,
    hunks: Vec<Hunk>,
    is_delete_only: bool,
    is_binary: bool,
    binary_new_size: Option<usize>,
    binary_old_size: Option<usize>,
    binary_content: String,
    header_lines: Vec<String>,
    literal_count: usize,
}

impl PendingFile {
    fn new() -> Self {
        Self {
            path: String::new(),
            old_path: None,
            status: None,
            hunks: Vec::new(),
            is_delete_only: false,
            is_binary: false,
            binary_new_size: None,
            binary_old_size: None,
            binary_content: String::new(),
            header_lines: Vec::new(),
            literal_count: 0,
        }
    }

    fn start(&mut self, path: String) {
        self.path = path;
        self.old_path = None;
        self.status = Some(FileStatus::Modified);
        self.hunks.clear();
        self.is_delete_only = false;
        self.is_binary = false;
        self.binary_new_size = None;
        self.binary_old_size = None;
        self.binary_content.clear();
        self.header_lines.clear();
        self.literal_count = 0;
    }

    fn flush(&mut self, files: &mut Vec<FileEntry>) {
        if self.path.is_empty() {
            return;
        }

        let Some(mut status) = self.status else {
            return;
        };

        if self.is_delete_only {
            status = FileStatus::Deleted;
        }

        // Emit if there's any content worth showing
        if self.hunks.is_empty() && !self.is_binary && self.header_lines.is_empty() {
            return;
        }

        let additions: usize = self.hunks.iter().map(|h| h.additions).sum();
        let deletions: usize = self.hunks.iter().map(|h| h.deletions).sum();

        files.push(FileEntry {
            rel_path: self.path.clone(),
            old_path: self.old_path.clone(),
            status,
            hunks: std::mem::take(&mut self.hunks),
            additions,
            deletions,
            binary: self.is_binary,
            binary_new_size: self.binary_new_size,
            binary_old_size: self.binary_old_size,
            confirmed: false,
            header_lines: std::mem::take(&mut self.header_lines),
            binary_content: std::mem::take(&mut self.binary_content),
        });
    }
}

pub fn parse_diff(input: &str) -> Vec<FileEntry> {
    let clean = strip_ansi(input);
    let mut files: Vec<FileEntry> = Vec::new();
    let mut state = State::Init;
    let mut pending = PendingFile::new();

    for line in clean.lines() {
        match state {
            State::Init => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    if let Some(b_idx) = rest.rfind(" b/") {
                        pending.start(rest[b_idx + 3..].to_string());
                    }
                    state = State::FileHeader;
                }
            }
            State::FileHeader => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    pending.flush(&mut files);
                    if let Some(b_idx) = rest.rfind(" b/") {
                        pending.start(rest[b_idx + 3..].to_string());
                    }
                } else if line.starts_with("--- /dev/null") {
                    pending.status = Some(FileStatus::Added);
                } else if line.starts_with("+++ /dev/null") {
                    pending.is_delete_only = true;
                } else if line.starts_with("--- ") || line.starts_with("+++ ") {
                    // file markers — skip
                } else if line.starts_with("@@ ") {
                    pending.hunks.push(Hunk {
                        header: line.to_string(),
                        lines: Vec::new(),
                        confirmed: false,
                        additions: 0,
                        deletions: 0,
                    });
                    state = State::HunkContent;
                } else if let Some(rest) = line.strip_prefix("rename from ") {
                    pending.old_path = Some(rest.to_string());
                    pending.status = Some(FileStatus::Renamed);
                    pending.header_lines.push(line.to_string());
                } else if let Some(rest) = line.strip_prefix("copy from ") {
                    pending.old_path = Some(rest.to_string());
                    pending.status = Some(FileStatus::Copied);
                    pending.header_lines.push(line.to_string());
                } else if line.starts_with("Binary files ") {
                    pending.is_binary = true;
                    pending.binary_content.push_str(line);
                    pending.binary_content.push('\n');
                } else if line.starts_with("GIT binary patch") {
                    pending.is_binary = true;
                    pending.binary_content.push_str(line);
                    pending.binary_content.push('\n');
                    state = State::BinaryContent;
                } else if line.starts_with("index ")
                    || line.starts_with("old mode ")
                    || line.starts_with("new mode ")
                    || line.starts_with("new file mode ")
                    || line.starts_with("deleted file mode ")
                    || line.starts_with("similarity index ")
                    || line.starts_with("dissimilarity index ")
                    || line.starts_with("rename to ")
                    || line.starts_with("copy to ")
                {
                    pending.header_lines.push(line.to_string());
                } else {
                    panic!("unexpected line in FileHeader state: {:?}", line);
                }
            }
            State::HunkContent => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    pending.flush(&mut files);
                    if let Some(b_idx) = rest.rfind(" b/") {
                        pending.start(rest[b_idx + 3..].to_string());
                    }
                    state = State::FileHeader;
                } else if line.starts_with("@@ ") {
                    pending.hunks.push(Hunk {
                        header: line.to_string(),
                        lines: Vec::new(),
                        confirmed: false,
                        additions: 0,
                        deletions: 0,
                    });
                } else if let Some(rest) = line.strip_prefix('+') {
                    if let Some(hunk) = pending.hunks.last_mut() {
                        hunk.additions += 1;
                        hunk.lines.push(HunkLine::Addition(rest.to_string()));
                    }
                } else if let Some(rest) = line.strip_prefix('-') {
                    if let Some(hunk) = pending.hunks.last_mut() {
                        hunk.deletions += 1;
                        hunk.lines.push(HunkLine::Deletion(rest.to_string()));
                    }
                } else if let Some(rest) = line.strip_prefix(' ') {
                    if let Some(hunk) = pending.hunks.last_mut() {
                        hunk.lines.push(HunkLine::Context(rest.to_string()));
                    }
                } else if line.starts_with("\\ No newline at end of file")
                    || line.starts_with("Binary files ")
                {
                    // known non-diff lines within hunk context
                } else {
                    panic!("unexpected line in HunkContent state: {:?}", line);
                }
            }
            State::BinaryContent => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    pending.flush(&mut files);
                    if let Some(b_idx) = rest.rfind(" b/") {
                        pending.start(rest[b_idx + 3..].to_string());
                    }
                    state = State::FileHeader;
                } else {
                    pending.binary_content.push_str(line);
                    pending.binary_content.push('\n');

                    // Extract sizes from literal/delta lines
                    let size_str = line
                        .strip_prefix("literal ")
                        .or_else(|| line.strip_prefix("delta "));
                    if let Some(rest) = size_str
                        && let Ok(size) = rest.trim().parse::<usize>()
                    {
                        if pending.literal_count == 0 {
                            pending.binary_new_size = Some(size);
                        } else if pending.literal_count == 1 {
                            pending.binary_old_size = Some(size);
                        }
                        pending.literal_count += 1;
                    }
                }
            }
        }
    }

    // Flush last file
    pending.flush(&mut files);

    files
}

/// Format confirmed entries back into a unified diff.
#[allow(dead_code)]
pub fn format_confirmed_diff(files: &[FileEntry]) -> String {
    let mut output = String::new();

    for file in files {
        let a_path = file.old_path.as_deref().unwrap_or(&file.rel_path);

        if file.binary || (file.hunks.is_empty() && !file.header_lines.is_empty()) {
            // Binary or mode-change-only file: all-or-nothing confirmation
            if !file.confirmed {
                continue;
            }

            output.push_str(&format!(
                "diff --git a/{} b/{}\n",
                a_path, file.rel_path
            ));
            for hl in &file.header_lines {
                output.push_str(hl);
                output.push('\n');
            }
            if !file.binary_content.is_empty() {
                output.push_str(&file.binary_content);
            }
        } else {
            // Normal file with hunks: per-hunk confirmation
            let confirmed_hunks: Vec<&Hunk> =
                file.hunks.iter().filter(|h| h.confirmed).collect();
            if confirmed_hunks.is_empty() {
                continue;
            }

            output.push_str(&format!(
                "diff --git a/{} b/{}\n",
                a_path, file.rel_path
            ));

            for hl in &file.header_lines {
                output.push_str(hl);
                output.push('\n');
            }

            match file.status {
                FileStatus::Added => {
                    output.push_str("--- /dev/null\n");
                    output.push_str(&format!("+++ b/{}\n", file.rel_path));
                }
                FileStatus::Deleted => {
                    output.push_str(&format!("--- a/{}\n", a_path));
                    output.push_str("+++ /dev/null\n");
                }
                FileStatus::Modified | FileStatus::Renamed | FileStatus::Copied => {
                    output.push_str(&format!("--- a/{}\n", a_path));
                    output.push_str(&format!("+++ b/{}\n", file.rel_path));
                }
            }

            for hunk in confirmed_hunks {
                output.push_str(&hunk.header);
                output.push('\n');
                for line in &hunk.lines {
                    match line {
                        HunkLine::Context(s) => {
                            output.push(' ');
                            output.push_str(s);
                            output.push('\n');
                        }
                        HunkLine::Addition(s) => {
                            output.push('+');
                            output.push_str(s);
                            output.push('\n');
                        }
                        HunkLine::Deletion(s) => {
                            output.push('-');
                            output.push_str(s);
                            output.push('\n');
                        }
                    }
                }
            }
        }
    }

    output
}
