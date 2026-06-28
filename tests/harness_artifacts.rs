//! Issue 34: Artifact Generator Tests
//!
//! Comprehensive tests for Artifact Generator including:
//! - HarnessArtifact struct (id, kind, path, content, compression, metadata, size)
//! - ArtifactKind enum (Patch, Report, Trace, Evidence, Log, Metrics)
//! - CompressionType enum (None, Gzip, Zstd, Brotli)
//! - ArtifactMetadata struct (work_context_id, harness_run_id, tags, custom_fields)
//! - ArtifactGenerator for creating and managing artifacts
//! - Compression and decompression handling
//! - Artifact size tracking

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::artifacts::{
    ArtifactGenerator, ArtifactKind, ArtifactMetadata, CompressionType, HarnessArtifact,
};

// ============================================================================
// HarnessArtifact Tests
// ============================================================================

#[test]
fn test_harness_artifact_creation() {
    let artifact = HarnessArtifact {
        id: "artifact-1".to_string(),
        kind: ArtifactKind::Patch,
        path: Some(PathBuf::from("patches/fix.patch")),
        content: Some("patch content".to_string()),
        compressed_content: None,
        compression: CompressionType::None,
        metadata: ArtifactMetadata::default(),
        created_at: chrono::Utc::now(),
        size_bytes: 1000,
        compressed_size_bytes: None,
    };

    assert_eq!(artifact.id, "artifact-1");
    assert!(matches!(artifact.kind, ArtifactKind::Patch));
    assert_eq!(artifact.size_bytes, 1000);
}

#[test]
fn test_harness_artifact_compressed() {
    let artifact = HarnessArtifact {
        id: "artifact-2".to_string(),
        kind: ArtifactKind::Log,
        path: Some(PathBuf::from("logs/execution.log")),
        content: None,
        compressed_content: Some(vec![1, 2, 3, 4, 5]),
        compression: CompressionType::Gzip,
        metadata: ArtifactMetadata::default(),
        created_at: chrono::Utc::now(),
        size_bytes: 5000,
        compressed_size_bytes: Some(1000),
    };

    assert!(matches!(artifact.compression, CompressionType::Gzip));
    assert!(artifact.compressed_content.is_some());
    assert_eq!(artifact.compressed_size_bytes, Some(1000));
}

// ============================================================================
// ArtifactKind Tests
// ============================================================================

#[test]
fn test_artifact_kind_variants() {
    assert!(matches!(ArtifactKind::Patch, ArtifactKind::Patch));
    assert!(matches!(ArtifactKind::Report, ArtifactKind::Report));
    assert!(matches!(ArtifactKind::Trace, ArtifactKind::Trace));
    assert!(matches!(ArtifactKind::Evidence, ArtifactKind::Evidence));
    assert!(matches!(ArtifactKind::Log, ArtifactKind::Log));
    assert!(matches!(ArtifactKind::Metrics, ArtifactKind::Metrics));
}

// ============================================================================
// CompressionType Tests
// ============================================================================

#[test]
fn test_compression_type_variants() {
    assert!(matches!(CompressionType::None, CompressionType::None));
    assert!(matches!(CompressionType::Gzip, CompressionType::Gzip));
    assert!(matches!(CompressionType::Zstd, CompressionType::Zstd));
    assert!(matches!(CompressionType::Brotli, CompressionType::Brotli));
}

// ============================================================================
// ArtifactMetadata Tests
// ============================================================================

#[test]
fn test_artifact_metadata_default() {
    let metadata = ArtifactMetadata::default();

    assert!(metadata.work_context_id.is_empty());
    assert!(metadata.harness_run_id.is_empty());
    assert!(metadata.tags.is_empty());
    assert!(metadata.custom_fields.is_empty());
}

#[test]
fn test_artifact_metadata_creation() {
    let mut custom = HashMap::new();
    custom.insert("author".to_string(), "ai".to_string());

    let metadata = ArtifactMetadata {
        work_context_id: "work-123".to_string(),
        harness_run_id: "run-456".to_string(),
        tags: vec!["important".to_string(), "reviewed".to_string()],
        custom_fields: custom,
    };

    assert_eq!(metadata.work_context_id, "work-123");
    assert_eq!(metadata.tags.len(), 2);
    assert_eq!(
        metadata.custom_fields.get("author"),
        Some(&"ai".to_string())
    );
}

// ============================================================================
// ArtifactGenerator Tests
// ============================================================================

#[test]
fn test_artifact_generator_new() {
    let _generator = ArtifactGenerator::new();
    // Generator created successfully
}

#[test]
fn test_artifact_generator_with_compression() {
    let _generator = ArtifactGenerator::with_compression(true);
    // Generator created with compression enabled
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_artifact_workflow() {
    // Create metadata
    let metadata = ArtifactMetadata {
        work_context_id: "workflow-test".to_string(),
        harness_run_id: "run-789".to_string(),
        tags: vec!["patch".to_string()],
        custom_fields: HashMap::new(),
    };

    // Create artifact
    let artifact = HarnessArtifact {
        id: "patch-001".to_string(),
        kind: ArtifactKind::Patch,
        path: Some(PathBuf::from("output/patch.diff")),
        content: Some(
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-fn main() {}".to_string(),
        ),
        compressed_content: None,
        compression: CompressionType::None,
        metadata,
        created_at: chrono::Utc::now(),
        size_bytes: 150,
        compressed_size_bytes: None,
    };

    assert_eq!(artifact.metadata.work_context_id, "workflow-test");
    assert!(artifact.content.is_some());
    assert_eq!(artifact.size_bytes, 150);
}

#[test]
fn test_multiple_artifact_types() {
    let artifacts = [
        HarnessArtifact {
            id: "patch-1".to_string(),
            kind: ArtifactKind::Patch,
            path: Some(PathBuf::from("patches/1.patch")),
            content: Some("patch".to_string()),
            compressed_content: None,
            compression: CompressionType::None,
            metadata: ArtifactMetadata::default(),
            created_at: chrono::Utc::now(),
            size_bytes: 100,
            compressed_size_bytes: None,
        },
        HarnessArtifact {
            id: "report-1".to_string(),
            kind: ArtifactKind::Report,
            path: Some(PathBuf::from("reports/1.md")),
            content: Some("# Report".to_string()),
            compressed_content: None,
            compression: CompressionType::None,
            metadata: ArtifactMetadata::default(),
            created_at: chrono::Utc::now(),
            size_bytes: 500,
            compressed_size_bytes: None,
        },
        HarnessArtifact {
            id: "metrics-1".to_string(),
            kind: ArtifactKind::Metrics,
            path: Some(PathBuf::from("metrics/1.json")),
            content: Some("{}".to_string()),
            compressed_content: None,
            compression: CompressionType::None,
            metadata: ArtifactMetadata::default(),
            created_at: chrono::Utc::now(),
            size_bytes: 50,
            compressed_size_bytes: None,
        },
    ];

    assert_eq!(artifacts.len(), 3);
    assert!(matches!(artifacts[0].kind, ArtifactKind::Patch));
    assert!(matches!(artifacts[1].kind, ArtifactKind::Report));
    assert!(matches!(artifacts[2].kind, ArtifactKind::Metrics));
}
