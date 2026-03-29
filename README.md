# diffview

TUI diff review tool. Reads unified (git) diff from stdin and presents an interactive nested block layout for reviewing changes.

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

| Key | Action |
|-----|--------|
| `↑`/`↓` | Navigate |
| `j`/`k` | Jump to prev/next file |
| `←`/`→` | Fold/unfold folder or file |
| `Space` | Toggle confirmed (reviewed) |
| `Enter` | Confirm and advance |
| `a` | Invert confirmation |
| `f` | Fuzzy file search popup |
| `?` | Help |
| `q`/`Esc` | Quit |
