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
    /// 3-tier fallback: user+domain → domain default → global default
    pub fn resolve_playbook(&self, context: &WorkContext) -> Result<Option<WorkContextPlaybook>> {
        // Tier 1: Try user+domain specific playbook
        if let Some(ref domain_profile_id) = context.domain_profile_id
            && let Some(playbook) = self
                .db
                .get_playbook_by_user_and_domain(&context.user_id, domain_profile_id)?
        {
            return Ok(Some(playbook));
        }

        // Tier 2: Try domain default playbook (user_id = "domain-default")
        if let Some(ref domain_profile_id) = context.domain_profile_id
            && let Some(playbook) = self
                .db
                .get_playbook_by_user_and_domain("domain-default", domain_profile_id)?
        {
            return Ok(Some(playbook));
        }

        // Tier 3: Try global default playbook (user_id = "global", domain_profile_id = "global")
        if let Some(playbook) = self
            .db
            .get_playbook_by_user_and_domain("global", "global")?
        {
            return Ok(Some(playbook));
        }

        // Fallback: Score all user playbooks and return best match
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
        let user_playbooks =
            PlaybookOperations::get_playbooks_for_user(&*self.db, &context.user_id)?;

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
        if let Some(ref domain_profile_id) = context.domain_profile_id
            && playbook.domain_profile_id == *domain_profile_id
        {
            score += 0.3;
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
    use crate::work::playbook::FlowPreference;
    use crate::work::types::WorkDomain;

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

    #[test]
    fn test_calculate_score_with_flow_preferences() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );

        playbook.preferred_flows = vec![
            FlowPreference {
                flow_id: "planning.flow.yaml".to_string(),
                weight: 0.8,
                confidence: 0.9,
            },
            FlowPreference {
                flow_id: "coding.flow.yaml".to_string(),
                weight: 0.6,
                confidence: 0.8,
            },
        ];

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score > 0.5); // Base score with domain match
    }

    #[test]
    fn test_calculate_score_clamping() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );

        // Set high confidence and usage count
        playbook.confidence = 0.9;
        playbook.usage_count = 100;

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score <= 1.0); // Should be clamped to 1.0
    }

    #[test]
    fn test_calculate_score_min_clamping() {
        let db = Db::in_memory().unwrap();
        let resolver = PlaybookResolver::new(Arc::new(db));

        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Software Playbook".to_string(),
            "For software work".to_string(),
        );

        // Set low confidence
        playbook.confidence = 0.1;

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("business".to_string()); // No domain match

        let score = resolver.calculate_score(&playbook, &context);
        assert!(score >= 0.0); // Should be clamped to 0.0
    }

    #[test]
    fn test_score_playbooks_sorting() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db.clone());

        let mut playbook1 = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "High Confidence Playbook".to_string(),
            "High confidence playbook".to_string(),
        );
        playbook1.confidence = 0.9;

        let mut playbook2 = WorkContextPlaybook::new(
            "pb-2".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Low Confidence Playbook".to_string(),
            "Low confidence playbook".to_string(),
        );
        playbook2.confidence = 0.3;

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        // Store playbooks
        db.create_playbook(&playbook1).unwrap();
        db.create_playbook(&playbook2).unwrap();

        let scored = resolver.score_playbooks(&context).unwrap();
        assert_eq!(scored.len(), 2);
        assert!(scored[0].1 >= scored[1].1); // Sorted by score descending
    }

    #[test]
    fn test_resolve_playbook_tier1_user_domain() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db.clone());

        let playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "User Domain Playbook".to_string(),
            "User-specific domain playbook".to_string(),
        );

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        // Store playbook
        db.create_playbook(&playbook).unwrap();

        let resolved = resolver.resolve_playbook(&context).unwrap();
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().id, "pb-1");
    }

    #[test]
    fn test_resolve_playbook_no_match() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db);

        let context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        let resolved = resolver.resolve_playbook(&context).unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn test_update_playbook_usage() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db.clone());

        let playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Usage Test Playbook".to_string(),
            "Test usage tracking".to_string(),
        );

        // Store playbook
        db.create_playbook(&playbook).unwrap();

        // Update usage
        resolver.update_playbook_usage("pb-1").unwrap();

        // Verify usage count increased
        let retrieved = db.get_playbook("pb-1").unwrap().unwrap();
        assert_eq!(retrieved.usage_count, 1);
    }

    #[test]
    fn test_weighted_flow_selection() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db.clone());

        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Weighted Flow Playbook".to_string(),
            "Tests weighted flow selection".to_string(),
        );

        // Add flow preferences with different weights
        playbook.preferred_flows = vec![
            FlowPreference {
                flow_id: "planning.flow.yaml".to_string(),
                weight: 0.9,
                confidence: 0.95,
            },
            FlowPreference {
                flow_id: "coding.flow.yaml".to_string(),
                weight: 0.7,
                confidence: 0.85,
            },
            FlowPreference {
                flow_id: "review.flow.yaml".to_string(),
                weight: 0.5,
                confidence: 0.75,
            },
        ];

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        // Store playbook
        db.create_playbook(&playbook).unwrap();

        let resolved = resolver.resolve_playbook(&context).unwrap();
        assert!(resolved.is_some());

        let resolved_pb = resolved.unwrap();
        assert_eq!(resolved_pb.preferred_flows.len(), 3);

        // Verify flows are ordered by weight
        assert!(resolved_pb.preferred_flows[0].weight >= resolved_pb.preferred_flows[1].weight);
        assert!(resolved_pb.preferred_flows[1].weight >= resolved_pb.preferred_flows[2].weight);
    }

    #[test]
    fn test_exploration_factor_influence() {
        let db = Arc::new(Db::in_memory().unwrap());
        let resolver = PlaybookResolver::new(db.clone());

        let mut playbook_high_usage = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "High Usage Playbook".to_string(),
            "High usage playbook".to_string(),
        );
        playbook_high_usage.usage_count = 50;

        let mut playbook_low_usage = WorkContextPlaybook::new(
            "pb-2".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "Low Usage Playbook".to_string(),
            "Low usage playbook".to_string(),
        );
        playbook_low_usage.usage_count = 1;

        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );
        context.domain_profile_id = Some("software".to_string());

        // Store playbooks
        db.create_playbook(&playbook_high_usage).unwrap();
        db.create_playbook(&playbook_low_usage).unwrap();

        let scored = resolver.score_playbooks(&context).unwrap();

        // High usage playbook should score higher due to usage boost
        let high_usage_score = scored.iter().find(|(pb, _)| pb.id == "pb-1").unwrap().1;
        let low_usage_score = scored.iter().find(|(pb, _)| pb.id == "pb-2").unwrap().1;

        assert!(high_usage_score > low_usage_score);
    }
}
