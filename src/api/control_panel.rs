//! Control panel endpoints for monitoring and management
//!
//! This module provides API endpoints for the control panel interface,
//! including system metrics, job queue status, and skill/evolution management.

use crate::api::state::AppState;
use crate::db::repository::{EvolutionsOperations, SkillsOperations, FlowRunOperations};
use crate::queue::JobQueueStats;
use axum::{Router, extract::State, http::StatusCode, response::Json, routing::get};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sysinfo::System;
use std::time::Instant;

/// Global startup time for uptime calculation
static STARTUP_TIME: once_cell::sync::Lazy<Instant> = once_cell::sync::Lazy::new(Instant::now);

/// System health metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub uptime_seconds: u64,
    pub memory_usage_mb: u64,
    pub active_connections: usize,
    pub total_requests: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Control panel statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPanelStats {
    pub system: SystemMetrics,
    pub job_queue: JobQueueStats,
    pub skills_count: usize,
    pub evolutions_count: usize,
    pub active_flows: usize,
}

/// Skill summary for control panel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub usage_count: u32,
    pub success_rate: f64,
}

/// Evolution summary for control panel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSummary {
    pub id: String,
    pub playbook_id: String,
    pub version: u32,
    pub status: String,
    pub success_rate: f64,
}

/// Create control panel router
pub fn create_control_panel_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/stats", get(get_stats))
        .route("/metrics", get(get_metrics))
        .route("/skills", get(list_skills))
        .route("/evolutions", get(list_evolutions))
        .route("/job-queue/stats", get(get_job_queue_stats))
}

/// Get comprehensive control panel statistics
async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ControlPanelStats>, StatusCode> {
    let system_metrics = get_system_metrics().await;

    // Get job queue stats if available
    let job_queue_stats = JobQueueStats {
        total: 0,
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
        cancelled: 0,
    };

    // V1.5.2: Real database counts for skills, evolutions, and active flows
    let (skills_count, evolutions_count, active_flows) =
        if let Ok(db) = crate::db::repository::Db::new(&state.db_path) {
            (
                db.count_skills().unwrap_or(0) as usize,
                db.count_evolutions().unwrap_or(0) as usize,
                db.count_active_flow_runs().unwrap_or(0) as usize,
            )
        } else {
            (0, 0, 0)
        };

    let stats = ControlPanelStats {
        system: system_metrics,
        job_queue: job_queue_stats,
        skills_count,
        evolutions_count,
        active_flows,
    };

    Ok(Json(stats))
}

/// Get system metrics
async fn get_metrics(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<SystemMetrics>, StatusCode> {
    let metrics = get_system_metrics().await;
    Ok(Json(metrics))
}

/// List all skills
async fn list_skills(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<SkillSummary>>, StatusCode> {
    // Return empty list for now - would integrate with SkillKernel
    let skills: Vec<SkillSummary> = Vec::new();
    Ok(Json(skills))
}

/// List all evolutions
async fn list_evolutions(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<EvolutionSummary>>, StatusCode> {
    // Return empty list for now - would integrate with EvolutionEngine
    let evolutions: Vec<EvolutionSummary> = Vec::new();
    Ok(Json(evolutions))
}

/// Get job queue statistics
async fn get_job_queue_stats(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<JobQueueStats>, StatusCode> {
    let stats = JobQueueStats {
        total: 0,
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
        cancelled: 0,
    };
    Ok(Json(stats))
}

/// Get system metrics
/// 
/// Uses sysinfo to gather actual process and system metrics.
async fn get_system_metrics() -> SystemMetrics {
    // Calculate uptime
    let uptime_seconds = STARTUP_TIME.elapsed().as_secs();

    // Get actual memory usage using sysinfo
    let mut system = System::new_all();
    system.refresh_all();
    
    // Get current process memory usage
    let memory_usage_mb = sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid).map(|p| p.memory() / 1024))
        .unwrap_or(0);

    // Active connections - not tracked yet, return 0 as "not available"
    let active_connections = 0;

    // Total requests - not tracked yet, return 0 as "not available"  
    let total_requests = 0;

    SystemMetrics {
        uptime_seconds,
        memory_usage_mb,
        active_connections,
        total_requests,
        timestamp: chrono::Utc::now(),
    }
}
