//! WorkContextService - core business logic for WorkContext lifecycle

use anyhow::Result;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    CompletionCriterion,
    event::WorkContextEvent,
    types::{WorkContext, WorkPhase, WorkStatus},
};
use crate::db::Db;
use crate::db::repository::work_artifacts::WorkArtifactOperations;
use crate::db::repository::work_context::WorkContextOperations;
use crate::db::repository::work_context_events::WorkContextEventOperations;

/// WorkContextService - handles WorkContext CRUD and lifecycle operations
pub struct WorkContextService {
    db: Arc<Db>,
}

impl WorkContextService {
    /// Create a new WorkContextService with database access
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// Get the database instance
    pub fn get_db(&self) -> &Arc<Db> {
        &self.db
    }

    /// Create a new WorkContext
    pub fn create_context(
        &self,
        user_id: String,
        title: String,
        domain: super::types::WorkDomain,
        goal: String,
    ) -> Result<WorkContext> {
        let id = Uuid::new_v4().to_string();
        let mut context = WorkContext::new(id, user_id, title, domain, goal);

        // Persist to database
        let saved = WorkContextOperations::create_work_context(&*self.db, &context)?;

        // Log creation event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            saved.id.clone(),
            "context_created".to_string(),
            serde_json::json!({ "title": saved.title, "domain": saved.domain }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(saved)
    }

    /// Get a WorkContext by ID
    pub fn get_context(&self, id: &str) -> Result<Option<WorkContext>> {
        WorkContextOperations::get_work_context(&*self.db, id)
    }

    /// Update a WorkContext
    pub fn update_context(&self, context: &WorkContext) -> Result<()> {
        WorkContextOperations::update_work_context(&*self.db, context)
    }

    /// Add an artifact to a WorkContext
    pub fn add_artifact(&self, context: &mut WorkContext, artifact: super::Artifact) -> Result<()> {
        // Persist artifact to database
        WorkArtifactOperations::create_artifact(&*self.db, &artifact)?;

        // Add to context's artifact list (stores references)
        context.artifacts.push(artifact);
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log artifact event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "artifact_added".to_string(),
            serde_json::json!({ "artifact_count": context.artifacts.len() }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// Add a decision to a WorkContext
    pub fn add_decision(
        &self,
        context: &mut WorkContext,
        decision: super::DecisionRecord,
    ) -> Result<()> {
        context.decisions.push(decision);
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log decision event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "decision_added".to_string(),
            serde_json::json!({ "decision_count": context.decisions.len() }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// Update the status of a WorkContext
    pub fn update_status(&self, context: &mut WorkContext, status: WorkStatus) -> Result<()> {
        let old_status = context.status;
        context.status = status;
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log status change event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "status_changed".to_string(),
            serde_json::json!({ "from": old_status, "to": status }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// Update the phase of a WorkContext
    pub fn update_phase(&self, context: &mut WorkContext, phase: WorkPhase) -> Result<()> {
        let old_phase = context.current_phase;
        context.current_phase = phase;
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log phase transition event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "phase_transition".to_string(),
            serde_json::json!({ "from": old_phase, "to": phase }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// Add a completion criterion
    pub fn add_completion_criterion(
        &self,
        context: &mut WorkContext,
        description: String,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let criterion = CompletionCriterion::new(id, description);
        context.completion_criteria.push(criterion);
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)
    }

    /// Mark a completion criterion as satisfied
    pub fn satisfy_completion_criterion(
        &self,
        context: &mut WorkContext,
        criterion_id: &str,
    ) -> Result<()> {
        if let Some(criterion) = context
            .completion_criteria
            .iter_mut()
            .find(|c| c.id == criterion_id)
        {
            criterion.satisfy();
            context.touch();
            WorkContextOperations::update_work_context(&*self.db, context)?;
        }
        Ok(())
    }

    /// Set the blocked reason
    pub fn set_blocked_reason(&self, context: &mut WorkContext, reason: String) -> Result<()> {
        context.blocked_reason = Some(reason);
        context.status = WorkStatus::Blocked;
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log blocked event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "context_blocked".to_string(),
            serde_json::json!({ "reason": context.blocked_reason }),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// Clear the blocked reason
    pub fn clear_blocked_reason(&self, context: &mut WorkContext) -> Result<()> {
        context.blocked_reason = None;
        if context.status == WorkStatus::Blocked {
            context.status = WorkStatus::InProgress;
        }
        context.touch();
        WorkContextOperations::update_work_context(&*self.db, context)?;

        // Log unblocked event
        let event = WorkContextEvent::new(
            Uuid::new_v4().to_string(),
            context.id.clone(),
            "context_unblocked".to_string(),
            serde_json::json!({}),
        );
        let _ = WorkContextEventOperations::create_event(&*self.db, &event);

        Ok(())
    }

    /// List all contexts for a user
    pub fn list_contexts(&self, user_id: &str) -> Result<Vec<WorkContext>> {
        WorkContextOperations::list_work_contexts(&*self.db, user_id)
    }

    /// Get the active context for a conversation
    pub fn get_active_context_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<WorkContext>> {
        WorkContextOperations::get_active_context_for_conversation(&*self.db, conversation_id)
    }

    /// Set the active context for a conversation
    pub fn set_active_context_for_conversation(
        &self,
        conversation_id: &str,
        work_context_id: &str,
    ) -> Result<()> {
        WorkContextOperations::set_active_context_for_conversation(
            &*self.db,
            conversation_id,
            work_context_id,
        )
    }

    /// Route to the appropriate context based on priority
    pub fn route_context(
        &self,
        _user_id: &str,
        conversation_id: Option<&str>,
        explicit_context_id: Option<&str>,
    ) -> Result<Option<WorkContext>> {
        // Priority 1: Explicit context_id
        if let Some(id) = explicit_context_id {
            return self.get_context(id);
        }

        // Priority 2: Active context for conversation
        if let Some(conv_id) = conversation_id {
            if let Some(context) = self.get_active_context_for_conversation(conv_id)? {
                return Ok(Some(context));
            }
        }

        // Priority 3: Create new context (caller's responsibility)
        Ok(None)
    }

    /// V1.6-FIX-016: Persist EvidenceLog into WorkContext/RunDb
    pub fn persist_evidence_log(
        &self,
        work_context_id: &str,
        evidence_log: &EvidenceLog,
    ) -> Result<()> {
        use crate::harness::evidence_persistence::EvidencePersistenceManager;
        
        // Create persistence manager
        let persistence_manager = EvidencePersistenceManager::new();
        
        // Persist the evidence log to the work context
        let evidence_data = serde_json::to_value(evidence_log)?;
        
        // Store as artifact in the work context
        let artifact = super::Artifact {
            id: format!("evidence_log_{}", work_context_id),
            work_context_id: work_context_id.to_string(),
            kind: super::ArtifactKind::EvidenceLog,
            created_at: chrono::Utc::now(),
            data: evidence_data,
            metadata: serde_json::json!({
                "evidence_log_id": evidence_log.execution_id,
                "entries_count": evidence_log.entries.len(),
                "has_failures": evidence_log.has_failures(),
                "persistence_timestamp": chrono::Utc::now().to_rfc3339()
            }),
        };
        
        // Add artifact to work context
        let mut context = self.get_context(work_context_id)?;
        context.artifacts.push(artifact);
        context.touch();
        
        // Update the work context
        self.update_context(&context)?;
        
        tracing::info!(
            work_context_id = %work_context_id,
            entries_count = %entries_count,
            "Persisted EvidenceLog to WorkContext"
        );
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::types::WorkDomain;

    #[test]
    fn test_create_context() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        assert_eq!(context.user_id, "user-1");
        assert_eq!(context.title, "Build API");
        assert_eq!(context.status, WorkStatus::Draft);
    }

    #[test]
    fn test_get_context() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let created = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        let retrieved = service.get_context(&created.id).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, created.id);
    }

    #[test]
    fn test_update_status() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .update_status(&mut context, WorkStatus::InProgress)
            .unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert_eq!(retrieved.status, WorkStatus::InProgress);
    }

    #[test]
    fn test_update_phase() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .update_phase(&mut context, WorkPhase::Planning)
            .unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert_eq!(retrieved.current_phase, WorkPhase::Planning);
    }

    #[test]
    fn test_add_completion_criterion() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .add_completion_criterion(&mut context, "Tests pass".to_string())
            .unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert_eq!(retrieved.completion_criteria.len(), 1);
        assert_eq!(retrieved.completion_criteria[0].description, "Tests pass");
    }

    #[test]
    fn test_satisfy_completion_criterion() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .add_completion_criterion(&mut context, "Tests pass".to_string())
            .unwrap();
        let criterion_id = context.completion_criteria[0].id.clone();

        service
            .satisfy_completion_criterion(&mut context, &criterion_id)
            .unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert!(retrieved.completion_criteria[0].satisfied);
    }

    #[test]
    fn test_set_blocked_reason() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .set_blocked_reason(&mut context, "Waiting for approval".to_string())
            .unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert_eq!(retrieved.status, WorkStatus::Blocked);
        assert_eq!(
            retrieved.blocked_reason,
            Some("Waiting for approval".to_string())
        );
    }

    #[test]
    fn test_clear_blocked_reason() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));
        let mut context = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .set_blocked_reason(&mut context, "Waiting for approval".to_string())
            .unwrap();
        service.clear_blocked_reason(&mut context).unwrap();

        let retrieved = service.get_context(&context.id).unwrap().unwrap();
        assert_eq!(retrieved.status, WorkStatus::InProgress);
        assert!(retrieved.blocked_reason.is_none());
    }

    #[test]
    fn test_list_contexts() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));

        service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        service
            .create_context(
                "user-1".to_string(),
                "Write Docs".to_string(),
                WorkDomain::Software,
                "Write documentation".to_string(),
            )
            .unwrap();

        let contexts = service.list_contexts("user-1").unwrap();
        assert_eq!(contexts.len(), 2);
    }

    #[test]
    fn test_route_context_explicit() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));

        let created = service
            .create_context(
                "user-1".to_string(),
                "Build API".to_string(),
                WorkDomain::Software,
                "Create a REST API".to_string(),
            )
            .unwrap();

        let routed = service
            .route_context("user-1", None, Some(&created.id))
            .unwrap();
        assert!(routed.is_some());
        assert_eq!(routed.unwrap().id, created.id);
    }

    #[test]
    fn test_route_context_none() {
        let db = Db::in_memory().unwrap();
        let service = WorkContextService::new(Arc::new(db));

        let routed = service.route_context("user-1", None, None).unwrap();
        assert!(routed.is_none());
    }
}
