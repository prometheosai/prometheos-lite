use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeDomain {
    SoftwareHarness,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub path: PathBuf,
    pub line: usize,
    pub kind: String,
    pub snippet: String,
}

pub fn allow_raw_write_override() -> bool {
    std::env::var("PROMETHEOS_ALLOW_RAW_WRITE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

pub fn is_raw_write_allowed(domain: RuntimeDomain) -> bool {
    match domain {
        RuntimeDomain::SoftwareHarness => allow_raw_write_override(),
        RuntimeDomain::Other => true,
    }
}

fn is_allowed_detector_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("ci_enforcement.rs") | Some("review.rs") | Some("reproduction.rs")
    )
}

pub fn scan_runtime_placeholder_violations(repo_root: &Path) -> Result<Vec<PolicyViolation>> {
    let src_root = repo_root.join("src");
    let patterns = vec![
        ("todo_macro", Regex::new(r"\btodo!\s*\(")?),
        ("unimplemented_macro", Regex::new(r"\bunimplemented!\s*\(")?),
        ("todo_comment", Regex::new(r"\bTODO:")?),
        ("fixme_comment", Regex::new(r"\bFIXME:")?),
    ];

    let mut violations = Vec::new();
    for entry in WalkDir::new(&src_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        if is_allowed_detector_file(path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            for (kind, re) in &patterns {
                if re.is_match(line) {
                    violations.push(PolicyViolation {
                        path: path.to_path_buf(),
                        line: idx + 1,
                        kind: (*kind).to_string(),
                        snippet: line.trim().to_string(),
                    });
                }
            }
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_write_policy() {
        assert!(is_raw_write_allowed(RuntimeDomain::Other));
    }
}
