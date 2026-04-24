//! Core orchestration logic.
//!
//! # DEPRECATED
//! This module is deprecated in favor of the new flow-centric architecture.
//! Use the `flow` module instead, specifically:
//! - `crate::flow::Flow` for orchestration instead of `SequentialOrchestrator`
//! - `crate::flow::FlowBuilder` to construct flows
//! - `crate::flow::Node` trait for execution units
//!
//! Migration guide:
//! 1. Replace `SequentialOrchestrator` with `FlowBuilder`
//! 2. Wrap agents with `crate::flow::AgentNode::new(agent)`
//! 3. Use `Flow::run()` for execution

use anyhow::Result;

use crate::{
    agents::{Agent, CoderAgent, PlannerAgent, ReviewerAgent},
    llm::LlmClient,
    logger::{AgentRole, Logger},
};

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub task: String,
    pub plan: Option<String>,
    pub generated_output: Option<String>,
    pub review: Option<String>,
}

impl ExecutionContext {
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            plan: None,
            generated_output: None,
            review: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub context: ExecutionContext,
}

impl ExecutionResult {
    pub fn plan(&self) -> Option<&str> {
        self.context.plan.as_deref()
    }

    pub fn generated_output(&self) -> Option<&str> {
        self.context.generated_output.as_deref()
    }

    pub fn review(&self) -> Option<&str> {
        self.context.review.as_deref()
    }
}

#[deprecated(since = "0.2.0", note = "Use crate::flow::FlowBuilder instead")]
#[derive(Debug, Clone)]
pub struct SequentialOrchestrator {
    llm: LlmClient,
    logger: Logger,
}

impl SequentialOrchestrator {
    pub fn new(llm: LlmClient) -> Self {
        Self {
            llm,
            logger: Logger::default(),
        }
    }

    pub fn with_logger(llm: LlmClient, logger: Logger) -> Self {
        Self { llm, logger }
    }

    pub async fn run(&self, task: impl Into<String>) -> Result<ExecutionResult> {
        let mut context = ExecutionContext::new(task);
        let task_str = context.task.clone();

        self.logger.info(&format!("Starting task: {}", task_str));
        self.logger
            .log(AgentRole::Planner, "Initializing planning phase");

        let planner = PlannerAgent::new(self.llm.clone());
        let plan = planner.run(&context.task).await?;
        self.logger.log(AgentRole::Planner, "Planning complete");
        self.logger
            .log(AgentRole::Coder, "Initializing code generation phase");

        let coder = CoderAgent::new(self.llm.clone());
        let generated_output = coder.run(&plan).await?;
        self.logger
            .log(AgentRole::Coder, "Code generation complete");
        self.logger
            .log(AgentRole::Reviewer, "Initializing review phase");

        let reviewer = ReviewerAgent::new(self.llm.clone());
        let review = reviewer.run(&generated_output).await?;
        self.logger.log(AgentRole::Reviewer, "Review complete");

        context.plan = Some(plan);
        context.generated_output = Some(generated_output);
        context.review = Some(review);

        self.logger.success("All phases completed successfully");

        Ok(ExecutionResult { context })
    }
}
