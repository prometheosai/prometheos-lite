//! WorkContext event repository operations — Slice 1A provenanced journal.
//!
//! ONE writer path ([`record_event_conn`]): every journal insert carries
//! the complete typed provenance envelope, the canonical source-event
//! digest (over the ENTIRE writer-controlled record, not the payload
//! alone), and the flat identity columns derived from the envelope —
//! they cannot drift from it. Database triggers additionally enforce
//! complete provenance for new inserts and append-only journal rows.
//!
//! Reads re-verify the source digest against the stored record: any
//! mismatch (tampering, corruption) surfaces as an error, never as a
//! silently returned event. Legacy rows (pre-1A, honest NULL provenance)
//! surface as the native-only [`ProvenanceState::LegacyUnverified`]
//! state — never fabricated into an envelope.

use anyhow::Context;
use chrono::Utc;
use rusqlite::{Connection, params, types::Type};

use super::AsDb;
use crate::work::event::WorkContextEvent;
use crate::work::provenance::{ProvenanceEnvelope, compute_event_source_digest};

/// Parse the `data` JSON column fail-closed: corrupt durable rows are a
/// typed conversion error, never a fabricated `null` event payload.
fn parse_event_data(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<serde_json::Value> {
    let raw: String = row.get(col)?;
    serde_json::from_str(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, Type::Text, Box::new(e)))
}

/// Parse the `created_at` RFC 3339 column fail-closed: malformed durable
/// timestamps are a typed conversion error, never a panic.
fn parse_event_created_at(
    row: &rusqlite::Row<'_>,
    col: usize,
) -> rusqlite::Result<chrono::DateTime<Utc>> {
    let raw: String = row.get(col)?;
    chrono::DateTime::parse_from_rfc3339(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, Type::Text, Box::new(e)))
        .map(|dt| dt.with_timezone(&Utc))
}

/// The provenance state of one journal row as read from the store.
/// Legacy rows (written before Slice 1A) are surfaced as the native-only
/// `LegacyUnverified` state — they are not mapped into, or fabricated
/// as, envelope records.
#[derive(Debug, Clone, PartialEq)]
pub enum ProvenanceState {
    Verified(Box<ProvenanceEnvelope>),
    LegacyUnverified,
}

/// The stored row's raw and provenance columns, exactly as read by the
/// Slice 1A gate. The Slice 1B projection's source digest binds these
/// RAW bytes — the projection must see every byte the journal actually
/// stored, never a parsed/normalized reconstruction (review P1:
/// lexically different stored JSON or RFC 3339 text must produce a
/// different projection source digest).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredColumns {
    /// The RAW `data` TEXT as stored — byte-exact, never re-serialized
    /// from the parsed value. (Key order, whitespace, and number lexemes
    /// are binding.)
    pub data_text: String,
    /// The RAW `created_at` TEXT as stored — byte-exact (`Z` vs
    /// `+00:00` are DIFFERENT stored bytes and bind differently; the
    /// projected event's timestamp is this exact string).
    pub created_at_text: String,
    /// The canonical provenance bytes (parse-canonical verified at read).
    pub provenance_json: Option<String>,
    /// The row-level source digest (re-verified at read).
    pub source_digest: Option<String>,
    /// The derived flat identity columns (revalidated against the
    /// parsed envelope at read).
    pub run_id: Option<String>,
    pub principal_id: Option<String>,
    pub correlation_id: Option<String>,
}

/// One journal row with its verified provenance state.
#[derive(Debug, Clone)]
pub struct JournalRecord {
    pub seq: i64,
    pub event: WorkContextEvent,
    pub provenance: ProvenanceState,
    /// The complete stored row columns (Slice 1B digest binding).
    pub stored: StoredColumns,
}

/// The single journal writer (Slice 1A): records the event together with
/// its complete provenance. The envelope is serialized to canonical,
/// byte-validated JSON; the source digest covers the ENTIRE
/// writer-controlled record (`id`, `work_context_id`, `event_type`,
/// `data`, `created_at`, `provenance_json`); the flat identity columns
/// (`run_id`, `principal_id`, `correlation_id`) are derived from the
/// same envelope, so they always match it. Works on an arbitrary
/// Connection so callers inside transactions (iteration and completion
/// persistence) record provenance atomically with their effects.
pub fn record_event_conn(
    conn: &Connection,
    event: &WorkContextEvent,
    envelope: &ProvenanceEnvelope,
) -> anyhow::Result<()> {
    // P1 gap 1: enforce the canonical-bytes fixpoint and semantic
    // invariants at the write boundary — a malformed envelope can never
    // be stored, even from a caller that bypassed all upstream checks.
    envelope.validate_write_invariants()?;
    let provenance_json = envelope.to_canonical_json_string()?;
    let source_digest = compute_event_source_digest(
        &event.id,
        &event.work_context_id,
        &event.event_type,
        &event.data,
        &event.created_at.to_rfc3339(),
        &provenance_json,
    )?;

    conn.execute(
        "INSERT INTO work_context_events
             (id, work_context_id, event_type, data, created_at,
              provenance_json, source_digest, run_id, principal_id, correlation_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            &event.id,
            &event.work_context_id,
            &event.event_type,
            &serde_json::to_string(&event.data)?,
            &event.created_at.to_rfc3339(),
            &provenance_json,
            &source_digest,
            &envelope.run_query_key(),
            &envelope.principal_query_key(),
            &envelope.correlation_query_key(),
        ],
    )
    .context("Failed to insert work context event")?;
    Ok(())
}

/// Read one journal page with provenance verification. Rows written under
/// Slice 1A are verified: the source digest is re-computed over the
/// stored record and must match — a mismatch is tamper/corruption and
/// surfaces as an error, never as a returned event. Legacy rows surface
/// as [`ProvenanceState::LegacyUnverified`]. `after_seq = 0` reads from
/// the beginning; rows are ordered by durable `seq` ASC with `seq` as
/// the cursor.
pub fn read_journal_records_conn(
    conn: &Connection,
    work_context_id: &str,
    after_seq: i64,
    limit: i64,
) -> anyhow::Result<Vec<JournalRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, id, work_context_id, event_type, data, created_at,
                    provenance_json, source_digest, run_id, principal_id, correlation_id
             FROM work_context_events
             WHERE work_context_id = ?1 AND seq > ?2
             ORDER BY seq ASC
             LIMIT ?3",
        )
        .context("Failed to prepare journal records query")?;

    struct RawRow {
        seq: i64,
        event: WorkContextEvent,
        data_text: String,
        created_at_text: String,
        provenance_json: Option<String>,
        source_digest: Option<String>,
        stored_run_id: Option<String>,
        stored_principal_id: Option<String>,
        stored_correlation_id: Option<String>,
    }

    let rows = stmt
        .query_map(params![work_context_id, after_seq, limit], |row| {
            Ok(RawRow {
                seq: row.get(0)?,
                event: WorkContextEvent {
                    id: row.get(1)?,
                    work_context_id: row.get(2)?,
                    event_type: row.get(3)?,
                    data: parse_event_data(row, 4)?,
                    created_at: parse_event_created_at(row, 5)?,
                },
                data_text: row.get(4)?,
                created_at_text: row.get(5)?,
                provenance_json: row.get(6)?,
                source_digest: row.get(7)?,
                stored_run_id: row.get(8)?,
                stored_principal_id: row.get(9)?,
                stored_correlation_id: row.get(10)?,
            })
        })
        .context("Failed to query journal records")?;

    let mut records = Vec::new();
    for raw in rows {
        let raw = raw.context("Failed to parse journal record")?;
        let stored = StoredColumns {
            data_text: raw.data_text,
            created_at_text: raw.created_at_text,
            provenance_json: raw.provenance_json.clone(),
            source_digest: raw.source_digest.clone(),
            run_id: raw.stored_run_id.clone(),
            principal_id: raw.stored_principal_id.clone(),
            correlation_id: raw.stored_correlation_id.clone(),
        };
        let provenance = match (raw.provenance_json, raw.source_digest) {
            (Some(envelope_json), Some(stored_digest)) => {
                let envelope =
                    ProvenanceEnvelope::parse_canonical(&envelope_json).with_context(|| {
                        format!("corrupt provenance envelope for event {}", raw.event.id)
                    })?;
                let recomputed = compute_event_source_digest(
                    &raw.event.id,
                    &raw.event.work_context_id,
                    &raw.event.event_type,
                    &raw.event.data,
                    &raw.event.created_at.to_rfc3339(),
                    &envelope_json,
                )
                .context("stored source digest unavailable on read")?;
                if recomputed != stored_digest {
                    anyhow::bail!(
                        "journal event {} failed source-digest verification: stored {} != recomputed {} (tamper or corruption)",
                        raw.event.id,
                        stored_digest,
                        recomputed
                    );
                }
                // P1-2 repair: the derived flat identity columns are
                // revalidated against the parsed envelope on every read —
                // a mismatch is column drift (tamper or corruption) and
                // fails closed, never a silently returned event.
                let expected_run_id = envelope.run_query_key();
                if raw.stored_run_id.as_deref() != Some(expected_run_id.as_str()) {
                    anyhow::bail!(
                        "journal event {} run_id column drift: stored {:?} != envelope {:?}",
                        raw.event.id,
                        raw.stored_run_id,
                        expected_run_id
                    );
                }
                let expected_principal = envelope.principal_query_key();
                if raw.stored_principal_id != expected_principal {
                    anyhow::bail!(
                        "journal event {} principal_id column drift: stored {:?} != envelope {:?}",
                        raw.event.id,
                        raw.stored_principal_id,
                        expected_principal
                    );
                }
                let expected_correlation = envelope.correlation_query_key();
                if raw.stored_correlation_id.as_deref() != Some(expected_correlation.as_str()) {
                    anyhow::bail!(
                        "journal event {} correlation_id column drift: stored {:?} != envelope {:?}",
                        raw.event.id,
                        raw.stored_correlation_id,
                        expected_correlation
                    );
                }
                ProvenanceState::Verified(Box::new(envelope))
            }
            (None, None) => {
                // P1 gap 5 repair: a row with NULL provenance_json and
                // NULL source_digest is only a legitimate legacy row if
                // EVERY derived identity column is also NULL. Any
                // non-NULL derived column with a NULL envelope is a
                // mixed state — neither a clean legacy row nor a clean
                // provenanced row — and must be refused rather than
                // silently classified as LegacyUnverified.
                if raw.stored_run_id.is_some()
                    || raw.stored_principal_id.is_some()
                    || raw.stored_correlation_id.is_some()
                {
                    anyhow::bail!(
                        "journal event {} has a mixed provenance state: NULL envelope with non-NULL derived columns (run_id: {:?}, principal_id: {:?}, correlation_id: {:?}) — neither legacy nor provenanced",
                        raw.event.id,
                        raw.stored_run_id,
                        raw.stored_principal_id,
                        raw.stored_correlation_id
                    );
                }
                ProvenanceState::LegacyUnverified
            }
            // A row with only one of the two provenance columns is
            // neither a complete Slice 1A record nor an untouched legacy
            // row — refuse it rather than guess.
            (envelope_json, digest) => anyhow::bail!(
                "journal event {} has a partial provenance record (envelope: {}, digest: {})",
                raw.event.id,
                envelope_json.is_some(),
                digest.is_some()
            ),
        };
        records.push(JournalRecord {
            seq: raw.seq,
            event: raw.event,
            provenance,
            stored,
        });
    }
    Ok(records)
}

/// Look up the durable cancellation event for a context — the EXACT
/// parent that `execution_interrupted` evidence must reference. Because
/// `cancel_context` is idempotent (exactly one `context_cancelled` event
/// per context, written transactionally), this lookup is deterministic:
/// it asserts there is EXACTLY ONE and returns it. Zero is an honest
/// absence; more than one is a durability violation surfaced as an
/// error — never a "latest" inference that could pick the wrong parent.
pub fn cancellation_event_id_conn(
    conn: &Connection,
    work_context_id: &str,
) -> anyhow::Result<Option<String>> {
    let (count, event_id): (i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*), MAX(id) FROM work_context_events
             WHERE work_context_id = ?1 AND event_type = 'context_cancelled'",
            params![work_context_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("Failed to look up the durable cancellation event")?;
    match count {
        0 => Ok(None),
        1 => Ok(event_id),
        n => anyhow::bail!(
            "durability violation: {n} context_cancelled events for work context {work_context_id} (exactly one is guaranteed)"
        ),
    }
}

/// WorkContext event operations trait. All reads verify source digests
/// (Slice 1A); the unprovenanced `create_event` is gone — the single
/// writer is [`record_event_conn`], which requires the provenance
/// envelope (and the database triggers refuse any insert without it).
pub trait WorkContextEventOperations {
    fn get_events_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<WorkContextEvent>>;
    /// Cursorable read: return up to `limit` events for a context whose
    /// durable `seq` is strictly greater than `after_seq`, ordered by
    /// seq ASC.
    ///
    /// Each row is returned together with its `seq` — the `seq` IS the
    /// cursor. `seq` is an `INTEGER PRIMARY KEY AUTOINCREMENT` column:
    /// SQLite assigns it from `sqlite_sequence`, so it is strictly
    /// monotonic in insertion order, never reuses deleted maxima, and is
    /// stable across VACUUM and rebuilds. Iterating with
    /// `after_seq = <last returned seq>` therefore yields every event
    /// exactly once, with no gaps and no duplication, even after
    /// maintenance or deletions elsewhere in the table.
    fn get_events_for_context_after(
        &self,
        work_context_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, WorkContextEvent)>>;
    /// Provenanced read: journal records with their verified provenance
    /// state (legacy rows surface as `LegacyUnverified`).
    fn get_journal_records_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<JournalRecord>>;
}

impl<T: AsDb> WorkContextEventOperations for T {
    fn get_events_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<WorkContextEvent>> {
        let conn = self.as_db().conn();
        let records = read_journal_records_conn(conn, work_context_id, 0, i64::MAX)?;
        Ok(records.into_iter().map(|record| record.event).collect())
    }

    fn get_events_for_context_after(
        &self,
        work_context_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, WorkContextEvent)>> {
        let conn = self.as_db().conn();
        let records = read_journal_records_conn(conn, work_context_id, after_seq, limit as i64)?;
        Ok(records
            .into_iter()
            .map(|record| (record.seq, record.event))
            .collect())
    }

    fn get_journal_records_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<JournalRecord>> {
        let conn = self.as_db().conn();
        read_journal_records_conn(conn, work_context_id, 0, i64::MAX)
    }
}
