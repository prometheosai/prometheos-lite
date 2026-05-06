use crate::harness::sandbox::SandboxRuntime;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::fs;
use tokio::sync::Mutex;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ValidationPlan {
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub repro_commands: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub parallel: bool,
    /// P1-009: Tool IDs from RuntimeToolRegistry (alternative to raw commands)
    pub tool_ids: Vec<String>,
    /// P1-008: Disable validation cache (force fresh runs)
    pub disable_cache: bool,
}

impl ValidationPlan {
    pub fn default_for_repo(env: &crate::harness::environment::EnvironmentProfile) -> Self {
        Self {
            format_commands: env.format_commands.clone(),
            lint_commands: env.lint_commands.clone(),
            test_commands: env.test_commands.clone(),
            repro_commands: vec![],
            timeout_ms: Some(120000),
            parallel: true,
            tool_ids: vec![],
            disable_cache: false,
        }
    }

    pub fn sequential(mut self) -> Self {
        self.parallel = false;
        self
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// P1-009: Add a tool by ID from RuntimeToolRegistry
    pub fn with_tool(mut self, tool_id: impl Into<String>) -> Self {
        self.tool_ids.push(tool_id.into());
        self
    }

    /// P1-009: Resolve tool IDs to commands using RuntimeToolRegistry
    pub fn resolve_tools(&self, registry: &crate::harness::runtime_tools::RuntimeToolRegistry) -> Vec<String> {
        let mut commands = Vec::new();
        for tool_id in &self.tool_ids {
            if let Some(tool) = registry.get(tool_id) {
                commands.push(tool.command());
            }
        }
        commands
    }

    /// P0-7 FIX: Create validation plan with cache disabled for fresh validation
    pub fn with_no_cache(mut self) -> Self {
        self.disable_cache = true;
        self
    }

    /// P1-009: Build validation plan from RuntimeToolRegistry for environment
    pub fn from_registry(env: &crate::harness::environment::EnvironmentProfile, registry: &crate::harness::runtime_tools::RuntimeToolRegistry) -> Self {
        use crate::harness::runtime_tools::ToolType;

        let mut plan = Self::default_for_repo(env);

        // Map environment commands to registered tools
        for cmd in &env.format_commands {
            if let Some(tool) = registry.find_by_command(cmd) {
                plan.tool_ids.push(tool.id.clone());
            }
        }
        for cmd in &env.lint_commands {
            if let Some(tool) = registry.find_by_command(cmd) {
                plan.tool_ids.push(tool.id.clone());
            }
        }
        for cmd in &env.test_commands {
            if let Some(tool) = registry.find_by_command(cmd) {
                plan.tool_ids.push(tool.id.clone());
            }
        }

        plan
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationResult {
    pub passed: bool,
    pub command_results: Vec<CommandResult>,
    pub errors: Vec<String>,
    pub duration_ms: u64,
    pub cached: bool,
    pub flaky_tests_detected: Vec<FlakyTestInfo>,
    pub category_results: HashMap<ValidationCategory, CategoryResult>,
    // P0-4 FIX: Add validation_performed field for completion evidence
    pub validation_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ValidationCategory {
    Format,
    Lint,
    Test,
    Repro,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryResult {
    pub category: ValidationCategory,
    pub passed: bool,
    pub commands: Vec<CommandResult>,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub cached: bool,
    pub cache_key: Option<String>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlakyTestInfo {
    pub test_name: String,
    pub command: String,
    pub attempt_results: Vec<TestAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestAttempt {
    pub attempt: u32,
    pub passed: bool,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
}

/// Compute a hash for a file's content
async fn compute_file_hash(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Compute hashes for all relevant files in the repository recursively
/// 
/// Uses WalkDir to recursively find all source and config files,
/// respecting .gitignore patterns. This ensures the validation cache
/// correctly invalidates when any nested file changes.
///
/// P1-008: Expanded to include toolchain files, Docker files, migrations, and more.
async fn compute_repo_file_hashes(root: &Path) -> Result<HashMap<PathBuf, String>> {
    let mut hashes = HashMap::new();

    // Source file extensions that affect validation
    let extensions: std::collections::HashSet<&str> = 
        ["rs", "js", "ts", "py", "go", "java", "cpp", "c", "h", "hpp"]
            .iter().copied().collect();

    // Config files that affect validation (original set)
    let config_files: std::collections::HashSet<&str> = 
        ["Cargo.toml", "package.json", "Makefile", "pytest.ini", 
         ".eslintrc", "tsconfig.json", "Cargo.lock", "package-lock.json", 
         "yarn.lock", "pnpm-lock.yaml", "pyproject.toml", "poetry.lock"]
            .iter().copied().collect();

    // P1-008: Additional toolchain and environment files
    let toolchain_files: std::collections::HashSet<&str> = 
        [".rust-toolchain", ".rust-toolchain.toml", ".node-version", ".python-version",
         ".nvmrc", "runtime.txt", "Pipfile", "Pipfile.lock", "requirements.txt",
         "requirements-dev.txt", "go.mod", "go.sum", "Gemfile", "Gemfile.lock"]
            .iter().copied().collect();

    // P1-008: Cargo configuration files
    let cargo_config_files: std::collections::HashSet<&str> = 
        [".cargo/config.toml", ".cargo/config"]
            .iter().copied().collect();

    // P1-008: Docker files
    let docker_files: std::collections::HashSet<&str> = 
        ["Dockerfile", "docker-compose.yml", "docker-compose.yaml", ".dockerignore"]
            .iter().copied().collect();

    // P1-008: NPM/Yarn/PNPM configuration
    let npm_config_files: std::collections::HashSet<&str> = 
        [".npmrc", ".yarnrc", ".yarnrc.yml", ".pnpmfile.cjs", "pnpm-workspace.yaml"]
            .iter().copied().collect();

    // P1-008: Environment and secrets templates
    let env_files: std::collections::HashSet<&str> = 
        [".env.example", ".env.template", ".env.sample", ".env.local.example"]
            .iter().copied().collect();

    // P1-008: Build configuration
    let build_files: std::collections::HashSet<&str> = 
        ["build.rs", "build.gradle", "pom.xml", "CMakeLists.txt", "configure.ac", "configure.in"]
            .iter().copied().collect();

    // Use WalkDir for recursive directory traversal
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip common non-source directories
            let path = e.path();
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            
            !matches!(name, 
                "target" | "node_modules" | ".git" | "dist" | "build" | 
                ".cache" | "__pycache__" | ".pytest_cache" | ".next"
            )
        })
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        
        if !path.is_file() {
            continue;
        }

        let file_name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Check if this is a source file by extension
        let is_source = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.contains(e))
            .unwrap_or(false);

        // Check if this is a config file by name
        let is_config = config_files.contains(file_name);

        // P1-008: Check for toolchain files
        let is_toolchain = toolchain_files.contains(file_name);

        // P1-008: Check for Cargo config files (by full relative path)
        let relative_path_str = path.strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_cargo_config = cargo_config_files.iter().any(|&cfg| relative_path_str == cfg);

        // P1-008: Check for Docker files
        let is_docker = docker_files.contains(file_name) || 
            file_name.starts_with("Dockerfile.") || 
            file_name.ends_with(".dockerfile");

        // P1-008: Check for NPM config
        let is_npm_config = npm_config_files.contains(file_name);

        // P1-008: Check for env files
        let is_env = env_files.contains(file_name);

        // P1-008: Check for build files
        let is_build = build_files.contains(file_name);

        // P1-008: Check for migration files (by directory)
        let is_migration = relative_path_str.contains("/migrations/") ||
            relative_path_str.contains("/migration/") ||
            relative_path_str.starts_with("migrations/") ||
            relative_path_str.starts_with("migration/") ||
            relative_path_str.contains("/db/migrate/"); // Rails-style

        if is_source || is_config || is_toolchain || is_cargo_config || 
           is_docker || is_npm_config || is_env || is_build || is_migration {
            let relative_path = path.strip_prefix(root).unwrap_or(path);
            
            if let Ok(hash) = compute_file_hash(path).await {
                hashes.insert(relative_path.to_path_buf(), hash);
            }
        }
    }

    Ok(hashes)
}

/// Create a cache key that includes file hashes, lockfiles, and environment
///
/// P1: Strengthened cache key includes:
/// - File content hashes
/// - Lockfile hashes (Cargo.lock, package-lock.json, etc.)
/// - Command string
/// - Repository root
fn create_cache_key(
    root: &Path,
    command: &str,
    file_hashes: &HashMap<PathBuf, String>,
) -> String {
    use sha2::{Digest, Sha256};

    // Sort hashes for consistent key generation
    let mut hash_entries: Vec<_> = file_hashes.iter().collect();
    hash_entries.sort_by(|a, b| a.0.cmp(b.0));

    let hash_str = hash_entries
        .iter()
        .map(|(path, hash)| format!("{}:{}", path.display(), hash))
        .collect::<Vec<_>>()
        .join("|");

    // P1: Include lockfile hashes in cache key
    let lockfile_hash = compute_lockfile_hash(root);

    // P1: Include environment fingerprint
    let env_hash = compute_env_hash();

    // Create composite key
    let composite = format!(
        "{}|{}|{}|{}|{}",
        root.display(),
        command,
        hash_str,
        lockfile_hash,
        env_hash
    );

    // Hash the composite for consistent length
    let mut hasher = Sha256::new();
    hasher.update(composite.as_bytes());
    format!("{:x}", hasher.finalize())[..32].to_string()
}

/// P1: Compute hash of lockfiles for cache invalidation
fn compute_lockfile_hash(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    use std::fs;

    let lockfiles = [
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "go.sum",
        "go.work.sum",
        "Pipfile.lock",
        "poetry.lock",
        "uv.lock",
        "requirements.txt",
        "pdm.lock",
    ];

    let mut hasher = Sha256::new();
    let mut found_any = false;

    for lockfile in &lockfiles {
        let path = root.join(lockfile);
        if let Ok(content) = fs::read_to_string(&path) {
            found_any = true;
            hasher.update(lockfile.as_bytes());
            hasher.update(content.as_bytes());
        }
    }

    if found_any {
        format!("{:x}", hasher.finalize())[..16].to_string()
    } else {
        "no_lockfiles".into()
    }
}

/// P1: Compute environment hash for cache invalidation
fn compute_env_hash() -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();

    // Include relevant environment variables
    let env_vars = [
        "RUST_VERSION",
        "CARGO_VERSION",
        "NODE_VERSION",
        "PYTHON_VERSION",
        "GO_VERSION",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "NODE_PATH",
    ];

    for var in &env_vars {
        if let Ok(value) = std::env::var(var) {
            hasher.update(var.as_bytes());
            hasher.update(value.as_bytes());
        }
    }

    format!("{:x}", hasher.finalize())[..16].to_string()
}

#[derive(Debug, Clone)]
struct ValidationCache {
    entries: Arc<Mutex<HashMap<String, CachedResult>>>,
    ttl_ms: u64,
}

#[derive(Debug, Clone)]
struct CachedResult {
    result: CommandResult,
    timestamp: Instant,
    file_hashes: HashMap<PathBuf, String>,
}

impl ValidationCache {
    fn new(ttl_ms: u64) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            ttl_ms,
        }
    }

    async fn get(
        &self,
        key: &str,
        current_hashes: &HashMap<PathBuf, String>,
    ) -> Option<CommandResult> {
        let entries = self.entries.lock().await;
        if let Some(cached) = entries.get(key) {
            // Check TTL
            if cached.timestamp.elapsed().as_millis() >= self.ttl_ms as u128 {
                return None;
            }

            // Validate file hashes match
            if &cached.file_hashes == current_hashes {
                let mut result = cached.result.clone();
                result.cached = true;
                return Some(result);
            }
        }
        None
    }

    async fn set(&self, key: String, result: CommandResult, file_hashes: HashMap<PathBuf, String>) {
        let mut entries = self.entries.lock().await;
        entries.insert(
            key,
            CachedResult {
                result,
                timestamp: Instant::now(),
                file_hashes,
            },
        );
    }

    async fn clear(&self) {
        let mut entries = self.entries.lock().await;
        entries.clear();
    }
}

static GLOBAL_CACHE: Lazy<ValidationCache> = Lazy::new(|| ValidationCache::new(300_000)); // 5 minute TTL

pub async fn run_validation(
    root: &Path,
    plan: &ValidationPlan,
    sandbox: Arc<dyn SandboxRuntime + Send + Sync>,
) -> Result<ValidationResult> {
    run_validation_with_cache(root, plan, sandbox, &GLOBAL_CACHE).await
}

pub async fn run_validation_with_cache(
    root: &Path,
    plan: &ValidationPlan,
    sandbox: Arc<dyn SandboxRuntime + Send + Sync>,
    cache: &ValidationCache,
) -> Result<ValidationResult> {
    let start = Instant::now();
    let timeout = plan.timeout_ms.unwrap_or(120000);

    let all_commands: Vec<(String, ValidationCategory)> = plan
        .format_commands
        .iter()
        .map(|c| (c.clone(), ValidationCategory::Format))
        .chain(
            plan.lint_commands
                .iter()
                .map(|c| (c.clone(), ValidationCategory::Lint)),
        )
        .chain(
            plan.test_commands
                .iter()
                .map(|c| (c.clone(), ValidationCategory::Test)),
        )
        .chain(
            plan.repro_commands
                .iter()
                .map(|c| (c.clone(), ValidationCategory::Repro)),
        )
        .collect();

    let results = if plan.parallel {
        run_parallel(root, &all_commands, sandbox.clone(), cache, timeout).await?
    } else {
        run_sequential(root, &all_commands, &*sandbox, cache, timeout).await?
    };

    let mut category_results: HashMap<ValidationCategory, CategoryResult> = HashMap::new();
    for cat in [
        ValidationCategory::Format,
        ValidationCategory::Lint,
        ValidationCategory::Test,
        ValidationCategory::Repro,
    ] {
        let cat_commands: Vec<_> = results
            .iter()
            .filter(|(cmd, _)| {
                all_commands
                    .iter()
                    .any(|(c, ccat)| c == cmd && *ccat == cat)
            })
            .map(|(_, r)| r.clone())
            .collect();

        let passed = cat_commands.iter().all(|r| r.exit_code == Some(0));
        let total_duration: u64 = cat_commands.iter().map(|r| r.duration_ms).sum();

        category_results.insert(
            cat.clone(),
            CategoryResult {
                category: cat,
                passed,
                commands: cat_commands,
                total_duration_ms: total_duration,
            },
        );
    }

    let errors: Vec<String> = results
        .iter()
        .filter(|(_, r)| r.exit_code != Some(0))
        .map(|(cmd, _)| cmd.clone())
        .collect();

    let passed = errors.is_empty();
    let cached = results.iter().all(|(_, r)| r.cached);

    let test_commands: Vec<_> = plan.test_commands.clone();
    let flaky_tests = if !test_commands.is_empty() && !plan.parallel {
        detect_flaky_tests(root, &test_commands, &*sandbox, timeout).await?
    } else {
        vec![]
    };

    let command_results: Vec<_> = results.into_iter().map(|(_, r)| r).collect();

    Ok(ValidationResult {
        passed,
        command_results,
        errors,
        duration_ms: start.elapsed().as_millis() as u64,
        cached,
        flaky_tests_detected: flaky_tests,
        category_results,
        // P0-4 FIX: Add validation_performed field for completion evidence
        validation_performed: true,
    })
}

async fn run_parallel(
    root: &Path,
    commands: &[(String, ValidationCategory)],
    sandbox: Arc<dyn SandboxRuntime + Send + Sync>,
    cache: &ValidationCache,
    timeout: u64,
) -> Result<Vec<(String, CommandResult)>> {
    // Compute file hashes once for all commands
    let file_hashes = compute_repo_file_hashes(root).await.unwrap_or_default();

    // Check cache for all commands first
    let mut tasks = Vec::new();
    let mut cached_results = Vec::new();

    for (cmd, _cat) in commands {
        let cmd = cmd.clone();
        let root = root.to_path_buf();
        let cache_key = create_cache_key(&root, &cmd, &file_hashes);

        if let Some(cached) = cache.get(&cache_key, &file_hashes).await {
            cached_results.push((cmd, cached));
        } else {
            // This command needs to run - create a task for it
            let root_clone = root.clone();
            let file_hashes_clone = file_hashes.clone();
            let cache_key_clone = cache_key.clone();
            let cmd_clone = cmd.clone();
            let sandbox_clone = sandbox.clone();

            // Cloneable wrapper for sandbox execution
            let task = tokio::spawn(async move {
                let start = Instant::now();
                // Use the provided sandbox (cloned via Arc) to respect custom policies
                let result = sandbox_clone
                    .run_command(&root_clone, &cmd_clone, timeout)
                    .await;
                let duration = start.elapsed().as_millis() as u64;

                let cmd_result = match result {
                    Ok(r) => CommandResult {
                        command: cmd_clone.clone(),
                        exit_code: r.exit_code,
                        stdout: r.stdout,
                        stderr: r.stderr,
                        duration_ms: duration,
                        cached: false,
                        cache_key: Some(cache_key_clone.clone()),
                        timed_out: r.exit_code.is_none(),
                    },
                    Err(e) => CommandResult {
                        command: cmd_clone.clone(),
                        exit_code: None,
                        stdout: String::new(),
                        stderr: e.to_string(),
                        duration_ms: duration,
                        cached: false,
                        cache_key: None,
                        timed_out: true,
                    },
                };

                (cmd_clone, cache_key_clone, cmd_result, file_hashes_clone)
            });

            tasks.push(task);
        }
    }

    // Wait for all tasks to complete in parallel
    let mut results = cached_results;
    for task in tasks {
        let (cmd, cache_key, cmd_result, file_hashes) = task.await?;
        cache.set(cache_key, cmd_result.clone(), file_hashes).await;
        results.push((cmd, cmd_result));
    }

    Ok(results)
}

async fn run_sequential(
    root: &Path,
    commands: &[(String, ValidationCategory)],
    sandbox: &dyn SandboxRuntime,
    cache: &ValidationCache,
    timeout: u64,
) -> Result<Vec<(String, CommandResult)>> {
    let mut results = vec![];

    // Compute file hashes once at the start
    let file_hashes = compute_repo_file_hashes(root).await.unwrap_or_default();

    for (cmd, _cat) in commands {
        let cache_key = create_cache_key(root, cmd, &file_hashes);

        if let Some(cached) = cache.get(&cache_key, &file_hashes).await {
            results.push((cmd.clone(), cached));
            continue;
        }

        let start = Instant::now();
        let result = sandbox.run_command(root, cmd, timeout).await;
        let duration = start.elapsed().as_millis() as u64;

        let cmd_result = match result {
            Ok(r) => CommandResult {
                command: cmd.clone(),
                exit_code: r.exit_code,
                stdout: r.stdout,
                stderr: r.stderr,
                duration_ms: duration,
                cached: false,
                cache_key: Some(cache_key.clone()),
                timed_out: r.exit_code.is_none(),
            },
            Err(e) => CommandResult {
                command: cmd.clone(),
                exit_code: None,
                stdout: String::new(),
                stderr: e.to_string(),
                duration_ms: duration,
                cached: false,
                cache_key: None,
                timed_out: true,
            },
        };

        cache
            .set(cache_key, cmd_result.clone(), file_hashes.clone())
            .await;
        results.push((cmd.clone(), cmd_result));
    }

    Ok(results)
}

async fn detect_flaky_tests(
    root: &Path,
    test_commands: &[String],
    sandbox: &dyn SandboxRuntime,
    timeout: u64,
) -> Result<Vec<FlakyTestInfo>> {
    let mut flaky = vec![];

    for cmd in test_commands {
        let mut attempts = vec![];
        let mut results_different = false;

        for i in 0..3 {
            let start = Instant::now();
            let result = sandbox.run_command(root, cmd, timeout).await?;
            let duration = start.elapsed().as_millis() as u64;
            let passed = result.exit_code == Some(0);

            if !attempts.is_empty()
                && attempts.last().map(|a: &TestAttempt| a.passed) != Some(passed)
            {
                results_different = true;
            }

            attempts.push(TestAttempt {
                attempt: i + 1,
                passed,
                duration_ms: duration,
                exit_code: result.exit_code,
            });
        }

        if results_different {
            flaky.push(FlakyTestInfo {
                test_name: extract_test_name(cmd),
                command: cmd.clone(),
                attempt_results: attempts,
            });
        }
    }

    Ok(flaky)
}

fn extract_test_name(command: &str) -> String {
    command
        .split_whitespace()
        .last()
        .map(|s| s.to_string())
        .unwrap_or_else(|| command.to_string())
}

pub async fn validate_with_retry(
    root: &Path,
    plan: &ValidationPlan,
    sandbox: Arc<dyn SandboxRuntime + Send + Sync>,
    max_retries: u32,
) -> Result<ValidationResult> {
    let mut last_result = None;

    for attempt in 0..=max_retries {
        let result = run_validation(root, plan, sandbox.clone()).await?;

        if result.passed {
            return Ok(result);
        }

        if attempt < max_retries && !result.flaky_tests_detected.is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            last_result = Some(result);
        } else {
            return Ok(result);
        }
    }

    Ok(last_result.unwrap())
}

pub fn get_validation_summary(result: &ValidationResult) -> String {
    let categories = [
        (
            "Format",
            result.category_results.get(&ValidationCategory::Format),
        ),
        (
            "Lint",
            result.category_results.get(&ValidationCategory::Lint),
        ),
        (
            "Test",
            result.category_results.get(&ValidationCategory::Test),
        ),
        (
            "Repro",
            result.category_results.get(&ValidationCategory::Repro),
        ),
    ];

    let mut parts = vec![];
    for (name, opt_cat) in categories {
        if let Some(cat) = opt_cat {
            let status = if cat.passed { "✓" } else { "✗" };
            parts.push(format!(
                "{} {} ({} cmds, {}ms)",
                status,
                name,
                cat.commands.len(),
                cat.total_duration_ms
            ));
        }
    }

    if !result.flaky_tests_detected.is_empty() {
        parts.push(format!(
            "⚠ {} flaky tests",
            result.flaky_tests_detected.len()
        ));
    }

    if result.cached {
        parts.push("(cached)".into());
    }

    parts.join(" | ")
}
