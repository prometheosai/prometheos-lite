//! PlaybookResolver - Resolve and score playbooks for WorkContext
//!
//! This module provides the PlaybookResolver, which selects appropriate
//! playbooks for a given WorkContext based on domain matching and scoring.

use anyhow::Result;
use std::sync::Arc;

use super::playbook::WorkContextPlaybook;
use super::types::WorkContext;
use crate::db::Db;
use crate::db::repository::PlaybookOperations;

/// PlaybookResolver - Resolve and score playbooks for a given WorkContext
pub struct PlaybookResolver {
    db: Arc<Db>,
}

impl PlaybookResolver {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// Resolve the best playbook for a given WorkContext
    pub fn resolve_playbook(&self, context: &WorkContext) -> Result<Option<WorkContextPlaybook>> {
        let playbooks = self.db.get_playbooks_for_user(&context.user_id)?;

        if playbooks.is_empty() {
            return Ok(None);
        }

        let mut scored_playbooks: Vec<(f32, WorkContextPlaybook)> = playbooks
            .into_iter()
            .map(|pb| (self.calculate_score(&pb, context), pb))
            .collect();

        scored_playbooks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored_playbooks.first().map(|(_, pb)| pb.clone()))
    }

    /// Score all playbooks for a given WorkContext
    pub fn score_playbooks(
        &self,
        context: &WorkContext,
    ) -> Result<Vec<(WorkContextPlaybook, f32)>> {
        let user_playbooks = PlaybookOperations::get_playbooks_for_user(&*self.db, &context.user_id)?;

        let mut scored: Vec<(WorkContextPlaybook, f32)> = user_playbooks
            .into_iter()
            .map(|playbook| {
                let score = self.calculate_score(&playbook, context);
                (playbook, score)
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored)
    }

    /// Update playbook usage count
    pub fn update_playbook_usage(&self, playbook_id: &str) -> Result<()> {
        let mut playbook = PlaybookOperations::get_playbook(&*self.db, playbook_id)?
            .ok_or_else(|| anyhow::anyhow!("Playbook not found: {}", playbook_id))?;

        playbook.record_usage();
        PlaybookOperations::update_playbook(&*self.db, &playbook)?;

        Ok(())
    }

    /// Calculate score for a playbook against a context
    fn calculate_score(&self, playbook: &WorkContextPlaybook, context: &WorkContext) -> f32 {
        let mut score = playbook.confidence;

        // Domain match bonus
        if let Some(ref domain_profile_id) = context.domain_profile_id {
            if playbook.domain_profile_id == *domain_profile_id {
                score += 0.3;
            }
        }

        // Usage boost with diminishing returns
        let usage_boost = 0.1 * (playbook.usage_count as f32 + 1.0).ln();
        score += usage_boost;

        // Clamp to 0.0-1.0
        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::playbook::{CreativityLevel, ResearchDepth};
    use crate::work::types::{ApprovalPolicy, WorkDomain};

    #[test]
    fn test_calculate_score_domain_match() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score > 0.5); // Base 0.5 + domain bonus 0.3 = 0.8
    }

    #[test]
    fn test_calculate_score_no_domain_match() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("business".to_string());

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score < 0.6); // Base 0.5, no domain bonus
    }

    #[test]
    fn test_calculate_score_usage_boost() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );
        playbook.record_usage();
        playbook.record_usage();
        playbook.record_usage();

        let context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score > 0.5); // Base 0.5 + usage boost
    }
}
