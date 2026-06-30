use prometheos_lite::runtime_policy::scan_runtime_placeholder_violations;

#[test]
fn runtime_production_code_has_no_placeholder_macros_or_todo_markers() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let violations = scan_runtime_placeholder_violations(&repo_root)
        .expect("runtime placeholder scan should complete");

    if !violations.is_empty() {
        let preview: Vec<String> = violations
            .iter()
            .take(20)
            .map(|v| {
                format!(
                    "{}:{} {} -> {}",
                    v.path.display(),
                    v.line,
                    v.kind,
                    v.snippet
                )
            })
            .collect();
        panic!(
            "runtime policy violations detected ({} total):\n{}",
            violations.len(),
            preview.join("\n")
        );
    }
}
