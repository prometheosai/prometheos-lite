//! Agent interfaces and implementations.
//!
//! # DEPRECATED
//! This module is deprecated in favor of the new flow-centric architecture.
//! Use the `flow` module instead, specifically:
//! - `crate::flow::Node` trait instead of `Agent` trait
//! - `crate::flow::AgentNode` adapter to wrap existing agents
//! - `crate::flow::Flow` for orchestration instead of `SequentialOrchestrator`
//!
//! Migration guide:
//! 1. Wrap existing agents with `AgentNode::new(agent)`
//! 2. Build flows using `FlowBuilder`
//! 3. Use `Flow::run()` for execution

mod coder;
mod planner;
mod reviewer;

use anyhow::Result;
use async_trait::async_trait;

pub use coder::CoderAgent;
pub use planner::PlannerAgent;
pub use reviewer::ReviewerAgent;

#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;

    async fn run(&self, input: &str) -> Result<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Planner,
    Builder,
    Reviewer,
}

impl AgentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Builder => "builder",
            Self::Reviewer => "reviewer",
        }
    }
}
