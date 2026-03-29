/// Parse unified (git) diff format from a string into a list of file entries.

/// Strip ANSI escape sequences from input.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... (final byte is 0x40-0x7E)
            if let Some(next) = chars.next() {
                if next == '[' {
                    for c2 in chars.by_ref() {
                        if c2.is_ascii() && (0x40..=0x7E).contains(&(c2 as u8)) {
                            break;
                        }
                    }
                }
                // else: skip the single char after ESC
            }
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
pub struct FileEntry {
    pub rel_path: String,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
}

impl FileEntry {
    pub fn all_confirmed(&self) -> bool {
        !self.hunks.is_empty() && self.hunks.iter().all(|h| h.confirmed)
    }

    pub fn none_confirmed(&self) -> bool {
        self.hunks.iter().all(|h| !h.confirmed)
    }

    pub fn confirmed_count(&self) -> usize {
        self.hunks.iter().filter(|h| h.confirmed).count()
    }
}

enum State {
    Init,
    FileHeader,
    HunkContent,
}

pub fn parse_diff(input: &str) -> Vec<FileEntry> {
    let clean = strip_ansi(input);
    let mut files: Vec<FileEntry> = Vec::new();
    let mut state = State::Init;
    let mut current_path = String::new();
    let mut current_status: Option<FileStatus> = None;
    let mut current_hunks: Vec<Hunk> = Vec::new();
    let mut is_delete_only = false;

    for line in clean.lines() {
        match state {
            State::Init => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    // Extract path from b/... portion
                    if let Some(b_idx) = rest.rfind(" b/") {
                        current_path = rest[b_idx + 3..].to_string();
                    }
                    current_status = Some(FileStatus::Modified);
                    current_hunks.clear();
                    is_delete_only = false;
                    state = State::FileHeader;
                }
            }
            State::FileHeader => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    flush_file(
                        &mut files,
                        &current_path,
                        current_status,
                        &mut current_hunks,
                        is_delete_only,
                    );
                    if let Some(b_idx) = rest.rfind(" b/") {
                        current_path = rest[b_idx + 3..].to_string();
                    }
                    current_status = Some(FileStatus::Modified);
                    current_hunks.clear();
                    is_delete_only = false;
                } else if line.starts_with("--- /dev/null") {
                    current_status = Some(FileStatus::Added);
                } else if line.starts_with("+++ /dev/null") {
                    is_delete_only = true;
                } else if line.starts_with("--- ") || line.starts_with("+++ ") {
                    // file markers
                } else if line.starts_with("@@ ") {
                    let header = parse_hunk_header(line);
                    current_hunks.push(Hunk {
                        header,
                        lines: Vec::new(),
                        confirmed: false,
                        additions: 0,
                        deletions: 0,
                    });
                    state = State::HunkContent;
                } else if line.starts_with("index ")
                    || line.starts_with("Binary files ")
                    || line.starts_with("old mode ")
                    || line.starts_with("new mode ")
                    || line.starts_with("new file mode ")
                    || line.starts_with("deleted file mode ")
                    || line.starts_with("similarity index ")
                    || line.starts_with("dissimilarity index ")
                    || line.starts_with("rename from ")
                    || line.starts_with("rename to ")
                    || line.starts_with("copy from ")
                    || line.starts_with("copy to ")
                    || line.starts_with("GIT binary patch")
                {
                    // known git diff metadata
                } else {
                    panic!("unexpected line in FileHeader state: {:?}", line);
                }
            }
            State::HunkContent => {
                if let Some(rest) = line.strip_prefix("diff --git ") {
                    flush_file(
                        &mut files,
                        &current_path,
                        current_status,
                        &mut current_hunks,
                        is_delete_only,
                    );
                    if let Some(b_idx) = rest.rfind(" b/") {
                        current_path = rest[b_idx + 3..].to_string();
                    }
                    current_status = Some(FileStatus::Modified);
                    current_hunks.clear();
                    is_delete_only = false;
                    state = State::FileHeader;
                } else if line.starts_with("@@ ") {
                    let header = parse_hunk_header(line);
                    current_hunks.push(Hunk {
                        header,
                        lines: Vec::new(),
                        confirmed: false,
                        additions: 0,
                        deletions: 0,
                    });
                } else if let Some(rest) = line.strip_prefix('+') {
                    if let Some(hunk) = current_hunks.last_mut() {
                        hunk.additions += 1;
                        hunk.lines.push(HunkLine::Addition(rest.to_string()));
                    }
                } else if let Some(rest) = line.strip_prefix('-') {
                    if let Some(hunk) = current_hunks.last_mut() {
                        hunk.deletions += 1;
                        hunk.lines.push(HunkLine::Deletion(rest.to_string()));
                    }
                } else if let Some(rest) = line.strip_prefix(' ') {
                    if let Some(hunk) = current_hunks.last_mut() {
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
        }
    }

    // Flush last file
    flush_file(
        &mut files,
        &current_path,
        current_status,
        &mut current_hunks,
        is_delete_only,
    );

    files
}

fn flush_file(
    files: &mut Vec<FileEntry>,
    path: &str,
    status: Option<FileStatus>,
    hunks: &mut Vec<Hunk>,
    is_delete_only: bool,
) {
    if path.is_empty() || hunks.is_empty() {
        return;
    }

    let Some(mut status) = status else { return };

    if is_delete_only {
        status = FileStatus::Deleted;
    }

    let additions: usize = hunks.iter().map(|h| h.additions).sum();
    let deletions: usize = hunks.iter().map(|h| h.deletions).sum();

    files.push(FileEntry {
        rel_path: path.to_string(),
        status,
        hunks: std::mem::take(hunks),
        additions,
        deletions,
    });
}

fn parse_hunk_header(line: &str) -> String {
    // Keep the full @@ ... @@ header
    line.to_string()
}
