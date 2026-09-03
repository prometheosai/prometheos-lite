//! Configuration loader
//!
//! Versioned and fail-closed. The loader enforces the
//! `CONFIG_SCHEMA_VERSION` declared below: a config file whose
//! `configVersion` field has a different major is rejected with an
//! actionable error that names the field and the supported range.

use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};

use super::types::AppConfig;

pub const DEFAULT_CONFIG_PATH: &str = "prometheos.config.json";

/// The schema version the running Lite binary understands. The loader
/// rejects any config whose `configVersion` major differs from this.
/// Bumping this major is a breaking change for existing config files.
pub const CONFIG_SCHEMA_VERSION: &str = "1.0.0";

/// Parse a `MAJOR.MINOR.PATCH` semver string and return the major
/// component. Returns `Err` for malformed input. Used by the loader
/// to compare the major of the config's `configVersion` against
/// `CONFIG_SCHEMA_VERSION`.
fn semver_major(version: &str) -> Result<u64> {
    let mut parts = version.split('.');
    let major: u64 = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty semver"))?
        .parse()
        .with_context(|| format!("invalid semver major in {version:?}"))?;
    Ok(major)
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        Self::load_from(DEFAULT_CONFIG_PATH)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let mut config: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        // Fail-closed version check. The major component of the
        // config's `configVersion` must match the major of
        // `CONFIG_SCHEMA_VERSION`; minor/patch differences are
        // forward-compatible. Empty / missing / malformed
        // `configVersion` is rejected with a message that names
        // the field and the supported range, so the operator knows
        // exactly what to add to their config.
        let supported_major = semver_major(CONFIG_SCHEMA_VERSION)?;
        let config_major = semver_major(&config.config_version).map_err(|_| {
            anyhow::anyhow!(
                "config file {} has an invalid or missing `configVersion` field ({:?}); expected major {} (= CONFIG_SCHEMA_VERSION = {:?})",
                path.display(),
                config.config_version,
                supported_major,
                CONFIG_SCHEMA_VERSION,
            )
        })?;
        if config_major != supported_major {
            bail!(
                "config file {} has configVersion={:?} (major {config_major}), which is incompatible with the running Lite binary's supported major {supported_major} (CONFIG_SCHEMA_VERSION={:?}); please update the config or run a matching Lite version",
                path.display(),
                config.config_version,
                CONFIG_SCHEMA_VERSION,
            );
        }

        if let Ok(base_url) = env::var("PROMETHEOS_BASE_URL") {
            config.base_url = base_url;
        }

        if let Ok(model) = env::var("PROMETHEOS_MODEL") {
            config.model = model;
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_default_version_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(
            &path,
            format!(r#"{{"configVersion":"{CONFIG_SCHEMA_VERSION}"}}"#),
        )
        .unwrap();
        let cfg = AppConfig::load_from(&path).unwrap();
        assert_eq!(cfg.config_version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn load_from_missing_config_version_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{}"#).unwrap();
        // Use the chain formatter so the inner serde error (which
        // names the missing field) is visible alongside the
        // context wrap.
        let result = AppConfig::load_from(&path);
        let err = match result {
            Ok(_) => panic!("expected missing configVersion to be rejected"),
            Err(e) => format!("{e:#}"),
        };
        // The error chain must name the field. (The serde parse
        // error names the field name; the supported major/version
        // are named by the loader's own message when the field IS
        // present but mismatched. Missing-field parse errors come
        // from serde before the loader's check runs.)
        assert!(err.contains("configVersion"), "got: {err}");
    }

    #[test]
    fn load_from_unknown_major_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"configVersion":"2.0.0"}"#).unwrap();
        let err = AppConfig::load_from(&path).unwrap_err().to_string();
        assert!(err.contains("incompatible"), "got: {err}");
        assert!(err.contains("2.0.0"), "got: {err}");
        assert!(err.contains(CONFIG_SCHEMA_VERSION), "got: {err}");
    }

    #[test]
    fn load_from_minor_bump_accepts() {
        // A minor/patch bump is forward-compatible: the config is
        // accepted; new fields default-fill.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"configVersion":"1.1.0"}"#).unwrap();
        let cfg = AppConfig::load_from(&path).unwrap();
        assert_eq!(cfg.config_version, "1.1.0");
    }

    #[test]
    fn load_from_unknown_field_fails() {
        // `deny_unknown_fields` at the top level catches typos and
        // obsolete keys. The error must mention the offending key.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(
            &path,
            format!(r#"{{"configVersion":"{CONFIG_SCHEMA_VERSION}","unknownKey":1}}"#),
        )
        .unwrap();
        let result = AppConfig::load_from(&path);
        let err = match result {
            Ok(_) => panic!("expected deny_unknown_fields to reject unknownKey, got Ok"),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            err.contains("unknownKey") || err.contains("unknown field"),
            "got: {err}"
        );
    }
}
