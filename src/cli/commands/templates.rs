//! CLI commands for template management

use anyhow::Result;
use clap::{Parser, Subcommand};
use prometheos_lite::work::TemplateLoader;

#[derive(Debug, Parser)]
pub struct TemplatesCommand {
    #[command(subcommand)]
    command: TemplatesSubcommand,
}

#[derive(Debug, Subcommand)]
enum TemplatesSubcommand {
    /// List all available domain templates
    List,
    /// Show details for a specific template
    Show { name: String },
    /// Install default templates
    Install,
    /// Create a new custom template
    Create { name: String },
}

impl TemplatesCommand {
    pub async fn execute(self) -> Result<()> {
        let loader = TemplateLoader::from_default_templates_dir()?;

        match self.command {
            TemplatesSubcommand::List => {
                self.list_templates(&loader)?;
            }
            TemplatesSubcommand::Show { ref name } => {
                self.show_template(&loader, name)?;
            }
            TemplatesSubcommand::Install => {
                self.install_templates(&loader)?;
            }
            TemplatesSubcommand::Create { ref name } => {
                self.create_template(&loader, name)?;
            }
        }

        Ok(())
    }

    fn list_templates(&self, loader: &TemplateLoader) -> Result<()> {
        let profiles = loader.load_all_profiles()?;

        if profiles.is_empty() {
            println!("No templates found. Run 'prometheos templates install' to install defaults.");
            return Ok(());
        }

        println!("Available Templates:");
        println!();
        for profile in &profiles {
            println!("  - {} ({})", profile.name, profile.id);
            println!("    Default Flows: {:?}", profile.default_flows);
            println!("    Artifact Kinds: {:?}", profile.artifact_kinds);
            println!("    Approval Policy: {:?}", profile.approval_defaults);
            println!();
        }

        Ok(())
    }

    fn show_template(&self, loader: &TemplateLoader, name: &str) -> Result<()> {
        let profile = loader
            .load_profile(name)?
            .ok_or_else(|| anyhow::anyhow!("Template '{}' not found", name))?;

        println!("Template: {}", profile.name);
        println!("ID: {}", profile.id);
        println!();
        println!("Default Settings:");
        println!("  Flows: {:?}", profile.default_flows);
        println!("  Artifact Kinds: {:?}", profile.artifact_kinds);
        println!("  Approval Policy: {:?}", profile.approval_defaults);
        println!();
        println!("Lifecycle Template:");
        println!("  Phases: {:?}", profile.lifecycle_template.phases);
        println!(
            "  Transitions: {:?}",
            profile.lifecycle_template.transitions
        );

        Ok(())
    }

    fn install_templates(&self, loader: &TemplateLoader) -> Result<()> {
        println!("Installing default templates...");
        loader.install_defaults()?;
        println!("Default templates installed successfully.");
        Ok(())
    }

    fn create_template(&self, loader: &TemplateLoader, name: &str) -> Result<()> {
        println!("Creating new template: {}", name);
        println!("This will create a basic template that you can customize manually.");
        println!();

        let profile = prometheos_lite::work::WorkDomainProfile::new(
            name.to_lowercase().replace(' ', "_"),
            name.to_string(),
            vec!["task.flow.yaml".to_string()],
            vec!["Artifact".to_string()],
        );

        loader.save_profile(&profile)?;
        println!("Template '{}' created successfully.", name);
        println!("You can customize it by editing the template file in the templates directory.");

        Ok(())
    }
}
