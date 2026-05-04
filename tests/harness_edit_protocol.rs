//! Issue 4: Edit Protocol Tests
//!
//! Comprehensive tests for the Edit Protocol including:
//! - EditOperation enum variants (SearchReplace, UnifiedDiff, WholeFile, CreateFile, DeleteFile, RenameFile)
//! - SearchReplaceEdit struct creation
//! - UnifiedDiff parsing and application
//! - WholeFileEdit struct creation
//! - CreateFileEdit struct creation
//! - DeleteFileEdit struct creation
//! - RenameFileEdit struct creation
//! - ParsedDiff and DiffHunk handling
//! - Edit response parsing
//! - Edit operation validation
//! - Edit summary generation
//! - Edit merging

use std::path::PathBuf;

use prometheos_lite::harness::edit_protocol::{
    apply_unified_diff, parse_edit_response, parse_unified_diff, CreateFileEdit,
    DeleteFileEdit, DiffHunk, DiffLine, EditOperation, ParsedDiff, RenameFileEdit,
    SearchReplaceEdit, UnifiedDiffEdit, WholeFileEdit, get_edit_summary, merge_edits,
};

// ============================================================================
// Basic Structure Tests
// ============================================================================

#[test]
fn test_edit_operation_variants() {
    // Test SearchReplace variant
    let sr = SearchReplaceEdit {
        file: PathBuf::from("src/main.rs"),
        search: "old_code".to_string(),
        replace: "new_code".to_string(),
    };
    let op = EditOperation::SearchReplace(sr);
    assert!(matches!(op, EditOperation::SearchReplace(_)));

    // Test UnifiedDiff variant
    let ud = UnifiedDiffEdit {
        diff: "--- a/file\n+++ b/file".to_string(),
        target_file: Some(PathBuf::from("file.rs")),
    };
    let op = EditOperation::UnifiedDiff(ud);
    assert!(matches!(op, EditOperation::UnifiedDiff(_)));

    // Test WholeFile variant
    let wf = WholeFileEdit {
        file: PathBuf::from("config.toml"),
        content: "key = value".to_string(),
    };
    let op = EditOperation::WholeFile(wf);
    assert!(matches!(op, EditOperation::WholeFile(_)));
}

#[test]
fn test_search_replace_edit_creation() {
    let edit = SearchReplaceEdit {
        file: PathBuf::from("src/lib.rs"),
        search: "fn old()".to_string(),
        replace: "fn new()".to_string(),
    };

    assert_eq!(edit.file, PathBuf::from("src/lib.rs"));
    assert_eq!(edit.search, "fn old()");
    assert_eq!(edit.replace, "fn new()");
}

#[test]
fn test_unified_diff_edit_creation() {
    let edit = UnifiedDiffEdit {
        diff: "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n line1\n-old\n+new\n line3".to_string(),
        target_file: Some(PathBuf::from("src/main.rs")),
    };

    assert!(!edit.diff.is_empty());
    assert_eq!(edit.target_file, Some(PathBuf::from("src/main.rs")));
}

#[test]
fn test_whole_file_edit_creation() {
    let edit = WholeFileEdit {
        file: PathBuf::from("README.md"),
        content: "# New Project\n\nDescription".to_string(),
    };

    assert_eq!(edit.file, PathBuf::from("README.md"));
    assert_eq!(edit.content, "# New Project\n\nDescription");
}

#[test]
fn test_create_file_edit_creation() {
    let edit = CreateFileEdit {
        file: PathBuf::from("new_file.rs"),
        content: "// New file content".to_string(),
        open_after_create: Some(true),
    };

    assert_eq!(edit.file, PathBuf::from("new_file.rs"));
    assert_eq!(edit.content, "// New file content");
    assert_eq!(edit.open_after_create, Some(true));
}

#[test]
fn test_delete_file_edit_creation() {
    let edit = DeleteFileEdit {
        file: PathBuf::from("old_file.rs"),
    };

    assert_eq!(edit.file, PathBuf::from("old_file.rs"));
}

#[test]
fn test_rename_file_edit_creation() {
    let edit = RenameFileEdit {
        from: PathBuf::from("old_name.rs"),
        to: PathBuf::from("new_name.rs"),
    };

    assert_eq!(edit.from, PathBuf::from("old_name.rs"));
    assert_eq!(edit.to, PathBuf::from("new_name.rs"));
}

// ============================================================================
// Diff Parsing Tests
// ============================================================================

#[test]
fn test_parse_unified_diff_basic() {
    let diff_text = r#"--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    let x = 1;
+    let x = 2;
     println!("{}", x);
 }
"#;

    let diffs = parse_unified_diff(diff_text).unwrap();
    assert_eq!(diffs.len(), 1);
    
    let parsed = &diffs[0];
    assert_eq!(parsed.new_file, PathBuf::from("src/main.rs"));
    assert_eq!(parsed.hunks.len(), 1);
    assert_eq!(parsed.hunks[0].old_start, 1);
    assert_eq!(parsed.hunks[0].old_lines, 5);
}

#[test]
fn test_parse_unified_diff_multiple_files() {
    let diff_text = r#"--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 line1
-old
+new
 line3
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
 fn lib() {}
-    old
+    new
"#;

    let diffs = parse_unified_diff(diff_text).unwrap();
    assert_eq!(diffs.len(), 2);
    
    assert_eq!(diffs[0].new_file, PathBuf::from("src/main.rs"));
    assert_eq!(diffs[1].new_file, PathBuf::from("src/lib.rs"));
}

#[test]
fn test_parse_unified_diff_empty() {
    let diffs = parse_unified_diff("").unwrap();
    assert!(diffs.is_empty());
}

#[test]
fn test_parse_unified_diff_new_file() {
    let diff_text = r#"--- /dev/null
+++ b/new_file.rs
@@ -0,0 +1,3 @@
+fn new() {}
+    
+    
"#;

    let diffs = parse_unified_diff(diff_text).unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].new_file, PathBuf::from("new_file.rs"));
    assert!(diffs[0].old_file.is_none());
}

// ============================================================================
// Diff Application Tests
// ============================================================================

#[test]
fn test_apply_unified_diff_simple() {
    let original = "line1\nold\nline3";
    
    let diff = ParsedDiff {
        old_file: Some(PathBuf::from("file.rs")),
        new_file: PathBuf::from("file.rs"),
        hunks: vec![DiffHunk {
            old_start: 1,
            old_lines: 3,
            new_start: 1,
            new_lines: 3,
            lines: vec![
                DiffLine::Context("line1".to_string()),
                DiffLine::Removed("old".to_string()),
                DiffLine::Added("new".to_string()),
                DiffLine::Context("line3".to_string()),
            ],
        }],
    };

    let result = apply_unified_diff(original, &diff).unwrap();
    assert_eq!(result, "line1\nnew\nline3");
}

#[test]
fn test_apply_unified_diff_add_line() {
    let original = "line1\nline2";
    
    let diff = ParsedDiff {
        old_file: Some(PathBuf::from("file.rs")),
        new_file: PathBuf::from("file.rs"),
        hunks: vec![DiffHunk {
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 3,
            lines: vec![
                DiffLine::Context("line1".to_string()),
                DiffLine::Context("line2".to_string()),
                DiffLine::Added("line3".to_string()),
            ],
        }],
    };

    let result = apply_unified_diff(original, &diff).unwrap();
    assert_eq!(result, "line1\nline2\nline3");
}

#[test]
fn test_apply_unified_diff_remove_line() {
    let original = "line1\nremove_me\nline3";
    
    let diff = ParsedDiff {
        old_file: Some(PathBuf::from("file.rs")),
        new_file: PathBuf::from("file.rs"),
        hunks: vec![DiffHunk {
            old_start: 1,
            old_lines: 3,
            new_start: 1,
            new_lines: 2,
            lines: vec![
                DiffLine::Context("line1".to_string()),
                DiffLine::Removed("remove_me".to_string()),
                DiffLine::Context("line3".to_string()),
            ],
        }],
    };

    let result = apply_unified_diff(original, &diff).unwrap();
    assert_eq!(result, "line1\nline3");
}

// ============================================================================
// Edit Response Parsing Tests
// ============================================================================

#[test]
fn test_parse_edit_response_json() {
    let response = r#"[{"type": "whole_file", "file": "config.toml", "content": "key = value"}]"#;
    
    let edits = parse_edit_response(response).unwrap();
    assert_eq!(edits.len(), 1);
    assert!(matches!(edits[0], EditOperation::WholeFile(_)));
}

#[test]
fn test_parse_edit_response_search_replace_block() {
    let response = r#"<<<<<<< SEARCH
old code
=======
new code
>>>>>>> REPLACE"#;

    let edits = parse_edit_response(response).unwrap();
    assert!(!edits.is_empty());
}

#[test]
fn test_parse_edit_response_empty() {
    let edits = parse_edit_response("").unwrap();
    assert!(edits.is_empty());
}

// ============================================================================
// Edit Summary Tests
// ============================================================================

#[test]
fn test_get_edit_summary_empty() {
    let summary = get_edit_summary(&[]);
    assert!(summary.contains("0 modifications"));
    assert!(summary.contains("0 creations"));
    assert!(summary.contains("0 deletions"));
}

#[test]
fn test_get_edit_summary_with_edits() {
    let edits = vec![
        EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/main.rs"),
            search: "old".to_string(),
            replace: "new".to_string(),
        }),
        EditOperation::CreateFile(CreateFileEdit {
            file: PathBuf::from("new.rs"),
            content: "content".to_string(),
            open_after_create: None,
        }),
        EditOperation::DeleteFile(DeleteFileEdit {
            file: PathBuf::from("old.rs"),
        }),
    ];

    let summary = get_edit_summary(&edits);
    assert!(summary.contains("1 modifications"));
    assert!(summary.contains("1 creations"));
    assert!(summary.contains("1 deletions"));
}

// ============================================================================
// Edit Merging Tests
// ============================================================================

#[test]
fn test_merge_edits_same_file() {
    let edits = vec![
        EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/main.rs"),
            search: "old1".to_string(),
            replace: "new1".to_string(),
        }),
        EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/main.rs"),
            search: "old2".to_string(),
            replace: "new2".to_string(),
        }),
    ];

    let merged = merge_edits(edits);
    // Should merge edits for the same file
    assert!(!merged.is_empty());
}

#[test]
fn test_merge_edits_different_files() {
    let edits = vec![
        EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/main.rs"),
            search: "old".to_string(),
            replace: "new".to_string(),
        }),
        EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from("src/lib.rs"),
            search: "old".to_string(),
            replace: "new".to_string(),
        }),
    ];

    let merged = merge_edits(edits);
    assert_eq!(merged.len(), 2);
}
