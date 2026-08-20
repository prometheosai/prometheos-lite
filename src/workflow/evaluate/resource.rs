//! Bounded validation resource policy.
//!
//! Defines the typed, validated resource policy for isolated validation
//! commands and the classification strings used when a limit is exceeded.
//!
//! # Enforcement honesty
//!
//! - **Timeout** and **output cap** are enforced deterministically (the
//!   validation process tree is terminated and the run fails closed).
//! - **Disk pressure** is enforced as a preflight/execution boundary in
//!   [`crate::workflow::evaluate::preflight`].
//! - **Memory** and **CPU** limits are recorded, validated, and surfaced as
//!   classifications; OS-enforced process-tree capping (POSIX `setrlimit` /
//!   `RLIMIT_CPU`, Windows Job Objects) and measured-budget termination are
//!   layered on this same contract in a follow-up change. Where the runtime
//!   only monitors, it terminates-and-records rather than claiming a hard
//!   sandbox.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The kind of resource whose limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLimitKind {
    /// Wall-clock CPU time budget.
    Cpu,
    /// Process-tree memory budget.
    Memory,
    /// Available disk space below the required reserve.
    Disk,
    /// Wall-clock validation timeout.
    Timeout,
    /// Total captured validation output exceeded the cap.
    Output,
}

impl ResourceLimitKind {
    /// Human-readable label used in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceLimitKind::Cpu => "cpu",
            ResourceLimitKind::Memory => "memory",
            ResourceLimitKind::Disk => "disk",
            ResourceLimitKind::Timeout => "timeout",
            ResourceLimitKind::Output => "output",
        }
    }
}

/// Classification string recorded when the validation wall-clock timeout is hit.
pub const CLASSIFICATION_TIMEOUT: &str = "resource_timeout";
/// Classification string recorded when captured output exceeds the cap.
pub const CLASSIFICATION_OUTPUT: &str = "resource_output_limit";
/// Classification string recorded when the CPU budget is exceeded.
pub const CLASSIFICATION_CPU: &str = "resource_cpu_exhausted";
/// Classification string recorded when the memory budget is exceeded.
pub const CLASSIFICATION_MEMORY: &str = "resource_memory_exhausted";
/// Classification string recorded when disk pressure is detected.
pub const CLASSIFICATION_DISK: &str = "resource_disk_exhausted";

/// Return the classification string for a resource kind, used as the
/// `failure_classification` in the evidence bundle. All resource failures map
/// to the existing `InfraBlocked` terminal state (see
/// [`crate::workflow::evaluate::validation::failure_to_terminal_state`]).
pub fn classification_for_resource(kind: ResourceLimitKind) -> &'static str {
    match kind {
        ResourceLimitKind::Cpu => CLASSIFICATION_CPU,
        ResourceLimitKind::Memory => CLASSIFICATION_MEMORY,
        ResourceLimitKind::Disk => CLASSIFICATION_DISK,
        ResourceLimitKind::Timeout => CLASSIFICATION_TIMEOUT,
        ResourceLimitKind::Output => CLASSIFICATION_OUTPUT,
    }
}

/// Typed, validated resource policy for isolated validation.
///
/// Every limit is optional; `None` means "no limit". When a limit is set it
/// must be strictly positive and within representable bounds. `TaskManifest`
/// also carries a `min_disk_bytes`; the orchestrator resolves any conflict by
/// taking the *larger* effective reserve so neither policy can silently weaken
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum wall-clock duration for the validation command.
    pub validation_timeout: Option<Duration>,
    /// Maximum total captured stdout+stderr bytes before termination.
    pub max_output_bytes: Option<u64>,
    /// Maximum process-tree memory budget.
    pub max_memory_bytes: Option<u64>,
    /// Maximum CPU time budget for the validation process tree.
    pub max_cpu_time: Option<Duration>,
    /// Minimum free disk bytes required before/during execution.
    pub min_free_disk_bytes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            validation_timeout: Some(Duration::from_secs(300)),
            max_output_bytes: Some(8 * 1024 * 1024),
            max_memory_bytes: None,
            max_cpu_time: None,
            min_free_disk_bytes: Some(256 * 1024 * 1024),
        }
    }
}

impl ResourceLimits {
    /// Construct an unbounded policy (all limits disabled). Intended for tests
    /// and callers that explicitly opt out of bounds.
    pub fn unbounded() -> Self {
        ResourceLimits {
            validation_timeout: None,
            max_output_bytes: None,
            max_memory_bytes: None,
            max_cpu_time: None,
            min_free_disk_bytes: None,
        }
    }

    /// Fail closed on absurd or invalid configuration.
    pub fn validate(&self) -> Result<()> {
        if let Some(t) = self.validation_timeout
            && t.is_zero()
        {
            bail!("validation_timeout must be positive");
        }
        if let Some(n) = self.max_output_bytes
            && n == 0
        {
            bail!("max_output_bytes must be positive");
        }
        if let Some(n) = self.max_memory_bytes
            && n == 0
        {
            bail!("max_memory_bytes must be positive");
        }
        if let Some(t) = self.max_cpu_time
            && t.is_zero()
        {
            bail!("max_cpu_time must be positive");
        }
        if let Some(n) = self.min_free_disk_bytes
            && n == 0
        {
            bail!("min_free_disk_bytes must be positive");
        }
        Ok(())
    }

    /// Resolve the effective disk reserve from this policy and the manifest's
    /// own `min_disk_bytes`, taking the larger so neither can weaken the other.
    pub fn effective_min_free_disk_bytes(self, manifest_min: Option<u64>) -> Option<u64> {
        match (self.min_free_disk_bytes, manifest_min) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Merge the manifest's `min_disk_bytes` into this policy (taking the larger
    /// reserve) and return the resolved policy. Convenience for callers that hold
    /// a `TaskManifest`.
    pub fn with_manifest_disk(mut self, manifest_min: u64) -> Self {
        self.min_free_disk_bytes = self.effective_min_free_disk_bytes(Some(manifest_min));
        self
    }

    /// Build the effective policy: start from the safe [`Default`] production
    /// limits and override individual bounds from environment variables when
    /// present. Defaults remain fail-safe even if no variable is set, and an
    /// operator can tighten or relax any single bound without code changes.
    ///
    /// Malformed environment values fail closed (they are rejected, not silently
    /// replaced by the default) so a typo in configuration cannot silently weaken
    /// enforcement.
    pub fn from_environment() -> Result<Self> {
        let mut limits = ResourceLimits::default();
        if let Ok(v) = std::env::var("PROMETHEOS_VALIDATION_TIMEOUT_SECS") {
            let secs = v
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid PROMETHEOS_VALIDATION_TIMEOUT_SECS: {v:?}"))?;
            limits.validation_timeout = Some(Duration::from_secs(secs));
        }
        if let Ok(v) = std::env::var("PROMETHEOS_MAX_OUTPUT_BYTES") {
            let n = v
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid PROMETHEOS_MAX_OUTPUT_BYTES: {v:?}"))?;
            limits.max_output_bytes = Some(n);
        }
        if let Ok(v) = std::env::var("PROMETHEOS_MAX_MEMORY_BYTES") {
            let n = v
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid PROMETHEOS_MAX_MEMORY_BYTES: {v:?}"))?;
            limits.max_memory_bytes = Some(n);
        }
        if let Ok(v) = std::env::var("PROMETHEOS_MAX_CPU_SECS") {
            let secs = v
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid PROMETHEOS_MAX_CPU_SECS: {v:?}"))?;
            limits.max_cpu_time = Some(Duration::from_secs(secs));
        }
        if let Ok(v) = std::env::var("PROMETHEOS_MIN_FREE_DISK_BYTES") {
            let n = v
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid PROMETHEOS_MIN_FREE_DISK_BYTES: {v:?}"))?;
            limits.min_free_disk_bytes = Some(n);
        }
        limits
            .validate()
            .context("environment resource limits invalid")?;
        Ok(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_valid_and_bounded() {
        let l = ResourceLimits::default();
        assert!(l.validate().is_ok());
        assert!(l.validation_timeout.is_some());
        assert!(l.max_output_bytes.is_some());
    }

    #[test]
    fn unbounded_is_valid() {
        assert!(ResourceLimits::unbounded().validate().is_ok());
    }

    #[test]
    fn rejects_zero_limits() {
        let l = ResourceLimits {
            validation_timeout: Some(Duration::ZERO),
            ..ResourceLimits::unbounded()
        };
        assert!(l.validate().is_err());
        let l2 = ResourceLimits {
            max_output_bytes: Some(0),
            ..ResourceLimits::unbounded()
        };
        assert!(l2.validate().is_err());
    }

    #[test]
    fn effective_disk_takes_larger() {
        let l = ResourceLimits {
            min_free_disk_bytes: Some(100),
            ..ResourceLimits::unbounded()
        };
        assert_eq!(l.effective_min_free_disk_bytes(Some(50)), Some(100));
        assert_eq!(l.effective_min_free_disk_bytes(Some(200)), Some(200));
        assert_eq!(l.effective_min_free_disk_bytes(None), Some(100));
    }

    #[test]
    fn classification_strings() {
        assert_eq!(
            classification_for_resource(ResourceLimitKind::Timeout),
            CLASSIFICATION_TIMEOUT
        );
        assert_eq!(
            classification_for_resource(ResourceLimitKind::Output),
            CLASSIFICATION_OUTPUT
        );
        assert_eq!(
            classification_for_resource(ResourceLimitKind::Cpu),
            CLASSIFICATION_CPU
        );
        assert_eq!(
            classification_for_resource(ResourceLimitKind::Memory),
            CLASSIFICATION_MEMORY
        );
        assert_eq!(
            classification_for_resource(ResourceLimitKind::Disk),
            CLASSIFICATION_DISK
        );
    }
}
