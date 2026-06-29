//! Message operations

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::AsDb;
use crate::db::models::{CreateMessage, Message};

/// Message operations trait
pub trait MessageOperations {
    fn create_message(&self, input: CreateMessage) -> anyhow::Result<Message>;
    fn get_messages(&self, conversation_id: &str) -> anyhow::Result<Vec<Message>>;
}

impl<T: AsDb> MessageOperations for T {
    fn create_message(&self, input: CreateMessage) -> anyhow::Result<Message> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, &input.conversation_id, &input.role, &input.content, &now.to_rfc3339()],
        ).context("Failed to insert message")?;

        Ok(Message {
            id,
            conversation_id: input.conversation_id,
            role: input.role,
            content: input.content,
            created_at: now,
        })
    }

    fn get_messages(&self, conversation_id: &str) -> anyhow::Result<Vec<Message>> {
        let conn = self.as_db().conn();
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content, created_at FROM messages WHERE conversation_id = ?1 ORDER BY created_at ASC"
        ).context("Failed to prepare messages query")?;

        let messages = stmt
            .query_map(params![conversation_id], |row| {
                let created_str: String = row.get(4)?;
                Ok(Message {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .context("Failed to query messages")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect messages")?;

        Ok(messages)
    }
}
