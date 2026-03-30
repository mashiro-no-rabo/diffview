//! Tests for confirmed diff output format, merged folder paths, and edge cases.

#[path = "../src/parser.rs"]
mod parser;

#[path = "../src/fuzzy/mod.rs"]
mod fuzzy;

#[path = "../src/model.rs"]
mod model;

use model::App;
use parser::{format_confirmed_diff, parse_diff};

fn folder_items(app: &App) -> Vec<(String, usize)> {
    let items = app.visible_items();
    items
        .iter()
        .filter_map(|i| match &i.kind {
            model::VisibleKind::Folder(p) => Some((p.clone(), i.depth)),
            _ => None,
        })
        .collect()
}

// ── Helpers ──

fn make_diff(files: &[(&str, &str, &[&str])]) -> String {
    let mut out = String::new();
    for &(path, status, hunks) in files {
        out.push_str(&format!("diff --git a/{path} b/{path}\n"));
        match status {
            "A" => {
                out.push_str("new file mode 100644\n");
                out.push_str("--- /dev/null\n");
                out.push_str(&format!("+++ b/{path}\n"));
            }
            "D" => {
                out.push_str("deleted file mode 100644\n");
                out.push_str(&format!("--- a/{path}\n"));
                out.push_str("+++ /dev/null\n");
            }
            _ => {
                out.push_str(&format!("--- a/{path}\n"));
                out.push_str(&format!("+++ b/{path}\n"));
            }
        }
        for hunk in hunks {
            out.push_str(hunk);
            if !hunk.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

const HUNK_A: &str = "\
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
";

const HUNK_B: &str = "\
@@ -10,3 +11,3 @@
 ctx
-old
+new
 ctx
";

const HUNK_C: &str = "\
@@ -20,2 +20,3 @@
 before
+inserted
 after
";

// ── Output format: single hunk ──

#[test]
fn single_hunk_confirmed() {
    let diff = make_diff(&[("src/foo.rs", "M", &[HUNK_A])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Output format: multiple hunks ──

#[test]
fn multiple_hunks_all_confirmed() {
    let diff = make_diff(&[("src/foo.rs", "M", &[HUNK_A, HUNK_B])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    files[0].hunks[1].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn multiple_hunks_partial_selection() {
    let diff = make_diff(&[("src/foo.rs", "M", &[HUNK_A, HUNK_B, HUNK_C])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    // hunk[1] NOT confirmed
    files[0].hunks[2].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Output format: no selection ──

#[test]
fn no_selection_produces_empty_output() {
    let diff = make_diff(&[("src/foo.rs", "M", &[HUNK_A, HUNK_B])]);
    let files = parse_diff(&diff);
    assert!(format_confirmed_diff(&files).is_empty());
}

// ── Output format: multiple files ──

#[test]
fn multiple_files_mixed_selection() {
    let diff = make_diff(&[
        ("src/a.rs", "M", &[HUNK_A]),
        ("src/b.rs", "M", &[HUNK_B]),
        ("src/c.rs", "M", &[HUNK_C]),
    ]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    // files[1] NOT confirmed
    files[2].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn multiple_files_no_selection() {
    let diff = make_diff(&[
        ("src/a.rs", "M", &[HUNK_A]),
        ("src/b.rs", "M", &[HUNK_B]),
    ]);
    let files = parse_diff(&diff);
    assert!(format_confirmed_diff(&files).is_empty());
}

// ── Output format: file statuses ──

#[test]
fn added_file_format() {
    let diff = make_diff(&[("new_file.rs", "A", &[HUNK_A])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn deleted_file_format() {
    let hunk_del = "@@ -1,3 +0,0 @@\n-line1\n-line2\n-line3\n";
    let diff = make_diff(&[("old_file.rs", "D", &[hunk_del])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Output format: renamed file ──

#[test]
fn renamed_file_with_hunks() {
    let diff = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 90%
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
";
    let mut files = parse_diff(diff);
    assert_eq!(files[0].rel_path, "new_name.rs");
    assert_eq!(files[0].old_path.as_deref(), Some("old_name.rs"));
    assert_eq!(files[0].status, parser::FileStatus::Renamed);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Output format: copied file ──

#[test]
fn copied_file() {
    let diff = "\
diff --git a/orig.rs b/copy.rs
similarity index 95%
copy from orig.rs
copy to copy.rs
--- a/orig.rs
+++ b/copy.rs
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
";
    let mut files = parse_diff(diff);
    assert_eq!(files[0].status, parser::FileStatus::Copied);
    assert_eq!(files[0].old_path.as_deref(), Some("orig.rs"));
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Binary files ──

#[test]
fn binary_file_differ() {
    let diff = "\
diff --git a/image.png b/image.png
index abc1234..def5678 100644
Binary files a/image.png and b/image.png differ
";
    let files = parse_diff(diff);
    assert_eq!(files.len(), 1);
    assert!(files[0].binary);
    assert!(files[0].hunks.is_empty());
}

#[test]
fn binary_file_git_patch() {
    let diff = "\
diff --git a/image.png b/image.png
new file mode 100644
index 0000000..abc1234
GIT binary patch
literal 5432
zcmVdata1234

literal 0
Hc$@<O00001

";
    let files = parse_diff(diff);
    assert_eq!(files.len(), 1);
    assert!(files[0].binary);
    assert_eq!(files[0].binary_new_size, Some(5432));
    assert_eq!(files[0].binary_old_size, Some(0));
}

#[test]
fn binary_file_confirmed_output() {
    let diff = "\
diff --git a/image.png b/image.png
index abc1234..def5678 100644
GIT binary patch
literal 2048
zcmVdata

literal 1024
zcmVolddata

";
    let mut files = parse_diff(diff);
    files[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn binary_file_not_confirmed_empty_output() {
    let diff = "\
diff --git a/image.png b/image.png
index abc1234..def5678 100644
Binary files a/image.png and b/image.png differ
";
    let files = parse_diff(diff);
    assert!(format_confirmed_diff(&files).is_empty());
}

// ── Mode change only ──

#[test]
fn mode_change_only() {
    let diff = "\
diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
";
    let files = parse_diff(diff);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].rel_path, "script.sh");
    assert!(files[0].hunks.is_empty());
    assert!(!files[0].binary);
}

#[test]
fn mode_change_confirmed_output() {
    let diff = "\
diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
";
    let mut files = parse_diff(diff);
    files[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

// ── Hunk ordering ──

#[test]
fn hunk_order_preserved() {
    let diff = make_diff(&[("src/foo.rs", "M", &[HUNK_A, HUNK_B, HUNK_C])]);
    let mut files = parse_diff(&diff);
    for h in &mut files[0].hunks {
        h.confirmed = true;
    }
    let output = format_confirmed_diff(&files);
    let a_pos = output.find("@@ -1,3 +1,4 @@").unwrap();
    let b_pos = output.find("@@ -10,3 +11,3 @@").unwrap();
    let c_pos = output.find("@@ -20,2 +20,3 @@").unwrap();
    assert!(a_pos < b_pos);
    assert!(b_pos < c_pos);
}

#[test]
fn file_order_preserved() {
    let diff = make_diff(&[
        ("src/a.rs", "M", &[HUNK_A]),
        ("src/b.rs", "M", &[HUNK_B]),
        ("src/c.rs", "M", &[HUNK_C]),
    ]);
    let mut files = parse_diff(&diff);
    for f in &mut files {
        for h in &mut f.hunks {
            h.confirmed = true;
        }
    }
    let output = format_confirmed_diff(&files);
    let a_pos = output.find("a/src/a.rs").unwrap();
    let b_pos = output.find("a/src/b.rs").unwrap();
    let c_pos = output.find("a/src/c.rs").unwrap();
    assert!(a_pos < b_pos);
    assert!(b_pos < c_pos);
}

// ── ANSI stripping ──

#[test]
fn ansi_stripped_before_parse() {
    let diff = "\
\x1b[1mdiff --git a/src/foo.rs b/src/foo.rs\x1b[0m
\x1b[1m--- a/src/foo.rs\x1b[0m
\x1b[1m+++ b/src/foo.rs\x1b[0m
\x1b[36m@@ -1,3 +1,4 @@\x1b[0m
 line1
\x1b[32m+added\x1b[0m
 line2
 line3
";
    let mut files = parse_diff(diff);
    assert_eq!(files.len(), 1);
    files[0].hunks[0].confirmed = true;
    let output = format_confirmed_diff(&files);
    assert!(!output.contains('\x1b'));
    insta::assert_snapshot!(output);
}

// ── Round-trip ──

#[test]
fn round_trip_all_confirmed() {
    let diff = make_diff(&[
        ("src/a.rs", "M", &[HUNK_A, HUNK_B]),
        ("lib/b.rs", "A", &[HUNK_C]),
    ]);
    let mut files = parse_diff(&diff);
    for f in &mut files {
        for h in &mut f.hunks {
            h.confirmed = true;
        }
    }
    let output = format_confirmed_diff(&files);
    let reparsed = parse_diff(&output);
    assert_eq!(reparsed.len(), 2);
    assert_eq!(reparsed[0].hunks.len(), 2);
    assert_eq!(reparsed[1].hunks.len(), 1);
    assert_eq!(reparsed[0].rel_path, "src/a.rs");
    assert_eq!(reparsed[1].rel_path, "lib/b.rs");
}

// ── Merged folder paths ──

#[test]
fn single_child_folders_merge() {
    let diff = make_diff(&[("src/app/components/Foo.rs", "M", &[HUNK_A])]);
    let app = App::new(parse_diff(&diff));
    insta::assert_debug_snapshot!(folder_items(&app));
}

#[test]
fn branching_folders_not_merged() {
    let diff = make_diff(&[
        ("src/app/components/Foo.rs", "M", &[HUNK_A]),
        ("src/app/utils/Bar.rs", "M", &[HUNK_B]),
    ]);
    let app = App::new(parse_diff(&diff));
    insta::assert_debug_snapshot!(folder_items(&app));
}

#[test]
fn folder_with_files_and_subfolder_not_merged() {
    let diff = make_diff(&[
        ("src/lib.rs", "M", &[HUNK_A]),
        ("src/app/main.rs", "M", &[HUNK_B]),
    ]);
    let app = App::new(parse_diff(&diff));
    insta::assert_debug_snapshot!(folder_items(&app));
}

#[test]
fn root_level_files_have_no_folders() {
    let diff = make_diff(&[("Cargo.toml", "M", &[HUNK_A])]);
    let app = App::new(parse_diff(&diff));
    assert!(folder_items(&app).is_empty());
}

#[test]
fn deep_merge_chain() {
    let diff = make_diff(&[("a/b/c/d/file.rs", "M", &[HUNK_A])]);
    let app = App::new(parse_diff(&diff));
    insta::assert_debug_snapshot!(folder_items(&app));
}

#[test]
fn mixed_root_and_nested_files() {
    let diff = make_diff(&[
        ("Cargo.toml", "M", &[HUNK_A]),
        ("src/main.rs", "M", &[HUNK_B]),
        ("tests/test.rs", "M", &[HUNK_C]),
    ]);
    let app = App::new(parse_diff(&diff));
    insta::assert_debug_snapshot!(folder_items(&app));
}

// ── Binary files in model ──

#[test]
fn binary_files_default_folded() {
    let diff = "\
diff --git a/src/code.rs b/src/code.rs
--- a/src/code.rs
+++ b/src/code.rs
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
diff --git a/image.png b/image.png
index abc1234..def5678 100644
Binary files a/image.png and b/image.png differ
";
    let app = App::new(parse_diff(diff));
    assert!(app.folded_files.contains(&1), "binary file should be folded by default");
    assert!(!app.folded_files.contains(&0), "normal file should not be folded");
}

#[test]
fn lock_files_default_folded() {
    let diff = make_diff(&[
        ("src/main.rs", "M", &[HUNK_A]),
        ("Cargo.lock", "M", &[HUNK_B]),
    ]);
    let app = App::new(parse_diff(&diff));
    assert!(app.folded_files.contains(&1), "lock file should be folded by default");
    assert!(!app.folded_files.contains(&0), "normal file should not be folded");
}

// ── Edge cases ──

#[test]
fn hunk_with_only_additions() {
    let hunk = "@@ -5,0 +6,2 @@\n+new1\n+new2\n";
    let diff = make_diff(&[("src/foo.rs", "A", &[hunk])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn hunk_with_only_deletions() {
    let hunk = "@@ -1,3 +1,0 @@\n-gone1\n-gone2\n-gone3\n";
    let diff = make_diff(&[("src/foo.rs", "D", &[hunk])]);
    let mut files = parse_diff(&diff);
    files[0].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn binary_and_text_files_mixed() {
    let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
diff --git a/image.png b/image.png
index abc..def 100644
Binary files a/image.png and b/image.png differ
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +11,3 @@
 ctx
-old
+new
 ctx
";
    let mut files = parse_diff(diff);
    assert_eq!(files.len(), 3);
    // Confirm text files, confirm binary
    files[0].hunks[0].confirmed = true;
    files[1].confirmed = true;
    files[2].hunks[0].confirmed = true;
    insta::assert_snapshot!(format_confirmed_diff(&files));
}

#[test]
fn file_total_units_and_counts() {
    let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
@@ -10,3 +11,3 @@
 ctx
-old
+new
 ctx
diff --git a/image.png b/image.png
index abc..def 100644
Binary files a/image.png and b/image.png differ
";
    let files = parse_diff(diff);
    // Text file with 2 hunks
    assert_eq!(files[0].total_units(), 2);
    assert_eq!(files[0].confirmed_count(), 0);
    // Binary file = 1 unit
    assert_eq!(files[1].total_units(), 1);
    assert_eq!(files[1].confirmed_count(), 0);
}

// ── Navigation: j/k highlight tracking ──

// 5 files across nested folders.
// app/
//   core/engine/processor.rs       (merged chain)
//   plugins/
//     auth/login.rs
//     storage/cache.rs
//     network/client.rs
//     logging/output.rs
const NAV_DIFF: &str = "\
diff --git a/app/core/engine/processor.rs b/app/core/engine/processor.rs
--- a/app/core/engine/processor.rs
+++ b/app/core/engine/processor.rs
@@ -1,5 +1,6 @@
 fn process() {
+    validate_input();
     let data = fetch();
     transform(data);
     store(data);
 }
diff --git a/app/plugins/auth/login.rs b/app/plugins/auth/login.rs
--- a/app/plugins/auth/login.rs
+++ b/app/plugins/auth/login.rs
@@ -10,5 +10,6 @@
 fn authenticate() {
+    check_rate_limit();
     let token = create_token();
     verify(token);
     set_session(token);
 }
diff --git a/app/plugins/storage/cache.rs b/app/plugins/storage/cache.rs
--- a/app/plugins/storage/cache.rs
+++ b/app/plugins/storage/cache.rs
@@ -20,5 +20,6 @@
 fn cache_lookup() {
+    log_access();
     let key = hash(input);
     if let Some(val) = store.get(key) {
         return val;
 }
diff --git a/app/plugins/network/client.rs b/app/plugins/network/client.rs
--- a/app/plugins/network/client.rs
+++ b/app/plugins/network/client.rs
@@ -30,5 +30,6 @@
 fn connect() {
+    set_timeout(30);
     let addr = resolve(host);
     let sock = open(addr);
     handshake(sock);
 }
diff --git a/app/plugins/logging/output.rs b/app/plugins/logging/output.rs
--- a/app/plugins/logging/output.rs
+++ b/app/plugins/logging/output.rs
@@ -40,5 +40,6 @@
 fn write_log() {
+    rotate_if_needed();
     let msg = format_entry();
     file.write(msg);
     file.flush();
 }
";

fn highlighted_file(app: &App) -> String {
    let items = app.visible_items();
    items.get(app.cursor).map(|vi| match &vi.kind {
        model::VisibleKind::Folder(p) => format!("[folder] {}", p),
        model::VisibleKind::File(idx) => app.files[*idx].rel_path.clone(),
        model::VisibleKind::HunkHeader(fi, hi) => {
            format!("[hunk {}.{}]", app.files[*fi].rel_path, hi)
        }
        model::VisibleKind::HunkLine(fi, hi, li) => {
            format!("[line {}.{}.{}]", app.files[*fi].rel_path, hi, li)
        }
    }).unwrap_or_else(|| "none".into())
}

fn trace_navigation(app: &mut App, keys: &[&str]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("start: {}", highlighted_file(app)));
    for &key in keys {
        match key {
            "j" => app.next_file(),
            "k" => app.prev_file(),
            "down" => app.cursor_down(),
            "up" => app.cursor_up(),
            _ => {}
        }
        lines.push(format!("{:>5}: {}", key, highlighted_file(app)));
    }
    lines.join("\n")
}

#[test]
fn j_cycles_through_files() {
    let mut app = App::new(parse_diff(NAV_DIFF));
    // j 6 times: 5 files + wrap to first
    insta::assert_snapshot!(trace_navigation(
        &mut app,
        &["j", "j", "j", "j", "j", "j"]
    ));
}

#[test]
fn k_cycles_through_files_reverse() {
    let mut app = App::new(parse_diff(NAV_DIFF));
    // k wraps to last file, then walks backwards
    insta::assert_snapshot!(trace_navigation(
        &mut app,
        &["k", "k", "k", "k", "k", "k"]
    ));
}

#[test]
fn j_then_k_round_trips() {
    let mut app = App::new(parse_diff(NAV_DIFF));
    insta::assert_snapshot!(trace_navigation(
        &mut app,
        &["j", "j", "j", "k", "k", "k"]
    ));
}

#[test]
fn down_walks_all_targets() {
    let mut app = App::new(parse_diff(NAV_DIFF));
    // down visits folders, files, and hunk headers in order
    let keys: Vec<&str> = (0..20).map(|_| "down").collect();
    insta::assert_snapshot!(trace_navigation(&mut app, &keys));
}
