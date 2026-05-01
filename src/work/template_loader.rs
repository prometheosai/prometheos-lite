//! TemplateLoader - loads and manages WorkContext domain templates

use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use super::domain::{LifecycleTemplate, WorkDomainProfile};
use super::types::ApprovalPolicy;

/// TemplateLoader - loads domain profiles from YAML files
pub struct TemplateLoader {
    templates_dir: PathBuf,
}

impl TemplateLoader {
    /// Create a new TemplateLoader with the specified templates directory
    pub fn new(templates_dir: PathBuf) -> Self {
        Self { templates_dir }
    }

    /// Create a TemplateLoader with the default templates directory
    pub fn default() -> Result<Self> {
        let templates_dir = PathBuf::from("templates");
        Ok(Self::new(templates_dir))
    }

    /// Load all domain profiles from the templates directory
    pub fn load_all_profiles(&self) -> Result<Vec<WorkDomainProfile>> {
        let mut profiles = Vec::new();

        if !self.templates_dir.exists() {
            // Return empty list if templates directory doesn't exist
            return Ok(profiles);
        }

        for entry in fs::read_dir(&self.templates_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "yaml").unwrap_or(false) {
                let content = fs::read_to_string(&path)?;
                let profile: WorkDomainProfile = serde_yaml::from_str(&content)?;
                profiles.push(profile);
            }
        }

        Ok(profiles)
    }

    /// Load a specific domain profile by name
    pub fn load_profile(&self, name: &str) -> Result<Option<WorkDomainProfile>> {
        let profiles = self.load_all_profiles()?;
        Ok(profiles.into_iter().find(|p| p.name == name))
    }

    /// Get profile for a specific domain
    pub fn get_profile_for_domain(&self, domain: &str) -> Result<Option<WorkDomainProfile>> {
        let profiles = self.load_all_profiles()?;
        Ok(profiles.into_iter().find(|p| p.id == domain))
    }

    /// Save a domain profile to the templates directory
    pub fn save_profile(&self, profile: &WorkDomainProfile) -> Result<()> {
        fs::create_dir_all(&self.templates_dir)?;

        let filename = format!("{}.yaml", profile.id.to_lowercase().replace(' ', "_"));
        let path = self.templates_dir.join(filename);

        let yaml = serde_yaml::to_string(profile)?;
        fs::write(&path, yaml)?;

        Ok(())
    }

    /// Install default templates if none exist
    pub fn install_defaults(&self) -> Result<()> {
        if self.templates_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&self.templates_dir)?.collect();
            if !entries.is_empty() {
                // Templates already exist, skip installation
                return Ok(());
            }
        }

        fs::create_dir_all(&self.templates_dir)?;

        let defaults = self.get_default_profiles();
        for profile in defaults {
            self.save_profile(&profile)?;
        }

        Ok(())
    }

    /// Get default domain profiles
    fn get_default_profiles(&self) -> Vec<WorkDomainProfile> {
        vec![
            WorkDomainProfile::new(
                "software".to_string(),
                "Software Development".to_string(),
                vec![
                    "planning.flow.yaml".to_string(),
                    "codegen.flow.yaml".to_string(),
                ],
                vec!["Code".to_string(), "Document".to_string()],
            ),
            WorkDomainProfile::new(
                "business".to_string(),
                "Business Operations".to_string(),
                vec![
                    "planning.flow.yaml".to_string(),
                    "analysis.flow.yaml".to_string(),
                ],
                vec!["Report".to_string(), "Document".to_string()],
            ),
            WorkDomainProfile::new(
                "marketing".to_string(),
                "Marketing".to_string(),
                vec![
                    "content.flow.yaml".to_string(),
                    "campaign.flow.yaml".to_string(),
                ],
                vec!["Content".to_string(), "Asset".to_string()],
            ),
            WorkDomainProfile::new(
                "personal".to_string(),
                "Personal".to_string(),
                vec!["task.flow.yaml".to_string()],
                vec!["Note".to_string(), "Task".to_string()],
            ),
            WorkDomainProfile::new(
                "creative".to_string(),
                "Creative".to_string(),
                vec![
                    "design.flow.yaml".to_string(),
                    "content.flow.yaml".to_string(),
                ],
                vec!["Design".to_string(), "Content".to_string()],
            ),
            WorkDomainProfile::new(
                "research".to_string(),
                "Research".to_string(),
                vec![
                    "investigation.flow.yaml".to_string(),
                    "analysis.flow.yaml".to_string(),
                ],
                vec!["Document".to_string(), "Data".to_string()],
            ),
            WorkDomainProfile::new(
                "operations".to_string(),
                "Operations".to_string(),
                vec![
                    "maintenance.flow.yaml".to_string(),
                    "monitoring.flow.yaml".to_string(),
                ],
                vec!["Log".to_string(), "Report".to_string()],
            ),
            WorkDomainProfile::new(
                "general".to_string(),
                "General".to_string(),
                vec!["task.flow.yaml".to_string()],
                vec!["Artifact".to_string()],
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_template_loader_load_all() {
        let temp_dir = TempDir::new().unwrap();
        let loader = TemplateLoader::new(temp_dir.path().to_path_buf());

        // Install defaults
        loader.install_defaults().unwrap();

        // Load profiles
        let profiles = loader.load_all_profiles().unwrap();
        assert!(!profiles.is_empty());
        assert!(profiles.iter().any(|p| p.id == "software"));
    }

    #[test]
    fn test_template_loader_load_by_name() {
        let temp_dir = TempDir::new().unwrap();
        let loader = TemplateLoader::new(temp_dir.path().to_path_buf());

        loader.install_defaults().unwrap();

        let profile = loader.load_profile("Software Development").unwrap();
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().id, "software");
    }

    #[test]
    fn test_template_loader_get_profile_for_domain() {
        let temp_dir = TempDir::new().unwrap();
        let loader = TemplateLoader::new(temp_dir.path().to_path_buf());

        loader.install_defaults().unwrap();

        let profile = loader.get_profile_for_domain("software").unwrap();
        assert!(profile.is_some());
    }

    #[test]
    fn test_template_loader_save_profile() {
        let temp_dir = TempDir::new().unwrap();
        let loader = TemplateLoader::new(temp_dir.path().to_path_buf());

        let profile = WorkDomainProfile::new(
            "test".to_string(),
            "Test Profile".to_string(),
            vec!["test.flow.yaml".to_string()],
            vec!["Artifact".to_string()],
        );

        loader.save_profile(&profile).unwrap();

        let loaded = loader.load_profile("Test Profile").unwrap();
        assert!(loaded.is_some());
    }
}
