//! Agent interfaces and implementations.

mod coder;
mod planner;
mod reviewer;

use anyhow::Result;
use async_trait::async_trait;

pub use coder::CoderAgent;
pub use planner::PlannerAgent;
pub use reviewer::ReviewerAgent;

#[async_trait]
pub trait Agent {
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
