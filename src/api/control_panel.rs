//! Control panel endpoints for monitoring and management
//!
//! This module provides API endpoints for the control panel interface,
//! including system metrics, job queue status, and skill/evolution management.

use crate::api::state::AppState;
use crate::db::Db;
use crate::db::repository::{EvolutionsOperations, FlowRunOperations, SkillsOperations};
use crate::queue::JobQueueStats;
use axum::{Router, extract::State, http::StatusCode, response::Json, routing::get};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use sysinfo::System;

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
    let system_metrics = get_system_metrics(&state).await;
    let db = Arc::new(Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let job_queue_stats =
        build_job_queue_stats(&db).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let skills_count = db
        .count_skills()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;
    let evolutions_count = db
        .count_evolutions()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;
    let active_flows = db
        .count_active_flow_runs()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;

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
    State(state): State<Arc<AppState>>,
) -> Result<Json<SystemMetrics>, StatusCode> {
    let metrics = get_system_metrics(&state).await;
    Ok(Json(metrics))
}

/// List all skills
async fn list_skills(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SkillSummary>>, StatusCode> {
    let db = Arc::new(Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT id, name, description, usage_count, success_rate
             FROM skills
             ORDER BY usage_count DESC, updated_at DESC
             LIMIT 100",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SkillSummary {
                id: row.get::<_, String>(0)?,
                name: row.get::<_, String>(1)?,
                description: row.get::<_, String>(2)?,
                usage_count: row.get::<_, i64>(3)? as u32,
                success_rate: row.get::<_, f64>(4)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut skills = Vec::new();
    for row in rows {
        skills.push(row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    }

    Ok(Json(skills))
}

/// List all evolutions
async fn list_evolutions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<EvolutionSummary>>, StatusCode> {
    let db = Arc::new(Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT id, playbook_id, version, status, success_rate
             FROM playbook_evolutions
             ORDER BY created_at DESC
             LIMIT 100",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(EvolutionSummary {
                id: row.get::<_, String>(0)?,
                playbook_id: row.get::<_, String>(1)?,
                version: row.get::<_, i64>(2).map(|v| v as u32).or_else(|_| {
                    row.get::<_, String>(2)
                        .ok()
                        .and_then(|v| v.parse::<u32>().ok())
                        .ok_or(rusqlite::Error::InvalidColumnType(
                            2,
                            "version".to_string(),
                            rusqlite::types::Type::Text,
                        ))
                })?,
                status: row.get::<_, String>(3)?,
                success_rate: row.get::<_, f64>(4).or_else(|_| {
                    row.get::<_, String>(4)
                        .ok()
                        .and_then(|v| v.parse::<f64>().ok())
                        .ok_or(rusqlite::Error::InvalidColumnType(
                            4,
                            "success_rate".to_string(),
                            rusqlite::types::Type::Text,
                        ))
                })?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut evolutions = Vec::new();
    for row in rows {
        evolutions.push(row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    }

    Ok(Json(evolutions))
}

/// Get job queue statistics
async fn get_job_queue_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<JobQueueStats>, StatusCode> {
    let db = Arc::new(Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let stats = build_job_queue_stats(&db).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(stats))
}

/// Get system metrics
///
/// Uses sysinfo to gather actual process and system metrics.
async fn get_system_metrics(state: &Arc<AppState>) -> SystemMetrics {
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

    let active_connections = state.ws_manager.active_connections().await;
    let total_requests = state.total_requests();

    SystemMetrics {
        uptime_seconds,
        memory_usage_mb,
        active_connections,
        total_requests,
        timestamp: chrono::Utc::now(),
    }
}

fn count_flow_runs_by_status(db: &Db, status: &str) -> anyhow::Result<i64> {
    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM flow_runs WHERE status = ?1",
        params![status],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn build_job_queue_stats(db: &Db) -> anyhow::Result<JobQueueStats> {
    let total = db.count_flow_runs()? as usize;
    let running = db.count_active_flow_runs()? as usize;
    let pending = count_flow_runs_by_status(db, "pending")? as usize;
    let completed = count_flow_runs_by_status(db, "completed")? as usize;
    let failed = count_flow_runs_by_status(db, "failed")? as usize;
    let cancelled = count_flow_runs_by_status(db, "cancelled")? as usize;

    Ok(JobQueueStats {
        total,
        pending,
        running,
        completed,
        failed,
        cancelled,
    })
}
