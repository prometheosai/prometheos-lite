//! WorkContext module for the V1.2 Operation Layer
//!
//! This module provides the persistent operational layer that manages
//! real-world work across time through WorkContext objects.

pub mod artifact;
pub mod artifact_mapper;
pub mod decision;
pub mod domain;
pub mod event;
pub mod execution_service;
pub mod orchestrator;
pub mod phase_controller;
pub mod plan;
pub mod playbook;
pub mod playbook_resolver;
pub mod service;
pub mod template_loader;
pub mod templates;
pub mod types;

pub use artifact::{Artifact, ArtifactKind, ArtifactStorage};
pub use artifact_mapper::ArtifactMapper;
pub use decision::DecisionRecord;
pub use domain::{LifecycleTemplate, WorkDomainProfile};
pub use event::WorkContextEvent;
pub use execution_service::WorkExecutionService;
pub use orchestrator::{ExecutionLimits, WorkOrchestrator};
pub use phase_controller::PhaseController;
pub use plan::{ExecutionPlan, PlanStep, StepStatus};
pub use playbook::{CreativityLevel, ResearchDepth, WorkContextPlaybook};
pub use playbook_resolver::PlaybookResolver;
pub use service::WorkContextService;
pub use template_loader::TemplateLoader;
pub use templates::{bug_fix_template, planning_template, research_template, software_development_template};
pub use types::{
    ApprovalPolicy, AutonomyLevel, CompletionCriterion, WorkContext, WorkDomain, WorkPhase,
    WorkPriority, WorkStatus,
};
