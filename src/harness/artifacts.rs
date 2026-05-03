use crate::harness::execution_loop::HarnessExecutionResult;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessArtifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub path: Option<PathBuf>,
    pub content: Option<String>,
    pub compressed_content: Option<Vec<u8>>,
    pub compression: CompressionType,
    pub metadata: ArtifactMetadata,
    pub created_at: DateTime<Utc>,
    pub size_bytes: usize,
    pub compressed_size_bytes: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactKind {
    Patch,
    Report,
    Trace,
    Evidence,
    Log,
    Metrics,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressionType {
    None,
    Gzip,
    Zstd,
    Brotli,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ArtifactMetadata {
    pub work_context_id: String,
    pub harness_run_id: String,
    pub tags: Vec<String>,
    pub custom_fields: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ArtifactGenerator {
    compression_enabled: bool,
    compression_threshold_bytes: usize,
    default_compression: CompressionType,
}

impl Default for ArtifactGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactGenerator {
    pub fn new() -> Self {
        Self {
            compression_enabled: true,
            compression_threshold_bytes: 1024, // Compress files > 1KB
            default_compression: CompressionType::Gzip,
        }
    }

    pub fn with_compression(enabled: bool) -> Self {
        Self {
            compression_enabled: enabled,
            compression_threshold_bytes: 1024,
            default_compression: CompressionType::Gzip,
        }
    }

    pub fn generate(
        &self,
        kind: ArtifactKind,
        content: String,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        let size_bytes = content.len();
        let (compressed_content, compressed_size, compression) =
            if self.compression_enabled && size_bytes > self.compression_threshold_bytes {
                let compressed = self.compress(&content, self.default_compression)?;
                let compressed_size = compressed.len();
                (
                    Some(compressed),
                    Some(compressed_size),
                    self.default_compression,
                )
            } else {
                (None, None, CompressionType::None)
            };

        Ok(HarnessArtifact {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            path: None,
            content: if compressed_content.is_some() {
                None
            } else {
                Some(content)
            },
            compressed_content,
            compression,
            metadata,
            created_at: Utc::now(),
            size_bytes,
            compressed_size_bytes: compressed_size,
        })
    }

    pub fn generate_patch_artifact(
        &self,
        patch_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(ArtifactKind::Patch, patch_content.to_string(), metadata)
    }

    pub fn generate_report_artifact(
        &self,
        report_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(ArtifactKind::Report, report_content.to_string(), metadata)
    }

    pub fn generate_trace_artifact(
        &self,
        trace_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(ArtifactKind::Trace, trace_content.to_string(), metadata)
    }

    pub fn generate_evidence_artifact(
        &self,
        evidence_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(
            ArtifactKind::Evidence,
            evidence_content.to_string(),
            metadata,
        )
    }

    pub fn generate_log_artifact(
        &self,
        log_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(ArtifactKind::Log, log_content.to_string(), metadata)
    }

    pub fn generate_metrics_artifact(
        &self,
        metrics_content: &str,
        metadata: ArtifactMetadata,
    ) -> Result<HarnessArtifact> {
        self.generate(ArtifactKind::Metrics, metrics_content.to_string(), metadata)
    }

    pub fn generate_all_artifacts(
        &self,
        result: &HarnessExecutionResult,
        work_context_id: String,
    ) -> Result<Vec<HarnessArtifact>> {
        let mut artifacts = vec![];
        let run_id = uuid::Uuid::new_v4().to_string();

        // 1. Patch artifact
        let patch_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id.clone(),
            tags: vec!["patch".to_string(), "code".to_string()],
            custom_fields: HashMap::new(),
        };
        let patch_content = serde_json::to_string_pretty(&result.artifacts)?;
        artifacts.push(self.generate_patch_artifact(&patch_content, patch_metadata)?);

        // 2. Report artifact
        let report_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id.clone(),
            tags: vec!["report".to_string(), "summary".to_string()],
            custom_fields: HashMap::new(),
        };
        let report_content = format!(
            "Harness Execution Report\n==========================\n\nStatus: {:?}\nCompletion: {:?}\n",
            result.completion_decision, result.completion_decision
        );
        artifacts.push(self.generate_report_artifact(&report_content, report_metadata)?);

        // 3. Trace artifact
        let trace_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id.clone(),
            tags: vec!["trace".to_string(), "execution".to_string()],
            custom_fields: HashMap::new(),
        };
        let trace_content = serde_json::to_string_pretty(&result.artifacts)?;
        artifacts.push(self.generate_trace_artifact(&trace_content, trace_metadata)?);

        // 4. Evidence artifact
        let evidence_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id.clone(),
            tags: vec!["evidence".to_string(), "validation".to_string()],
            custom_fields: HashMap::new(),
        };
        let evidence_content = serde_json::to_string_pretty(&result.artifacts)?;
        artifacts.push(self.generate_evidence_artifact(&evidence_content, evidence_metadata)?);

        // 5. Log artifact
        let log_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id.clone(),
            tags: vec!["log".to_string(), "debug".to_string()],
            custom_fields: HashMap::new(),
        };
        let log_content = format!(
            "Execution log for run {}\nFailures: {:?}",
            run_id, result.failures
        );
        artifacts.push(self.generate_log_artifact(&log_content, log_metadata)?);

        // 6. Metrics artifact
        let metrics_metadata = ArtifactMetadata {
            work_context_id: work_context_id.clone(),
            harness_run_id: run_id,
            tags: vec!["metrics".to_string(), "performance".to_string()],
            custom_fields: HashMap::new(),
        };
        let metrics_content = serde_json::json!({
            "step_count": result.step_count,
            "execution_metrics": result.execution_metrics,
        })
        .to_string();
        artifacts.push(self.generate_metrics_artifact(&metrics_content, metrics_metadata)?);

        Ok(artifacts)
    }

    fn compress(&self, content: &str, compression: CompressionType) -> Result<Vec<u8>> {
        match compression {
            CompressionType::Gzip => {
                use std::io::Write;
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(content.as_bytes())?;
                Ok(encoder.finish()?)
            }
            CompressionType::Zstd => Ok(zstd::encode_all(content.as_bytes(), 3)?),
            CompressionType::Brotli => {
                use std::io::Write;
                let mut writer = brotli::CompressorWriter::new(Vec::new(), 4096, 11, 22);
                writer.write_all(content.as_bytes())?;
                Ok(writer.into_inner())
            }
            CompressionType::None => Ok(content.as_bytes().to_vec()),
        }
    }

    pub fn decompress(&self, artifact: &HarnessArtifact) -> Result<String> {
        match artifact.compression {
            CompressionType::None => Ok(artifact.content.clone().unwrap_or_default()),
            CompressionType::Gzip => {
                use std::io::Read;
                let compressed = artifact
                    .compressed_content
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No compressed content"))?;
                let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
                let mut result = String::new();
                decoder.read_to_string(&mut result)?;
                Ok(result)
            }
            CompressionType::Zstd => {
                let compressed = artifact
                    .compressed_content
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No compressed content"))?;
                let decompressed = zstd::decode_all(compressed.as_slice())?;
                Ok(String::from_utf8(decompressed)?)
            }
            CompressionType::Brotli => {
                use std::io::Read;
                let compressed = artifact
                    .compressed_content
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No compressed content"))?;
                let mut reader = brotli::Decompressor::new(compressed.as_slice(), 4096);
                let mut result = String::new();
                reader.read_to_string(&mut result)?;
                Ok(result)
            }
        }
    }
}

pub fn generate_completion_artifact(r: &HarnessExecutionResult) -> Result<String> {
    Ok(serde_json::to_string_pretty(r)?)
}

pub fn format_artifact_summary(artifact: &HarnessArtifact) -> String {
    let compression_ratio = artifact
        .compressed_size_bytes
        .map(|c| {
            let ratio =
                (artifact.size_bytes as f64 - c as f64) / artifact.size_bytes as f64 * 100.0;
            format!(" ({:.0}% compressed)", ratio)
        })
        .unwrap_or_default();

    format!(
        "[{}] {:?} - {} bytes{}{}",
        artifact.id[..8].to_string(),
        artifact.kind,
        artifact.size_bytes,
        compression_ratio,
        if artifact.compression != CompressionType::None {
            format!(" using {:?}", artifact.compression)
        } else {
            String::new()
        }
    )
}
