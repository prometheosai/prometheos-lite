use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EnvironmentProfile {
    pub languages: Vec<String>,
    pub package_manager: Option<String>,
    pub build_commands: Vec<String>,
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub type_check_commands: Vec<String>,
    pub services: Vec<ServiceDependency>,
    pub detected_files: Vec<String>,
    pub ci_config: Option<CiConfig>,
    pub container_config: Option<ContainerConfig>,
    pub environment_variables: HashMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceDependency {
    pub name: String,
    pub required: bool,
    pub startup_command: Option<String>,
    pub health_check: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CiConfig {
    pub provider: String,
    pub config_file: String,
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerConfig {
    pub has_docker: bool,
    pub has_docker_compose: bool,
    pub has_kubernetes: bool,
    pub services: Vec<String>,
}

pub async fn fingerprint_environment(root: &Path) -> Result<EnvironmentProfile> {
    let mut profile = EnvironmentProfile::default();

    detect_rust(root, &mut profile)?;
    detect_nodejs(root, &mut profile)?;
    detect_python(root, &mut profile)?;
    detect_java(root, &mut profile)?;
    detect_go(root, &mut profile)?;
    detect_php(root, &mut profile)?;
    detect_ruby(root, &mut profile)?;
    detect_docker(root, &mut profile)?;
    detect_ci(root, &mut profile)?;
    detect_env_files(root, &mut profile)?;

    if profile.languages.is_empty() {
        profile.languages.push("unknown".into());
        profile
            .warnings
            .push("No recognized project type detected".into());
    }

    deduplicate_and_sort(&mut profile.languages);
    deduplicate_and_sort(&mut profile.build_commands);
    deduplicate_and_sort(&mut profile.format_commands);
    deduplicate_and_sort(&mut profile.lint_commands);
    deduplicate_and_sort(&mut profile.test_commands);
    deduplicate_and_sort(&mut profile.type_check_commands);

    Ok(profile)
}

fn detect_rust(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    if root.join("Cargo.toml").exists() {
        profile.languages.push("rust".into());
        profile.detected_files.push("Cargo.toml".into());
        profile.package_manager = Some("cargo".into());

        profile.build_commands.push("cargo build".into());
        profile.build_commands.push("cargo build --release".into());
        profile.format_commands.push("cargo fmt --check".into());
        profile
            .lint_commands
            .push("cargo clippy -- -D warnings".into());
        profile.test_commands.push("cargo test".into());
        profile
            .test_commands
            .push("cargo test --all-features".into());

        if root.join("Cargo.lock").exists() {
            profile.detected_files.push("Cargo.lock".into());
        }

        if root.join("rust-toolchain.toml").exists() {
            profile.detected_files.push("rust-toolchain.toml".into());
        }
    }

    Ok(())
}

fn detect_nodejs(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    if root.join("package.json").exists() {
        profile.languages.push("javascript".into());
        profile.detected_files.push("package.json".into());

        let package_json = fs::read_to_string(root.join("package.json"))?;
        let package: serde_json::Value = serde_json::from_str(&package_json)?;

        let has_typescript = root.join("tsconfig.json").exists();
        if has_typescript {
            profile.languages.push("typescript".into());
            profile.detected_files.push("tsconfig.json".into());
            profile.type_check_commands.push("tsc --noEmit".into());
        }

        let scripts = package.get("scripts").and_then(|s| s.as_object());

        let mut detected_pm = None;

        if root.join("pnpm-lock.yaml").exists() {
            detected_pm = Some("pnpm".to_string());
            profile.detected_files.push("pnpm-lock.yaml".into());
        } else if root.join("yarn.lock").exists() {
            detected_pm = Some("yarn".to_string());
            profile.detected_files.push("yarn.lock".into());
        } else if root.join("package-lock.json").exists() {
            detected_pm = Some("npm".to_string());
            profile.detected_files.push("package-lock.json".into());
        } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
            detected_pm = Some("bun".to_string());
            profile.detected_files.push("bun.lock".into());
        } else {
            detected_pm = Some("npm".to_string());
        }

        profile.package_manager = detected_pm.clone();
        let pm = detected_pm.as_deref().unwrap_or("npm");

        if let Some(scripts) = scripts {
            if scripts.contains_key("test") {
                profile.test_commands.push(format!("{} test", pm));
            }
            if scripts.contains_key("build") {
                profile.build_commands.push(format!("{} run build", pm));
            }
            if scripts.contains_key("lint") || scripts.contains_key("eslint") {
                profile.lint_commands.push(format!("{} run lint", pm));
            }
            if scripts.contains_key("format") || scripts.contains_key("prettier") {
                profile.format_commands.push(format!("{} run format", pm));
            }
            if scripts.contains_key("type-check") || scripts.contains_key("tsc") {
                profile
                    .type_check_commands
                    .push(format!("{} run type-check", pm));
            }
        }

        if profile.test_commands.is_empty() {
            profile.test_commands.push(format!("{} test", pm));
        }
        if profile.build_commands.is_empty() {
            profile.build_commands.push(format!("{} run build", pm));
        }

        if has_typescript {
            if profile.lint_commands.is_empty() {
                profile.lint_commands.push(format!("{} run lint", pm));
            }
            if profile.format_commands.is_empty() {
                profile
                    .format_commands
                    .push("npx prettier --check .".into());
            }
        }

        if root.join(".eslintrc.js").exists() || root.join(".eslintrc.json").exists() {
            profile.detected_files.push(".eslintrc.*".into());
        }
        if root.join(".prettierrc").exists() || root.join("prettier.config.js").exists() {
            profile.detected_files.push("prettier config".into());
        }

        if root.join("next.config.js").exists() || root.join("next.config.ts").exists() {
            profile.detected_files.push("next.config.*".into());
            profile
                .warnings
                .push("Next.js detected - ensure NODE_ENV is set correctly".into());
        }

        if root.join("vite.config.ts").exists() || root.join("vite.config.js").exists() {
            profile.detected_files.push("vite.config.*".into());
        }
    }

    Ok(())
}

fn detect_python(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_requirements = root.join("requirements.txt").exists();
    let has_pyproject = root.join("pyproject.toml").exists();
    let has_setup_py = root.join("setup.py").exists();
    let has_pipfile = root.join("Pipfile").exists();
    let has_poetry = root.join("poetry.lock").exists();

    if has_requirements || has_pyproject || has_setup_py || has_pipfile || has_poetry {
        profile.languages.push("python".into());

        if has_requirements {
            profile.detected_files.push("requirements.txt".into());
            profile.package_manager = Some("pip".into());
        }
        if has_pyproject {
            profile.detected_files.push("pyproject.toml".into());
        }
        if has_setup_py {
            profile.detected_files.push("setup.py".into());
        }
        if has_pipfile {
            profile.detected_files.push("Pipfile".into());
            profile.package_manager = Some("pipenv".into());
        }
        if has_poetry {
            profile.detected_files.push("poetry.lock".into());
            profile.package_manager = Some("poetry".into());
        }

        let pm = profile.package_manager.as_deref().unwrap_or("pip");

        match pm {
            "poetry" => {
                profile.build_commands.push("poetry build".into());
                profile.test_commands.push("poetry run pytest".into());
                profile.lint_commands.push("poetry run ruff check .".into());
                profile
                    .format_commands
                    .push("poetry run ruff format --check .".into());
                profile.type_check_commands.push("poetry run mypy .".into());
            }
            "pipenv" => {
                profile.test_commands.push("pipenv run pytest".into());
                profile.lint_commands.push("pipenv run flake8".into());
            }
            _ => {
                profile.test_commands.push("python -m pytest".into());
                profile
                    .test_commands
                    .push("python -m unittest discover".into());
                profile.lint_commands.push("flake8".into());
                profile.lint_commands.push("pylint".into());
                profile.format_commands.push("black --check .".into());
                profile.type_check_commands.push("mypy .".into());
            }
        }

        if root.join("tox.ini").exists() {
            profile.detected_files.push("tox.ini".into());
            profile.test_commands.push("tox".into());
        }

        if root.join("pytest.ini").exists() || root.join("setup.cfg").exists() {
            profile.detected_files.push("pytest config".into());
        }

        if root.join("conda.yml").exists() || root.join("environment.yml").exists() {
            profile.detected_files.push("conda environment".into());
            profile
                .warnings
                .push("Conda environment detected - ensure environment is activated".into());
        }
    }

    Ok(())
}

fn detect_java(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_pom = root.join("pom.xml").exists();
    let has_gradle = root.join("build.gradle").exists();
    let has_gradle_kts = root.join("build.gradle.kts").exists();

    if has_pom || has_gradle || has_gradle_kts {
        profile.languages.push("java".into());

        if has_pom {
            profile.detected_files.push("pom.xml".into());
            profile.package_manager = Some("maven".into());
            profile.build_commands.push("mvn compile".into());
            profile
                .build_commands
                .push("mvn package -DskipTests".into());
            profile.test_commands.push("mvn test".into());
            profile.lint_commands.push("mvn checkstyle:check".into());
            profile.format_commands.push("mvn spotless:check".into());
        }

        if has_gradle || has_gradle_kts {
            profile.detected_files.push("build.gradle*".into());
            profile.package_manager = Some("gradle".into());
            profile
                .build_commands
                .push("./gradlew build -x test".into());
            profile.test_commands.push("./gradlew test".into());
            profile
                .lint_commands
                .push("./gradlew checkstyleMain".into());
            profile
                .format_commands
                .push("./gradlew spotlessCheck".into());

            if root.join("gradlew").exists() {
                profile.detected_files.push("gradlew".into());
            }
        }

        if root.join(".java-version").exists() {
            profile.detected_files.push(".java-version".into());
        }
    }

    if root.join("build.sbt").exists() {
        profile.languages.push("scala".into());
        profile.detected_files.push("build.sbt".into());
        profile.package_manager = Some("sbt".into());
        profile.build_commands.push("sbt compile".into());
        profile.test_commands.push("sbt test".into());
    }

    if root.join("build.sc").exists() {
        profile.detected_files.push("build.sc".into());
    }

    Ok(())
}

fn detect_go(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_go_mod = root.join("go.mod").exists();
    let has_go_sum = root.join("go.sum").exists();

    if has_go_mod {
        profile.languages.push("go".into());
        profile.detected_files.push("go.mod".into());
        profile.package_manager = Some("go mod".into());

        if has_go_sum {
            profile.detected_files.push("go.sum".into());
        }

        profile.build_commands.push("go build ./...".into());
        profile.test_commands.push("go test ./...".into());
        profile.test_commands.push("go test -race ./...".into());
        profile.lint_commands.push("golangci-lint run".into());
        profile.format_commands.push("gofmt -l .".into());
        profile.format_commands.push("goimports -l .".into());

        if root.join("vendor").is_dir() {
            profile
                .detected_files
                .push("vendor/ (vendored deps)".into());
        }

        if root.join("Makefile").exists() {
            let makefile = fs::read_to_string(root.join("Makefile"))?;
            if makefile.contains("generate") {
                profile.build_commands.push("make generate".into());
            }
        }
    }

    Ok(())
}

fn detect_php(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_composer = root.join("composer.json").exists();
    let has_composer_lock = root.join("composer.lock").exists();

    if has_composer {
        profile.languages.push("php".into());
        profile.detected_files.push("composer.json".into());
        profile.package_manager = Some("composer".into());

        if has_composer_lock {
            profile.detected_files.push("composer.lock".into());
        }

        profile.build_commands.push("composer install".into());
        profile.test_commands.push("./vendor/bin/phpunit".into());
        profile.test_commands.push("composer test".into());
        profile.lint_commands.push("./vendor/bin/phpcs".into());
        profile
            .lint_commands
            .push("./vendor/bin/phpstan analyse".into());
        profile
            .format_commands
            .push("./vendor/bin/php-cs-fixer fix --dry-run".into());
    }

    Ok(())
}

fn detect_ruby(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_gemfile = root.join("Gemfile").exists();
    let has_gemspec = root.join("*.gemspec").exists();

    if has_gemfile {
        profile.languages.push("ruby".into());
        profile.detected_files.push("Gemfile".into());
        profile.package_manager = Some("bundler".into());

        if root.join("Gemfile.lock").exists() {
            profile.detected_files.push("Gemfile.lock".into());
        }

        profile.build_commands.push("bundle install".into());
        profile.test_commands.push("bundle exec rspec".into());
        profile.test_commands.push("bundle exec rake test".into());
        profile.lint_commands.push("bundle exec rubocop".into());

        if root.join("Rakefile").exists() {
            profile.detected_files.push("Rakefile".into());
        }
    }

    Ok(())
}

fn detect_docker(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let has_dockerfile = root.join("Dockerfile").exists();
    let has_docker_compose = root.join("docker-compose.yml").exists()
        || root.join("docker-compose.yaml").exists()
        || root.join("compose.yml").exists()
        || root.join("compose.yaml").exists();
    let has_k8s = root.join("k8s").is_dir() || root.join("kubernetes").is_dir();

    if has_dockerfile || has_docker_compose || has_k8s {
        let mut container_config = ContainerConfig {
            has_docker: has_dockerfile,
            has_docker_compose: has_docker_compose,
            has_kubernetes: has_k8s,
            services: vec![],
        };

        if has_dockerfile {
            profile.detected_files.push("Dockerfile".into());
            profile.build_commands.push("docker build -t app .".into());
        }

        if has_docker_compose {
            let compose_file = if root.join("docker-compose.yml").exists() {
                "docker-compose.yml"
            } else if root.join("docker-compose.yaml").exists() {
                "docker-compose.yaml"
            } else if root.join("compose.yml").exists() {
                "compose.yml"
            } else {
                "compose.yaml"
            };
            profile.detected_files.push(compose_file.into());

            if let Ok(content) = fs::read_to_string(root.join(compose_file)) {
                if let Ok(compose) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    if let Some(services) = compose.get("services").and_then(|s| s.as_mapping()) {
                        for (name, _) in services {
                            if let Some(name_str) = name.as_str() {
                                container_config.services.push(name_str.to_string());

                                profile.services.push(ServiceDependency {
                                    name: name_str.to_string(),
                                    required: true,
                                    startup_command: Some(format!(
                                        "docker compose up -d {}",
                                        name_str
                                    )),
                                    health_check: Some(format!(
                                        "docker compose ps {} | grep healthy",
                                        name_str
                                    )),
                                    port: None,
                                });
                            }
                        }
                    }
                }
            }

            profile
                .test_commands
                .push("docker compose run --rm app test".into());
            profile.build_commands.push("docker compose build".into());
        }

        if has_k8s {
            profile.detected_files.push("kubernetes/".into());
            container_config.has_kubernetes = true;
            profile.warnings.push(
                "Kubernetes configuration detected - requires cluster access for testing".into(),
            );
        }

        profile.container_config = Some(container_config);
    }

    Ok(())
}

fn detect_ci(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    let github_workflows = root.join(".github/workflows");
    let gitlab_ci = root.join(".gitlab-ci.yml");
    let azure_pipelines = root.join("azure-pipelines.yml");
    let jenkins = root.join("Jenkinsfile");
    let circleci = root.join(".circleci/config.yml");
    let travis = root.join(".travis.yml");

    if github_workflows.is_dir() {
        profile.detected_files.push(".github/workflows/".into());

        let mut ci_config = CiConfig {
            provider: "github-actions".into(),
            config_file: ".github/workflows/".into(),
            test_command: None,
            lint_command: None,
        };

        if let Ok(entries) = fs::read_dir(&github_workflows) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "yml" || ext == "yaml" {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            if content.contains("test")
                                || content.contains("pytest")
                                || content.contains("cargo test")
                            {
                                ci_config.test_command =
                                    Some("github actions test workflow".into());
                            }
                            if content.contains("lint")
                                || content.contains("clippy")
                                || content.contains("eslint")
                            {
                                ci_config.lint_command =
                                    Some("github actions lint workflow".into());
                            }
                        }
                    }
                }
            }
        }

        profile.ci_config = Some(ci_config);
    } else if gitlab_ci.exists() {
        profile.detected_files.push(".gitlab-ci.yml".into());
        profile.ci_config = Some(CiConfig {
            provider: "gitlab-ci".into(),
            config_file: ".gitlab-ci.yml".into(),
            test_command: Some("gitlab pipeline".into()),
            lint_command: None,
        });
    } else if azure_pipelines.exists() {
        profile.detected_files.push("azure-pipelines.yml".into());
        profile.ci_config = Some(CiConfig {
            provider: "azure-pipelines".into(),
            config_file: "azure-pipelines.yml".into(),
            test_command: Some("azure pipeline".into()),
            lint_command: None,
        });
    } else if jenkins.exists() {
        profile.detected_files.push("Jenkinsfile".into());
        profile.ci_config = Some(CiConfig {
            provider: "jenkins".into(),
            config_file: "Jenkinsfile".into(),
            test_command: Some("jenkins pipeline".into()),
            lint_command: None,
        });
    } else if circleci.exists() {
        profile.detected_files.push(".circleci/config.yml".into());
        profile.ci_config = Some(CiConfig {
            provider: "circleci".into(),
            config_file: ".circleci/config.yml".into(),
            test_command: Some("circleci test".into()),
            lint_command: None,
        });
    } else if travis.exists() {
        profile.detected_files.push(".travis.yml".into());
        profile
            .warnings
            .push("Travis CI detected - consider migrating to GitHub Actions".into());
    }

    Ok(())
}

fn detect_env_files(root: &Path, profile: &mut EnvironmentProfile) -> Result<()> {
    if root.join(".env").exists() {
        profile.detected_files.push(".env".into());
        profile.warnings.push(
            "WARNING: .env file detected in repository - ensure secrets are not committed".into(),
        );
    }

    if root.join(".env.example").exists() {
        profile.detected_files.push(".env.example".into());
    }

    if root.join(".env.local").exists() {
        profile.detected_files.push(".env.local".into());
    }

    if root.join(".envrc").exists() {
        profile.detected_files.push(".envrc (direnv)".into());
    }

    let sensitive_patterns = [
        ".aws/credentials",
        ".ssh/id_rsa",
        "id_rsa",
        ".docker/config.json",
    ];

    for pattern in &sensitive_patterns {
        if root.join(pattern).exists() {
            profile.warnings.push(format!(
                "CRITICAL: Potential sensitive file detected: {}",
                pattern
            ));
        }
    }

    Ok(())
}

fn deduplicate_and_sort(vec: &mut Vec<String>) {
    vec.sort();
    vec.dedup();
}

pub fn get_preferred_test_command(profile: &EnvironmentProfile) -> Option<String> {
    profile.test_commands.first().cloned()
}

pub fn get_preferred_lint_command(profile: &EnvironmentProfile) -> Option<String> {
    profile.lint_commands.first().cloned()
}

pub fn get_preferred_format_command(profile: &EnvironmentProfile) -> Option<String> {
    profile.format_commands.first().cloned()
}

pub fn can_run_in_container(profile: &EnvironmentProfile) -> bool {
    profile
        .container_config
        .as_ref()
        .map(|c| c.has_docker || c.has_docker_compose)
        .unwrap_or(false)
}

pub fn requires_services(profile: &EnvironmentProfile) -> bool {
    !profile.services.is_empty()
}

pub fn get_service_startup_commands(profile: &EnvironmentProfile) -> Vec<String> {
    profile
        .services
        .iter()
        .filter_map(|s| s.startup_command.clone())
        .collect()
}
