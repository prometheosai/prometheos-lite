//! Conversation operations

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::db::models::{Conversation, CreateConversation};
use super::AsDb;

/// Conversation operations trait
pub trait ConversationOperations {
    fn create_conversation(&self, input: CreateConversation) -> anyhow::Result<Conversation>;
    fn get_conversations(&self, project_id: &str) -> anyhow::Result<Vec<Conversation>>;
    fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>>;
}

impl<T: AsDb> ConversationOperations for T {
    fn create_conversation(&self, input: CreateConversation) -> anyhow::Result<Conversation> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        conn.execute(
            "INSERT INTO conversations (id, project_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, &input.project_id, &input.title, &now.to_rfc3339(), &now.to_rfc3339()],
        ).context("Failed to insert conversation")?;

        Ok(Conversation {
            id,
            project_id: input.project_id,
            title: input.title,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_conversations(&self, project_id: &str) -> anyhow::Result<Vec<Conversation>> {
        let conn = self.as_db().conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, title, created_at, updated_at FROM conversations WHERE project_id = ?1 ORDER BY created_at DESC"
        ).context("Failed to prepare conversations query")?;

        let conversations = stmt.query_map(params![project_id], |row| {
            let created_str: String = row.get(3)?;
            let updated_str: String = row.get(4)?;
            Ok(Conversation {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).context("Failed to query conversations")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to collect conversations")?;

        Ok(conversations)
    }

    fn get_conversation(&self, id: &str) -> anyhow::Result<Option<Conversation>> {
        let conn = self.as_db().conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, title, created_at, updated_at FROM conversations WHERE id = ?1"
        ).context("Failed to prepare conversation query")?;

        let conversation = stmt.query_row(params![id], |row| {
            let created_str: String = row.get(3)?;
            let updated_str: String = row.get(4)?;
            Ok(Conversation {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        }).optional().context("Failed to query conversation")?;

        Ok(conversation)
    }
}
