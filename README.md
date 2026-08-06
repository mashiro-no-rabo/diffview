# diffview

TUI diff review tool. Reads a unified (git) diff from stdin and presents an interactive flat file list for reviewing changes.

## Install

```
cargo install --path .
```

## Usage

```
jj diff | diffview
git diff | diffview
```

### Jujutsu integration

```toml
[ui]
diff-formatter = ":git"

[[--scope]]
--when.commands = ["diff"]
[--scope.ui]
pager = ["diffview"]
```

## Keybindings

### Main view

| Key | Action |
|-----|--------|
| `↑`/`↓` | Move cursor to prev/next visible item; scrolls the viewport when the next item is off-screen |
| Mouse wheel | Scroll viewport one line |
| `j`/`k` | Jump to next/prev file (wraps) |
| `←` | Fold the current file (when on a hunk header) |
| `→` | Unfold a folded file, or step into its first hunk |
| `Space` | Toggle confirmed (reviewed) on the current file or hunk |
| `Enter` | Same as `Space`, then advance to the next item |
| `a` | Invert confirmation on the current file or hunk |
| `w` | Toggle word wrap (on by default) |
| `Tab` | Enter file view (scroll line-by-line through one file) |
| `f` | Fuzzy file search popup |
| `?` | Help |
| `q`/`Esc` | Quit |
| `Ctrl+C` | Force quit |

`.lock`, deleted, and binary files are folded by default.

### File view

| Key | Action |
|-----|--------|
| `↑`/`↓` | Move cursor one line |
| `j`/`k` | Move cursor half a page down/up |
| `Space` | Toggle the current hunk's confirmation |
| `Enter` | Toggle and advance to the next line |
| `w` | Toggle word wrap (on by default) |
| `Tab`/`Esc` | Return to the main view |
| `q` / `Ctrl+C` | Quit |
