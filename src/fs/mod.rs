//! File parsing and writing utilities.

use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub path: String,
    pub content: String,
    pub language: String,
}

pub struct FileParser;

impl FileParser {
    pub fn parse_files(output: &str) -> Result<Vec<ParsedFile>> {
        let mut files = Vec::new();

        let header_regex = Regex::new(r"###\s+([^\n]+)")?;
        let code_block_regex = Regex::new(r"```(\w+)?\n([\s\S]*?)```")?;

        let mut current_path: Option<String> = None;

        let mut pos = 0;
        while pos < output.len() {
            if let Some(header_caps) = header_regex.captures(&output[pos..]) {
                let header_end = pos + header_caps.get(0).unwrap().end();

                if let Some(path_match) = header_caps.get(1) {
                    current_path = Some(path_match.as_str().to_string());
                }

                pos = header_end;

                if let Some(code_caps) = code_block_regex.captures(&output[pos..]) {
                    let code_end = pos + code_caps.get(0).unwrap().end();

                    if let Some(path) = current_path.take() {
                        let language = code_caps
                            .get(1)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_else(|| "text".to_string());
                        let content = code_caps
                            .get(2)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default();

                        files.push(ParsedFile {
                            path,
                            content,
                            language,
                        });
                    }

                    pos = code_end;
                }
            } else {
                break;
            }
        }

        Ok(files)
    }

    pub fn extract_file_blocks(output: &str) -> Result<Vec<(String, String, String)>> {
        let files = Self::parse_files(output)?;
        Ok(files
            .into_iter()
            .map(|f| (f.path, f.content, f.language))
            .collect())
    }
}

pub struct FileWriter {
    output_dir: PathBuf,
}

impl FileWriter {
    pub fn new() -> Result<Self> {
        Self::with_directory("prometheos-output")
    }

    pub fn with_directory(dir: impl AsRef<Path>) -> Result<Self> {
        let output_dir = PathBuf::from(dir.as_ref());

        if !output_dir.exists() {
            fs::create_dir_all(&output_dir).with_context(|| {
                format!(
                    "failed to create output directory: {}",
                    output_dir.display()
                )
            })?;
        }

        Ok(Self { output_dir })
    }

    pub fn write_file(&self, parsed_file: &ParsedFile) -> Result<PathBuf> {
        let file_path = self.output_dir.join(&parsed_file.path);

        if let Some(parent) = file_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory: {}", parent.display())
            })?;
        }

        if file_path.exists() {
            fs::write(&file_path, &parsed_file.content)
                .with_context(|| format!("failed to overwrite file: {}", file_path.display()))?;
        } else {
            fs::write(&file_path, &parsed_file.content)
                .with_context(|| format!("failed to write file: {}", file_path.display()))?;
        }

        Ok(file_path)
    }

    pub fn write_files(&self, files: &[ParsedFile]) -> Result<Vec<PathBuf>> {
        let mut written_paths = Vec::new();

        for file in files {
            let path = self.write_file(file)?;
            written_paths.push(path);
        }

        Ok(written_paths)
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

impl Default for FileWriter {
    fn default() -> Self {
        Self::new().expect("failed to create default file writer")
    }
}
