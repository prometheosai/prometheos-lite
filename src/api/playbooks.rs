//! Playbook API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::state::AppState;
use crate::db::Db;
use crate::db::repository::PlaybookOperations;
use crate::work::playbook::{CreativityLevel, FlowPreference, NodePreference, PatternRecord, PatternType, ResearchDepth, WorkContextPlaybook};

/// Request to create a new Playbook
#[derive(Debug, Deserialize)]
pub struct CreatePlaybookRequest {
    pub user_id: String,
    pub domain_profile_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub preferred_flows: Vec<FlowPreference>,
    #[serde(default)]
    pub preferred_nodes: Vec<NodePreference>,
    #[serde(default)]
    pub default_research_depth: String,
    #[serde(default)]
    pub default_creativity_level: String,
}

/// Request to update a Playbook
#[derive(Debug, Deserialize)]
pub struct UpdatePlaybookRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub preferred_flows: Option<Vec<FlowPreference>>,
    #[serde(default)]
    pub preferred_nodes: Option<Vec<NodePreference>>,
    #[serde(default)]
    pub default_research_depth: Option<String>,
    #[serde(default)]
    pub default_creativity_level: Option<String>,
}

/// Response with Playbook details
#[derive(Debug, Serialize)]
pub struct PlaybookResponse {
    pub id: String,
    pub user_id: String,
    pub domain_profile_id: String,
    pub name: String,
    pub description: String,
    pub preferred_flows: Vec<FlowPreference>,
    pub preferred_nodes: Vec<NodePreference>,
    pub default_research_depth: String,
    pub default_creativity_level: String,
    pub success_patterns: Vec<PatternRecord>,
    pub failure_patterns: Vec<PatternRecord>,
    pub confidence: f32,
    pub usage_count: u32,
    pub updated_at: String,
}

/// List all playbooks for a user
pub async fn list_playbooks(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PlaybookResponse>>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let playbooks = PlaybookOperations::get_playbooks_for_user(&db, "api-user")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response: Vec<PlaybookResponse> = playbooks
        .into_iter()
        .map(|pb| PlaybookResponse {
            id: pb.id,
            user_id: pb.user_id,
            domain_profile_id: pb.domain_profile_id,
            name: pb.name,
            description: pb.description,
            preferred_flows: pb.preferred_flows,
            preferred_nodes: pb.preferred_nodes,
            default_research_depth: format!("{:?}", pb.default_research_depth),
            default_creativity_level: format!("{:?}", pb.default_creativity_level),
            success_patterns: pb.success_patterns,
            failure_patterns: pb.failure_patterns,
            confidence: pb.confidence,
            usage_count: pb.usage_count,
            updated_at: pb.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(response))
}

/// Get a specific playbook
pub async fn get_playbook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<PlaybookResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let playbook = PlaybookOperations::get_playbook(&db, &id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let response = PlaybookResponse {
        id: playbook.id,
        user_id: playbook.user_id,
        domain_profile_id: playbook.domain_profile_id,
        name: playbook.name,
        description: playbook.description,
        preferred_flows: playbook.preferred_flows,
        preferred_nodes: playbook.preferred_nodes,
        default_research_depth: format!("{:?}", playbook.default_research_depth),
        default_creativity_level: format!("{:?}", playbook.default_creativity_level),
        success_patterns: playbook.success_patterns,
        failure_patterns: playbook.failure_patterns,
        confidence: playbook.confidence,
        usage_count: playbook.usage_count,
        updated_at: playbook.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}

/// Create a new playbook
pub async fn create_playbook(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePlaybookRequest>,
) -> Result<Json<PlaybookResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let research_depth = match req.default_research_depth.to_lowercase().as_str() {
        "minimal" => ResearchDepth::Minimal,
        "standard" => ResearchDepth::Standard,
        "deep" => ResearchDepth::Deep,
        "exhaustive" => ResearchDepth::Exhaustive,
        _ => ResearchDepth::Standard,
    };

    let creativity_level = match req.default_creativity_level.to_lowercase().as_str() {
        "conservative" => CreativityLevel::Conservative,
        "balanced" => CreativityLevel::Balanced,
        "creative" => CreativityLevel::Creative,
        _ => CreativityLevel::Balanced,
    };

    let playbook = WorkContextPlaybook::new(
        uuid::Uuid::new_v4().to_string(),
        req.user_id,
        req.domain_profile_id,
        req.name,
        req.description,
    );

    let mut playbook = WorkContextPlaybook {
        preferred_flows: req.preferred_flows,
        preferred_nodes: req.preferred_nodes,
        default_research_depth: research_depth,
        default_creativity_level: creativity_level,
        ..playbook
    };

    let playbook = PlaybookOperations::create_playbook(&db, &playbook)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = PlaybookResponse {
        id: playbook.id,
        user_id: playbook.user_id,
        domain_profile_id: playbook.domain_profile_id,
        name: playbook.name,
        description: playbook.description,
        preferred_flows: playbook.preferred_flows,
        preferred_nodes: playbook.preferred_nodes,
        default_research_depth: format!("{:?}", playbook.default_research_depth),
        default_creativity_level: format!("{:?}", playbook.default_creativity_level),
        success_patterns: playbook.success_patterns,
        failure_patterns: playbook.failure_patterns,
        confidence: playbook.confidence,
        usage_count: playbook.usage_count,
        updated_at: playbook.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}

/// Update a playbook
pub async fn update_playbook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePlaybookRequest>,
) -> Result<Json<PlaybookResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut playbook = PlaybookOperations::get_playbook(&db, &id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if let Some(name) = req.name {
        playbook.name = name;
    }
    if let Some(description) = req.description {
        playbook.description = description;
    }
    if let Some(preferred_flows) = req.preferred_flows {
        playbook.preferred_flows = preferred_flows;
    }
    if let Some(preferred_nodes) = req.preferred_nodes {
        playbook.preferred_nodes = preferred_nodes;
    }
    if let Some(research_depth) = req.default_research_depth {
        playbook.default_research_depth = match research_depth.to_lowercase().as_str() {
            "minimal" => ResearchDepth::Minimal,
            "standard" => ResearchDepth::Standard,
            "deep" => ResearchDepth::Deep,
            "exhaustive" => ResearchDepth::Exhaustive,
            _ => ResearchDepth::Standard,
        };
    }
    if let Some(creativity_level) = req.default_creativity_level {
        playbook.default_creativity_level = match creativity_level.to_lowercase().as_str() {
            "conservative" => CreativityLevel::Conservative,
            "balanced" => CreativityLevel::Balanced,
            "creative" => CreativityLevel::Creative,
            _ => CreativityLevel::Balanced,
        };
    }

    let playbook = PlaybookOperations::update_playbook(&db, &playbook)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = PlaybookResponse {
        id: playbook.id,
        user_id: playbook.user_id,
        domain_profile_id: playbook.domain_profile_id,
        name: playbook.name,
        description: playbook.description,
        preferred_flows: playbook.preferred_flows,
        preferred_nodes: playbook.preferred_nodes,
        default_research_depth: format!("{:?}", playbook.default_research_depth),
        default_creativity_level: format!("{:?}", playbook.default_creativity_level),
        success_patterns: playbook.success_patterns,
        failure_patterns: playbook.failure_patterns,
        confidence: playbook.confidence,
        usage_count: playbook.usage_count,
        updated_at: playbook.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}
