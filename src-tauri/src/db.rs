use crate::models::*;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

type V2CodeSalvageRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    i64,
    i64,
);
type V2CodingSalvageRow = (
    String,
    String,
    String,
    Option<i32>,
    Option<i32>,
    String,
    Option<String>,
    i64,
);
type AffectedCodedSegmentRow = (
    String,
    String,
    String,
    String,
    Option<i32>,
    Option<i32>,
    String,
);
type ExistingCodedSegmentRow = (String, String, String, Option<i32>, Option<i32>, String);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS project_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS codebook (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  definition TEXT,
  inclusion_criteria TEXT,
  exclusion_criteria TEXT,
  example TEXT,
  parent_id TEXT,
  color TEXT NOT NULL DEFAULT '#8a6410',
  sort_order INTEGER NOT NULL DEFAULT 0,
  is_retired INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS interviews (
  id TEXT PRIMARY KEY,
  participant_label TEXT NOT NULL,
  interview_date TEXT,
  modality TEXT,
  diagnosis_notes TEXT,
  interviewers TEXT NOT NULL DEFAULT '[]',
  hub_memo TEXT,
  audio_path TEXT,
  raw_vtt_path TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transcript_segments (
  id TEXT PRIMARY KEY,
  interview_id TEXT NOT NULL,
  segment_index INTEGER NOT NULL,
  speaker TEXT NOT NULL,
  timestamp_start TEXT NOT NULL,
  timestamp_end TEXT,
  text TEXT NOT NULL,
  block_id TEXT,
  section_tag TEXT,
  is_substantive INTEGER NOT NULL DEFAULT 1,
  FOREIGN KEY (interview_id) REFERENCES interviews(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS coded_segments (
  id TEXT PRIMARY KEY,
  interview_id TEXT NOT NULL,
  segment_id TEXT NOT NULL,
  code_ids TEXT NOT NULL,
  coder_name TEXT NOT NULL,
  memo TEXT,
  char_start INTEGER,
  char_end INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (interview_id) REFERENCES interviews(id) ON DELETE CASCADE,
  FOREIGN KEY (segment_id) REFERENCES transcript_segments(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS coder_activity_log (
  id TEXT PRIMARY KEY,
  coder_name TEXT NOT NULL,
  action TEXT NOT NULL,
  detail TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_memos (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
";

use std::time::SystemTime;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn system_time_to_iso(t: SystemTime) -> String {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    run_migrations(conn)
}

/// Schema changes, applied in order. An entry's index plus one is the
/// `PRAGMA user_version` a database carries once that entry has run.
///
/// 🔴 **`SCHEMA` above is the v0 baseline and must not gain another column.**
/// It runs as `CREATE TABLE IF NOT EXISTS`, which is a no-op against a database
/// that already exists — so a column added there reaches newly created projects
/// only and silently skips every project already on disk. Every schema change
/// from here on goes in this array instead, which is the only path that touches
/// both. Tested by `migrations_bring_a_v0_database_up_to_date`.
const MIGRATIONS: &[&str] = &[
    // 1 — sync bookkeeping.
    //
    // `sync_dirty` defaults to 1 so that rows written before sync existed are
    // treated as pending upload rather than as already-synced. `deleted` makes
    // removal a tombstone: a hard DELETE cannot propagate, because the other
    // machine has no way to tell "this row was removed" from "I have not seen
    // this row yet" and simply re-inserts it on the next pull.
    "ALTER TABLE coded_segments ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE coded_segments ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE coded_segments ADD COLUMN sync_dirty INTEGER NOT NULL DEFAULT 1;
     ALTER TABLE codebook ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE codebook ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE codebook ADD COLUMN sync_dirty INTEGER NOT NULL DEFAULT 1;
     CREATE TABLE IF NOT EXISTS sync_state (
       key TEXT PRIMARY KEY,
       value TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_coded_segments_dirty
       ON coded_segments(sync_dirty);
     CREATE INDEX IF NOT EXISTS idx_codebook_dirty
       ON codebook(sync_dirty);",
    // 2 — the interview roster syncs too.
    //
    // Not for the interview's own sake: transcripts never leave the machine, so
    // there is nothing about an interview worth transmitting except the fact
    // that it exists. That fact is what lets the app say "your coder has P07
    // and you do not" instead of leaving a coder to notice on their own, and
    // lets it check that the file they imported is the same one.
    // `remote_*` record what the other coder reports for this interview, so the
    // app can tell "you have not imported P07 yet" apart from "you imported a
    // different file for P07" — two situations with the same symptom and very
    // different fixes. They are the other machine's claim, never this one's.
    "ALTER TABLE interviews ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE interviews ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE interviews ADD COLUMN sync_dirty INTEGER NOT NULL DEFAULT 1;
     ALTER TABLE interviews ADD COLUMN remote_segment_count INTEGER;
     ALTER TABLE interviews ADD COLUMN remote_content_hash TEXT;
     CREATE INDEX IF NOT EXISTS idx_interviews_dirty
       ON interviews(sync_dirty);",
    // 3 — somewhere for coding to wait for its transcript.
    //
    // 🔴 This fixes a bug that made joining a study impossible. `coded_segments`
    // carries a foreign key to `transcript_segments`, and a joining coder pulls
    // the other coder's coding *before* importing any transcript — so every one
    // of those rows referenced a passage that did not exist locally yet. The
    // insert raised FOREIGN KEY constraint failed, and because the whole pull
    // applies in one transaction, the entire batch rolled back: no codebook, no
    // interview roster, nothing. The design comment in `sync::plan_corpus`
    // asserted these rows would "sit inert until the matching transcript is
    // imported"; they could not, and nothing tested the claim.
    //
    // They sit here instead, with no foreign key to violate, and are drained
    // into `coded_segments` by `import_segments` the moment the passages they
    // name exist. Deterministic ids are what make that work: the coding and the
    // transcript find each other with no matching step and no user action.
    "CREATE TABLE IF NOT EXISTS pending_coded_segments (
       id TEXT PRIMARY KEY,
       interview_id TEXT NOT NULL,
       segment_id TEXT NOT NULL,
       code_ids TEXT NOT NULL,
       coder_name TEXT NOT NULL,
       revision INTEGER NOT NULL DEFAULT 0,
       deleted INTEGER NOT NULL DEFAULT 0,
       updated_at TEXT NOT NULL,
       char_start INTEGER,
       char_end INTEGER
     );
     CREATE INDEX IF NOT EXISTS idx_pending_coded_segment
       ON pending_coded_segments(segment_id);",
    // 4 — span is part of the coded-row identity, locally as well as on the
    // server. Un-coding used to null `char_start`/`char_end`, which moved a
    // highlight onto the whole-turn unique slot (`*:*`) and 409'd every push;
    // SQLite also had no unique index, so those flattened tombstones stacked.
    // Collapse existing duplicates (live over tombstone, then highest
    // revision, then oldest row), copy a stranded memo onto the keeper, then
    // enforce the same natural key the server does.
    "UPDATE coded_segments
        SET memo = (
          SELECT e.memo FROM coded_segments e
           WHERE e.segment_id = coded_segments.segment_id
             AND e.coder_name = coded_segments.coder_name
             AND ifnull(e.char_start, -1) = ifnull(coded_segments.char_start, -1)
             AND ifnull(e.char_end, -1) = ifnull(coded_segments.char_end, -1)
             AND e.memo IS NOT NULL AND length(e.memo) > 0
             AND e.id != coded_segments.id
           LIMIT 1
        )
      WHERE (memo IS NULL OR memo = '')
        AND id IN (
          SELECT id FROM (
            SELECT id,
                   row_number() OVER (
                     PARTITION BY segment_id, coder_name,
                                  ifnull(char_start, -1), ifnull(char_end, -1)
                     ORDER BY deleted ASC, revision DESC, created_at ASC
                   ) AS rn
              FROM coded_segments
          ) WHERE rn = 1
        );
     DELETE FROM coded_segments
      WHERE id NOT IN (
        SELECT id FROM (
          SELECT id,
                 row_number() OVER (
                   PARTITION BY segment_id, coder_name,
                                ifnull(char_start, -1), ifnull(char_end, -1)
                   ORDER BY deleted ASC, revision DESC, created_at ASC
                 ) AS rn
            FROM coded_segments
        ) WHERE rn = 1
      );
     CREATE UNIQUE INDEX IF NOT EXISTS idx_coded_segments_span
       ON coded_segments(
         segment_id, coder_name, ifnull(char_start, -1), ifnull(char_end, -1)
       );",
    // 5 — base_code_ids for three-way merge on coded_segments.
    //
    // Records the code set as of the last successful sync of that row.
    // Existing clean rows get base_code_ids = code_ids; dirty rows get NULL
    // (meaning no known base ancestor, falling back to union merge).
    "ALTER TABLE coded_segments ADD COLUMN base_code_ids TEXT;
     UPDATE coded_segments SET base_code_ids = code_ids WHERE sync_dirty = 0;",
    "CREATE TABLE IF NOT EXISTS sync_outbox (
       op_id TEXT PRIMARY KEY,
       project_id TEXT NOT NULL,
       generation TEXT,
       device_id TEXT NOT NULL,
       client_seq INTEGER NOT NULL,
       entity_type TEXT NOT NULL,
       entity_id TEXT NOT NULL,
       op_kind TEXT NOT NULL,
       payload_json TEXT NOT NULL,
       base_versions_json TEXT NOT NULL DEFAULT '{}',
       created_at TEXT NOT NULL,
       attempt_count INTEGER NOT NULL DEFAULT 0,
       last_attempt_at TEXT,
       last_error TEXT,
       blocked_reason TEXT,
       UNIQUE(device_id, client_seq)
     );
     CREATE INDEX IF NOT EXISTS idx_sync_outbox_project_created
       ON sync_outbox(project_id, created_at);
     CREATE INDEX IF NOT EXISTS idx_sync_outbox_blocked_reason
       ON sync_outbox(blocked_reason);
     CREATE TABLE IF NOT EXISTS coding_assignments (
       interview_id TEXT NOT NULL,
       segment_id TEXT NOT NULL,
       actor_key TEXT NOT NULL,
       span_key TEXT NOT NULL,
       code_id TEXT NOT NULL,
       char_start INTEGER,
       char_end INTEGER,
       present INTEGER NOT NULL DEFAULT 1,
       server_seq INTEGER,
       created_at TEXT NOT NULL,
       updated_at TEXT NOT NULL,
       PRIMARY KEY(interview_id, segment_id, actor_key, span_key, code_id)
     );
     CREATE INDEX IF NOT EXISTS idx_coding_assignments_segment
       ON coding_assignments(interview_id, segment_id, present);
     CREATE INDEX IF NOT EXISTS idx_coding_assignments_code
       ON coding_assignments(code_id, present);
     CREATE TABLE IF NOT EXISTS sync_applied_changes (
       project_id TEXT NOT NULL,
       generation TEXT NOT NULL,
       server_seq INTEGER NOT NULL,
       op_id TEXT NOT NULL UNIQUE,
       applied_at TEXT NOT NULL,
       PRIMARY KEY(project_id, generation, server_seq)
     );
     CREATE TABLE IF NOT EXISTS sync_conflicts (
       conflict_id TEXT PRIMARY KEY,
       project_id TEXT NOT NULL,
       generation TEXT NOT NULL,
       entity_type TEXT NOT NULL,
       entity_id TEXT NOT NULL,
       field_name TEXT NOT NULL,
       current_json TEXT NOT NULL,
       proposed_json TEXT NOT NULL,
       base_json TEXT,
       originating_op_id TEXT NOT NULL,
       status TEXT NOT NULL DEFAULT 'unresolved',
       created_seq INTEGER,
       resolved_seq INTEGER,
       created_at TEXT NOT NULL,
       resolved_at TEXT
     );
     CREATE INDEX IF NOT EXISTS idx_sync_conflicts_project_status
       ON sync_conflicts(project_id, status, created_at);
     INSERT OR IGNORE INTO sync_state(key, value) VALUES
       ('sync_protocol', '1'),
       ('sync_generation', ''),
       ('sync_device_id', ''),
       ('sync_actor_key', ''),
       ('sync_client_seq', '0'),
       ('sync_server_seq', '0'),
       ('sync_observed_head', '0'),
       ('sync_last_realtime_at', ''),
       ('sync_last_success_at', '');
     INSERT OR IGNORE INTO coding_assignments
       (interview_id, segment_id, actor_key, span_key, code_id, char_start,
        char_end, present, server_seq, created_at, updated_at)
       SELECT cs.interview_id,
              cs.segment_id,
              'legacy:' || lower(hex(cs.coder_name)),
              CASE
                WHEN cs.char_start IS NULL OR cs.char_end IS NULL THEN 'whole'
                ELSE 'span:' || cs.char_start || ':' || cs.char_end
              END,
              CAST(json_each.value AS TEXT),
              cs.char_start,
              cs.char_end,
              1,
              NULL,
              cs.created_at,
              cs.updated_at
         FROM coded_segments cs
         JOIN json_each(CASE WHEN json_valid(cs.code_ids) THEN cs.code_ids ELSE '[]' END)
        WHERE cs.deleted = 0 AND typeof(json_each.value) = 'text';",
    "CREATE TABLE IF NOT EXISTS sync_field_versions (
       entity_type TEXT NOT NULL,
       entity_id TEXT NOT NULL,
       field_name TEXT NOT NULL,
       server_seq INTEGER NOT NULL,
       PRIMARY KEY(entity_type, entity_id, field_name)
     );
     CREATE INDEX IF NOT EXISTS idx_sync_field_versions_entity
       ON sync_field_versions(entity_type, entity_id);
     CREATE TABLE IF NOT EXISTS sync_repair_state (
       key TEXT PRIMARY KEY,
       value TEXT NOT NULL
     );",
    "ALTER TABLE sync_conflicts ADD COLUMN proposer_label TEXT;",
    // 9 — local review marks on transcript segments.
    "CREATE TABLE segment_reviews (
       segment_id TEXT PRIMARY KEY
         REFERENCES transcript_segments(id) ON DELETE CASCADE,
       updated_at TEXT NOT NULL
     );",
];

/// Bring a project database up to the current schema version.
///
/// Idempotent: each entry runs at most once per database, inside its own
/// transaction, and `user_version` only advances if the whole entry succeeded.
fn run_migrations_with(conn: &Connection, migrations: &[&str]) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in migrations.iter().enumerate() {
        let version = (index + 1) as i64;
        if version <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        // PRAGMA does not take a bound parameter, and `version` is derived from
        // this array's own length rather than from anything a caller supplies.
        tx.execute_batch(&format!("PRAGMA user_version = {version};"))?;
        tx.commit()?;
    }

    Ok(())
}

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    run_migrations_with(conn, MIGRATIONS)
}

pub fn sync_state_value(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM sync_state WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn set_sync_state_value(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_state(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn local_sync_protocol(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(sync_state_value(conn, "sync_protocol")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1))
}

#[derive(Debug, Clone)]
pub struct V2LocalState {
    pub project_id: String,
    pub device_id: String,
    pub generation: Option<String>,
    pub protocol: i64,
    pub server_seq: i64,
    pub observed_head: i64,
    pub actor_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct V2OutboxOperation {
    pub op_id: String,
    pub project_id: String,
    pub generation: String,
    pub device_id: String,
    pub client_seq: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub op_kind: String,
    pub payload: serde_json::Value,
    pub base_field_versions: serde_json::Value,
}

fn sync_state_i64(conn: &Connection, key: &str) -> rusqlite::Result<i64> {
    Ok(sync_state_value(conn, key)?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0))
}

pub fn v2_local_state(conn: &Connection) -> rusqlite::Result<V2LocalState> {
    let project_id = sync_state_value(conn, "project_id")?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Missing project_id".into()))?;
    let device_id = sync_state_value(conn, "sync_device_id")?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Missing sync_device_id".into()))?;
    Ok(V2LocalState {
        project_id,
        device_id,
        generation: sync_state_value(conn, "sync_generation")?.filter(|value| !value.is_empty()),
        protocol: local_sync_protocol(conn)?,
        server_seq: sync_state_i64(conn, "sync_server_seq")?,
        observed_head: sync_state_i64(conn, "sync_observed_head")?,
        actor_key: sync_state_value(conn, "sync_actor_key")?.filter(|value| !value.is_empty()),
    })
}

pub fn v2_legacy_pending_count(conn: &Connection) -> rusqlite::Result<i64> {
    let dirty: i64 = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM codebook WHERE sync_dirty != 0) +
           (SELECT COUNT(*) FROM interviews WHERE sync_dirty != 0) +
           (SELECT COUNT(*) FROM coded_segments WHERE sync_dirty != 0) +
           (SELECT COUNT(*) FROM pending_coded_segments WHERE deleted = 0)",
        [],
        |row| row.get(0),
    )?;
    let has_completed_v1_pull =
        sync_state_value(conn, "last_synced_at")?.is_some_and(|value| !value.is_empty());
    Ok(dirty + i64::from(!has_completed_v1_pull))
}

fn update_v2_lag_state_tx(
    tx: &Transaction<'_>,
    local_sequence: i64,
    observed_head: i64,
) -> rusqlite::Result<()> {
    if observed_head > local_sequence {
        let existing = sync_state_value(tx, "sync_lag_observed_at")?;
        if existing.is_none_or(|value| value.is_empty()) {
            sync_state_set_tx(tx, "sync_lag_observed_at", &now_iso())?;
        }
    } else {
        sync_state_set_tx(tx, "sync_lag_observed_at", "")?;
    }
    Ok(())
}

pub fn note_v2_realtime_hint(
    conn: &Connection,
    generation: Option<&str>,
    head: Option<i64>,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    sync_state_set_tx(&tx, "sync_last_realtime_at", &now_iso())?;
    if local_sync_protocol(&tx)? == 2 {
        let local_generation = sync_state_value(&tx, "sync_generation")?;
        let generation_matches = generation.is_none_or(|hint| {
            local_generation
                .as_deref()
                .is_some_and(|local| local == hint)
        });
        if generation_matches {
            if let Some(head) = head.filter(|value| *value >= 0) {
                let local_sequence = sync_state_value(&tx, "sync_server_seq")?
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0);
                let observed = sync_state_value(&tx, "sync_observed_head")?
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0)
                    .max(head);
                sync_state_set_tx(&tx, "sync_observed_head", &observed.to_string())?;
                update_v2_lag_state_tx(&tx, local_sequence, observed)?;
            }
        }
    }
    tx.commit()
}

pub fn record_v2_registration(
    conn: &Connection,
    protocol: i64,
    generation: Option<&str>,
    head: i64,
    actor_key: Option<&str>,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO sync_state(key, value) VALUES ('sync_protocol', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![protocol.to_string()],
    )?;
    if protocol == 2 {
        let generation = generation
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Protocol 2 requires a generation".into())
            })?;
        tx.execute(
            "INSERT INTO sync_state(key, value) VALUES ('sync_generation', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![generation],
        )?;
        tx.execute(
            "INSERT INTO sync_state(key, value) VALUES ('sync_observed_head', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![head.to_string()],
        )?;
        let local_sequence = sync_state_value(&tx, "sync_server_seq")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        update_v2_lag_state_tx(&tx, local_sequence, head)?;
        if let Some(actor_key) = actor_key.filter(|value| !value.is_empty()) {
            tx.execute(
                "INSERT INTO sync_state(key, value) VALUES ('sync_actor_key', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![actor_key],
            )?;
        }
    }
    tx.commit()
}

pub fn list_v2_outbox(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<V2OutboxOperation>> {
    let mut statement = conn.prepare(
        "SELECT op_id, project_id, generation, device_id, client_seq, entity_type, entity_id,
                op_kind, payload_json, base_versions_json
           FROM sync_outbox
          WHERE blocked_reason IS NULL
          ORDER BY client_seq
          LIMIT ?1",
    )?;
    let sqlite_limit = if limit == 0 { -1 } else { limit as i64 };
    let rows = statement.query_map(params![sqlite_limit], |row| {
        let payload_json: String = row.get(8)?;
        let base_versions_json: String = row.get(9)?;
        let payload = serde_json::from_str(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let base_field_versions = serde_json::from_str(&base_versions_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(V2OutboxOperation {
            op_id: row.get(0)?,
            project_id: row.get(1)?,
            generation: row.get(2)?,
            device_id: row.get(3)?,
            client_seq: row.get(4)?,
            entity_type: row.get(5)?,
            entity_id: row.get(6)?,
            op_kind: row.get(7)?,
            payload,
            base_field_versions,
        })
    })?;
    rows.collect()
}

pub fn note_v2_outbox_attempt(
    conn: &Connection,
    op_ids: &[String],
    public_error: Option<&str>,
) -> rusqlite::Result<()> {
    if op_ids.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let timestamp = now_iso();
    for op_id in op_ids {
        tx.execute(
            "UPDATE sync_outbox
                SET attempt_count = attempt_count + 1,
                    last_attempt_at = ?2,
                    last_error = ?3
              WHERE op_id = ?1",
            params![op_id, timestamp, public_error],
        )?;
    }
    tx.commit()
}

pub fn complete_v2_outbox_receipts(
    conn: &Connection,
    receipts: &[(String, String)],
    observed_head: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (op_id, status) in receipts {
        match status.as_str() {
            "applied" | "conflicted" => {
                tx.execute("DELETE FROM sync_outbox WHERE op_id = ?1", params![op_id])?;
            }
            "blocked" | "rejected" => {
                tx.execute(
                    "UPDATE sync_outbox
                        SET blocked_reason = ?2, last_error = ?2
                      WHERE op_id = ?1",
                    params![op_id, status],
                )?;
            }
            _ => {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "Unknown v2 receipt status: {status}"
                )));
            }
        }
    }
    tx.execute(
        "INSERT INTO sync_state(key, value) VALUES ('sync_observed_head', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![observed_head.to_string()],
    )?;
    let local_sequence = sync_state_value(&tx, "sync_server_seq")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    update_v2_lag_state_tx(&tx, local_sequence, observed_head)?;
    tx.commit()
}

#[derive(Debug, Clone)]
pub struct V2IncomingChange {
    pub seq: i64,
    pub op_id: String,
    pub actor_key: String,
    pub entity_type: String,
    pub entity_id: String,
    pub op_kind: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct V2IncomingConflict {
    pub conflict_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub field_name: String,
    pub current_value: serde_json::Value,
    pub proposed_value: serde_json::Value,
    pub base_value: Option<serde_json::Value>,
    pub originating_op_id: String,
    pub proposer_label: Option<String>,
    pub status: String,
    pub created_at: String,
    pub resolved_seq: Option<i64>,
}

fn sync_state_set_tx(tx: &Transaction<'_>, key: &str, value: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO sync_state(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn record_v2_field_version(
    tx: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    field_name: &str,
    server_seq: i64,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO sync_field_versions(entity_type, entity_id, field_name, server_seq)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(entity_type, entity_id, field_name) DO UPDATE
           SET server_seq = excluded.server_seq",
        params![entity_type, entity_id, field_name, server_seq],
    )?;
    Ok(())
}

fn value_string(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(serde_json::Value::as_str).map(str::to_owned)
}

fn payload_patch(
    value: &serde_json::Value,
) -> rusqlite::Result<&serde_json::Map<String, serde_json::Value>> {
    value
        .get("patch")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Invalid v2 patch payload".into()))
}

fn apply_v2_code_create(tx: &Transaction<'_>, change: &V2IncomingChange) -> rusqlite::Result<()> {
    let payload = change
        .payload
        .as_object()
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Invalid v2 code payload".into()))?;
    let name = value_string(payload.get("name")).ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("V2 code create is missing name".into())
    })?;
    tx.execute(
        "INSERT INTO codebook (
           id, name, definition, inclusion_criteria, exclusion_criteria, example, parent_id,
           color, sort_order, is_retired, created_at, updated_at, revision, deleted, sync_dirty
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, 0, ?12, 0)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           definition = excluded.definition,
           inclusion_criteria = excluded.inclusion_criteria,
           exclusion_criteria = excluded.exclusion_criteria,
           example = excluded.example,
           parent_id = excluded.parent_id,
           color = excluded.color,
           sort_order = excluded.sort_order,
           is_retired = excluded.is_retired,
           deleted = excluded.deleted,
           updated_at = excluded.updated_at,
           sync_dirty = 0",
        params![
            change.entity_id,
            name,
            value_string(payload.get("definition")),
            value_string(payload.get("inclusion_criteria")),
            value_string(payload.get("exclusion_criteria")),
            value_string(payload.get("example")),
            value_string(payload.get("parent_id")),
            value_string(payload.get("color")).unwrap_or_else(|| "#8a6410".into()),
            payload
                .get("sort_order")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            payload
                .get("is_retired")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false) as i32,
            change.created_at,
            payload
                .get("deleted")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false) as i32,
        ],
    )?;
    for field in [
        "name",
        "definition",
        "inclusion_criteria",
        "exclusion_criteria",
        "example",
        "parent_id",
        "color",
        "sort_order",
        "is_retired",
        "deleted",
    ] {
        record_v2_field_version(tx, "code", &change.entity_id, field, change.seq)?;
    }
    Ok(())
}

fn apply_v2_code_patch(tx: &Transaction<'_>, change: &V2IncomingChange) -> rusqlite::Result<()> {
    let patch = payload_patch(&change.payload)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM codebook WHERE id = ?1)",
        params![change.entity_id],
        |row| row.get::<_, i64>(0),
    )? == 0
    {
        return Ok(());
    }
    for (field, value) in patch {
        match field.as_str() {
            "name" => {
                if let Some(name) = value.as_str() {
                    tx.execute("UPDATE codebook SET name = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, name, change.created_at])?;
                }
            }
            "definition" | "inclusion_criteria" | "exclusion_criteria" | "example"
            | "parent_id" => {
                let column = match field.as_str() {
                    "definition" => "definition",
                    "inclusion_criteria" => "inclusion_criteria",
                    "exclusion_criteria" => "exclusion_criteria",
                    "example" => "example",
                    _ => "parent_id",
                };
                let sql = format!("UPDATE codebook SET {column} = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1");
                tx.execute(
                    &sql,
                    params![
                        change.entity_id,
                        value_string(Some(value)),
                        change.created_at
                    ],
                )?;
            }
            "color" => {
                if let Some(color) = value.as_str() {
                    tx.execute("UPDATE codebook SET color = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, color, change.created_at])?;
                }
            }
            "sort_order" => {
                if let Some(sort_order) = value.as_i64() {
                    tx.execute("UPDATE codebook SET sort_order = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, sort_order, change.created_at])?;
                }
            }
            "is_retired" => {
                if let Some(is_retired) = value.as_bool() {
                    tx.execute("UPDATE codebook SET is_retired = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, is_retired as i32, change.created_at])?;
                }
            }
            "deleted" => {
                if let Some(deleted) = value.as_bool() {
                    tx.execute("UPDATE codebook SET deleted = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, deleted as i32, change.created_at])?;
                }
            }
            _ => {
                return Err(rusqlite::Error::InvalidParameterName(
                    "Unknown v2 code field".into(),
                ))
            }
        }
        record_v2_field_version(tx, "code", &change.entity_id, field, change.seq)?;
    }
    Ok(())
}

fn apply_v2_interview_patch(
    tx: &Transaction<'_>,
    change: &V2IncomingChange,
) -> rusqlite::Result<()> {
    let patch = payload_patch(&change.payload)?;
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM interviews WHERE id = ?1)",
        params![change.entity_id],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !exists {
        let label =
            value_string(patch.get("study_label")).unwrap_or_else(|| change.entity_id.clone());
        tx.execute(
            "INSERT INTO interviews (
              id, participant_label, interviewers, created_at, updated_at, revision, deleted,
              sync_dirty, remote_segment_count, remote_content_hash
            ) VALUES (?1, ?2, '[]', ?3, ?3, 0, ?4, 0, ?5, ?6)",
            params![
                change.entity_id,
                label,
                change.created_at,
                patch
                    .get("deleted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false) as i32,
                patch
                    .get("segment_count")
                    .and_then(serde_json::Value::as_i64),
                value_string(patch.get("content_hash")),
            ],
        )?;
    } else {
        for (field, value) in patch {
            match field.as_str() {
                "study_label" => {
                    if let Some(label) = value.as_str() {
                        tx.execute("UPDATE interviews SET participant_label = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, label, change.created_at])?;
                    }
                }
                "segment_count" => {
                    tx.execute("UPDATE interviews SET remote_segment_count = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, value.as_i64(), change.created_at])?;
                }
                "content_hash" => {
                    tx.execute("UPDATE interviews SET remote_content_hash = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, value_string(Some(value)), change.created_at])?;
                }
                "deleted" => {
                    if let Some(deleted) = value.as_bool() {
                        tx.execute("UPDATE interviews SET deleted = ?2, updated_at = ?3, sync_dirty = 0 WHERE id = ?1", params![change.entity_id, deleted as i32, change.created_at])?;
                    }
                }
                _ => {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "Unknown v2 interview field".into(),
                    ))
                }
            }
        }
    }
    for field in patch.keys() {
        record_v2_field_version(tx, "interview", &change.entity_id, field, change.seq)?;
    }
    Ok(())
}

fn materialize_v2_assignment_projection(
    tx: &Transaction<'_>,
    interview_id: &str,
    segment_id: &str,
    actor_key: &str,
    char_start: Option<i32>,
    char_end: Option<i32>,
    timestamp: &str,
) -> rusqlite::Result<()> {
    let segment_exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM transcript_segments WHERE id = ?1 AND interview_id = ?2)",
        params![segment_id, interview_id],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !segment_exists {
        return Ok(());
    }
    let code_ids = {
        let mut statement = tx.prepare(
            "SELECT code_id FROM coding_assignments
              WHERE interview_id = ?1 AND segment_id = ?2 AND actor_key = ?3
                AND span_key = ?4 AND present != 0 ORDER BY code_id",
        )?;
        let rows = statement
            .query_map(
                params![
                    interview_id,
                    segment_id,
                    actor_key,
                    assignment_span_key(char_start, char_end),
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let local_actor = sync_state_value(tx, "sync_actor_key")?;
    let display_actor = if local_actor.as_deref() == Some(actor_key) {
        sync_state_value(tx, crate::sync::KEY_CODER_NAME)?.unwrap_or_else(|| actor_key.to_string())
    } else {
        actor_key.to_string()
    };
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM coded_segments WHERE segment_id = ?1 AND coder_name = ?2
              AND char_start IS ?3 AND char_end IS ?4 ORDER BY created_at LIMIT 1",
            params![segment_id, display_actor, char_start, char_end],
            |row| row.get(0),
        )
        .optional()?;
    if code_ids.is_empty() {
        if let Some(id) = existing {
            tx.execute(
                "UPDATE coded_segments SET deleted = 1, code_ids = '[]', sync_dirty = 0, updated_at = ?2 WHERE id = ?1",
                params![id, timestamp],
            )?;
        }
    } else if let Some(id) = existing {
        tx.execute(
            "UPDATE coded_segments SET code_ids = ?2, deleted = 0, sync_dirty = 0, updated_at = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(&code_ids).unwrap_or_else(|_| "[]".into()), timestamp],
        )?;
    } else {
        tx.execute(
            "INSERT INTO coded_segments (
              id, interview_id, segment_id, code_ids, coder_name, char_start, char_end,
              created_at, updated_at, revision, deleted, sync_dirty
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, 0, 0)",
            params![
                Uuid::new_v4().to_string(),
                interview_id,
                segment_id,
                serde_json::to_string(&code_ids).unwrap_or_else(|_| "[]".into()),
                display_actor,
                char_start,
                char_end,
                timestamp,
            ],
        )?;
    }
    Ok(())
}

fn apply_v2_coding_patch(tx: &Transaction<'_>, change: &V2IncomingChange) -> rusqlite::Result<()> {
    let payload = change
        .payload
        .as_object()
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("Invalid v2 coding payload".into()))?;
    for (key, present) in [("adds", true), ("removes", false)] {
        let edges = payload
            .get(key)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Invalid v2 coding edges".into())
            })?;
        for edge in edges {
            let edge = edge.as_object().ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Invalid v2 coding edge".into())
            })?;
            let interview_id = value_string(edge.get("interview_id")).ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "V2 coding edge is missing interview_id".into(),
                )
            })?;
            let segment_id = value_string(edge.get("segment_id")).ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("V2 coding edge is missing segment_id".into())
            })?;
            let code_id = value_string(edge.get("code_id")).ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("V2 coding edge is missing code_id".into())
            })?;
            let char_start = edge
                .get("char_start")
                .and_then(serde_json::Value::as_i64)
                .map(|n| n as i32);
            let char_end = edge
                .get("char_end")
                .and_then(serde_json::Value::as_i64)
                .map(|n| n as i32);
            let span_key = assignment_span_key(char_start, char_end);
            tx.execute(
                "INSERT INTO coding_assignments (
                   interview_id, segment_id, actor_key, span_key, code_id, char_start, char_end,
                   present, server_seq, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                 ON CONFLICT(interview_id, segment_id, actor_key, span_key, code_id) DO UPDATE SET
                   present = excluded.present, server_seq = excluded.server_seq, updated_at = excluded.updated_at",
                params![
                    interview_id,
                    segment_id,
                    change.actor_key,
                    span_key,
                    code_id,
                    char_start,
                    char_end,
                    present as i32,
                    change.seq,
                    change.created_at,
                ],
            )?;
            materialize_v2_assignment_projection(
                tx,
                &interview_id,
                &segment_id,
                &change.actor_key,
                char_start,
                char_end,
                &change.created_at,
            )?;
        }
    }
    Ok(())
}

fn apply_v2_change(tx: &Transaction<'_>, change: &V2IncomingChange) -> rusqlite::Result<()> {
    match (change.entity_type.as_str(), change.op_kind.as_str()) {
        ("code", "code.create") => apply_v2_code_create(tx, change),
        ("code", "code.patch") => apply_v2_code_patch(tx, change),
        ("code", "code.retire") => {
            tx.execute(
                "UPDATE codebook SET is_retired = 1, sync_dirty = 0, updated_at = ?2 WHERE id = ?1",
                params![change.entity_id, change.created_at],
            )?;
            record_v2_field_version(tx, "code", &change.entity_id, "is_retired", change.seq)
        }
        ("code", "code.purge") => {
            tx.execute(
                "UPDATE codebook SET deleted = 1, sync_dirty = 0, updated_at = ?2 WHERE id = ?1",
                params![change.entity_id, change.created_at],
            )?;
            tx.execute(
                "UPDATE coding_assignments SET present = 0, server_seq = ?2, updated_at = ?3
                  WHERE code_id = ?1 AND present != 0",
                params![change.entity_id, change.seq, change.created_at],
            )?;
            record_v2_field_version(tx, "code", &change.entity_id, "deleted", change.seq)
        }
        ("coding", "coding.patch") => apply_v2_coding_patch(tx, change),
        ("interview", "interview.patch") => apply_v2_interview_patch(tx, change),
        ("conflict", "conflict.resolve") => Ok(()),
        _ => Err(rusqlite::Error::InvalidParameterName(
            "Unsupported v2 change".into(),
        )),
    }
}

fn project_v2_change_through_conflicts(
    change: &V2IncomingChange,
    conflicts: &[V2IncomingConflict],
) -> Option<V2IncomingChange> {
    let relevant = conflicts
        .iter()
        .filter(|conflict| conflict.originating_op_id == change.op_id)
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return Some(change.clone());
    }
    match (change.entity_type.as_str(), change.op_kind.as_str()) {
        ("code", "code.patch") | ("interview", "interview.patch") => {
            let mut projected = change.clone();
            let patch = projected.payload.get_mut("patch")?.as_object_mut()?;
            for conflict in relevant {
                patch.remove(&conflict.field_name);
            }
            (!patch.is_empty()).then_some(projected)
        }
        ("code", "code.create") | ("code", "code.retire") | ("code", "code.purge") => {
            let blocks_entity = relevant.iter().any(|conflict| {
                matches!(
                    conflict.field_name.as_str(),
                    "__entity__" | "name" | "deleted"
                )
            });
            (!blocks_entity).then_some(change.clone())
        }
        ("coding", "coding.patch") => {
            let conflicted_code_ids = relevant
                .iter()
                .filter_map(|conflict| {
                    conflict
                        .proposed_value
                        .get("code_id")
                        .and_then(serde_json::Value::as_str)
                })
                .collect::<HashSet<_>>();
            let mut projected = change.clone();
            let payload = projected.payload.as_object_mut()?;
            for key in ["adds", "removes"] {
                let edges = payload.get_mut(key)?.as_array_mut()?;
                edges.retain(|edge| {
                    edge.get("code_id")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|code_id| !conflicted_code_ids.contains(code_id))
                });
            }
            let has_edges = ["adds", "removes"].into_iter().any(|key| {
                projected
                    .payload
                    .get(key)
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|edges| !edges.is_empty())
            });
            has_edges.then_some(projected)
        }
        _ => Some(change.clone()),
    }
}

fn apply_v2_conflict_resolution(
    tx: &Transaction<'_>,
    change: &V2IncomingChange,
    conflicts: &[V2IncomingConflict],
) -> rusqlite::Result<()> {
    let conflict_id = value_string(change.payload.get("conflict_id")).ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(
            "V2 conflict resolution is missing conflict_id".into(),
        )
    })?;
    let resolution = value_string(change.payload.get("resolution")).ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("V2 conflict resolution is missing resolution".into())
    })?;
    if resolution == "keep_current" {
        return Ok(());
    }
    let Some(conflict) = conflicts
        .iter()
        .find(|conflict| conflict.conflict_id == conflict_id)
    else {
        return Ok(());
    };
    if conflict.status != "resolved" || conflict.field_name == "__entity__" {
        return Ok(());
    }
    let value = change
        .payload
        .get("value")
        .cloned()
        .filter(serde_json::Value::is_object)
        .ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("V2 conflict resolution has invalid value".into())
        })?;
    let creates_code =
        conflict.entity_type == "code" && conflict.field_name == "name" && resolution == "custom";
    let code_exists = if creates_code {
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM codebook WHERE id = ?1)",
            params![conflict.entity_id],
            |row| row.get::<_, i64>(0),
        )? != 0
    } else {
        false
    };
    let projected = V2IncomingChange {
        seq: change.seq,
        op_id: change.op_id.clone(),
        actor_key: change.actor_key.clone(),
        entity_type: conflict.entity_type.clone(),
        entity_id: conflict.entity_id.clone(),
        op_kind: match conflict.entity_type.as_str() {
            "code" if creates_code => {
                if code_exists {
                    "code.patch".into()
                } else {
                    "code.create".into()
                }
            }
            "code" => "code.patch".into(),
            "interview" => "interview.patch".into(),
            _ => return Ok(()),
        },
        payload: if creates_code && !code_exists {
            value
        } else {
            serde_json::json!({ "patch": value })
        },
        created_at: change.created_at.clone(),
    };
    apply_v2_change(tx, &projected)
}

fn replace_v2_conflicts_tx(
    tx: &Transaction<'_>,
    project_id: &str,
    generation: &str,
    conflicts: &[V2IncomingConflict],
) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM sync_conflicts WHERE project_id = ?1 AND generation = ?2",
        params![project_id, generation],
    )?;
    for conflict in conflicts {
        tx.execute(
            "INSERT INTO sync_conflicts (
              conflict_id, project_id, generation, entity_type, entity_id, field_name,
              current_json, proposed_json, base_json, originating_op_id, proposer_label, status,
              created_seq, resolved_seq, created_at, resolved_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, NULL)",
            params![
                conflict.conflict_id,
                project_id,
                generation,
                conflict.entity_type,
                conflict.entity_id,
                conflict.field_name,
                serde_json::to_string(&conflict.current_value).unwrap_or_else(|_| "null".into()),
                serde_json::to_string(&conflict.proposed_value).unwrap_or_else(|_| "null".into()),
                conflict
                    .base_value
                    .as_ref()
                    .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".into())),
                conflict.originating_op_id,
                conflict.proposer_label,
                conflict.status,
                conflict.resolved_seq,
                conflict.created_at,
            ],
        )?;
    }
    Ok(())
}

pub fn apply_v2_pull_page(
    conn: &Connection,
    project_id: &str,
    generation: &str,
    expected_start: i64,
    head: i64,
    changes: &[V2IncomingChange],
    conflicts: &[V2IncomingConflict],
) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let local_sequence = sync_state_value(&tx, "sync_server_seq")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if local_sequence != expected_start {
        return Err(rusqlite::Error::InvalidParameterName(
            "V2 pull cursor changed before application".into(),
        ));
    }
    let mut expected = expected_start + 1;
    for change in changes {
        if change.seq != expected {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "V2 sequence gap: expected {expected}, got {}",
                change.seq
            )));
        }
        let existing_op: Option<i64> = tx
            .query_row(
                "SELECT server_seq FROM sync_applied_changes WHERE op_id = ?1",
                params![change.op_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing_op.is_some_and(|sequence| sequence != change.seq) {
            return Err(rusqlite::Error::InvalidParameterName(
                "V2 duplicate operation has a conflicting sequence".into(),
            ));
        }
        if existing_op.is_none() {
            if let Some(projected) = project_v2_change_through_conflicts(change, conflicts) {
                if projected.entity_type == "conflict" && projected.op_kind == "conflict.resolve" {
                    apply_v2_conflict_resolution(&tx, &projected, conflicts)?;
                } else {
                    apply_v2_change(&tx, &projected)?;
                }
            }
            tx.execute(
                "INSERT INTO sync_applied_changes(project_id, generation, server_seq, op_id, applied_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![project_id, generation, change.seq, change.op_id, change.created_at],
            )?;
        }
        expected += 1;
    }
    let advanced_to = expected - 1;
    if advanced_to > head {
        return Err(rusqlite::Error::InvalidParameterName(
            "V2 pull advanced beyond authoritative head".into(),
        ));
    }
    sync_state_set_tx(&tx, "sync_server_seq", &advanced_to.to_string())?;
    sync_state_set_tx(&tx, "sync_observed_head", &head.to_string())?;
    update_v2_lag_state_tx(&tx, advanced_to, head)?;
    replace_v2_conflicts_tx(&tx, project_id, generation, conflicts)?;
    tx.commit()?;
    Ok(advanced_to)
}

pub fn reset_v2_synced_projection(conn: &Connection) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM coding_assignments", [])?;
    tx.execute("DELETE FROM sync_field_versions", [])?;
    tx.execute("DELETE FROM sync_applied_changes", [])?;
    tx.execute("DELETE FROM sync_conflicts", [])?;
    tx.execute("DELETE FROM codebook", [])?;
    tx.execute("UPDATE interviews SET deleted = 1, sync_dirty = 0", [])?;
    tx.execute("UPDATE coded_segments SET deleted = 1, sync_dirty = 0", [])?;
    sync_state_set_tx(&tx, "sync_server_seq", "0")?;
    tx.commit()
}

fn restore_snapshot_field_versions(
    tx: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    value: &serde_json::Value,
) -> rusqlite::Result<()> {
    if let Some(versions) = value
        .get("field_versions")
        .and_then(serde_json::Value::as_object)
    {
        for (field, sequence) in versions {
            if let Some(sequence) = sequence.as_i64() {
                record_v2_field_version(tx, entity_type, entity_id, field, sequence)?;
            }
        }
    }
    Ok(())
}

// The server snapshot contract intentionally exposes each materialized collection
// separately; grouping it at this persistence boundary would obscure that protocol.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_v2_snapshot(
    conn: &Connection,
    project_id: &str,
    generation: &str,
    snapshot_seq: i64,
    codes: &[serde_json::Value],
    interviews: &[serde_json::Value],
    assignments: &[serde_json::Value],
    conflicts: &[V2IncomingConflict],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let preserved_outbox = list_v2_outbox(&tx, 0)?;
    tx.execute("DELETE FROM coding_assignments", [])?;
    tx.execute("DELETE FROM sync_field_versions", [])?;
    tx.execute("DELETE FROM sync_applied_changes", [])?;
    tx.execute("DELETE FROM sync_conflicts", [])?;
    tx.execute("DELETE FROM codebook", [])?;
    tx.execute("UPDATE interviews SET deleted = 1, sync_dirty = 0", [])?;
    tx.execute("UPDATE coded_segments SET deleted = 1, sync_dirty = 0", [])?;

    for value in codes {
        let mut payload = value.as_object().cloned().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("Invalid v2 snapshot code".into())
        })?;
        let entity_id = value_string(payload.get("id")).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("V2 snapshot code is missing id".into())
        })?;
        payload.remove("id");
        payload.remove("field_versions");
        payload.remove("tombstone_seq");
        let change = V2IncomingChange {
            seq: snapshot_seq,
            op_id: format!("snapshot:code:{entity_id}"),
            actor_key: "system:snapshot".into(),
            entity_type: "code".into(),
            entity_id: entity_id.clone(),
            op_kind: "code.create".into(),
            payload: serde_json::Value::Object(payload),
            created_at: now_iso(),
        };
        apply_v2_code_create(&tx, &change)?;
        restore_snapshot_field_versions(&tx, "code", &entity_id, value)?;
    }

    for value in interviews {
        let payload = value.as_object().cloned().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("Invalid v2 snapshot interview".into())
        })?;
        let entity_id = value_string(payload.get("id")).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("V2 snapshot interview is missing id".into())
        })?;
        let patch = serde_json::json!({
            "study_label": payload.get("study_label").cloned().unwrap_or(serde_json::Value::String(entity_id.clone())),
            "segment_count": payload.get("segment_count").cloned().unwrap_or(serde_json::Value::from(0)),
            "content_hash": payload.get("content_hash").cloned().unwrap_or(serde_json::Value::Null),
            "deleted": payload.get("deleted").cloned().unwrap_or(serde_json::Value::Bool(false)),
        });
        let change = V2IncomingChange {
            seq: snapshot_seq,
            op_id: format!("snapshot:interview:{entity_id}"),
            actor_key: "system:snapshot".into(),
            entity_type: "interview".into(),
            entity_id: entity_id.clone(),
            op_kind: "interview.patch".into(),
            payload: serde_json::json!({ "patch": patch }),
            created_at: now_iso(),
        };
        apply_v2_interview_patch(&tx, &change)?;
        restore_snapshot_field_versions(&tx, "interview", &entity_id, value)?;
    }

    for value in assignments {
        let assignment = value.as_object().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("Invalid v2 snapshot assignment".into())
        })?;
        let actor_key = value_string(assignment.get("actor_key")).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("V2 snapshot assignment is missing actor".into())
        })?;
        let present = assignment
            .get("present")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let edge = serde_json::json!({
            "interview_id": assignment.get("interview_id").cloned().unwrap_or(serde_json::Value::Null),
            "segment_id": assignment.get("segment_id").cloned().unwrap_or(serde_json::Value::Null),
            "code_id": assignment.get("code_id").cloned().unwrap_or(serde_json::Value::Null),
            "char_start": assignment.get("char_start").cloned().unwrap_or(serde_json::Value::Null),
            "char_end": assignment.get("char_end").cloned().unwrap_or(serde_json::Value::Null),
        });
        let change = V2IncomingChange {
            seq: assignment
                .get("version_seq")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(snapshot_seq),
            op_id: format!("snapshot:assignment:{}", Uuid::new_v4()),
            actor_key,
            entity_type: "coding".into(),
            entity_id: "snapshot".into(),
            op_kind: "coding.patch".into(),
            payload: if present {
                serde_json::json!({ "adds": [edge], "removes": [] })
            } else {
                serde_json::json!({ "adds": [], "removes": [edge] })
            },
            created_at: now_iso(),
        };
        apply_v2_coding_patch(&tx, &change)?;
    }

    for operation in preserved_outbox {
        let change = V2IncomingChange {
            seq: 0,
            op_id: operation.op_id,
            actor_key: sync_state_value(&tx, "sync_actor_key")?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "legacy:local".into()),
            entity_type: operation.entity_type,
            entity_id: operation.entity_id,
            op_kind: operation.op_kind,
            payload: operation.payload,
            created_at: now_iso(),
        };
        apply_v2_change(&tx, &change)?;
    }
    for value in codes {
        if let Some(entity_id) = value_string(value.get("id")) {
            restore_snapshot_field_versions(&tx, "code", &entity_id, value)?;
        }
    }
    for value in interviews {
        if let Some(entity_id) = value_string(value.get("id")) {
            restore_snapshot_field_versions(&tx, "interview", &entity_id, value)?;
        }
    }
    replace_v2_conflicts_tx(&tx, project_id, generation, conflicts)?;
    sync_state_set_tx(&tx, "sync_generation", generation)?;
    sync_state_set_tx(&tx, "sync_server_seq", &snapshot_seq.to_string())?;
    sync_state_set_tx(&tx, "sync_observed_head", &snapshot_seq.to_string())?;
    update_v2_lag_state_tx(&tx, snapshot_seq, snapshot_seq)?;
    let integrity: String = tx.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::InvalidParameterName(
            "SQLite integrity check failed after v2 repair".into(),
        ));
    }
    tx.commit()
}

pub fn verify_v2_repair_state(
    conn: &Connection,
    expected_head: i64,
    minimum_code_count: usize,
    minimum_interview_count: usize,
    expected_outbox_count: i64,
) -> rusqlite::Result<()> {
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::InvalidParameterName(
            "SQLite integrity check failed after v2 repair verification".into(),
        ));
    }
    let local_sequence = sync_state_value(conn, "sync_server_seq")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let observed_head = sync_state_value(conn, "sync_observed_head")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let code_count: i64 = conn.query_row("SELECT COUNT(*) FROM codebook", [], |row| row.get(0))?;
    let interview_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM interviews", [], |row| row.get(0))?;
    let outbox_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))?;
    if local_sequence != expected_head
        || observed_head != expected_head
        || code_count < minimum_code_count as i64
        || interview_count < minimum_interview_count as i64
        || outbox_count != expected_outbox_count
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "Sync Protocol 2 repair verification failed".into(),
        ));
    }
    Ok(())
}

pub fn local_schema_version(conn: &Connection) -> rusqlite::Result<i32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn legacy_actor_key(coder_name: &str) -> String {
    let encoded = coder_name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("legacy:{encoded}")
}

fn assignment_span_key(char_start: Option<i32>, char_end: Option<i32>) -> String {
    match (char_start, char_end) {
        (Some(start), Some(end)) => format!("span:{start}:{end}"),
        _ => "whole".into(),
    }
}

// Legacy projection mirroring needs the exact row identity, code set, and mutation
// timestamp together so a v1 client cannot observe a partial coding assignment.
#[allow(clippy::too_many_arguments)]
fn mirror_legacy_assignment_edges(
    tx: &Transaction<'_>,
    interview_id: &str,
    segment_id: &str,
    coder_name: &str,
    char_start: Option<i32>,
    char_end: Option<i32>,
    code_ids: &[String],
    present: bool,
    timestamp: &str,
) -> rusqlite::Result<()> {
    let actor_key = legacy_actor_key(coder_name);
    let span_key = assignment_span_key(char_start, char_end);
    tx.execute(
        "UPDATE coding_assignments
         SET present = 0, updated_at = ?5
         WHERE interview_id = ?1 AND segment_id = ?2 AND actor_key = ?3 AND span_key = ?4
           AND present != 0",
        params![interview_id, segment_id, actor_key, span_key, timestamp],
    )?;

    if !present {
        return Ok(());
    }

    let mut unique = HashSet::new();
    for code_id in code_ids {
        if !unique.insert(code_id) {
            continue;
        }
        tx.execute(
            "INSERT INTO coding_assignments
               (interview_id, segment_id, actor_key, span_key, code_id, char_start, char_end,
                present, server_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, NULL, ?8, ?8)
             ON CONFLICT(interview_id, segment_id, actor_key, span_key, code_id)
             DO UPDATE SET char_start = excluded.char_start,
                           char_end = excluded.char_end,
                           present = 1,
                           updated_at = excluded.updated_at",
            params![
                interview_id,
                segment_id,
                actor_key,
                span_key,
                code_id,
                char_start,
                char_end,
                timestamp,
            ],
        )?;
    }
    Ok(())
}

fn required_v2_state(tx: &Transaction<'_>, key: &str) -> rusqlite::Result<String> {
    sync_state_value(tx, key)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(format!("Missing local sync metadata: {key}"))
        })
}

fn v2_patch_fields(op_kind: &str, payload: &serde_json::Value) -> Vec<String> {
    match op_kind {
        "code.patch" | "interview.patch" => payload
            .get("patch")
            .and_then(serde_json::Value::as_object)
            .map(|patch| patch.keys().cloned().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn v2_base_field_versions(
    tx: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    fields: &[String],
) -> rusqlite::Result<serde_json::Value> {
    let mut versions = serde_json::Map::new();
    for field_name in fields {
        let version = tx
            .query_row(
                "SELECT server_seq FROM sync_field_versions
                  WHERE entity_type = ?1 AND entity_id = ?2 AND field_name = ?3",
                params![entity_type, entity_id, field_name],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        versions.insert(field_name.clone(), serde_json::Value::from(version));
    }
    Ok(serde_json::Value::Object(versions))
}

fn enqueue_v2_operation(
    tx: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    op_kind: &str,
    payload: serde_json::Value,
    timestamp: &str,
) -> rusqlite::Result<String> {
    let project_id = required_v2_state(tx, "project_id")?;
    let generation = required_v2_state(tx, "sync_generation")?;
    let device_id = required_v2_state(tx, "sync_device_id")?;
    let client_seq = sync_state_value(tx, "sync_client_seq")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        + 1;
    tx.execute(
        "INSERT INTO sync_state(key, value) VALUES ('sync_client_seq', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![client_seq.to_string()],
    )?;
    let op_id = Uuid::new_v4().to_string();
    let base_versions = v2_base_field_versions(
        tx,
        entity_type,
        entity_id,
        &v2_patch_fields(op_kind, &payload),
    )?;
    tx.execute(
        "INSERT INTO sync_outbox
           (op_id, project_id, generation, device_id, client_seq, entity_type, entity_id,
            op_kind, payload_json, base_versions_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            op_id,
            project_id,
            generation,
            device_id,
            client_seq,
            entity_type,
            entity_id,
            op_kind,
            serde_json::to_string(&payload)
                .map_err(|error| { rusqlite::Error::InvalidParameterName(error.to_string()) })?,
            serde_json::to_string(&base_versions)
                .map_err(|error| { rusqlite::Error::InvalidParameterName(error.to_string()) })?,
            timestamp,
        ],
    )?;
    Ok(op_id)
}

fn v2_actor_key(tx: &Transaction<'_>, coder_name: &str) -> rusqlite::Result<String> {
    Ok(sync_state_value(tx, "sync_actor_key")?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| legacy_actor_key(coder_name)))
}

fn set_v2_assignment_edge(
    tx: &Transaction<'_>,
    input: &MutateCodingEdgeInput,
    present: bool,
    timestamp: &str,
) -> rusqlite::Result<()> {
    let actor_key = v2_actor_key(tx, &input.coder_name)?;
    let span_key = assignment_span_key(input.char_start, input.char_end);
    tx.execute(
        "INSERT INTO coding_assignments
           (interview_id, segment_id, actor_key, span_key, code_id, char_start, char_end,
            present, server_seq, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9)
         ON CONFLICT(interview_id, segment_id, actor_key, span_key, code_id)
         DO UPDATE SET char_start = excluded.char_start,
                       char_end = excluded.char_end,
                       present = excluded.present,
                       updated_at = excluded.updated_at",
        params![
            input.interview_id,
            input.segment_id,
            actor_key,
            span_key,
            input.code_id,
            input.char_start,
            input.char_end,
            present as i32,
            timestamp,
        ],
    )?;
    Ok(())
}

pub(crate) fn build_v2_coding_edge(
    interview_id: &str,
    segment_id: &str,
    code_id: &str,
    char_start: Option<i32>,
    char_end: Option<i32>,
) -> rusqlite::Result<serde_json::Value> {
    match (char_start, char_end) {
        (Some(start), Some(end)) => Ok(serde_json::json!({
            "interview_id": interview_id,
            "segment_id": segment_id,
            "code_id": code_id,
            "char_start": start,
            "char_end": end,
        })),
        (None, None) => Ok(serde_json::json!({
            "interview_id": interview_id,
            "segment_id": segment_id,
            "code_id": code_id,
        })),
        _ => Err(rusqlite::Error::InvalidParameterName(
            "Mixed char_start and char_end are invalid for coding edge".into(),
        )),
    }
}

fn enqueue_v2_coding_patch(
    tx: &Transaction<'_>,
    input: &MutateCodingEdgeInput,
    timestamp: &str,
) -> rusqlite::Result<()> {
    let edge = build_v2_coding_edge(
        &input.interview_id,
        &input.segment_id,
        &input.code_id,
        input.char_start,
        input.char_end,
    )?;
    let payload = if input.present {
        serde_json::json!({ "adds": [edge], "removes": [] })
    } else {
        serde_json::json!({ "adds": [], "removes": [edge] })
    };
    let entity_id = format!(
        "{}:{}:{}:{}",
        input.interview_id,
        input.segment_id,
        assignment_span_key(input.char_start, input.char_end),
        input.code_id,
    );
    enqueue_v2_operation(tx, "coding", &entity_id, "coding.patch", payload, timestamp)?;
    Ok(())
}

pub fn salvage_legacy_v1_state_for_v2(conn: &Connection) -> rusqlite::Result<usize> {
    if local_sync_protocol(conn)? != 2 {
        return Ok(0);
    }
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    let code_rows: Vec<V2CodeSalvageRow> = {
        let mut statement = tx.prepare(
            "SELECT id, name, definition, inclusion_criteria, exclusion_criteria, example,
                    parent_id, color, sort_order, deleted
               FROM codebook WHERE sync_dirty != 0",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let interview_rows: Vec<(String, String, i64, Option<String>, i64)> = {
        let mut statement = tx.prepare(
            "SELECT i.id, i.participant_label,
                    (SELECT COUNT(*) FROM transcript_segments ts WHERE ts.interview_id = i.id),
                    i.remote_content_hash, i.deleted
               FROM interviews i WHERE i.sync_dirty != 0",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let coding_rows: Vec<V2CodingSalvageRow> = {
        let mut statement = tx.prepare(
            "SELECT interview_id, segment_id, code_ids, char_start, char_end,
                    coder_name, base_code_ids, deleted
               FROM coded_segments WHERE sync_dirty != 0",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut salvaged = 0usize;
    for (
        id,
        name,
        definition,
        inclusion_criteria,
        exclusion_criteria,
        example,
        parent_id,
        color,
        sort_order,
        deleted,
    ) in code_rows
    {
        let (op_kind, payload) = if deleted != 0 {
            (
                "code.patch",
                serde_json::json!({ "patch": { "deleted": true } }),
            )
        } else {
            (
                "code.create",
                serde_json::json!({
                    "name": name,
                    "definition": definition,
                    "inclusion_criteria": inclusion_criteria,
                    "exclusion_criteria": exclusion_criteria,
                    "example": example,
                    "parent_id": parent_id,
                    "color": color,
                    "sort_order": sort_order,
                }),
            )
        };
        enqueue_v2_operation(&tx, "code", &id, op_kind, payload, &timestamp)?;
        salvaged += 1;
    }

    for (id, study_label, segment_count, remote_hash, deleted) in interview_rows {
        let content_hash = match remote_hash {
            Some(hash) => Some(hash),
            None => crate::sync::interview_content_hash(&tx, &id)?,
        };
        enqueue_v2_operation(
            &tx,
            "interview",
            &id,
            "interview.patch",
            serde_json::json!({
                "patch": {
                    "study_label": study_label,
                    "segment_count": segment_count,
                    "content_hash": content_hash,
                    "deleted": deleted != 0,
                }
            }),
            &timestamp,
        )?;
        salvaged += 1;
    }

    for (
        interview_id,
        segment_id,
        code_ids_json,
        char_start,
        char_end,
        coder_name,
        base_json,
        deleted,
    ) in coding_rows
    {
        let current = if deleted == 0 {
            serde_json::from_str::<Vec<String>>(&code_ids_json).unwrap_or_default()
        } else {
            Vec::new()
        };
        let base = base_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .unwrap_or_else(|| {
                if deleted != 0 {
                    serde_json::from_str::<Vec<String>>(&code_ids_json).unwrap_or_default()
                } else {
                    Vec::new()
                }
            });
        let current = current.into_iter().collect::<HashSet<_>>();
        let base = base.into_iter().collect::<HashSet<_>>();
        if char_start.is_some() ^ char_end.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Mixed char_start and char_end are invalid for coding edge".into(),
            ));
        }
        let adds = current.difference(&base).collect::<Vec<_>>();
        let removes = base.difference(&current).collect::<Vec<_>>();
        // Normal v2 edits key one coding entity PER CODE —
        // `{interview}:{segment}:{span}:{code_id}` (see mutate_coding_edge).
        // Salvage must emit the exact same per-code entities: merging two codes
        // on one span into a single span-keyed entity diverges from what any
        // normal edit would ever produce and resists reset/repair.
        for code_id in &adds {
            let edge =
                build_v2_coding_edge(&interview_id, &segment_id, code_id, char_start, char_end)?;
            let entity_id = format!(
                "{}:{}:{}:{}",
                interview_id,
                segment_id,
                assignment_span_key(char_start, char_end),
                code_id,
            );
            enqueue_v2_operation(
                &tx,
                "coding",
                &entity_id,
                "coding.patch",
                serde_json::json!({ "adds": [edge], "removes": [] }),
                &timestamp,
            )?;
            salvaged += 1;
        }
        for code_id in &removes {
            let edge =
                build_v2_coding_edge(&interview_id, &segment_id, code_id, char_start, char_end)?;
            let entity_id = format!(
                "{}:{}:{}:{}",
                interview_id,
                segment_id,
                assignment_span_key(char_start, char_end),
                code_id,
            );
            enqueue_v2_operation(
                &tx,
                "coding",
                &entity_id,
                "coding.patch",
                serde_json::json!({ "adds": [], "removes": [edge] }),
                &timestamp,
            )?;
            salvaged += 1;
        }
        let _ = coder_name;
    }

    tx.execute(
        "UPDATE codebook SET sync_dirty = 0 WHERE sync_dirty != 0",
        [],
    )?;
    tx.execute(
        "UPDATE interviews SET sync_dirty = 0 WHERE sync_dirty != 0",
        [],
    )?;
    tx.execute(
        "UPDATE coded_segments SET sync_dirty = 0 WHERE sync_dirty != 0",
        [],
    )?;
    tx.commit()?;
    Ok(salvaged)
}

/// Mark a coded segment as removed instead of deleting the row.
///
/// A hard `DELETE` cannot sync. The other machine has no way to tell "this row
/// was removed" apart from "I have not seen this row yet", so it re-inserts on
/// the next pull and the un-coding silently undoes itself. Codes and memo go:
/// a removed coding should not leave its content sitting in the file. Offsets
/// stay — they are the row's identity (passage, coder, span), and nulling them
/// would move a highlight tombstone onto the whole-turn unique slot, which is
/// how a 409 on `coded_segments_span_unique` became unrecoverable. Keeping them
/// is also what lets `apply_codes` revive the same row instead of minting a
/// second id the server will reject.
///
/// `?1` is the row id, `?2` the timestamp.
const TOMBSTONE_CODED_SEGMENT: &str = "UPDATE coded_segments
     SET deleted = 1, code_ids = '[]', memo = NULL,
         sync_dirty = 1, revision = revision + 1, updated_at = ?2
     WHERE id = ?1 AND deleted = 0";

/// Same reasoning as [`TOMBSTONE_CODED_SEGMENT`], for a codebook entry.
///
/// The label and definition stay: they are analytic constructs rather than
/// anything a participant said, they are what the other coder needs in order to
/// understand what disappeared, and they are already the one text pair sync
/// carries. `?1` is the code id, `?2` the timestamp.
const TOMBSTONE_CODE: &str = "UPDATE codebook
     SET deleted = 1, sync_dirty = 1, revision = revision + 1, updated_at = ?2
     WHERE id = ?1 AND deleted = 0";

fn apply_connection_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;",
    )
}

/// Fold the write-ahead log back into `project.db` and truncate it to zero.
///
/// In WAL mode the recent commits live in `project.db-wal`, not `project.db`.
/// That is invisible on one machine — SQLite reads both — but a project folder
/// is the unit we hand between coders over Drive, and Drive syncs the three
/// files independently. A teammate can end up with a `project.db` that opens
/// cleanly and is silently missing the last session's coding. Checkpointing
/// before anything copies the folder makes `project.db` self-sufficient.
///
/// Best-effort by design: a checkpoint can legitimately return SQLITE_BUSY,
/// and no caller should fail a save or a quit over it.
pub fn checkpoint(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
}

/// Extension for projects created from now on.
pub const PROJECT_EXT: &str = "fleuron";

/// Extensions the app opens. `codemap` and `qcproj` are pre-rename names and
/// stay supported indefinitely — projects already on Drive must keep working.
const PROJECT_EXTS: [&str; 3] = ["fleuron", "codemap", "qcproj"];

pub fn create_project(input: &CreateProjectInput) -> rusqlite::Result<PathBuf> {
    let already_suffixed = PROJECT_EXTS
        .iter()
        .any(|ext| input.project_name.ends_with(&format!(".{ext}")));
    let folder_name = if already_suffixed {
        input.project_name.clone()
    } else {
        format!("{}.{}", input.project_name, PROJECT_EXT)
    };
    let project_path = Path::new(&input.parent_dir).join(&folder_name);

    if project_path.exists() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Project folder already exists".into(),
        ));
    }

    fs::create_dir_all(&project_path).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("Failed to create project folder: {e}"))
    })?;
    fs::create_dir_all(project_path.join("interviews")).ok();
    fs::create_dir_all(project_path.join("exports")).ok();
    fs::create_dir_all(project_path.join("codebook")).ok();

    let project_json = serde_json::json!({
        "title": input.title,
        "methodology": "reflexive-ta",
        "coders": input.coders,
        "version": 1,
        "created_at": now_iso(),
    });
    fs::write(
        project_path.join("project.json"),
        serde_json::to_string_pretty(&project_json).unwrap(),
    )
    .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    let db_path = project_path.join("project.db");
    let conn = Connection::open(&db_path)?;
    apply_connection_pragmas(&conn)?;
    init_schema(&conn)?;

    set_meta(&conn, "title", &input.title)?;
    set_meta(&conn, "methodology", "reflexive-ta")?;
    set_meta(
        &conn,
        "coders",
        &serde_json::to_string(&input.coders).unwrap(),
    )?;

    log_activity(&conn, "system", "create_project", Some(&input.title))?;

    Ok(project_path)
}

pub fn is_project_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            let lower = n.to_ascii_lowercase();
            PROJECT_EXTS
                .iter()
                .any(|ext| lower.ends_with(&format!(".{ext}")))
        })
        .unwrap_or(false)
}

pub fn inspect_join_target(
    parent_dir: &str,
    slug: &str,
    project_id: &str,
) -> rusqlite::Result<crate::models::JoinTargetVerdict> {
    let clean_slug = slug.trim();
    let already_suffixed = PROJECT_EXTS
        .iter()
        .any(|ext| clean_slug.ends_with(&format!(".{ext}")));
    let base_name = if already_suffixed {
        let mut s = clean_slug.to_string();
        for ext in PROJECT_EXTS {
            if s.ends_with(&format!(".{ext}")) {
                s.truncate(s.len() - (ext.len() + 1));
                break;
            }
        }
        s
    } else {
        clean_slug.to_string()
    };

    let folder_name = format!("{}.{}", base_name, PROJECT_EXT);
    let target_path = Path::new(parent_dir).join(&folder_name);
    let target_path_str = target_path.to_string_lossy().to_string();

    if !target_path.exists() {
        return Ok(crate::models::JoinTargetVerdict::Available {
            suggested_name: folder_name,
            suggested_path: target_path_str,
        });
    }

    // Find suffixed name -2 .. -99
    let mut suffixed_name = None;
    let mut suffixed_path_str = None;
    for i in 2..=99 {
        let candidate_folder = format!("{}-{}.{}", base_name, i, PROJECT_EXT);
        let candidate_path = Path::new(parent_dir).join(&candidate_folder);
        if !candidate_path.exists() {
            suffixed_name = Some(candidate_folder);
            suffixed_path_str = Some(candidate_path.to_string_lossy().to_string());
            break;
        }
    }

    let (suggested_name, suggested_path) = match (suffixed_name, suffixed_path_str) {
        (Some(n), Some(p)) => (n, p),
        _ => {
            return Err(rusqlite::Error::InvalidParameterName(
                "All suffixed folders from -2 to -99 are already in use".into(),
            ));
        }
    };

    let db_path = target_path.join("project.db");
    if !db_path.exists() {
        return Ok(crate::models::JoinTargetVerdict::Occupied {
            path: target_path_str,
            suggested_name,
            suggested_path,
        });
    }

    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => {
            return Ok(crate::models::JoinTargetVerdict::Occupied {
                path: target_path_str,
                suggested_name,
                suggested_path,
            });
        }
    };

    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='sync_state'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !table_exists {
        return Ok(crate::models::JoinTargetVerdict::AdoptableUnbound {
            path: target_path_str,
        });
    }

    let bound: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_state WHERE key = 'group_bound'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);

    let stored_project_id: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_state WHERE key = 'project_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);

    if bound.as_deref() == Some("1") {
        if stored_project_id.as_deref() == Some(project_id) {
            Ok(crate::models::JoinTargetVerdict::AlreadySetUpHere {
                path: target_path_str,
            })
        } else {
            Ok(crate::models::JoinTargetVerdict::BoundElsewhere {
                path: target_path_str,
                suggested_name,
                suggested_path,
            })
        }
    } else {
        Ok(crate::models::JoinTargetVerdict::AdoptableUnbound {
            path: target_path_str,
        })
    }
}

pub fn open_project(path: &str) -> rusqlite::Result<Connection> {
    let project_path = Path::new(path);
    if !is_project_path(project_path) {
        return Err(rusqlite::Error::InvalidParameterName(
            "Not a Fleuron project. Choose a folder ending in .fleuron (or .codemap / .qcproj)."
                .into(),
        ));
    }
    let db_path = project_path.join("project.db");
    if !db_path.exists() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Not a valid project: project.db not found inside the project folder.".into(),
        ));
    }
    let conn = Connection::open(db_path)?;
    apply_connection_pragmas(&conn)?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn open_project_snapshot_inner(
    path: &str,
) -> rusqlite::Result<(Connection, ProjectOpenSnapshot)> {
    let t0 = std::time::Instant::now();
    let project_path = Path::new(path);
    if !is_project_path(project_path) {
        return Err(rusqlite::Error::InvalidParameterName(
            "Not a Fleuron project. Choose a folder ending in .fleuron (or .codemap / .qcproj)."
                .into(),
        ));
    }
    let db_path = project_path.join("project.db");
    if !db_path.exists() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Not a valid project: project.db not found inside the project folder.".into(),
        ));
    }
    let t_before_conn = std::time::Instant::now();
    let conn = Connection::open(db_path)?;
    apply_connection_pragmas(&conn)?;
    let conn_ms = t_before_conn.elapsed().as_millis() as u64;

    let t_before_schema = std::time::Instant::now();
    init_schema(&conn)?;
    let schema_ms = t_before_schema.elapsed().as_millis() as u64;

    let snapshot = get_open_project_snapshot(&conn, path, conn_ms, schema_ms, t0)?;
    Ok((conn, snapshot))
}

pub fn get_open_project_snapshot(
    conn: &Connection,
    path: &str,
    conn_ms: u64,
    schema_ms: u64,
    total_t0: std::time::Instant,
) -> rusqlite::Result<ProjectOpenSnapshot> {
    let t_queries = std::time::Instant::now();

    let project = get_project_info(conn, path)?;
    let codes = list_codes(conn)?;
    let interviews = list_interviews(conn)?;
    let workspace = get_workspace_state(conn)?;

    let active_interview_id = if let Some(ref stored_id) = workspace.active_interview_id {
        if interviews.iter().any(|i| &i.id == stored_id) {
            Some(stored_id.clone())
        } else {
            interviews
                .iter()
                .find(|i| i.segment_count > 0)
                .map(|i| i.id.clone())
                .or_else(|| interviews.first().map(|i| i.id.clone()))
        }
    } else {
        interviews
            .iter()
            .find(|i| i.segment_count > 0)
            .map(|i| i.id.clone())
            .or_else(|| interviews.first().map(|i| i.id.clone()))
    };

    let segments = if let Some(ref iv_id) = active_interview_id {
        get_segments(conn, iv_id)?
    } else {
        Vec::new()
    };

    let selected_segment_id = if let Some(ref stored_seg_id) = workspace.selected_segment_id {
        if segments.iter().any(|s| &s.id == stored_seg_id) {
            Some(stored_seg_id.clone())
        } else {
            segments.first().map(|s| s.id.clone())
        }
    } else {
        segments.first().map(|s| s.id.clone())
    };

    let coded_segments = if let Some(ref iv_id) = active_interview_id {
        list_coded_segments(conn, Some(iv_id), None)?
    } else {
        Vec::new()
    };

    let total_coded_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c.max(0) as usize)
        .unwrap_or(0);

    let mut recent_stmt = conn.prepare(
        "SELECT code_ids FROM coded_segments WHERE deleted = 0 ORDER BY updated_at DESC, id DESC LIMIT 50",
    )?;
    let code_ids_rows = recent_stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut recent_code_ids = Vec::new();
    let mut seen_codes = HashSet::new();
    for row in code_ids_rows.flatten() {
        if let Ok(ids) = serde_json::from_str::<Vec<String>>(&row) {
            for id in ids {
                if seen_codes.insert(id.clone()) {
                    recent_code_ids.push(id);
                    if recent_code_ids.len() >= 6 {
                        break;
                    }
                }
            }
        }
        if recent_code_ids.len() >= 6 {
            break;
        }
    }

    let schema_version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    let query_ms = t_queries.elapsed().as_millis() as u64;
    let total_ms = total_t0.elapsed().as_millis() as u64;

    let diagnostics = ProjectOpenDiagnostics {
        schema_version,
        counts: DiagnosticCounts {
            codes: codes.len(),
            interviews: interviews.len(),
            segments: segments.len(),
            coded_segments: coded_segments.len(),
        },
        timings_ms: DiagnosticTimings {
            connection: conn_ms,
            schema: schema_ms,
            snapshot_queries: query_ms,
            total: total_ms,
        },
    };

    let reviewed_segment_ids = match active_interview_id.as_deref() {
        Some(id) => list_segment_reviews(conn, id)?,
        None => Vec::new(),
    };

    Ok(ProjectOpenSnapshot {
        project,
        codes,
        interviews,
        workspace,
        active_interview_id,
        selected_segment_id,
        segments,
        coded_segments,
        total_coded_count,
        recent_code_ids,
        reviewed_segment_ids,
        diagnostics,
        // Bound by the caller in commands.rs once the connection is live:
        // the database layer must not mint workspace identity.
        project_key: None,
        workspace_epoch: None,
    })
}

fn grouped_live_coded_segments(
    conn: &Connection,
    interview_id: Option<&str>,
) -> rusqlite::Result<Vec<CodedSegment>> {
    let mut rows = list_coded_segments(conn, interview_id, None)?;
    if local_sync_protocol(conn)? != 2 {
        return Ok(rows);
    }

    let local_actor = sync_state_value(conn, "sync_actor_key")?;
    rows.retain_mut(|row| {
        let legacy_actor = legacy_actor_key(&row.coder_name);
        let assignment_rows = conn
            .prepare(
                "SELECT code_id, present FROM coding_assignments
                 WHERE interview_id = ?1 AND segment_id = ?2
                   AND span_key = ?3
                   AND (actor_key = ?4 OR actor_key = ?5)
                 ORDER BY code_id",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(
                        params![
                            row.interview_id,
                            row.segment_id,
                            assignment_span_key(row.char_start, row.char_end),
                            legacy_actor,
                            local_actor.as_deref().unwrap_or(&legacy_actor),
                        ],
                        |record| Ok((record.get::<_, String>(0)?, record.get::<_, i64>(1)?)),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()
            });
        match assignment_rows {
            Ok(assignments) if assignments.is_empty() => true,
            Ok(assignments) => {
                row.code_ids = assignments
                    .into_iter()
                    .filter_map(|(code_id, present)| (present != 0).then_some(code_id))
                    .collect();
                !row.code_ids.is_empty()
            }
            Err(_) => true,
        }
    });
    Ok(rows)
}

pub fn get_live_workspace_snapshot(
    conn: &Connection,
    path: &str,
    requested_interview_id: Option<&str>,
) -> rusqlite::Result<LiveWorkspaceSnapshot> {
    let tx = conn.unchecked_transaction()?;
    let project = get_project_info(&tx, path)?;
    let interviews = list_interviews(&tx)?;
    let codes = list_codes(&tx)?;
    let retired_codes = list_retired_codes(&tx)?;
    let workspace = get_workspace_state(&tx)?;
    let active_interview_id = requested_interview_id
        .filter(|id| interviews.iter().any(|interview| interview.id == *id))
        .map(str::to_owned)
        .or_else(|| {
            workspace
                .active_interview_id
                .filter(|id| interviews.iter().any(|interview| interview.id == *id))
        })
        .or_else(|| {
            interviews
                .iter()
                .find(|interview| interview.segment_count > 0)
                .map(|interview| interview.id.clone())
        })
        .or_else(|| interviews.first().map(|interview| interview.id.clone()));
    let segments = match active_interview_id.as_deref() {
        Some(interview_id) => get_segments(&tx, interview_id)?,
        None => Vec::new(),
    };
    let selected_segment_id = workspace
        .selected_segment_id
        .filter(|id| segments.iter().any(|segment| segment.id == *id))
        .or_else(|| segments.first().map(|segment| segment.id.clone()));
    let coded_segments = grouped_live_coded_segments(&tx, active_interview_id.as_deref())?;
    let coded_count = tx
        .query_row(
            "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)?;
    let pending_coded_count = pending_coded_count(&tx)?;
    let conflicts = {
        let mut statement = tx.prepare(
            "SELECT conflict_id, entity_type, entity_id, field_name, status, created_at
             FROM sync_conflicts
             WHERE status = 'unresolved'
             ORDER BY created_at, conflict_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(SyncConflictSummary {
                    id: row.get(0)?,
                    entity_type: row.get(1)?,
                    entity_id: row.get(2)?,
                    field_name: row.get(3)?,
                    status: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let parse_sequence = |key: &str| {
        sync_state_value(&tx, key)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let protocol = local_sync_protocol(&tx)?;
    let outbox_count = tx.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))?;
    let blocked_count = tx.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE blocked_reason IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let sync_status = LiveWorkspaceSyncStatus {
        protocol,
        generation: sync_state_value(&tx, "sync_generation")?,
        local_sequence: parse_sequence("sync_server_seq"),
        observed_head: parse_sequence("sync_observed_head"),
        outbox_count,
        blocked_count,
        unresolved_conflict_count: conflicts.len() as i64,
    };
    let local_revision = parse_sequence("sync_client_seq").max(parse_sequence("sync_server_seq"));
    let reviewed_segment_ids = match active_interview_id.as_deref() {
        Some(id) => list_segment_reviews(&tx, id)?,
        None => Vec::new(),
    };
    tx.commit()?;

    Ok(LiveWorkspaceSnapshot {
        project,
        interviews,
        codes,
        retired_codes,
        active_interview_id,
        selected_segment_id,
        segments,
        coded_segments,
        pending_coded_count,
        coded_count,
        conflicts,
        sync_status,
        local_revision,
        reviewed_segment_ids,
        // Bound by the caller in commands.rs from live AppState.
        project_key: None,
        workspace_epoch: None,
    })
}

pub fn list_sync_conflict_details(conn: &Connection) -> rusqlite::Result<Vec<SyncConflictDetail>> {
    let mut statement = conn.prepare(
        "SELECT conflict_id, entity_type, entity_id, field_name, current_json, proposed_json,
                proposer_label, status, created_at
           FROM sync_conflicts
          WHERE status = 'unresolved'
          ORDER BY created_at, conflict_id",
    )?;
    let rows = statement.query_map([], |row| {
        let entity_type: String = row.get(1)?;
        let entity_id: String = row.get(2)?;
        let entity_label = match entity_type.as_str() {
            "code" => conn
                .query_row(
                    "SELECT name FROM codebook WHERE id = ?1",
                    params![entity_id],
                    |value| value.get::<_, String>(0),
                )
                .optional()?
                .unwrap_or_else(|| "Code".into()),
            "interview" => conn
                .query_row(
                    "SELECT participant_label FROM interviews WHERE id = ?1",
                    params![entity_id],
                    |value| value.get::<_, String>(0),
                )
                .optional()?
                .unwrap_or_else(|| "Interview".into()),
            _ => "Coding assignment".into(),
        };
        let current_raw: String = row.get(4)?;
        let proposed_raw: String = row.get(5)?;
        Ok(SyncConflictDetail {
            id: row.get(0)?,
            entity_type,
            entity_label,
            field_name: row.get(3)?,
            current_value: serde_json::from_str(&current_raw).unwrap_or(serde_json::Value::Null),
            proposed_value: serde_json::from_str(&proposed_raw).unwrap_or(serde_json::Value::Null),
            proposer_label: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO project_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(crate) fn get_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM project_meta WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

/// Swap one name for another in the project's coder list.
///
/// Runs alongside a group rename: the server rewrites the member row and the
/// coded rows, and this keeps the local project in step immediately instead of
/// waiting for its next pull. Matching the picker list is case-insensitive to
/// mirror the server's uniqueness rule. If the old name isn't in the list (the
/// project was created before the name was chosen, for instance), the new name
/// is appended instead.
pub fn rename_project_coder(conn: &Connection, old: &str, new: &str) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let coders_json = get_meta(&tx, "coders")?.unwrap_or_else(|| "[]".into());
    let mut coders: Vec<String> = serde_json::from_str(&coders_json).unwrap_or_default();
    // `to_lowercase` rather than `eq_ignore_ascii_case`: the server's
    // uniqueness rule is Postgres `lower()`, which is Unicode-aware, and coder
    // names are not guaranteed ASCII.
    let old_lower = old.to_lowercase();
    let new_lower = new.to_lowercase();
    let mut found = false;
    for c in coders.iter_mut() {
        if c.to_lowercase() == old_lower {
            *c = new.to_string();
            found = true;
        }
    }
    if !found && !coders.iter().any(|c| c.to_lowercase() == new_lower) {
        coders.push(new.to_string());
    }
    // Starting a group (or joining with a chosen name) often appends the
    // group name on top of the local setup name, then rewrites the local one
    // to match. Without this, the picker lists the same person twice.
    let mut seen = HashSet::new();
    coders.retain(|c| seen.insert(c.to_lowercase()));

    // These rows were already renamed remotely, so preserve their dirty state:
    // a local edit made just before renaming must still be pushed, while a row
    // received from the server must not echo straight back. The next pull sees
    // the server's bumped revision and settles timestamps normally.
    tx.execute(
        "UPDATE coded_segments SET coder_name = ?1 WHERE coder_name = ?2",
        params![new, old],
    )?;
    tx.execute(
        "UPDATE pending_coded_segments SET coder_name = ?1 WHERE coder_name = ?2",
        params![new, old],
    )?;
    set_meta(
        &tx,
        "coders",
        &serde_json::to_string(&coders).unwrap_or_default(),
    )?;
    tx.commit()
}

fn next_block_id(conn: &Connection) -> rusqlite::Result<i64> {
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM project_meta WHERE key = 'next_block_id'",
            [],
            |r| r.get(0),
        )
        .ok();
    let n = v.and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
    Ok(n)
}

fn bump_block_id(conn: &Connection, to: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO project_meta (key, value) VALUES ('next_block_id', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![to.to_string()],
    )?;
    Ok(())
}

pub fn get_project_info(conn: &Connection, path: &str) -> rusqlite::Result<ProjectInfo> {
    let title = get_meta(conn, "title")?.unwrap_or_else(|| "Untitled Project".into());
    let methodology = get_meta(conn, "methodology")?.unwrap_or_else(|| "reflexive-ta".into());
    let coders_json = get_meta(conn, "coders")?.unwrap_or_else(|| "[]".into());
    let coders: Vec<String> = serde_json::from_str(&coders_json).unwrap_or_default();
    let last_saved_by = get_meta(conn, "last_saved_by")?;
    let last_saved_at = get_meta(conn, "last_saved_at")?;

    Ok(ProjectInfo {
        path: path.to_string(),
        title,
        methodology,
        coders,
        last_saved_by,
        last_saved_at,
    })
}

pub fn get_workspace_state(conn: &Connection) -> rusqlite::Result<WorkspaceState> {
    let raw = get_meta(conn, "workspace_state")?;
    match raw {
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        None => Ok(WorkspaceState::default()),
    }
}

pub fn save_workspace_state(conn: &Connection, state: &WorkspaceState) -> rusqlite::Result<()> {
    let json = serde_json::to_string(state).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("Failed to serialize workspace: {e}"))
    })?;
    set_meta(conn, "workspace_state", &json)
}

pub fn log_activity(
    conn: &Connection,
    coder_name: &str,
    action: &str,
    detail: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO coder_activity_log (id, coder_name, action, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            Uuid::new_v4().to_string(),
            coder_name,
            action,
            detail,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn list_activity(conn: &Connection) -> rusqlite::Result<Vec<ActivityLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, coder_name, action, detail, created_at
         FROM coder_activity_log ORDER BY created_at DESC LIMIT 50",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActivityLogEntry {
            id: row.get(0)?,
            coder_name: row.get(1)?,
            action: row.get(2)?,
            detail: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Distinct passages per code id.
///
/// `code_ids` is a JSON array in a TEXT column, so this cannot be a GROUP BY.
/// Counting in Rust also lets us count *segments* rather than rows, which is
/// the number a coder means by "this code is on 12 passages" — two coders
/// tagging the same passage is one passage.
fn code_usage_counts(conn: &Connection) -> rusqlite::Result<HashMap<String, i32>> {
    use std::collections::HashSet;

    let mut stmt =
        conn.prepare("SELECT segment_id, code_ids FROM coded_segments WHERE deleted = 0")?;
    let rows = stmt.query_map([], |row| {
        let segment_id: String = row.get(0)?;
        let code_ids_json: String = row.get(1)?;
        Ok((segment_id, code_ids_json))
    })?;

    let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
    for row in rows {
        let (segment_id, code_ids_json) = row?;
        let code_ids: Vec<String> = serde_json::from_str(&code_ids_json).unwrap_or_default();
        for code_id in code_ids {
            seen.entry(code_id).or_default().insert(segment_id.clone());
        }
    }

    Ok(seen
        .into_iter()
        .map(|(code_id, segments)| (code_id, segments.len() as i32))
        .collect())
}

fn read_codes(conn: &Connection, retired: bool) -> rusqlite::Result<Vec<Code>> {
    let usage = code_usage_counts(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, definition, inclusion_criteria, exclusion_criteria, example,
                parent_id, color, sort_order, is_retired
         FROM codebook WHERE is_retired = ?1 AND deleted = 0 ORDER BY sort_order, name",
    )?;
    let rows = stmt.query_map(params![retired as i32], |row| {
        let id: String = row.get(0)?;
        let usage_count = usage.get(&id).copied().unwrap_or(0);
        Ok(Code {
            id,
            name: row.get(1)?,
            definition: row.get(2)?,
            inclusion_criteria: row.get(3)?,
            exclusion_criteria: row.get(4)?,
            example: row.get(5)?,
            parent_id: row.get(6)?,
            color: row.get(7)?,
            sort_order: row.get(8)?,
            is_retired: row.get::<_, i32>(9)? != 0,
            usage_count,
        })
    })?;
    rows.collect()
}

pub fn list_codes(conn: &Connection) -> rusqlite::Result<Vec<Code>> {
    read_codes(conn, false)
}

/// Retired codes, so the codebook can offer them back. Without this, retiring
/// is a one-way trip and the "safe" delete option is the destructive one.
pub fn list_retired_codes(conn: &Connection) -> rusqlite::Result<Vec<Code>> {
    read_codes(conn, true)
}

/// Every code regardless of retirement, for resolving names.
///
/// Export must use this, not `list_codes`: a passage coded with a
/// since-retired code still carries its id, and looking that id up in a
/// retirement-filtered map would silently drop the label from the CSV.
fn code_name_map(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT id, name FROM codebook")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect()
}

pub fn update_code(conn: &Connection, input: &UpdateCodeInput) -> rusqlite::Result<Code> {
    if let Some(parent_id) = &input.parent_id {
        if parent_id == &input.id {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some("A code cannot be its own parent.".into()),
            ));
        }
        let parent_has_parent: bool = conn
            .query_row(
                "SELECT parent_id IS NOT NULL FROM codebook WHERE id = ?1 AND deleted = 0",
                params![parent_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if parent_has_parent {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some("Hierarchy is limited to two levels. A sub-code cannot have children.".into()),
            ));
        }
        let has_children: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM codebook WHERE parent_id = ?1 AND deleted = 0",
                params![input.id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if has_children {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some(
                    "Cannot move a parent code under another code. Move its sub-codes first."
                        .into(),
                ),
            ));
        }
    }

    let protocol = local_sync_protocol(conn)?;
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE codebook
         SET name = ?2, definition = ?3, inclusion_criteria = ?4, exclusion_criteria = ?5,
             example = ?6, parent_id = ?7, color = ?8, sync_dirty = 1, revision = revision + 1,
             updated_at = ?9
         WHERE id = ?1",
        params![
            input.id,
            input.name,
            input.definition,
            input.inclusion_criteria,
            input.exclusion_criteria,
            input.example,
            input.parent_id,
            input.color,
            timestamp,
        ],
    )?;
    if protocol == 2 {
        enqueue_v2_operation(
            &tx,
            "code",
            &input.id,
            "code.patch",
            serde_json::json!({
                "patch": {
                    "name": input.name,
                    "definition": input.definition,
                    "inclusion_criteria": input.inclusion_criteria,
                    "exclusion_criteria": input.exclusion_criteria,
                    "example": input.example,
                    "parent_id": input.parent_id,
                    "color": input.color,
                }
            }),
            &timestamp,
        )?;
    }
    tx.commit()?;

    read_codes(conn, false)?
        .into_iter()
        .find(|c| c.id == input.id)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Bring a retired code back into the working codebook.
pub fn restore_code(conn: &Connection, code_id: &str) -> rusqlite::Result<()> {
    let protocol = local_sync_protocol(conn)?;
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE codebook
         SET is_retired = 0, sync_dirty = 1, revision = revision + 1, updated_at = ?2
         WHERE id = ?1",
        params![code_id, timestamp],
    )?;
    if protocol == 2 {
        enqueue_v2_operation(
            &tx,
            "code",
            code_id,
            "code.patch",
            serde_json::json!({ "patch": { "is_retired": false } }),
            &timestamp,
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Retire or purge a code. See `DeleteCodeMode` for why both exist.
pub fn delete_code(
    conn: &Connection,
    code_id: &str,
    mode: DeleteCodeMode,
) -> rusqlite::Result<DeleteCodeResult> {
    let protocol = local_sync_protocol(conn)?;
    let code_name: String = conn.query_row(
        "SELECT name FROM codebook WHERE id = ?1",
        params![code_id],
        |row| row.get(0),
    )?;

    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    let promoted_child_ids = if protocol == 2 {
        let mut statement = tx.prepare("SELECT id FROM codebook WHERE parent_id = ?1")?;
        let child_ids = statement
            .query_map(params![code_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        child_ids
    } else {
        Vec::new()
    };
    tx.execute(
        "UPDATE codebook
         SET parent_id = NULL, sync_dirty = 1, revision = revision + 1, updated_at = ?2
         WHERE parent_id = ?1",
        params![code_id, timestamp],
    )?;
    if protocol == 2 {
        for child_id in promoted_child_ids {
            enqueue_v2_operation(
                &tx,
                "code",
                &child_id,
                "code.patch",
                serde_json::json!({ "patch": { "parent_id": null } }),
                &timestamp,
            )?;
        }
    }

    if mode == DeleteCodeMode::Retire {
        tx.execute(
            "UPDATE codebook
             SET is_retired = 1, sync_dirty = 1, revision = revision + 1, updated_at = ?2
             WHERE id = ?1",
            params![code_id, timestamp],
        )?;
        if protocol == 2 {
            enqueue_v2_operation(
                &tx,
                "code",
                code_id,
                "code.retire",
                serde_json::json!({}),
                &timestamp,
            )?;
        }
        tx.commit()?;
        return Ok(DeleteCodeResult {
            mode,
            code_name,
            segments_updated: 0,
            coded_segments_removed: 0,
        });
    }

    let mirror_legacy = protocol != 2;

    // Read every row that mentions the code, rewrite its array, and drop the
    // row entirely when the code was its only one — a coded segment with an
    // empty code list is not a record of anything.
    let affected: Vec<AffectedCodedSegmentRow> = {
        let mut stmt = tx.prepare(
            "SELECT id, interview_id, segment_id, coder_name, char_start, char_end, code_ids
             FROM coded_segments WHERE deleted = 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i32>>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut segments_updated = 0;
    let mut coded_segments_removed = 0;
    let ts = timestamp;

    for (row_id, interview_id, segment_id, coder_name, char_start, char_end, code_ids_json) in
        affected
    {
        let code_ids: Vec<String> = serde_json::from_str(&code_ids_json).unwrap_or_default();
        if !code_ids.iter().any(|c| c == code_id) {
            continue;
        }
        let remaining: Vec<String> = code_ids.into_iter().filter(|c| c != code_id).collect();

        if remaining.is_empty() {
            tx.execute(TOMBSTONE_CODED_SEGMENT, params![row_id, ts])?;
            coded_segments_removed += 1;
        } else {
            tx.execute(
                "UPDATE coded_segments
                 SET code_ids = ?2, sync_dirty = 1, revision = revision + 1, updated_at = ?3
                 WHERE id = ?1",
                params![row_id, serde_json::to_string(&remaining).unwrap(), ts],
            )?;
        }
        if mirror_legacy {
            mirror_legacy_assignment_edges(
                &tx,
                &interview_id,
                &segment_id,
                &coder_name,
                char_start,
                char_end,
                &remaining,
                !remaining.is_empty(),
                &ts,
            )?;
        }
        segments_updated += 1;
    }

    if !mirror_legacy {
        tx.execute(
            "UPDATE coding_assignments
             SET present = 0, updated_at = ?2
             WHERE code_id = ?1 AND present != 0",
            params![code_id, ts],
        )?;
    }

    tx.execute(TOMBSTONE_CODE, params![code_id, ts])?;
    if protocol == 2 {
        enqueue_v2_operation(
            &tx,
            "code",
            code_id,
            "code.purge",
            serde_json::json!({}),
            &ts,
        )?;
    }
    tx.commit()?;

    Ok(DeleteCodeResult {
        mode,
        code_name,
        segments_updated,
        coded_segments_removed,
    })
}

fn create_code_in_tx(tx: &Transaction<'_>, input: &CreateCodeInput) -> rusqlite::Result<Code> {
    if let Some(parent_id) = &input.parent_id {
        let parent_has_parent: bool = tx
            .query_row(
                "SELECT parent_id IS NOT NULL FROM codebook WHERE id = ?1 AND deleted = 0",
                params![parent_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if parent_has_parent {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some("Hierarchy is limited to two levels. A sub-code cannot have children.".into()),
            ));
        }
    }

    let id = Uuid::new_v4().to_string();
    let ts = now_iso();
    let color = input.color.clone().unwrap_or_else(|| "#8a6410".into());
    let max_order: i32 = tx.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) FROM codebook",
        [],
        |row| row.get(0),
    )?;

    tx.execute(
        "INSERT INTO codebook (id, name, definition, inclusion_criteria, exclusion_criteria,
         example, parent_id, color, sort_order, is_retired, created_at, updated_at,
         revision, deleted, sync_dirty)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?10, 0, 0, 1)",
        params![
            id,
            input.name,
            input.definition,
            input.inclusion_criteria,
            input.exclusion_criteria,
            input.example,
            input.parent_id,
            color,
            max_order + 1,
            ts,
        ],
    )?;

    Ok(Code {
        id,
        name: input.name.clone(),
        definition: input.definition.clone(),
        inclusion_criteria: input.inclusion_criteria.clone(),
        exclusion_criteria: input.exclusion_criteria.clone(),
        example: input.example.clone(),
        parent_id: input.parent_id.clone(),
        color,
        sort_order: max_order + 1,
        is_retired: false,
        usage_count: 0,
    })
}

pub fn create_code(conn: &Connection, input: &CreateCodeInput) -> rusqlite::Result<Code> {
    let protocol = local_sync_protocol(conn)?;
    let tx = conn.unchecked_transaction()?;
    let code = create_code_in_tx(&tx, input)?;
    if protocol == 2 {
        let payload = serde_json::json!({
            "name": code.name,
            "definition": code.definition,
            "inclusion_criteria": code.inclusion_criteria,
            "exclusion_criteria": code.exclusion_criteria,
            "example": code.example,
            "parent_id": code.parent_id,
            "color": code.color,
            "sort_order": code.sort_order,
        });
        enqueue_v2_operation(&tx, "code", &code.id, "code.create", payload, &now_iso())?;
    }
    tx.commit()?;
    Ok(code)
}

pub fn list_interviews(conn: &Connection) -> rusqlite::Result<Vec<Interview>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.participant_label, i.interview_date, i.modality, i.diagnosis_notes,
                i.interviewers, i.hub_memo, i.audio_path,
                (SELECT COUNT(*) FROM transcript_segments ts WHERE ts.interview_id = i.id),
                i.remote_segment_count
         FROM interviews i WHERE i.deleted = 0
         ORDER BY i.interview_date DESC, i.participant_label",
    )?;
    let rows = stmt.query_map([], |row| {
        let interviewers_json: String = row.get(5)?;
        Ok(Interview {
            id: row.get(0)?,
            participant_label: row.get(1)?,
            interview_date: row.get(2)?,
            modality: row.get(3)?,
            diagnosis_notes: row.get(4)?,
            interviewers: serde_json::from_str(&interviewers_json).unwrap_or_default(),
            hub_memo: row.get(6)?,
            audio_path: row.get(7)?,
            segment_count: row.get(8)?,
            remote_segment_count: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn create_interview(
    conn: &Connection,
    input: &CreateInterviewInput,
) -> rusqlite::Result<Interview> {
    // Derived from the study label so that both coders, creating this interview
    // independently on their own machines, arrive at the same id. See `ids`.
    let id = crate::ids::interview_id(&input.participant_label);
    let ts = now_iso();
    let interviewers_json = serde_json::to_string(&input.interviewers).unwrap();

    // Two interviews with one label are one interview. Because the id is now a
    // function of the label, the second insert would collide on the primary key
    // and surface as a bare constraint error; say what actually happened.
    let taken: bool = conn
        .query_row(
            "SELECT 1 FROM interviews WHERE id = ?1",
            params![id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if taken {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some(format!(
                "This project already has an interview labelled \"{}\". \
                 Study labels identify the participant, so each one is used once.",
                input.participant_label.trim()
            )),
        ));
    }

    let protocol = local_sync_protocol(conn)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO interviews (id, participant_label, interview_date, modality, diagnosis_notes,
         interviewers, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            id,
            input.participant_label,
            input.interview_date,
            input.modality,
            input.diagnosis_notes,
            interviewers_json,
            ts,
        ],
    )?;

    if protocol == 2 {
        enqueue_v2_operation(
            &tx,
            "interview",
            &id,
            "interview.patch",
            serde_json::json!({
                "patch": {
                    "study_label": input.participant_label,
                    "segment_count": 0,
                    "content_hash": null,
                    "deleted": false,
                }
            }),
            &ts,
        )?;
    }
    tx.commit()?;

    Ok(Interview {
        id,
        participant_label: input.participant_label.clone(),
        interview_date: input.interview_date.clone(),
        modality: input.modality.clone(),
        diagnosis_notes: input.diagnosis_notes.clone(),
        interviewers: input.interviewers.clone(),
        hub_memo: None,
        audio_path: None,
        segment_count: 0,
        remote_segment_count: None,
    })
}

pub fn update_interview(
    conn: &Connection,
    input: &UpdateInterviewInput,
) -> rusqlite::Result<Interview> {
    let protocol = local_sync_protocol(conn)?;
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE interviews
         SET participant_label = ?2, interview_date = ?3, modality = ?4,
             diagnosis_notes = ?5, updated_at = ?6
         WHERE id = ?1",
        params![
            input.id,
            input.participant_label,
            input.interview_date,
            input.modality,
            input.diagnosis_notes,
            timestamp,
        ],
    )?;
    if protocol == 2 {
        enqueue_v2_operation(
            &tx,
            "interview",
            &input.id,
            "interview.patch",
            serde_json::json!({ "patch": { "study_label": input.participant_label } }),
            &timestamp,
        )?;
    }
    tx.commit()?;

    list_interviews(conn)?
        .into_iter()
        .find(|i| i.id == input.id)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// What deleting this interview would take with it, for the confirm dialog.
pub fn interview_delete_impact(
    conn: &Connection,
    interview_id: &str,
) -> rusqlite::Result<InterviewDeleteImpact> {
    let (participant_label, hub_memo): (String, Option<String>) = conn.query_row(
        "SELECT participant_label, hub_memo FROM interviews WHERE id = ?1",
        params![interview_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let segment_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM transcript_segments WHERE interview_id = ?1",
        params![interview_id],
        |row| row.get(0),
    )?;

    // Distinct passages, matching how coverage is counted everywhere else.
    let coded_segment_count: i32 = conn.query_row(
        "SELECT COUNT(DISTINCT segment_id) FROM coded_segments
         WHERE interview_id = ?1 AND deleted = 0",
        params![interview_id],
        |row| row.get(0),
    )?;

    Ok(InterviewDeleteImpact {
        participant_label,
        segment_count,
        coded_segment_count,
        has_hub_memo: hub_memo.is_some_and(|m| !m.trim().is_empty()),
    })
}

/// Delete an interview and everything hanging off it.
///
/// Transcript segments and coded segments go with it via `ON DELETE CASCADE`,
/// which only fires because `apply_connection_pragmas` turns foreign keys on —
/// without that pragma SQLite would leave orphaned rows behind that still
/// appear in project-wide counts.
pub fn delete_interview(conn: &Connection, interview_id: &str) -> rusqlite::Result<()> {
    let protocol = local_sync_protocol(conn)?;
    if protocol != 2 {
        conn.execute(
            "DELETE FROM interviews WHERE id = ?1",
            params![interview_id],
        )?;
        return Ok(());
    }
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE interviews
            SET deleted = 1, sync_dirty = 1, revision = revision + 1, updated_at = ?2
          WHERE id = ?1",
        params![interview_id, timestamp],
    )?;
    enqueue_v2_operation(
        &tx,
        "interview",
        interview_id,
        "interview.patch",
        serde_json::json!({ "patch": { "deleted": true } }),
        &timestamp,
    )?;
    tx.commit()?;
    Ok(())
}

/// Load a transcript into an interview, replacing whatever was there.
///
/// 🔑 **Re-importing no longer destroys coding.** This used to delete every
/// coded segment for the interview first, because segment ids were random and a
/// re-import could not possibly line up with what had been coded. Ids are now
/// derived from the passage itself, so a passage that survives the re-import
/// keeps its id — and keeps its coding. Only passages that genuinely changed or
/// disappeared lose theirs, via the foreign key, which is the honest outcome:
/// the words they were attached to are no longer in the transcript.
pub fn import_segments(conn: &Connection, input: &ImportSegmentsInput) -> rusqlite::Result<i32> {
    let protocol = local_sync_protocol(conn)?;
    let timestamp = now_iso();
    let tx = conn.unchecked_transaction()?;

    if let Some(vtt_path) = &input.raw_vtt_path {
        tx.execute(
            "UPDATE interviews SET raw_vtt_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![vtt_path, timestamp, input.interview_id],
        )?;
    }

    let mut block_counter = 0;
    let mut incoming_ids: Vec<String> = Vec::with_capacity(input.segments.len());
    for (index, seg) in input.segments.iter().enumerate() {
        let id = crate::ids::segment_id(&input.interview_id, index as i64, &seg.text);
        incoming_ids.push(id.clone());
        // Upsert rather than insert: the same passage re-imported is the same
        // row, and replacing it would cascade its coding away for nothing.
        // `block_id` is left alone deliberately — it is assigned when a passage
        // is first coded and is what exports and memos refer to.
        tx.execute(
            "INSERT INTO transcript_segments (id, interview_id, segment_index, speaker,
             timestamp_start, timestamp_end, text, section_tag, is_substantive)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)
             ON CONFLICT(id) DO UPDATE SET
               segment_index   = excluded.segment_index,
               speaker         = excluded.speaker,
               timestamp_start = excluded.timestamp_start,
               timestamp_end   = excluded.timestamp_end,
               section_tag     = excluded.section_tag",
            params![
                id,
                input.interview_id,
                index as i32,
                seg.speaker,
                seg.timestamp_start,
                seg.timestamp_end,
                seg.text,
                seg.section_tag,
            ],
        )?;
        block_counter += 1;
    }

    // Drop passages the new transcript no longer contains. Coding on them goes
    // with them through ON DELETE CASCADE, which is correct — there is no
    // passage left for it to be about.
    {
        let placeholders = std::iter::repeat_n("?", incoming_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM transcript_segments
             WHERE interview_id = ?1 AND id NOT IN ({})",
            if placeholders.is_empty() {
                "''".to_string()
            } else {
                placeholders
            }
        );
        let mut args: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(incoming_ids.len() + 1);
        args.push(&input.interview_id);
        for id in &incoming_ids {
            args.push(id);
        }
        tx.execute(&sql, args.as_slice())?;
    }

    tx.execute(
        "UPDATE interviews SET updated_at = ?1 WHERE id = ?2",
        params![timestamp, input.interview_id],
    )?;

    drain_pending_coded(&tx)?;
    if protocol == 2 {
        let content_hash = crate::sync::interview_content_hash(&tx, &input.interview_id)?;
        enqueue_v2_operation(
            &tx,
            "interview",
            &input.interview_id,
            "interview.patch",
            serde_json::json!({
                "patch": {
                    "segment_count": input.segments.len(),
                    "content_hash": content_hash,
                }
            }),
            &timestamp,
        )?;
    }

    tx.commit()?;
    Ok(block_counter)
}

/// A coded row as the far side knows it, mid-merge.
///
/// The field set is exactly `sync::CodedSegmentPayload`'s, minus the project
/// id (implicit: the open project) and with `code_ids` already serialised.
pub struct RemoteCoded<'a> {
    pub id: &'a str,
    pub interview_id: &'a str,
    pub segment_id: &'a str,
    pub code_ids_json: &'a str,
    pub coder_name: &'a str,
    pub revision: i64,
    pub deleted: bool,
    pub updated_at: &'a str,
    pub char_start: Option<i64>,
    pub char_end: Option<i64>,
}

/// Three-way set merge for coded segment code IDs.
///
/// added_locally   = local − base
/// removed_locally = base − local
/// merged          = (remote ∪ added_locally) − removed_locally
///
/// When base is None (no known common ancestor), falls back to union (local ∪ remote).
pub fn three_way_merge_codes(
    base: Option<&[String]>,
    local: &[String],
    remote: &[String],
) -> Vec<String> {
    use std::collections::BTreeSet;
    let local_set: BTreeSet<&str> = local.iter().map(|s| s.as_str()).collect();
    let remote_set: BTreeSet<&str> = remote.iter().map(|s| s.as_str()).collect();

    let merged_set: BTreeSet<&str> = if let Some(b) = base {
        let base_set: BTreeSet<&str> = b.iter().map(|s| s.as_str()).collect();
        let added_locally: BTreeSet<&str> = local_set.difference(&base_set).copied().collect();
        let removed_locally: BTreeSet<&str> = base_set.difference(&local_set).copied().collect();
        let mut merged: BTreeSet<&str> = remote_set.union(&added_locally).copied().collect();
        for r in removed_locally {
            merged.remove(r);
        }
        merged
    } else {
        local_set.union(&remote_set).copied().collect()
    };

    merged_set.into_iter().map(|s| s.to_string()).collect()
}

/// How one incoming coded row was folded in.
#[derive(Debug, PartialEq, Eq)]
pub enum CodedMerge {
    /// Local content changed: a fresh insert, a clean remote update, or a
    /// clean twin re-keyed onto the server's id with the server's content.
    Applied,
    /// The revision guard refused the row because local revision was higher and clean;
    /// nothing changed. Safe to advance past.
    Superseded,
    /// The local row was dirty and was three-way merged or kept locally;
    /// sync_dirty remains 1 so it must be pushed on next run, and cursor must not advance past it.
    Deferred,
}

/// Fold one remote coded row into `coded_segments`.
///
/// Three-way match, in order:
///
/// 1. **Same id** — the ordinary case: a revision-guarded update or three-way merge.
/// 2. **Same (passage, coder, span), different id** — one coder coded the
///    same passage on two machines, and each machine minted its own id. The
///    server's unique constraint keys on the natural triple, so leaving both
///    rows in place means every push from this machine fails that constraint
///    forever. Higher revision wins, ties go to the server — the same rule as
///    an id match — and the survivor takes the *server's* id, which is what
///    lets the next push merge rather than collide.
/// 3. **Neither** — a fresh insert.
///
/// `memo` follows the pull rule throughout: never written from the wire,
/// preserved across updates, cleared by a tombstone. In case 2 the memo on
/// the local twin belongs to the person at this keyboard, so it survives even
/// when the server's content wins.
///
/// `coder_name` is written on update as well as insert: renaming yourself in
/// the group rewrites your rows on the server, and they land here by id.
pub fn merge_remote_coded(conn: &Connection, row: &RemoteCoded) -> rusqlite::Result<CodedMerge> {
    let remote_code_ids: Vec<String> = serde_json::from_str(row.code_ids_json).unwrap_or_default();

    let by_id: Option<(i64, i64, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT revision, sync_dirty, code_ids, base_code_ids, deleted FROM coded_segments WHERE id = ?1",
            params![row.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;

    if let Some((
        local_rev,
        local_dirty,
        local_code_ids_json,
        local_base_code_ids_json,
        _local_deleted,
    )) = by_id
    {
        if local_dirty == 0 {
            if row.revision >= local_rev {
                conn.execute(
                    "UPDATE coded_segments SET
                       code_ids      = ?2,
                       base_code_ids = ?2,
                       coder_name    = ?3,
                       deleted       = ?4,
                       revision      = ?5,
                       updated_at    = ?6,
                       char_start    = ?7,
                       char_end      = ?8,
                       memo          = CASE WHEN ?4 = 1 THEN NULL ELSE memo END,
                       sync_dirty    = 0
                     WHERE id = ?1",
                    params![
                        row.id,
                        row.code_ids_json,
                        row.coder_name,
                        row.deleted as i64,
                        row.revision,
                        row.updated_at,
                        row.char_start,
                        row.char_end,
                    ],
                )?;
                return Ok(CodedMerge::Applied);
            } else {
                return Ok(CodedMerge::Superseded);
            }
        } else {
            // Local is dirty: perform 3-way merge
            let local_code_ids: Vec<String> =
                serde_json::from_str(&local_code_ids_json).unwrap_or_default();
            let base_code_ids: Option<Vec<String>> = local_base_code_ids_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            let merged_code_ids =
                three_way_merge_codes(base_code_ids.as_deref(), &local_code_ids, &remote_code_ids);
            let merged_json =
                serde_json::to_string(&merged_code_ids).unwrap_or_else(|_| "[]".into());
            let is_tombstone = merged_code_ids.is_empty();
            let new_rev = std::cmp::max(local_rev, row.revision) + 1;

            conn.execute(
                "UPDATE coded_segments SET
                   code_ids      = ?2,
                   base_code_ids = ?3,
                   coder_name    = ?4,
                   deleted       = ?5,
                   revision      = ?6,
                   updated_at    = ?7,
                   char_start    = ?8,
                   char_end      = ?9,
                   memo          = CASE WHEN ?5 = 1 THEN NULL ELSE memo END,
                   sync_dirty    = 1
                 WHERE id = ?1",
                params![
                    row.id,
                    merged_json,
                    row.code_ids_json,
                    row.coder_name,
                    if is_tombstone { 1 } else { 0 },
                    new_rev,
                    row.updated_at,
                    row.char_start,
                    row.char_end,
                ],
            )?;
            return Ok(CodedMerge::Deferred);
        }
    }

    // `IS` rather than `=` for the span: two nulls are never equal in SQL, so
    // an `=` comparison never matches a whole-turn row against itself.
    let twin: Option<(String, i64, i64, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT id, revision, sync_dirty, code_ids, base_code_ids, deleted FROM coded_segments
             WHERE segment_id = ?1 AND coder_name = ?2
               AND char_start IS ?3 AND char_end IS ?4
             ORDER BY created_at LIMIT 1",
            params![row.segment_id, row.coder_name, row.char_start, row.char_end],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;

    if let Some((
        twin_id,
        twin_rev,
        twin_dirty,
        twin_code_ids_json,
        twin_base_code_ids_json,
        _twin_deleted,
    )) = twin
    {
        if twin_dirty == 0 {
            if row.revision >= twin_rev {
                // The server's record wins. The local row takes the server's id —
                // that is what lets the next push merge instead of colliding on
                // the (passage, coder, span) constraint again.
                conn.execute(
                    "UPDATE coded_segments SET
                       id            = ?2,
                       code_ids      = ?3,
                       base_code_ids = ?3,
                       deleted       = ?4,
                       revision      = ?5,
                       updated_at    = ?6,
                       char_start    = ?7,
                       char_end      = ?8,
                       memo          = CASE WHEN ?4 = 1 THEN NULL ELSE memo END,
                       sync_dirty    = 0
                     WHERE id = ?1",
                    params![
                        twin_id,
                        row.id,
                        row.code_ids_json,
                        row.deleted as i64,
                        row.revision,
                        row.updated_at,
                        row.char_start,
                        row.char_end,
                    ],
                )?;
                return Ok(CodedMerge::Applied);
            } else {
                conn.execute(
                    "UPDATE coded_segments SET id = ?2 WHERE id = ?1",
                    params![twin_id, row.id],
                )?;
                return Ok(CodedMerge::Superseded);
            }
        } else {
            // Twin is dirty: 3-way merge
            let twin_code_ids: Vec<String> =
                serde_json::from_str(&twin_code_ids_json).unwrap_or_default();
            let base_code_ids: Option<Vec<String>> = twin_base_code_ids_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            let merged_code_ids =
                three_way_merge_codes(base_code_ids.as_deref(), &twin_code_ids, &remote_code_ids);
            let merged_json =
                serde_json::to_string(&merged_code_ids).unwrap_or_else(|_| "[]".into());
            let is_tombstone = merged_code_ids.is_empty();
            let new_rev = std::cmp::max(twin_rev, row.revision) + 1;

            conn.execute(
                "UPDATE coded_segments SET
                   id            = ?2,
                   code_ids      = ?3,
                   base_code_ids = ?4,
                   deleted       = ?5,
                   revision      = ?6,
                   updated_at    = ?7,
                   char_start    = ?8,
                   char_end      = ?9,
                   memo          = CASE WHEN ?5 = 1 THEN NULL ELSE memo END,
                   sync_dirty    = 1
                 WHERE id = ?1",
                params![
                    twin_id,
                    row.id,
                    merged_json,
                    row.code_ids_json,
                    if is_tombstone { 1 } else { 0 },
                    new_rev,
                    row.updated_at,
                    row.char_start,
                    row.char_end,
                ],
            )?;
            return Ok(CodedMerge::Deferred);
        }
    }

    conn.execute(
        "INSERT INTO coded_segments
           (id, interview_id, segment_id, code_ids, base_code_ids, coder_name, memo,
            char_start, char_end, created_at, updated_at, revision, deleted, sync_dirty)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, NULL, ?6, ?7, ?8, ?8, ?9, ?10, 0)",
        params![
            row.id,
            row.interview_id,
            row.segment_id,
            row.code_ids_json,
            row.coder_name,
            row.char_start,
            row.char_end,
            row.updated_at,
            row.revision,
            row.deleted as i64,
        ],
    )?;
    Ok(CodedMerge::Applied)
}

/// Move coding that arrived before its transcript into `coded_segments`.
///
/// This is the second half of the fix described on migration 3. A joining coder
/// pulls their colleague's coding before importing anything, so those rows wait
/// in `pending_coded_segments`; the moment the passages they name exist, they
/// become real coding with no matching step and nothing for the user to do.
///
/// `sync_dirty = 0` on the way in: these rows came *from* the server and are
/// already known there. Marking them dirty would push them straight back as
/// though this machine had authored them.
///
/// Each row goes through [`merge_remote_coded`], so a passage coded locally
/// between the pull that staged the row and the import that drains it resolves
/// exactly as it would on a pull — including the two-machines case, where the
/// local row and the staged row name the same span under different ids.
fn drain_pending_coded(tx: &Connection) -> rusqlite::Result<usize> {
    let staged = {
        let mut stmt = tx.prepare(
            "SELECT p.id, p.interview_id, p.segment_id, p.code_ids, p.coder_name,
                    p.revision, p.deleted, p.updated_at, p.char_start, p.char_end
             FROM pending_coded_segments p
             JOIN transcript_segments ts ON ts.id = p.segment_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut moved = 0;
    for (
        id,
        interview_id,
        segment_id,
        code_ids,
        coder_name,
        revision,
        deleted,
        updated_at,
        char_start,
        char_end,
    ) in staged
    {
        if matches!(
            merge_remote_coded(
                tx,
                &RemoteCoded {
                    id: &id,
                    interview_id: &interview_id,
                    segment_id: &segment_id,
                    code_ids_json: &code_ids,
                    coder_name: &coder_name,
                    revision,
                    deleted: deleted != 0,
                    updated_at: &updated_at,
                    char_start,
                    char_end,
                },
            )?,
            CodedMerge::Applied | CodedMerge::Deferred
        ) {
            moved += 1;
        }
        tx.execute(
            "DELETE FROM pending_coded_segments WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(moved)
}

/// How much pulled coding is still waiting for a transcript.
///
/// Surfaced so the app can say "your colleague's coding is here, import P07 to
/// see it" rather than showing an empty transcript with no explanation.
pub fn pending_coded_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM pending_coded_segments WHERE deleted = 0",
        [],
        |r| r.get(0),
    )
}

pub fn get_segments(
    conn: &Connection,
    interview_id: &str,
) -> rusqlite::Result<Vec<TranscriptSegment>> {
    let mut stmt = conn.prepare(
        "SELECT id, interview_id, segment_index, speaker, timestamp_start, timestamp_end,
                text, block_id, section_tag
         FROM transcript_segments WHERE interview_id = ?1 ORDER BY segment_index",
    )?;
    let rows = stmt.query_map(params![interview_id], |row| {
        Ok(TranscriptSegment {
            id: row.get(0)?,
            interview_id: row.get(1)?,
            segment_index: row.get(2)?,
            speaker: row.get(3)?,
            timestamp_start: row.get(4)?,
            timestamp_end: row.get(5)?,
            text: row.get(6)?,
            block_id: row.get(7)?,
            section_tag: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn set_segment_speaker(
    conn: &Connection,
    segment_id: &str,
    new_speaker: &str,
    include_following: bool,
) -> rusqlite::Result<Vec<SegmentSpeakerChange>> {
    let trimmed = new_speaker.trim();
    if trimmed.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Speaker cannot be empty".into(),
        ));
    }

    let tx = conn.unchecked_transaction()?;

    let (interview_id, target_index, old_speaker): (String, i32, String) = tx.query_row(
        "SELECT interview_id, segment_index, speaker FROM transcript_segments WHERE id = ?1",
        params![segment_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    let mut affected: Vec<(String, i32, String)> = Vec::new();
    affected.push((segment_id.to_string(), target_index, old_speaker.clone()));

    if include_following {
        let mut stmt = tx.prepare(
            "SELECT id, segment_index, speaker FROM transcript_segments
             WHERE interview_id = ?1 AND segment_index > ?2 AND speaker = ?3
             ORDER BY segment_index ASC",
        )?;
        let rows = stmt.query_map(params![interview_id, target_index, old_speaker], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            affected.push(row?);
        }
    }

    let mut changes = Vec::with_capacity(affected.len());
    let mut update_stmt =
        tx.prepare("UPDATE transcript_segments SET speaker = ?1 WHERE id = ?2")?;

    for (id, _idx, old) in affected {
        update_stmt.execute(params![trimmed, id])?;
        changes.push(SegmentSpeakerChange {
            segment_id: id,
            old_speaker: old,
            new_speaker: trimmed.to_string(),
        });
    }
    drop(update_stmt);

    tx.commit()?;
    Ok(changes)
}

pub fn restore_segment_speakers(
    conn: &Connection,
    changes: &[SegmentSpeakerChange],
) -> rusqlite::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;

    let mut verify_stmt = tx.prepare("SELECT speaker FROM transcript_segments WHERE id = ?1")?;

    for change in changes {
        let current_speaker: String =
            verify_stmt.query_row([&change.segment_id], |row| row.get(0))?;
        if current_speaker != change.new_speaker {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Speaker on segment {} has changed (expected '{}', found '{}')",
                change.segment_id, change.new_speaker, current_speaker
            )));
        }
    }
    drop(verify_stmt);

    let mut update_stmt =
        tx.prepare("UPDATE transcript_segments SET speaker = ?1 WHERE id = ?2")?;

    for change in changes {
        update_stmt.execute(params![change.old_speaker, change.segment_id])?;
    }
    drop(update_stmt);

    tx.commit()?;
    Ok(())
}

pub fn set_segment_reviewed(
    conn: &Connection,
    segment_id: &str,
    reviewed: bool,
) -> rusqlite::Result<()> {
    if reviewed {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM transcript_segments WHERE id = ?1)",
            params![segment_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let now = now_iso();
        conn.execute(
            "INSERT INTO segment_reviews (segment_id, updated_at) VALUES (?1, ?2)
             ON CONFLICT(segment_id) DO UPDATE SET updated_at = ?2",
            params![segment_id, now],
        )?;
    } else {
        conn.execute(
            "DELETE FROM segment_reviews WHERE segment_id = ?1",
            params![segment_id],
        )?;
    }
    Ok(())
}

pub fn list_segment_reviews(
    conn: &Connection,
    interview_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT sr.segment_id
         FROM segment_reviews sr
         JOIN transcript_segments ts ON ts.id = sr.segment_id
         WHERE ts.interview_id = ?1
         ORDER BY ts.segment_index ASC",
    )?;
    let rows = stmt.query_map(params![interview_id], |row| row.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn apply_codes_in_tx(
    tx: &Transaction<'_>,
    input: &ApplyCodesInput,
    mirror_legacy: bool,
    timestamp: &str,
) -> rusqlite::Result<String> {
    let id = Uuid::new_v4().to_string();
    let code_ids_json = serde_json::to_string(&input.code_ids).unwrap();

    let block_id: Option<String> = tx.query_row(
        "SELECT block_id FROM transcript_segments WHERE id = ?1",
        params![input.segment_id],
        |row| row.get(0),
    )?;

    if block_id.is_none() {
        let n = next_block_id(tx)?;
        let new_block = format!("q{:03}", n);
        tx.execute(
            "UPDATE transcript_segments SET block_id = ?1 WHERE id = ?2",
            params![new_block, input.segment_id],
        )?;
        bump_block_id(tx, n + 1)?;
    }

    // One row per (passage, coder, **span**) — re-applying to the same span
    // revises that row rather than stacking another one.
    //
    // 🔑 The span is part of the key, and that is what makes highlight-level
    // coding possible. Keying on (passage, coder) alone meant one coder could
    // hold exactly one opinion per speaker turn, so coding a second phrase
    // inside a turn silently overwrote the first. A whole-turn coding keeps
    // `NULL` offsets and is therefore its own distinct row, separate from every
    // span inside it — a coder may say "this turn is about masking" *and* mark
    // two specific phrases within it.
    //
    // `IS` rather than `=`: in SQL, `NULL = NULL` is NULL, not true, so an `=`
    // comparison never matches a whole-turn row against itself and every
    // re-code would open a duplicate. `IS` is null-safe.
    //
    // This used to be an unconditional INSERT, so coding a passage, changing
    // your mind, and coding it again left two rows: the UI showed one (it reads
    // the first match) while the CSV exported both, and the second was
    // undeletable because nothing could remove a coded segment at all. A
    // different coder tagging the same passage still gets their own row; in
    // reflexive TA that divergence is data, not a duplicate.
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM coded_segments
             WHERE segment_id = ?1 AND coder_name = ?2
               AND char_start IS ?3 AND char_end IS ?4
             ORDER BY created_at LIMIT 1",
            params![
                input.segment_id,
                input.coder_name,
                input.char_start,
                input.char_end
            ],
            |row| row.get(0),
        )
        .optional()?;

    let row_id = match existing {
        Some(existing_id) => {
            // The lookup above deliberately does not filter tombstones, so
            // re-coding a passage you had un-coded revives the original row
            // rather than opening a second one. The id already exists on the
            // other coder's machine; a new one would leave two rows claiming
            // the same (passage, coder) and only one of them live.
            tx.execute(
                "UPDATE coded_segments
                 SET code_ids = ?2, memo = ?3, updated_at = ?4,
                     deleted = 0, sync_dirty = 1, revision = revision + 1
                 WHERE id = ?1",
                params![existing_id, code_ids_json, input.memo, timestamp],
            )?;
            existing_id
        }
        None => {
            tx.execute(
                "INSERT INTO coded_segments (id, interview_id, segment_id, code_ids, coder_name, memo,
                 char_start, char_end, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    id,
                    input.interview_id,
                    input.segment_id,
                    code_ids_json,
                    input.coder_name,
                    input.memo,
                    input.char_start,
                    input.char_end,
                    timestamp,
                ],
            )?;
            id
        }
    };

    if mirror_legacy {
        mirror_legacy_assignment_edges(
            tx,
            &input.interview_id,
            &input.segment_id,
            &input.coder_name,
            input.char_start,
            input.char_end,
            &input.code_ids,
            !input.code_ids.is_empty(),
            timestamp,
        )?;
    }

    Ok(row_id)
}

pub fn apply_codes(conn: &Connection, input: &ApplyCodesInput) -> rusqlite::Result<CodedSegment> {
    let protocol = local_sync_protocol(conn)?;
    let mirror_legacy = protocol != 2;
    let ts = now_iso();
    let tx = conn.unchecked_transaction()?;
    let previous_code_ids: Vec<String> = tx
        .query_row(
            "SELECT code_ids FROM coded_segments
              WHERE segment_id = ?1 AND coder_name = ?2
                AND char_start IS ?3 AND char_end IS ?4
              ORDER BY created_at LIMIT 1",
            params![
                input.segment_id,
                input.coder_name,
                input.char_start,
                input.char_end,
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let row_id = apply_codes_in_tx(&tx, input, mirror_legacy, &ts)?;
    if protocol == 2 {
        let previous: HashSet<_> = previous_code_ids.into_iter().collect();
        let requested: HashSet<_> = input.code_ids.iter().cloned().collect();
        for (code_id, present) in previous
            .difference(&requested)
            .map(|code_id| (code_id.clone(), false))
            .chain(
                requested
                    .difference(&previous)
                    .map(|code_id| (code_id.clone(), true)),
            )
        {
            let edge = MutateCodingEdgeInput {
                interview_id: input.interview_id.clone(),
                segment_id: input.segment_id.clone(),
                code_id,
                coder_name: input.coder_name.clone(),
                char_start: input.char_start,
                char_end: input.char_end,
                present,
            };
            set_v2_assignment_edge(&tx, &edge, present, &ts)?;
            enqueue_v2_coding_patch(&tx, &edge, &ts)?;
        }
    }
    tx.commit()?;
    get_coded_segment_by_id(conn, &row_id)
}

fn normalized_code_name(name: &str) -> String {
    name.trim().to_lowercase()
}

pub fn ensure_code_and_apply(
    conn: &Connection,
    input: &EnsureCodeAndApplyInput,
) -> rusqlite::Result<EnsureCodeAndApplyResult> {
    let name = input.name.trim();
    let coder_name = input.coder_name.trim();
    if name.is_empty() || coder_name.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "A code name and coder identity are required.".into(),
        ));
    }
    if (input.char_start.is_some() || input.char_end.is_some())
        && !(matches!((input.char_start, input.char_end), (Some(start), Some(end)) if start >= 0 && end > start))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "A coding span must have valid start and end offsets.".into(),
        ));
    }

    let protocol = local_sync_protocol(conn)?;
    let ts = now_iso();
    let tx = conn.unchecked_transaction()?;
    let target_exists: bool = tx.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM transcript_segments
            WHERE id = ?1 AND interview_id = ?2
         )",
        params![input.segment_id, input.interview_id],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !target_exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }

    let normalized_name = normalized_code_name(name);
    let candidates: Vec<Code> = {
        let mut stmt = tx.prepare(
            "SELECT id, name, definition, inclusion_criteria, exclusion_criteria, example,
                    parent_id, color, sort_order, is_retired
             FROM codebook WHERE deleted = 0 AND is_retired = 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Code {
                id: row.get(0)?,
                name: row.get(1)?,
                definition: row.get(2)?,
                inclusion_criteria: row.get(3)?,
                exclusion_criteria: row.get(4)?,
                example: row.get(5)?,
                parent_id: row.get(6)?,
                color: row.get(7)?,
                sort_order: row.get(8)?,
                is_retired: row.get::<_, i64>(9)? != 0,
                usage_count: 0,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|candidate| normalized_code_name(&candidate.name) == normalized_name)
            .collect()
    };
    if candidates.len() > 1 {
        return Err(rusqlite::Error::InvalidParameterName(
            "More than one live code has this name; resolve the duplicate before coding.".into(),
        ));
    }
    let (code, created) = if let Some(code) = candidates.into_iter().next() {
        (code, false)
    } else {
        (
            create_code_in_tx(
                &tx,
                &CreateCodeInput {
                    name: name.to_string(),
                    definition: None,
                    inclusion_criteria: None,
                    exclusion_criteria: None,
                    example: None,
                    parent_id: None,
                    color: input.color.clone(),
                },
            )?,
            true,
        )
    };

    let existing: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT id, code_ids, memo FROM coded_segments
             WHERE segment_id = ?1 AND coder_name = ?2
               AND char_start IS ?3 AND char_end IS ?4
             ORDER BY created_at LIMIT 1",
            params![
                input.segment_id,
                coder_name,
                input.char_start,
                input.char_end
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let mut code_ids = existing
        .as_ref()
        .and_then(|(_, ids, _)| serde_json::from_str::<Vec<String>>(ids).ok())
        .unwrap_or_default();
    let changed = if code_ids.iter().any(|id| id == &code.id) {
        false
    } else {
        code_ids.push(code.id.clone());
        true
    };
    let row_id = if changed {
        apply_codes_in_tx(
            &tx,
            &ApplyCodesInput {
                interview_id: input.interview_id.clone(),
                segment_id: input.segment_id.clone(),
                code_ids: code_ids.clone(),
                coder_name: coder_name.to_string(),
                memo: existing.as_ref().and_then(|(_, _, memo)| memo.clone()),
                char_start: input.char_start,
                char_end: input.char_end,
            },
            protocol != 2,
            &ts,
        )?
    } else {
        existing
            .as_ref()
            .map(|(id, _, _)| id.clone())
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?
    };

    let edge_input = MutateCodingEdgeInput {
        interview_id: input.interview_id.clone(),
        segment_id: input.segment_id.clone(),
        code_id: code.id.clone(),
        coder_name: coder_name.to_string(),
        char_start: input.char_start,
        char_end: input.char_end,
        present: true,
    };
    if protocol == 2 {
        if created {
            enqueue_v2_operation(
                &tx,
                "code",
                &code.id,
                "code.create",
                serde_json::json!({
                    "name": code.name,
                    "definition": code.definition,
                    "inclusion_criteria": code.inclusion_criteria,
                    "exclusion_criteria": code.exclusion_criteria,
                    "example": code.example,
                    "parent_id": code.parent_id,
                    "color": code.color,
                    "sort_order": code.sort_order,
                }),
                &ts,
            )?;
        }
        if changed {
            set_v2_assignment_edge(&tx, &edge_input, true, &ts)?;
            enqueue_v2_coding_patch(&tx, &edge_input, &ts)?;
        }
    }
    tx.commit()?;

    let code = list_codes(conn)?
        .into_iter()
        .find(|candidate| candidate.id == code.id)
        .unwrap_or(code);
    let coded_segment = get_coded_segment_by_id(conn, &row_id)?;
    Ok(EnsureCodeAndApplyResult {
        code,
        coded_segment,
        created,
        changed_edges: if changed {
            vec![ChangedCodingEdge {
                interview_id: edge_input.interview_id,
                segment_id: edge_input.segment_id,
                code_id: edge_input.code_id,
                char_start: edge_input.char_start,
                char_end: edge_input.char_end,
                present: true,
            }]
        } else {
            Vec::new()
        },
    })
}

pub fn mutate_coding_edge(
    conn: &Connection,
    input: &MutateCodingEdgeInput,
) -> rusqlite::Result<MutateCodingEdgeResult> {
    if input.coder_name.trim().is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "A coder identity is required.".into(),
        ));
    }
    if (input.char_start.is_some() || input.char_end.is_some())
        && !(matches!((input.char_start, input.char_end), (Some(start), Some(end)) if start >= 0 && end > start))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "A coding span must have valid start and end offsets.".into(),
        ));
    }

    let protocol = local_sync_protocol(conn)?;
    let ts = now_iso();
    let tx = conn.unchecked_transaction()?;
    let target_exists: bool = tx.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM transcript_segments
            WHERE id = ?1 AND interview_id = ?2
         )",
        params![input.segment_id, input.interview_id],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !target_exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    if input.present {
        let code_is_live: bool = tx.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM codebook
                WHERE id = ?1 AND deleted = 0 AND is_retired = 0
             )",
            params![input.code_id],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if !code_is_live {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
    }
    let existing: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT id, code_ids, memo FROM coded_segments
             WHERE segment_id = ?1 AND coder_name = ?2
               AND char_start IS ?3 AND char_end IS ?4
             ORDER BY created_at LIMIT 1",
            params![
                input.segment_id,
                input.coder_name,
                input.char_start,
                input.char_end,
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let mut code_ids = existing
        .as_ref()
        .and_then(|(_, ids, _)| serde_json::from_str::<Vec<String>>(ids).ok())
        .unwrap_or_default();
    let was_present = code_ids.iter().any(|code_id| code_id == &input.code_id);
    let changed = was_present != input.present;
    if input.present && !was_present {
        code_ids.push(input.code_id.clone());
    }
    if !input.present && was_present {
        code_ids.retain(|code_id| code_id != &input.code_id);
    }
    let row_id = if changed && !code_ids.is_empty() {
        apply_codes_in_tx(
            &tx,
            &ApplyCodesInput {
                interview_id: input.interview_id.clone(),
                segment_id: input.segment_id.clone(),
                code_ids: code_ids.clone(),
                coder_name: input.coder_name.clone(),
                memo: existing.as_ref().and_then(|(_, _, memo)| memo.clone()),
                char_start: input.char_start,
                char_end: input.char_end,
            },
            protocol != 2,
            &ts,
        )?
    } else if changed {
        let id = existing
            .as_ref()
            .map(|(id, _, _)| id.clone())
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.execute(TOMBSTONE_CODED_SEGMENT, params![id, &ts])?;
        if protocol != 2 {
            mirror_legacy_assignment_edges(
                &tx,
                &input.interview_id,
                &input.segment_id,
                &input.coder_name,
                input.char_start,
                input.char_end,
                &[],
                false,
                &ts,
            )?;
        }
        id
    } else {
        existing
            .as_ref()
            .map(|(id, _, _)| id.clone())
            .unwrap_or_default()
    };

    if protocol == 2 && changed {
        set_v2_assignment_edge(&tx, input, input.present, &ts)?;
        enqueue_v2_coding_patch(&tx, input, &ts)?;
    }
    tx.commit()?;
    let coded_segment = if row_id.is_empty() || (changed && code_ids.is_empty()) {
        None
    } else {
        get_coded_segment_by_id(conn, &row_id).ok()
    };
    Ok(MutateCodingEdgeResult {
        coded_segment,
        changed_edge: ChangedCodingEdge {
            interview_id: input.interview_id.clone(),
            segment_id: input.segment_id.clone(),
            code_id: input.code_id.clone(),
            char_start: input.char_start,
            char_end: input.char_end,
            present: input.present,
        },
    })
}

pub fn patch_coding_memo(
    conn: &Connection,
    input: &PatchCodingMemoInput,
) -> rusqlite::Result<CodedSegment> {
    let changed = conn.execute(
        "UPDATE coded_segments
         SET memo = ?2, updated_at = ?3
         WHERE id = ?1 AND deleted = 0",
        params![input.coded_segment_id, input.memo, now_iso()],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    get_coded_segment_by_id(conn, &input.coded_segment_id)
}

/// Remove one coder's coding from one passage.
///
/// The only previous way out of a mis-code was `clear_workspace`, which wipes
/// every coded segment in the interview or the whole project. Needing to
/// destroy an afternoon's work to fix one wrong tag is not a workable undo.
pub fn delete_coded_segment(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    let protocol = local_sync_protocol(conn)?;
    let mirror_legacy = protocol != 2;
    let tx = conn.unchecked_transaction()?;
    let row: Option<ExistingCodedSegmentRow> = tx
        .query_row(
            "SELECT interview_id, segment_id, coder_name, char_start, char_end, code_ids
             FROM coded_segments WHERE id = ?1 AND deleted = 0",
            params![id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let ts = now_iso();
    tx.execute(TOMBSTONE_CODED_SEGMENT, params![id, ts])?;
    if mirror_legacy {
        if let Some((interview_id, segment_id, coder_name, char_start, char_end, _)) = &row {
            mirror_legacy_assignment_edges(
                &tx,
                interview_id,
                segment_id,
                coder_name,
                *char_start,
                *char_end,
                &[],
                false,
                &ts,
            )?;
        }
    } else if let Some((interview_id, segment_id, coder_name, char_start, char_end, code_ids)) = row
    {
        for code_id in serde_json::from_str::<Vec<String>>(&code_ids).unwrap_or_default() {
            let edge = MutateCodingEdgeInput {
                interview_id: interview_id.clone(),
                segment_id: segment_id.clone(),
                code_id,
                coder_name: coder_name.clone(),
                char_start,
                char_end,
                present: false,
            };
            set_v2_assignment_edge(&tx, &edge, false, &ts)?;
            enqueue_v2_coding_patch(&tx, &edge, &ts)?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn get_coded_segment_by_id(conn: &Connection, id: &str) -> rusqlite::Result<CodedSegment> {
    conn.query_row(
        "SELECT cs.id, cs.interview_id, cs.segment_id, cs.code_ids, cs.coder_name, cs.memo,
                cs.char_start, cs.char_end, ts.text, ts.block_id, ts.timestamp_start, i.participant_label
         FROM coded_segments cs
         JOIN transcript_segments ts ON ts.id = cs.segment_id
         JOIN interviews i ON i.id = cs.interview_id
         WHERE cs.id = ?1 AND cs.deleted = 0",
        params![id],
        |row| {
            let code_ids_json: String = row.get(3)?;
            Ok(CodedSegment {
                id: row.get(0)?,
                interview_id: row.get(1)?,
                segment_id: row.get(2)?,
                code_ids: serde_json::from_str(&code_ids_json).unwrap_or_default(),
                coder_name: row.get(4)?,
                memo: row.get(5)?,
                char_start: row.get(6)?,
                char_end: row.get(7)?,
                quote_text: coded_quote(&row.get::<_, String>(8)?, row.get(6)?, row.get(7)?),
                block_id: row.get(9)?,
                timestamp_start: row.get(10)?,
                participant_label: row.get(11)?,
            })
        },
    )
}

pub fn list_coded_segments(
    conn: &Connection,
    interview_id: Option<&str>,
    code_id: Option<&str>,
) -> rusqlite::Result<Vec<CodedSegment>> {
    let mut results = Vec::new();
    let base_query = "SELECT cs.id, cs.interview_id, cs.segment_id, cs.code_ids, cs.coder_name, cs.memo,
                cs.char_start, cs.char_end, ts.text, ts.block_id, ts.timestamp_start, i.participant_label
         FROM coded_segments cs
         JOIN transcript_segments ts ON ts.id = cs.segment_id
         JOIN interviews i ON i.id = cs.interview_id
         WHERE cs.deleted = 0";

    // Build rollup match set if filtering by code_id
    let target_code_ids: Option<HashSet<String>> = if let Some(cid) = code_id {
        let mut ids = HashSet::new();
        ids.insert(cid.to_string());
        let mut child_stmt =
            conn.prepare("SELECT id FROM codebook WHERE parent_id = ?1 AND deleted = 0")?;
        let child_rows = child_stmt.query_map(params![cid], |r| r.get::<_, String>(0))?;
        for c in child_rows.flatten() {
            ids.insert(c);
        }
        Some(ids)
    } else {
        None
    };

    if let Some(iid) = interview_id {
        let mut stmt = conn.prepare(&format!(
            "{base_query} AND cs.interview_id = ?1 ORDER BY ts.segment_index"
        ))?;
        let rows = stmt.query_map(params![iid], map_coded_segment_row)?;
        for row in rows {
            let seg = row?;
            if let Some(ref targets) = target_code_ids {
                if seg.code_ids.iter().any(|c| targets.contains(c)) {
                    results.push(seg);
                }
            } else {
                results.push(seg);
            }
        }
    } else {
        let mut stmt = conn.prepare(&format!(
            "{base_query} ORDER BY i.participant_label, ts.segment_index"
        ))?;
        let rows = stmt.query_map([], map_coded_segment_row)?;
        for row in rows {
            let seg = row?;
            if let Some(ref targets) = target_code_ids {
                if seg.code_ids.iter().any(|c| targets.contains(c)) {
                    results.push(seg);
                }
            } else {
                results.push(seg);
            }
        }
    }
    Ok(results)
}

pub fn get_project_deletion_summary(path: &str) -> Result<ProjectDeletionSummary, String> {
    let p = Path::new(path);
    // Legacy probe: a nested `.codemap/project.db` layout that predates the flat
    // one below. Deliberately NOT renamed to `.fleuron` — that would probe a
    // layout which has never existed on disk.
    let db_path = p.join(".codemap").join("project.db");
    let actual_db = if db_path.exists() {
        db_path
    } else {
        p.join("project.db")
    };

    if !actual_db.exists() {
        return Err("Not a valid Fleuron project database".into());
    }

    let conn = Connection::open_with_flags(
        &actual_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;

    let interview_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM interviews WHERE deleted = 0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;
    let coded_segment_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;
    let segment_memos: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0 AND memo IS NOT NULL AND trim(memo) != ''",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;
    let interview_memos: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM interviews WHERE deleted = 0 AND hub_memo IS NOT NULL AND trim(hub_memo) != ''",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    Ok(ProjectDeletionSummary {
        interview_count,
        coded_segment_count,
        memo_count: segment_memos + interview_memos,
    })
}

/// Check if a path string points into a Box cloud storage mount.
///
/// Hard rule, forever: never read, write, move, or delete anything inside a Box mount.
pub fn is_box_path_str(s: &str) -> bool {
    let normalized = s.replace('\\', "/");
    normalized.contains("/Library/CloudStorage/Box") || normalized.contains("/Box Sync/")
}

/// Check if a Path points into a Box cloud storage mount.
pub fn is_box_path(path: &Path) -> bool {
    is_box_path_str(&path.to_string_lossy())
}

/// Delete a project folder, safely guarded by path shape and location.
///
/// 1. If path does not exist, returns Ok(()) (already absent is success).
/// 2. Box hard-rule: refuses any path resolving under a Box cloud mount.
/// 3. Shape guard: refuses unless the folder contains `project.db` or `project.json`.
/// 4. Attempts trashing via NsFileManager on macOS (avoiding AppleScript/Finder errors like -8013).
/// 5. Falls back to permanent recursive delete (`remove_dir_all`) if trashing fails,
///    which unlinks iCloud dataless placeholders without needing downloads.
pub fn delete_project_folder_impl(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(());
    }

    let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if is_box_path(&canon) || is_box_path(p) {
        return Err("Refusing to delete inside the Box folder.".into());
    }

    if !p.join("project.db").exists() && !p.join("project.json").exists() {
        return Err("Refusing to delete folder: not a Fleuron project (missing project.db or project.json).".into());
    }

    #[cfg(target_os = "macos")]
    let trash_result = {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(p)
    };

    #[cfg(not(target_os = "macos"))]
    let trash_result = trash::delete(p);

    if let Err(trash_err) = trash_result {
        // Windows refuses to unlink a file that still has an open handle, and
        // a just-written project.db routinely has one for a moment -- an
        // antivirus/indexer scanning it, or a handle the OS has not released
        // yet. Unix unlinks regardless, which is why this only ever bites on
        // Windows. Retry briefly before calling it a failure; the same
        // transient hits real users deleting a study they just closed.
        let mut perm_result = std::fs::remove_dir_all(p);
        for attempt in 1..=5 {
            match &perm_result {
                Ok(()) => break,
                Err(_) if p.exists() => {
                    std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                    perm_result = std::fs::remove_dir_all(p);
                }
                Err(_) => break,
            }
        }
        if let Err(perm_err) = perm_result {
            if !p.exists() {
                return Ok(());
            }
            return Err(format!(
                "Could not delete project folder: Trash failed ({trash_err}), and permanent delete fallback failed ({perm_err})."
            ));
        }
    }

    Ok(())
}

fn map_coded_segment_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodedSegment> {
    let code_ids_json: String = row.get(3)?;
    Ok(CodedSegment {
        id: row.get(0)?,
        interview_id: row.get(1)?,
        segment_id: row.get(2)?,
        code_ids: serde_json::from_str(&code_ids_json).unwrap_or_default(),
        coder_name: row.get(4)?,
        memo: row.get(5)?,
        char_start: row.get(6)?,
        char_end: row.get(7)?,
        quote_text: coded_quote(&row.get::<_, String>(8)?, row.get(6)?, row.get(7)?),
        block_id: row.get(9)?,
        timestamp_start: row.get(10)?,
        participant_label: row.get(11)?,
    })
}

pub fn update_hub_memo(conn: &Connection, interview_id: &str, memo: &str) -> rusqlite::Result<()> {
    // Requires exactly one live row: a missing or deleted interview used to
    // report success while writing nothing, which the old autosave then
    // acknowledged as Saved.
    let changed = conn.execute(
        "UPDATE interviews SET hub_memo = ?1, updated_at = ?2 WHERE id = ?3 AND deleted = 0",
        params![memo, now_iso(), interview_id],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn clear_workspace(
    conn: &Connection,
    input: &ClearWorkspaceInput,
) -> rusqlite::Result<ClearWorkspaceResult> {
    let scope = match &input.interview_id {
        Some(id) => format!("interview:{}", id),
        None => "all".to_string(),
    };

    let protocol = local_sync_protocol(conn)?;
    let tx = conn.unchecked_transaction()?;

    // Tombstoned rather than deleted, for the reason on TOMBSTONE_CODED_SEGMENT:
    // a reset that only emptied the local table would be undone by the next pull.
    let ts = now_iso();
    let (cleared_coded_segments, cleared_block_ids) = if protocol == 2 {
        // Protocol 2 propagates through sync_outbox, not the sync_dirty flag, so a
        // blanket local tombstone (as on v1) would never reach the server or other
        // devices — and would be undone on the next pull. Clear this coder's own
        // coding edges, enqueue coding.patch removals so the reset actually syncs,
        // and tombstone this coder's local projection to match. v2 edges are
        // per-actor; a coder can only remove their own (the server enforces this).
        let actor_key = v2_actor_key(&tx, &input.coder_name)?;
        // (interview_id, segment_id, code_id, char_start, char_end) for one edge.
        type ResetEdge = (String, String, String, Option<i32>, Option<i32>);
        let edges: Vec<ResetEdge> = if let Some(ref iid) = input.interview_id {
            let mut stmt = tx.prepare(
                "SELECT interview_id, segment_id, code_id, char_start, char_end
                       FROM coding_assignments
                      WHERE actor_key = ?1 AND present != 0 AND interview_id = ?2",
            )?;
            let rows = stmt
                .query_map(params![actor_key, iid], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        } else {
            let mut stmt = tx.prepare(
                "SELECT interview_id, segment_id, code_id, char_start, char_end
                       FROM coding_assignments
                      WHERE actor_key = ?1 AND present != 0",
            )?;
            let rows = stmt
                .query_map(params![actor_key], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        for (interview_id, segment_id, code_id, char_start, char_end) in &edges {
            let edge_input = crate::models::MutateCodingEdgeInput {
                interview_id: interview_id.clone(),
                segment_id: segment_id.clone(),
                code_id: code_id.clone(),
                coder_name: input.coder_name.clone(),
                char_start: *char_start,
                char_end: *char_end,
                present: false,
            };
            set_v2_assignment_edge(&tx, &edge_input, false, &ts)?;
            enqueue_v2_coding_patch(&tx, &edge_input, &ts)?;
        }

        let cs = if let Some(ref iid) = input.interview_id {
            tx.execute(
                "UPDATE coded_segments
                 SET deleted = 1, code_ids = '[]', memo = NULL,
                     sync_dirty = 1, revision = revision + 1, updated_at = ?3
                 WHERE coder_name = ?1 AND interview_id = ?2 AND deleted = 0",
                params![input.coder_name, iid, ts],
            )? as i32
        } else {
            tx.execute(
                "UPDATE coded_segments
                 SET deleted = 1, code_ids = '[]', memo = NULL,
                     sync_dirty = 1, revision = revision + 1, updated_at = ?2
                 WHERE coder_name = ?1 AND deleted = 0",
                params![input.coder_name, ts],
            )? as i32
        };
        let bi = if let Some(ref iid) = input.interview_id {
            tx.execute(
                "UPDATE transcript_segments SET block_id = NULL WHERE interview_id = ?1",
                params![iid],
            )? as i32
        } else {
            tx.execute("UPDATE transcript_segments SET block_id = NULL", [])? as i32
        };
        (cs, bi)
    } else if let Some(ref iid) = input.interview_id {
        let cs = tx.execute(
            "UPDATE coded_segments
             SET deleted = 1, code_ids = '[]', memo = NULL,
                 sync_dirty = 1, revision = revision + 1, updated_at = ?2
             WHERE interview_id = ?1 AND deleted = 0",
            params![iid, ts],
        )? as i32;
        let bi = tx.execute(
            "UPDATE transcript_segments SET block_id = NULL WHERE interview_id = ?1",
            params![iid],
        )? as i32;
        tx.execute(
            "UPDATE coding_assignments SET present = 0, updated_at = ?2
             WHERE interview_id = ?1 AND present != 0",
            params![iid, ts],
        )?;
        (cs, bi)
    } else {
        let cs = tx.execute(
            "UPDATE coded_segments
             SET deleted = 1, code_ids = '[]', memo = NULL,
                 sync_dirty = 1, revision = revision + 1, updated_at = ?1
             WHERE deleted = 0",
            params![ts],
        )? as i32;
        let bi = tx.execute("UPDATE transcript_segments SET block_id = NULL", [])? as i32;
        tx.execute(
            "UPDATE coding_assignments SET present = 0, updated_at = ?1 WHERE present != 0",
            params![ts],
        )?;
        (cs, bi)
    };

    let cleared_hub_memos = if input.clear_hub_memos {
        if let Some(ref iid) = input.interview_id {
            tx.execute(
                "UPDATE interviews SET hub_memo = NULL, updated_at = ?1 WHERE id = ?2",
                params![now_iso(), iid],
            )? as i32
        } else {
            tx.execute(
                "UPDATE interviews SET hub_memo = NULL, updated_at = ?1",
                params![now_iso()],
            )? as i32
        }
    } else {
        0
    };

    let cleared_activity_log = if input.clear_activity_log {
        tx.execute("DELETE FROM coder_activity_log", [])?;
        true
    } else {
        false
    };

    let detail = format!(
        "scope={}, cleared_coded_segments={}, cleared_block_ids={}, cleared_hub_memos={}, cleared_activity_log={}",
        scope, cleared_coded_segments, cleared_block_ids, cleared_hub_memos, cleared_activity_log
    );
    log_activity(&tx, &input.coder_name, "reset_workspace", Some(&detail))?;

    tx.commit()?;

    Ok(ClearWorkspaceResult {
        cleared_coded_segments,
        cleared_block_ids,
        cleared_hub_memos,
        cleared_activity_log,
        scope,
    })
}

/// Containers a transcript may arrive in.
///
/// This list is a *permission* boundary, not a convenience: the app will read
/// a file of one of these types from anywhere on disk, and anything else only
/// from inside the open project. Widened from `vtt` alone when import stopped
/// being Zoom-only — a transcript now routinely arrives as `.srt` from a
/// recorder, `.txt` from Otter or Rev, or `.docx` from a transcription service,
/// and all of them live in Box rather than in the project folder.
const TRANSCRIPT_EXTS: [&str; 7] = ["vtt", "srt", "txt", "md", "csv", "tsv", "docx"];

/// Whether this path may be read at all, and why.
///
/// Split out so the text and binary readers cannot drift apart — the binary
/// path exists for `.docx`, and a gate that guarded only one of them would be
/// no gate.
fn check_readable(project_path: Option<&Path>, p: &Path) -> Result<(), String> {
    const MAX_BYTES: u64 = 50 * 1024 * 1024;
    let len = fs::metadata(p).map_err(|e| e.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("File is {} MB; max is 50 MB.", len / 1024 / 1024));
    }

    let is_transcript = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| TRANSCRIPT_EXTS.iter().any(|t| e.eq_ignore_ascii_case(t)));

    let in_project = match project_path {
        Some(proj) => match (proj.canonicalize().ok(), p.canonicalize().ok()) {
            (Some(pr), Some(fc)) => fc.starts_with(&pr),
            _ => false,
        },
        None => false,
    };

    if !is_transcript && !in_project {
        return Err(format!(
            "Can only read files inside the open project, or a transcript ({}).",
            TRANSCRIPT_EXTS
                .iter()
                .map(|e| format!(".{e}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

pub fn read_text_file(project_path: Option<&Path>, path: &str) -> Result<String, String> {
    let p = Path::new(&path);
    check_readable(project_path, p)?;

    // Lossy rather than strict: transcription services emit stray Windows-1252
    // bytes inside otherwise-clean UTF-8, and losing a smart quote beats
    // refusing the interview.
    let bytes = fs::read(p).map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if s.starts_with('\u{FEFF}') {
        s.remove(0);
    }
    Ok(s)
}

pub fn read_binary_file(project_path: Option<&Path>, path: &str) -> Result<Vec<u8>, String> {
    let p = Path::new(&path);
    check_readable(project_path, p)?;
    fs::read(p).map_err(|e| e.to_string())
}

pub fn list_project_files(project_path: &Path) -> Result<Vec<ProjectFileEntry>, String> {
    let exports_dir = project_path.join("exports");
    if !exports_dir.exists() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();
    collect_project_files(&exports_dir, &exports_dir, &mut entries)?;
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries)
}

fn collect_project_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<ProjectFileEntry>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        if meta.is_dir() {
            collect_project_files(root, &path, out)?;
            continue;
        }

        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match kind.as_str() {
            "csv" => "csv",
            "md" => "markdown",
            "json" => "json",
            _ => "other",
        };

        out.push(ProjectFileEntry {
            path: path.to_string_lossy().to_string(),
            relative_path: relative,
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string(),
            size: meta.len(),
            modified_at: meta.modified().ok().map(system_time_to_iso),
            kind: kind.to_string(),
        });
    }
    Ok(())
}

pub fn copy_project_file(
    project_path: &Path,
    source: &str,
    destination: &str,
) -> Result<(), String> {
    // `destination` is intentionally unrestricted: callers MUST obtain it via a
    // native save dialog so the user picks the write location themselves.
    let source_path = Path::new(source)
        .canonicalize()
        .map_err(|e| format!("Source file not found: {e}"))?;
    let exports_root = project_path
        .join("exports")
        .canonicalize()
        .map_err(|e| format!("Exports folder not found: {e}"))?;
    if !source_path.starts_with(&exports_root) {
        return Err("Can only copy files from the project exports folder".into());
    }
    if let Some(parent) = Path::new(destination).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&source_path, destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Clone)]
struct ExportCodedRow {
    interview_id: String,
    participant_label: String,
    block_id: Option<String>,
    timestamp_start: String,
    speaker: String,
    code_ids: Vec<String>,
    quote_text: String,
    segment_memo: Option<String>,
    coder_name: String,
    created_at: String,
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn sanitize_filename(label: &str) -> String {
    let sanitized: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "interview".to_string()
    } else {
        sanitized
    }
}

fn code_names_for_ids(code_map: &HashMap<String, String>, code_ids: &[String]) -> String {
    code_ids
        .iter()
        .filter_map(|id| code_map.get(id).cloned())
        .collect::<Vec<_>>()
        .join("; ")
}

/// The words a coding actually refers to.
///
/// For a whole-turn coding that is the passage; for a highlight it is only the
/// marked stretch. Getting this wrong is quiet and expensive: an export that
/// prints the entire speaker turn for a phrase-level code hands the analyst a
/// quote they did not choose, and there is nothing in the file to reveal it.
///
/// Offsets are clamped rather than trusted. A passage can be re-imported with a
/// corrected transcript under the same id, leaving a stored offset past the new
/// end.
///
/// ⚠️ Offsets are produced by the browser, which counts **UTF-16 code units**,
/// while this slices by Unicode scalar. The two agree for everything in the
/// Basic Multilingual Plane — all ordinary speech transcription — and diverge
/// only where astral characters such as emoji appear. Falling back to the whole
/// passage on a boundary miss keeps that case merely imprecise rather than
/// panicking or truncating mid-character.
fn coded_quote(text: &str, char_start: Option<i64>, char_end: Option<i64>) -> String {
    let (Some(start), Some(end)) = (char_start, char_end) else {
        return text.to_string();
    };
    if start < 0 || end <= start {
        return text.to_string();
    }

    let mut indices = text.char_indices().map(|(i, _)| i).collect::<Vec<_>>();
    indices.push(text.len());

    let lo = indices.get(start as usize).copied();
    let hi = indices
        .get(end as usize)
        .copied()
        .or_else(|| indices.last().copied());

    match (lo, hi) {
        (Some(lo), Some(hi)) if hi > lo => text[lo..hi].to_string(),
        _ => text.to_string(),
    }
}

fn fetch_export_coded_rows(conn: &Connection) -> rusqlite::Result<Vec<ExportCodedRow>> {
    let mut stmt = conn.prepare(
        "SELECT cs.interview_id, i.participant_label, ts.block_id, ts.timestamp_start,
                ts.speaker, cs.code_ids, ts.text, cs.memo, cs.coder_name, cs.created_at,
                cs.char_start, cs.char_end
         FROM coded_segments cs
         JOIN transcript_segments ts ON ts.id = cs.segment_id
         JOIN interviews i ON i.id = cs.interview_id
         WHERE cs.deleted = 0
         ORDER BY i.participant_label, ts.segment_index, cs.created_at",
    )?;
    let rows = stmt.query_map([], |row| {
        let code_ids_json: String = row.get(5)?;
        Ok(ExportCodedRow {
            interview_id: row.get(0)?,
            participant_label: row.get(1)?,
            block_id: row.get(2)?,
            timestamp_start: row.get(3)?,
            speaker: row.get(4)?,
            code_ids: serde_json::from_str(&code_ids_json).unwrap_or_default(),
            quote_text: coded_quote(&row.get::<_, String>(6)?, row.get(10)?, row.get(11)?),
            segment_memo: row.get(7)?,
            coder_name: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    rows.collect()
}

/// Per-interview speaker aliases for redacted exports (T06).
///
/// Same rule as the frontend `speaker-alias.ts`: the exact name "Interviewer"
/// is exempt and never numbered; every other distinct name — including
/// "Unknown" — becomes Speaker 1..N in first-appearance (`segment_index`)
/// order, independently per interview.
fn speaker_alias_map(
    conn: &Connection,
    interview_id: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT speaker FROM transcript_segments
         WHERE interview_id = ?1 ORDER BY segment_index",
    )?;
    let speakers = stmt.query_map([interview_id], |row| row.get::<_, String>(0))?;
    let mut aliases = HashMap::new();
    let mut n = 0;
    for speaker in speakers {
        let name = speaker?.trim().to_string();
        if name.is_empty() || name == "Interviewer" || aliases.contains_key(&name) {
            continue;
        }
        n += 1;
        aliases.insert(name, format!("Speaker {n}"));
    }
    Ok(aliases)
}

fn write_coded_segments_csv(
    path: &Path,
    rows: &[ExportCodedRow],
    code_map: &HashMap<String, String>,
) -> rusqlite::Result<()> {
    let mut file = fs::File::create(path)
        .map_err(|e| rusqlite::Error::InvalidParameterName(format!("Failed to create CSV: {e}")))?;

    file.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| rusqlite::Error::InvalidParameterName(format!("Failed to write CSV: {e}")))?;

    writeln!(
        file,
        "participant_label,block_id,timestamp_start,speaker,code_names,quote_text,segment_memo,coder_name,created_at"
    )
    .map_err(|e| rusqlite::Error::InvalidParameterName(format!("Failed to write CSV: {e}")))?;

    for row in rows {
        let line = [
            csv_field(&row.participant_label),
            csv_field(row.block_id.as_deref().unwrap_or("")),
            csv_field(&row.timestamp_start),
            csv_field(&row.speaker),
            csv_field(&code_names_for_ids(code_map, &row.code_ids)),
            csv_field(&row.quote_text),
            csv_field(row.segment_memo.as_deref().unwrap_or("")),
            csv_field(&row.coder_name),
            csv_field(&row.created_at),
        ]
        .join(",");
        writeln!(file, "{line}").map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("Failed to write CSV: {e}"))
        })?;
    }

    Ok(())
}

pub fn export_with_config(
    conn: &Connection,
    target_dir: &Path,
    config: &ExportConfigInput,
    report_html: Option<&str>,
    framework_matrix_csv: Option<&str>,
    coder_name: &str,
    // Interviews with speaker redaction on (per-interview toggle, app
    // preferences, local-only). Their CSV speaker column carries aliases;
    // every other interview exports real names as before.
    redacted_interview_ids: &std::collections::HashSet<String>,
) -> rusqlite::Result<ExportResult> {
    let title: String = conn
        .query_row(
            "SELECT value FROM project_meta WHERE key = 'title'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "study".to_string());
    let unresolved_conflict_count = conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE status = 'unresolved'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let unresolved_conflict_count = unresolved_conflict_count.max(0) as usize;
    let exported_at = now_iso();
    let date_stamp = if exported_at.len() >= 10 {
        &exported_at[0..10]
    } else {
        "export"
    };

    let slug = sanitize_filename(&title);
    let base_folder_name = format!("{slug}-export-{date_stamp}");

    // Suffix collision handling: check target_dir.join(base_folder_name), then -2, -3...
    let mut chosen_dir = target_dir.join(&base_folder_name);
    let mut suffix = 2;
    while chosen_dir.exists() {
        let folder_name = format!("{slug}-export-{date_stamp}-{suffix}");
        chosen_dir = target_dir.join(&folder_name);
        suffix += 1;
    }

    fs::create_dir_all(&chosen_dir).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("Failed to create export folder: {e}"))
    })?;

    let mut written_files = Vec::new();

    // 1. report.html
    if config.items.iter().any(|i| i == "report-html") {
        if let Some(html) = report_html {
            let html_path = chosen_dir.join("report.html");
            fs::write(&html_path, html).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!("Failed to write report.html: {e}"))
            })?;
            written_files.push(html_path.to_string_lossy().to_string());
        }
    }

    // 1b. report.pdf
    if config.items.iter().any(|i| i == "report-pdf") {
        if let Some(html) = report_html {
            let pdf_path = chosen_dir.join("report.pdf");
            crate::pdf::render_report_pdf(html, &pdf_path).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!("Failed to write report.pdf: {e}"))
            })?;
            written_files.push(pdf_path.to_string_lossy().to_string());
        }
    }

    // 2. coded-segments.csv
    let mut coded_rows = fetch_export_coded_rows(conn)?;
    if config.include_participant_scope == "selected" {
        if let Some(ref selected_ids) = config.selected_participant_ids {
            coded_rows.retain(|r| selected_ids.contains(&r.interview_id));
        }
    }
    if config.include_coder_scope == "active-coder" && !coder_name.is_empty() {
        coded_rows.retain(|r| r.coder_name == coder_name);
    }

    // Speaker redaction (T06): alias the speaker column for toggled
    // interviews. The database keeps the real names; only the file changes.
    if !redacted_interview_ids.is_empty() {
        let mut maps: HashMap<String, HashMap<String, String>> = HashMap::new();
        for row in &mut coded_rows {
            if !redacted_interview_ids.contains(&row.interview_id) {
                continue;
            }
            if !maps.contains_key(&row.interview_id) {
                maps.insert(
                    row.interview_id.clone(),
                    speaker_alias_map(conn, &row.interview_id)?,
                );
            }
            if let Some(alias) = maps[&row.interview_id].get(row.speaker.trim()) {
                row.speaker = alias.clone();
            }
        }
    }

    if config.items.iter().any(|i| i == "coded-segments") {
        let code_map = code_name_map(conn)?;
        let csv_path = chosen_dir.join("coded-segments.csv");
        write_coded_segments_csv(&csv_path, &coded_rows, &code_map)?;
        written_files.push(csv_path.to_string_lossy().to_string());
    }

    // 3. framework-matrix.csv
    if config.items.iter().any(|i| i == "framework-matrix") {
        if let Some(matrix_csv) = framework_matrix_csv {
            let matrix_path = chosen_dir.join("framework-matrix.csv");
            fs::write(&matrix_path, matrix_csv).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!(
                    "Failed to write framework-matrix.csv: {e}"
                ))
            })?;
            written_files.push(matrix_path.to_string_lossy().to_string());
        }
    }

    // 4. export-manifest.json
    let manifest = serde_json::json!({
        "exported_at": exported_at,
        "exported_by": coder_name,
        "preset": config.preset,
        "items": config.items,
        "coded_segment_count": coded_rows.len(),
        "unresolved_conflict_count": unresolved_conflict_count,
        "shared_state_policy": "canonical_only",
        "files": written_files,
    });
    let manifest_path = chosen_dir.join("export-manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| rusqlite::Error::InvalidParameterName(format!("Failed to write manifest: {e}")))?;
    written_files.push(manifest_path.to_string_lossy().to_string());

    log_activity(
        conn,
        coder_name,
        "export",
        Some(&chosen_dir.to_string_lossy()),
    )?;

    Ok(ExportResult {
        exports_dir: chosen_dir.to_string_lossy().to_string(),
        coded_segment_count: coded_rows.len(),
        interview_file_count: 0,
        unresolved_conflict_count,
        files: written_files,
        exported_at,
        exported_by: coder_name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_project() -> (tempfile::TempDir, PathBuf, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = create_project(&CreateProjectInput {
            parent_dir: dir.path().to_string_lossy().to_string(),
            project_name: "p".into(),
            title: "T".into(),
            coders: vec!["Alice".into()],
        })
        .unwrap();
        let conn = open_project(&path.to_string_lossy()).unwrap();
        (dir, path, conn)
    }

    #[test]
    fn a_new_project_is_created_with_the_fleuron_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_project(&CreateProjectInput {
            parent_dir: dir.path().to_string_lossy().to_string(),
            project_name: "study".into(),
            title: "T".into(),
            coders: vec!["Alice".into()],
        })
        .unwrap();
        assert_eq!(path.extension().unwrap(), "fleuron");
        assert_eq!(PROJECT_EXT, "fleuron");
    }

    #[test]
    fn every_pre_rename_project_extension_still_opens() {
        // `.codemap` and `.qcproj` are pre-rename names. Projects sitting on
        // disk and on Drive must keep opening after the Fleuron rename.
        for ext in ["fleuron", "codemap", "qcproj"] {
            let dir = tempfile::tempdir().unwrap();
            let path = create_project(&CreateProjectInput {
                parent_dir: dir.path().to_string_lossy().to_string(),
                project_name: format!("study.{ext}"),
                title: "T".into(),
                coders: vec!["Alice".into()],
            })
            .unwrap();

            assert_eq!(
                path.extension().unwrap(),
                ext,
                "an explicit .{ext} suffix must not be double-suffixed"
            );
            assert!(is_project_path(&path), ".{ext} must be recognised");
            assert!(
                open_project(&path.to_string_lossy()).is_ok(),
                ".{ext} must open"
            );
        }
    }

    #[test]
    fn an_unknown_extension_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        let stray = dir.path().join("not-a-study.txt");
        std::fs::create_dir_all(&stray).unwrap();
        assert!(!is_project_path(&stray));
        assert!(open_project(&stray.to_string_lossy()).is_err());
    }

    fn seed_segment(conn: &Connection) -> (String, String) {
        let iv = create_interview(
            conn,
            &CreateInterviewInput {
                participant_label: "P07".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        import_segments(
            conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![SegmentInput {
                    speaker: "P07".into(),
                    timestamp_start: "00:00:00.000".into(),
                    timestamp_end: None,
                    text: "I rehearse the hello on the drive in, every single morning".into(),
                    section_tag: None,
                }],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let seg = get_segments(conn, &iv.id).unwrap()[0].id.clone();
        (iv.id, seg)
    }

    fn code_one(
        conn: &Connection,
        iv: &str,
        seg: &str,
        code: &str,
        span: Option<(i32, i32)>,
    ) -> CodedSegment {
        apply_codes(
            conn,
            &ApplyCodesInput {
                interview_id: iv.into(),
                segment_id: seg.into(),
                code_ids: vec![code.into()],
                coder_name: "Ada".into(),
                memo: None,
                char_start: span.map(|(a, _)| a),
                char_end: span.map(|(_, b)| b),
            },
        )
        .unwrap()
    }

    /// 🔑 Highlight-level coding, and the reason the row key had to change.
    ///
    /// Keyed on (passage, coder) alone, a coder held exactly one opinion per
    /// speaker turn, so marking a second phrase inside a turn silently
    /// overwrote the first.
    #[test]
    fn two_highlights_in_one_turn_are_two_codings() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);

        let first = code_one(&conn, &iv, &seg, "c1", Some((0, 16)));
        let second = code_one(&conn, &iv, &seg, "c2", Some((22, 34)));

        assert_ne!(
            first.id, second.id,
            "the second highlight replaced the first"
        );
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 2);
    }

    /// A whole-turn coding is its own row, distinct from any span inside it —
    /// "this turn is about masking" and "this phrase in particular" are two
    /// different claims and a coder may make both.
    #[test]
    fn a_whole_turn_coding_coexists_with_a_highlight_inside_it() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);

        code_one(&conn, &iv, &seg, "c1", None);
        code_one(&conn, &iv, &seg, "c2", Some((0, 16)));

        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 2);
    }

    /// The null-safety half of the key. `NULL = NULL` is NULL, not true, so an
    /// `=` comparison never matches a whole-turn row against itself and every
    /// re-code would open a duplicate instead of revising.
    #[test]
    fn recoding_the_same_span_revises_rather_than_duplicates() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);

        for (whole, span) in [(true, None), (false, Some((0, 16)))] {
            let a = code_one(&conn, &iv, &seg, "c1", span);
            let b = code_one(&conn, &iv, &seg, "c2", span);
            assert_eq!(a.id, b.id, "re-coding opened a second row (whole={whole})");
        }

        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 2, "one whole-turn row and one span row");
    }

    /// 🔴 An export that prints the whole speaker turn for a phrase-level code
    /// hands the analyst a quote they never chose, and nothing in the file
    /// reveals it. `quote_text` was `ts.text` regardless of the span.
    #[test]
    fn a_highlight_exports_only_the_words_it_marked() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);
        // "I rehearse the hello on the drive in, every single morning"
        code_one(&conn, &iv, &seg, "c1", Some((0, 20)));

        let rows = fetch_export_coded_rows(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].quote_text, "I rehearse the hello");
    }

    #[test]
    fn a_whole_turn_coding_still_exports_the_whole_turn() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);
        code_one(&conn, &iv, &seg, "c1", None);

        let rows = fetch_export_coded_rows(&conn).unwrap();
        assert!(rows[0].quote_text.starts_with("I rehearse"));
        assert!(rows[0].quote_text.ends_with("morning"));
    }

    /// Every way a stored offset can be wrong — a shorter re-import, an
    /// inverted pair, a negative — must degrade to the full passage rather than
    /// panic or truncate mid-character.
    #[test]
    fn a_nonsensical_span_falls_back_to_the_passage() {
        let text = "I rehearse the hello";
        for (a, b) in [
            (Some(0), Some(0)),
            (Some(12), Some(4)),
            (Some(-3), Some(8)),
            (Some(500), Some(900)),
            (Some(0), None),
            (None, Some(9)),
        ] {
            assert_eq!(coded_quote(text, a, b), text, "span {a:?}..{b:?}");
        }
    }

    /// Slicing by byte would panic here; slicing by scalar must not.
    #[test]
    fn a_span_over_multibyte_text_slices_on_character_boundaries() {
        let text = "je répète le bonjour — chaque matin";
        assert_eq!(coded_quote(text, Some(3), Some(9)), "répète");
        // Past the end clamps to the end rather than panicking.
        assert_eq!(coded_quote(text, Some(23), Some(9999)), "chaque matin");
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    /// The case that motivated the migration runner: a project created before
    /// sync existed. `CREATE TABLE IF NOT EXISTS` does nothing to it, so without
    /// a migration path its rows would never gain the sync columns and it would
    /// be silently excluded from sync while appearing to work.
    #[test]
    fn migrations_bring_a_v0_database_up_to_date() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO interviews (id, participant_label, interviewers, created_at, updated_at)
               VALUES ('i1','JH','[]','t','t');
             INSERT INTO transcript_segments
               (id, interview_id, segment_index, speaker, timestamp_start, text)
               VALUES ('s1','i1',0,'S','00:00:00','hello');
             INSERT INTO coded_segments
               (id, interview_id, segment_id, code_ids, coder_name, created_at, updated_at)
               VALUES ('c1','i1','s1','[\"x\"]','Ada','t','t');
             INSERT INTO codebook
               (id, name, created_at, updated_at)
               VALUES ('code-1','Existing code','t','t');",
        )
        .unwrap();

        assert_eq!(
            user_version(&conn),
            0,
            "a database predating sync carries no schema version"
        );

        run_migrations(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);

        let (deleted, dirty): (i64, i64) = conn
            .query_row(
                "SELECT deleted, sync_dirty FROM coded_segments WHERE id = 'c1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (deleted, dirty),
            (0, 1),
            "coding written before sync existed must survive and queue for upload"
        );

        let code_dirty: i64 = conn
            .query_row(
                "SELECT sync_dirty FROM codebook WHERE id = 'code-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            code_dirty, 1,
            "existing codebook rows must queue once for sync"
        );

        // `ALTER TABLE ADD COLUMN` is not idempotent, so re-running is only safe
        // because of the version gate. Assert that rather than trusting it.
        run_migrations(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    }

    #[test]
    fn a_fresh_project_is_already_at_the_current_schema_version() {
        let (_dir, _path, conn) = make_project();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    }

    fn connect_at_schema_v5() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for sql in &MIGRATIONS[..5] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch("PRAGMA user_version = 5;").unwrap();
        conn
    }

    #[test]
    fn migration_6_normalizes_live_edges_and_preserves_legacy_salvage_rows() {
        let conn = connect_at_schema_v5();
        conn.execute_batch(
            "INSERT INTO interviews (id, participant_label, interviewers, created_at, updated_at)
               VALUES ('iv-1', 'P-synthetic', '[]', 't0', 't0');
             INSERT INTO transcript_segments
               (id, interview_id, segment_index, speaker, timestamp_start, text)
               VALUES ('seg-1', 'iv-1', 0, 'Synthetic', '00:00:00', 'local only transcript'),
                      ('seg-2', 'iv-1', 1, 'Synthetic', '00:00:01', 'local only transcript');
             INSERT INTO coded_segments
               (id, interview_id, segment_id, code_ids, coder_name, memo, char_start, char_end,
                created_at, updated_at, revision, deleted, sync_dirty, base_code_ids)
               VALUES
               ('whole', 'iv-1', 'seg-1', '[\"code-a\",\"code-b\"]', 'Same Name', 'private memo', NULL, NULL, 't1', 't1', 0, 0, 1, NULL),
               ('span', 'iv-1', 'seg-1', '[\"code-b\"]', 'Same Name', 'private memo', 2, 7, 't2', 't2', 0, 0, 1, NULL),
               ('tomb', 'iv-1', 'seg-2', '[]', 'Same Name', NULL, NULL, NULL, 't3', 't3', 1, 1, 1, NULL);
             INSERT INTO pending_coded_segments
               (id, interview_id, segment_id, code_ids, coder_name, revision, deleted, updated_at)
               VALUES ('pending', 'iv-missing', 'seg-missing', '[\"code-a\"]', 'Same Name', 1, 0, 't4');",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let assignments: Vec<(String, String, String, i64, i64)> = conn
            .prepare(
                "SELECT actor_key, span_key, code_id, coalesce(char_start, -1), present
                 FROM coding_assignments ORDER BY span_key, code_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(assignments.len(), 3);
        assert!(assignments
            .iter()
            .all(|entry| entry.0.starts_with("legacy:")));
        assert!(assignments
            .iter()
            .any(|entry| entry.1 == "whole" && entry.2 == "code-a"));
        assert!(assignments
            .iter()
            .any(|entry| entry.1 == "whole" && entry.2 == "code-b"));
        assert!(assignments
            .iter()
            .any(|entry| entry.1 == "span:2:7" && entry.2 == "code-b"));
        assert!(assignments.iter().all(|entry| entry.4 == 1));

        let memo_column_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('coding_assignments') WHERE name = 'memo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(memo_column_count, 0);
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM coded_segments WHERE id = 'tomb' AND deleted = 1",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM pending_coded_segments WHERE id = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sync_state_value(&conn, "sync_protocol").unwrap().as_deref(),
            Some("1")
        );

        run_migrations(&conn).unwrap();
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM coding_assignments", [], |row| row
                .get(0))
                .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row::<String, _, _>("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap(),
            "ok"
        );
        let foreign_key_errors: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_errors, 0);
    }

    #[test]
    fn migration_6_failure_rolls_back_schema_and_version() {
        let conn = connect_at_schema_v5();
        let mut migrations = MIGRATIONS.to_vec();
        migrations[5] = "CREATE TABLE migration_rollback_probe (id INTEGER PRIMARY KEY); INSERT INTO missing_table VALUES (1);";

        assert!(run_migrations_with(&conn, &migrations).is_err());
        assert_eq!(user_version(&conn), 5);
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'migration_rollback_probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
    }

    #[test]
    fn protocol_1_coding_mutations_keep_normalized_edges_in_the_same_transaction() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        let input = ApplyCodesInput {
            interview_id,
            segment_id,
            code_ids: vec!["code-a".into(), "code-b".into()],
            coder_name: "Synthetic Coder".into(),
            memo: Some("local-only memo".into()),
            char_start: None,
            char_end: None,
        };
        let coded = apply_codes(&conn, &input).unwrap();
        let live_after_add: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coding_assignments WHERE present = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_after_add, 2);

        apply_codes(
            &conn,
            &ApplyCodesInput {
                code_ids: vec!["code-b".into()],
                ..input
            },
        )
        .unwrap();
        let (a_present, b_present): (i64, i64) = conn
            .query_row(
                "SELECT
                   max(CASE WHEN code_id = 'code-a' THEN present ELSE 0 END),
                   max(CASE WHEN code_id = 'code-b' THEN present ELSE 0 END)
                 FROM coding_assignments",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((a_present, b_present), (0, 1));

        delete_coded_segment(&conn, &coded.id).unwrap();
        let live_after_delete: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coding_assignments WHERE present = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_after_delete, 0);
    }

    #[test]
    fn every_historical_sqlite_version_reaches_protocol_v2_schema() {
        for version in 0..MIGRATIONS.len() {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            for sql in &MIGRATIONS[..version] {
                conn.execute_batch(sql).unwrap();
            }
            conn.execute_batch(&format!("PRAGMA user_version = {version};"))
                .unwrap();
            run_migrations(&conn).unwrap();
            assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
            assert_eq!(
                conn.query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sync_outbox'",
                    [],
                    |row| row.get(0),
                )
                .unwrap(),
                1
            );
        }
    }

    #[test]
    fn live_workspace_snapshot_groups_v2_edges_and_keeps_shared_slices_coherent() {
        let (_dir, path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        let first = create_code(
            &conn,
            &CreateCodeInput {
                name: "First".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap();
        let second = create_code(
            &conn,
            &CreateCodeInput {
                name: "Second".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap();
        apply_codes(
            &conn,
            &ApplyCodesInput {
                interview_id: interview_id.clone(),
                segment_id: segment_id.clone(),
                code_ids: vec![first.id.clone()],
                coder_name: "Synthetic Coder".into(),
                memo: Some("local-only".into()),
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        conn.execute(
            "UPDATE coding_assignments SET present = 0 WHERE code_id = ?1",
            params![first.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO coding_assignments
               (interview_id, segment_id, actor_key, span_key, code_id, present, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'whole', ?4, 1, 't', 't')",
            params![
                interview_id,
                segment_id,
                legacy_actor_key("Synthetic Coder"),
                second.id,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sync_conflicts
               (conflict_id, project_id, generation, entity_type, entity_id, field_name,
                current_json, proposed_json, originating_op_id, created_at)
             VALUES ('conflict-1', 'project', 'generation', 'code', 'code-1', 'name',
                     '{}', '{}', 'op-1', 't')",
            [],
        )
        .unwrap();

        let snapshot =
            get_live_workspace_snapshot(&conn, path.to_str().unwrap(), Some(&interview_id))
                .unwrap();

        assert_eq!(
            snapshot.active_interview_id.as_deref(),
            Some(interview_id.as_str())
        );
        assert_eq!(snapshot.coded_segments.len(), 1);
        assert_eq!(snapshot.coded_segments[0].code_ids, vec![second.id]);
        assert_eq!(snapshot.conflicts.len(), 1);
        assert_eq!(snapshot.sync_status.protocol, 2);
        assert_eq!(snapshot.coded_count, 1);
        assert_eq!(snapshot.segments.len(), 1);
    }

    #[test]
    fn ensure_code_and_apply_returns_the_committed_exact_id_and_is_case_stable() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        let input = EnsureCodeAndApplyInput {
            name: "Mac 1".into(),
            color: Some("#8a6410".into()),
            interview_id: interview_id.clone(),
            segment_id: segment_id.clone(),
            coder_name: "Synthetic Coder".into(),
            char_start: None,
            char_end: None,
        };

        let first = ensure_code_and_apply(&conn, &input).unwrap();
        assert!(first.created);
        assert_eq!(first.coded_segment.code_ids, vec![first.code.id.clone()]);
        assert_eq!(first.changed_edges.len(), 1);
        assert_eq!(first.changed_edges[0].code_id, first.code.id);

        let second = ensure_code_and_apply(
            &conn,
            &EnsureCodeAndApplyInput {
                name: "  mac 1  ".into(),
                ..input
            },
        )
        .unwrap();
        assert!(!second.created);
        assert_eq!(second.code.id, first.code.id);
        assert!(second.changed_edges.is_empty());
        assert_eq!(list_codes(&conn).unwrap().len(), 1);
        assert_eq!(
            list_coded_segments(&conn, Some(&interview_id), None)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn ensure_code_and_apply_rolls_back_the_code_when_the_target_is_invalid() {
        let (_dir, _path, conn) = make_project();
        let error = ensure_code_and_apply(
            &conn,
            &EnsureCodeAndApplyInput {
                name: "Synthetic".into(),
                color: None,
                interview_id: "missing-interview".into(),
                segment_id: "missing-segment".into(),
                coder_name: "Synthetic Coder".into(),
                char_start: None,
                char_end: None,
            },
        );
        assert!(error.is_err());
        assert!(list_codes(&conn).unwrap().is_empty());
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn protocol_2_ensure_commits_code_edge_and_adjacent_outbox_operations_together() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(&conn, "project_id", "project-synthetic").unwrap();
        set_sync_state_value(&conn, "sync_generation", "generation-synthetic").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-000000000001",
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_actor_key", "user:synthetic").unwrap();

        let result = ensure_code_and_apply(
            &conn,
            &EnsureCodeAndApplyInput {
                name: "Protocol 2 code".into(),
                color: None,
                interview_id,
                segment_id,
                coder_name: "Synthetic Coder".into(),
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();
        let ops: Vec<(i64, String, String)> = conn
            .prepare(
                "SELECT client_seq, op_kind, payload_json FROM sync_outbox ORDER BY client_seq",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            ops.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            ops.iter().map(|entry| entry.1.as_str()).collect::<Vec<_>>(),
            vec!["code.create", "coding.patch"]
        );
        assert!(ops.iter().all(|entry| !entry.2.contains("memo")));
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM coding_assignments WHERE code_id = ?1 AND present = 1",
                params![result.code.id],
                |row| row.get(0),
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn protocol_2_clear_workspace_enqueues_coding_removals_and_clears_locally() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(&conn, "project_id", "project-synthetic").unwrap();
        set_sync_state_value(&conn, "sync_generation", "generation-synthetic").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-000000000001",
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_actor_key", "user:synthetic").unwrap();

        let result = ensure_code_and_apply(
            &conn,
            &EnsureCodeAndApplyInput {
                name: "Reset me".into(),
                color: None,
                interview_id,
                segment_id,
                coder_name: "Synthetic Coder".into(),
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();

        // Isolate the reset: drop the create/add ops so the outbox holds only what
        // the reset itself enqueues.
        conn.execute("DELETE FROM sync_outbox", []).unwrap();

        clear_workspace(
            &conn,
            &ClearWorkspaceInput {
                interview_id: None,
                clear_hub_memos: false,
                clear_activity_log: false,
                coder_name: "Synthetic Coder".into(),
            },
        )
        .unwrap();

        // The reset must enqueue a coding.patch removal so it propagates on
        // protocol 2 (the bug: it previously only set the legacy sync_dirty flag,
        // which v2 ignores, so the reset never reached other devices).
        let ops: Vec<(String, String)> = conn
            .prepare("SELECT op_kind, payload_json FROM sync_outbox ORDER BY client_seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            ops.iter().any(|(kind, payload)| kind == "coding.patch"
                && payload.contains("removes")
                && payload.contains(result.code.id.as_str())),
            "reset on protocol 2 must enqueue a coding.patch removal that syncs, got {ops:?}"
        );

        // Local state must reflect the reset for this coder.
        let present_edges: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coding_assignments WHERE present = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(present_edges, 0, "coding edges should be removed locally");
        let live_coded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_coded, 0, "coded segments should be tombstoned locally");
    }

    #[test]
    fn protocol_2_ensure_rolls_back_everything_when_outbox_metadata_is_incomplete() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(&conn, "project_id", "project-synthetic").unwrap();

        let result = ensure_code_and_apply(
            &conn,
            &EnsureCodeAndApplyInput {
                name: "Rollback code".into(),
                color: None,
                interview_id,
                segment_id,
                coder_name: "Synthetic Coder".into(),
                char_start: None,
                char_end: None,
            },
        );

        assert!(result.is_err());
        assert!(list_codes(&conn).unwrap().is_empty());
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM coded_segments", [], |row| row
                .get(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM coding_assignments", [], |row| row
                .get(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn repeating_an_ensure_intent_does_not_duplicate_local_edges_or_outbox_operations() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(&conn, "project_id", "project-synthetic").unwrap();
        set_sync_state_value(&conn, "sync_generation", "generation-synthetic").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-000000000001",
        )
        .unwrap();
        let input = EnsureCodeAndApplyInput {
            name: "Retry stable".into(),
            color: None,
            interview_id,
            segment_id,
            coder_name: "Synthetic Coder".into(),
            char_start: None,
            char_end: None,
        };

        let first = ensure_code_and_apply(&conn, &input).unwrap();
        let second = ensure_code_and_apply(&conn, &input).unwrap();

        assert!(first.created);
        assert!(!second.created);
        assert!(second.changed_edges.is_empty());
        assert_eq!(list_codes(&conn).unwrap().len(), 1);
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM coding_assignments WHERE present = 1",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn inverse_edge_mutation_preserves_a_colleagues_unrelated_assignment() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, segment_id) = seed_segment(&conn);
        let code = create_code(
            &conn,
            &CreateCodeInput {
                name: "Synthetic".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap();
        let edge = MutateCodingEdgeInput {
            interview_id: interview_id.clone(),
            segment_id: segment_id.clone(),
            code_id: code.id.clone(),
            coder_name: "Local".into(),
            char_start: None,
            char_end: None,
            present: true,
        };
        mutate_coding_edge(&conn, &edge).unwrap();
        conn.execute(
            "INSERT INTO coding_assignments
               (interview_id, segment_id, actor_key, span_key, code_id, present, created_at, updated_at)
             VALUES (?1, ?2, 'user:colleague', 'whole', ?3, 1, 't', 't')",
            params![interview_id, segment_id, code.id],
        )
        .unwrap();
        mutate_coding_edge(
            &conn,
            &MutateCodingEdgeInput {
                present: false,
                ..edge
            },
        )
        .unwrap();
        let colleague_present: i64 = conn
            .query_row(
                "SELECT present FROM coding_assignments WHERE actor_key = 'user:colleague'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(colleague_present, 1);
    }

    fn connect_at_schema_v3() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for sql in &MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch("PRAGMA user_version = 3; PRAGMA foreign_keys = OFF;")
            .unwrap();
        conn
    }

    /// 🔴 The 409 that v0.18.0's rescue could not settle: SQLite allowed many
    /// rows for one (passage, coder, span), and un-coding had flattened
    /// highlights onto that slot. Migration 4 must collapse them before it
    /// can add the unique index that matches the server.
    #[test]
    fn migration_4_collapses_duplicate_spans_and_then_rejects_a_second() {
        let conn = connect_at_schema_v3();
        conn.execute_batch(
            "INSERT INTO interviews (id, participant_label, interviewers, created_at, updated_at)
               VALUES ('i1','P07','[]','t','t');
             INSERT INTO transcript_segments
               (id, interview_id, segment_index, speaker, timestamp_start, text)
               VALUES ('s-seven','i1',0,'S','00:00:00','hello'),
                      ('s-live','i1',1,'S','00:00:01','there');",
        )
        .unwrap();

        // Seven whole-turn tombstones on one passage — the 665de4cb shape.
        // Highest revision wins; a memo stranded on a loser is copied over.
        for (id, rev, created, memo) in [
            ("c1", 1, "t1", None),
            ("c2", 1, "t2", Some("stranded")),
            ("c3", 1, "t3", None),
            ("c4", 1, "t4", None),
            ("c5", 1, "t5", None),
            ("c6", 1, "t6", None),
            ("c7", 3, "t7", None),
        ] {
            conn.execute(
                "INSERT INTO coded_segments
                   (id, interview_id, segment_id, code_ids, coder_name, memo,
                    created_at, updated_at, revision, deleted, sync_dirty)
                 VALUES (?1, 'i1', 's-seven', '[]', 'Ada', ?2, ?3, ?3, ?4, 1, 1)",
                params![id, memo, created, rev],
            )
            .unwrap();
        }

        // Live beats a higher-revision tombstone on the same span.
        conn.execute_batch(
            "INSERT INTO coded_segments
               (id, interview_id, segment_id, code_ids, coder_name,
                created_at, updated_at, revision, deleted, sync_dirty)
             VALUES
               ('live-keep', 'i1', 's-live', '[\"x\"]', 'Ada', 't0', 't0', 0, 0, 1),
               ('tomb-drop', 'i1', 's-live', '[]', 'Ada', 't1', 't1', 9, 1, 1);",
        )
        .unwrap();

        assert_eq!(user_version(&conn), 3);
        run_migrations(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);

        let seven: Vec<(String, Option<String>)> = conn
            .prepare("SELECT id, memo FROM coded_segments WHERE segment_id = 's-seven'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            seven.len(),
            1,
            "the seven tombstones must collapse: {seven:?}"
        );
        assert_eq!(seven[0].0, "c7", "highest revision is the keeper");
        assert_eq!(
            seven[0].1.as_deref(),
            Some("stranded"),
            "a memo on a collapsed row must land on the keeper"
        );

        let live_id: String = conn
            .query_row(
                "SELECT id FROM coded_segments WHERE segment_id = 's-live'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live_id, "live-keep", "a live row beats a tombstone");

        let err = conn
            .execute(
                "INSERT INTO coded_segments
                   (id, interview_id, segment_id, code_ids, coder_name,
                    created_at, updated_at)
                 VALUES ('dup', 'i1', 's-seven', '[]', 'Ada', 't', 't')",
                [],
            )
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("unique"),
            "the index must reject a second row for the same span: {err}"
        );
    }

    fn make_interview(conn: &Connection, label: &str) -> Interview {
        create_interview(
            conn,
            &CreateInterviewInput {
                participant_label: label.into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap()
    }

    fn make_segment(text: &str, start: &str) -> SegmentInput {
        SegmentInput {
            speaker: "S".into(),
            timestamp_start: start.into(),
            timestamp_end: None,
            text: text.into(),
            section_tag: None,
        }
    }

    fn apply_one(conn: &Connection, iv_id: &str, seg_id: &str) -> CodedSegment {
        apply_codes(
            conn,
            &ApplyCodesInput {
                interview_id: iv_id.to_string(),
                segment_id: seg_id.to_string(),
                code_ids: vec!["c1".into()],
                coder_name: "Alice".into(),
                memo: None,
                char_start: None,
                char_end: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn renaming_a_group_coder_updates_local_and_pending_attribution() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "JH");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("the group agrees", "00:00:00.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segment_id = get_segments(&conn, &iv.id).unwrap()[0].id.clone();
        let coded = apply_one(&conn, &iv.id, &segment_id);
        conn.execute(
            "INSERT INTO pending_coded_segments
               (id, interview_id, segment_id, code_ids, coder_name, revision, deleted, updated_at)
             VALUES ('pending-1', ?1, 'not-here-yet', '[]', 'Alice', 0, 0, 't')",
            params![iv.id],
        )
        .unwrap();

        rename_project_coder(&conn, "Alice", "Alicia").unwrap();

        let (name, dirty): (String, i64) = conn
            .query_row(
                "SELECT coder_name, sync_dirty FROM coded_segments WHERE id = ?1",
                params![coded.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "Alicia");
        assert_eq!(dirty, 1, "a rename must not erase unsent local work");
        let pending: String = conn
            .query_row(
                "SELECT coder_name FROM pending_coded_segments WHERE id = 'pending-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pending, "Alicia");
        assert_eq!(
            get_meta(&conn, "coders").unwrap().as_deref(),
            Some(r#"["Alicia"]"#)
        );
    }

    #[test]
    fn adopting_a_group_name_collapses_the_local_setup_name() {
        let (_dir, _path, conn) = make_project();
        rename_project_coder(&conn, "alice@example.com", "Ada Lovelace").unwrap();
        rename_project_coder(&conn, "Alice", "Ada Lovelace").unwrap();
        assert_eq!(
            get_meta(&conn, "coders").unwrap().as_deref(),
            Some(r#"["Ada Lovelace"]"#),
            "the email-default membership and the local setup name are one person"
        );
    }

    fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i32 {
        conn.query_row(sql, params, |r| r.get(0)).unwrap()
    }

    #[test]
    fn un_coding_tombstones_the_row_and_re_coding_revives_it() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "JH");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("a", "00:00:00.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let seg_id = get_segments(&conn, &iv.id).unwrap()[0].id.clone();
        let coded = apply_one(&conn, &iv.id, &seg_id);

        delete_coded_segment(&conn, &coded.id).unwrap();

        assert!(
            list_coded_segments(&conn, None, None).unwrap().is_empty(),
            "an un-coded passage must be invisible to the app"
        );
        let (deleted, dirty, codes): (i64, i64, String) = conn
            .query_row(
                "SELECT deleted, sync_dirty, code_ids FROM coded_segments WHERE id = ?1",
                params![coded.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (deleted, dirty),
            (1, 1),
            "the row must survive as a tombstone queued for the other machine"
        );
        assert_eq!(codes, "[]", "a tombstone must not retain its codes");

        // Re-coding the same passage as the same coder must reuse the row id —
        // the other machine already knows it, and a second row would leave two
        // records of one (passage, coder) with only one of them live.
        let again = apply_one(&conn, &iv.id, &seg_id);
        assert_eq!(again.id, coded.id);
        assert_eq!(list_coded_segments(&conn, None, None).unwrap().len(), 1);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM coded_segments", &[]),
            1,
            "revival must not open a second row"
        );
    }

    /// Offsets are the row's identity. Nulling them on un-code moved a
    /// highlight onto the whole-turn unique slot and 409'd every later push.
    #[test]
    fn un_coding_a_highlight_keeps_its_span_and_re_coding_revives_it() {
        let (_dir, _path, conn) = make_project();
        let (iv, seg) = seed_segment(&conn);
        let coded = code_one(&conn, &iv, &seg, "c1", Some((494, 577)));

        delete_coded_segment(&conn, &coded.id).unwrap();

        let (deleted, start, end): (i64, Option<i32>, Option<i32>) = conn
            .query_row(
                "SELECT deleted, char_start, char_end FROM coded_segments WHERE id = ?1",
                params![coded.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(
            (start, end),
            (Some(494), Some(577)),
            "a highlight tombstone must keep the span that identifies it"
        );

        let again = code_one(&conn, &iv, &seg, "c2", Some((494, 577)));
        assert_eq!(
            again.id, coded.id,
            "re-coding the same highlight minted a new id"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM coded_segments", &[]),
            1,
            "revival must not open a second row"
        );
    }

    #[test]
    fn deleting_a_code_everywhere_tombstones_it_rather_than_dropping_the_row() {
        let (_dir, _path, conn) = make_project();
        let code = create_code(
            &conn,
            &CreateCodeInput {
                name: "Technique".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap();

        delete_code(&conn, &code.id, DeleteCodeMode::Purge).unwrap();

        assert!(
            list_codes(&conn).unwrap().iter().all(|c| c.id != code.id),
            "a deleted code must not appear in the codebook"
        );
        let (deleted, dirty): (i64, i64) = conn
            .query_row(
                "SELECT deleted, sync_dirty FROM codebook WHERE id = ?1",
                params![code.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((deleted, dirty), (1, 1));
    }

    #[test]
    fn codebook_mutations_bump_revision_and_queue_sync() {
        let (_dir, _path, conn) = make_project();
        let code = create_code(
            &conn,
            &CreateCodeInput {
                name: "Technique".into(),
                definition: Some("A starting definition".into()),
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: Some("#123456".into()),
            },
        )
        .unwrap();

        conn.execute(
            "UPDATE codebook SET sync_dirty = 0, revision = 7 WHERE id = ?1",
            params![code.id],
        )
        .unwrap();
        update_code(
            &conn,
            &UpdateCodeInput {
                id: code.id.clone(),
                name: "Renamed technique".into(),
                definition: Some("A revised definition".into()),
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: "#654321".into(),
            },
        )
        .unwrap();

        let (dirty, revision): (i64, i64) = conn
            .query_row(
                "SELECT sync_dirty, revision FROM codebook WHERE id = ?1",
                params![code.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((dirty, revision), (1, 8));

        conn.execute(
            "UPDATE codebook SET sync_dirty = 0 WHERE id = ?1",
            params![code.id],
        )
        .unwrap();
        delete_code(&conn, &code.id, DeleteCodeMode::Retire).unwrap();
        let (retired, dirty, revision): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_retired, sync_dirty, revision FROM codebook WHERE id = ?1",
                params![code.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((retired, dirty, revision), (1, 1, 9));

        conn.execute(
            "UPDATE codebook SET sync_dirty = 0 WHERE id = ?1",
            params![code.id],
        )
        .unwrap();
        restore_code(&conn, &code.id).unwrap();
        let (retired, dirty, revision): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_retired, sync_dirty, revision FROM codebook WHERE id = ?1",
                params![code.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((retired, dirty, revision), (0, 1, 10));
    }

    /// The property the whole deterministic-id change exists for: two coders
    /// importing the same transcript on their own machines produce the same
    /// ids, so their coding refers to the same passages without either of them
    /// ever sending the other a project file.
    #[test]
    fn two_independent_imports_of_one_transcript_agree_on_ids() {
        let segments = || {
            vec![
                make_segment("she practised smiling in the mirror", "00:00:00.000"),
                make_segment("it takes the whole evening to recover", "00:00:04.000"),
            ]
        };

        let ids_from_a_fresh_project = || {
            let (_dir, _path, conn) = make_project();
            let iv = make_interview(&conn, "P07");
            import_segments(
                &conn,
                &ImportSegmentsInput {
                    interview_id: iv.id.clone(),
                    segments: segments(),
                    raw_vtt_path: None,
                },
            )
            .unwrap();
            (
                iv.id.clone(),
                get_segments(&conn, &iv.id)
                    .unwrap()
                    .into_iter()
                    .map(|s| s.id)
                    .collect::<Vec<_>>(),
            )
        };

        let (ada_iv, ada_segs) = ids_from_a_fresh_project();
        let (sam_iv, sam_segs) = ids_from_a_fresh_project();

        assert_eq!(ada_iv, sam_iv, "the study label decides the id");
        assert_eq!(
            ada_segs, sam_segs,
            "identical transcripts must yield identical passage ids on two machines"
        );
        assert_eq!(ada_segs.len(), 2);
    }

    #[test]
    fn re_importing_the_same_transcript_keeps_its_coding() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "P07");
        let load = |text: &str| {
            import_segments(
                &conn,
                &ImportSegmentsInput {
                    interview_id: iv.id.clone(),
                    segments: vec![
                        make_segment("she practised smiling in the mirror", "00:00:00.000"),
                        make_segment(text, "00:00:04.000"),
                    ],
                    raw_vtt_path: None,
                },
            )
            .unwrap()
        };

        load("it takes the whole evening to recover");
        let first = get_segments(&conn, &iv.id).unwrap()[0].id.clone();
        apply_one(&conn, &iv.id, &first);
        assert_eq!(list_coded_segments(&conn, None, None).unwrap().len(), 1);

        // A corrected transcript: the second passage is retyped, the first is
        // untouched. Before ids were derived from content this wiped both.
        load("it takes the entire evening to recover");

        let coded = list_coded_segments(&conn, None, None).unwrap();
        assert_eq!(
            coded.len(),
            1,
            "coding on an unchanged passage must survive a re-import"
        );
        assert_eq!(coded[0].segment_id, first);
        assert_eq!(
            get_segments(&conn, &iv.id).unwrap().len(),
            2,
            "the retyped passage replaced the old one rather than adding to it"
        );
    }

    #[test]
    fn one_study_label_is_one_interview() {
        let (_dir, _path, conn) = make_project();
        make_interview(&conn, "P07");
        let again = create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "p07".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        );
        assert!(
            again.is_err(),
            "a repeated study label must be refused, not silently collide"
        );
    }

    /// Passages that the new transcript no longer contains take their coding
    /// with them. Renamed from `..._clears_coded_first`, which described the
    /// pre-deterministic-id behaviour of wiping every coded segment on import —
    /// that is no longer what happens, and the surviving case is covered by
    /// `re_importing_the_same_transcript_keeps_its_coding` below.
    #[test]
    fn import_drops_coding_only_for_passages_that_disappeared() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "A");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    make_segment("a", "00:00:00.000"),
                    make_segment("b", "00:00:01.000"),
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO coded_segments (id,interview_id,segment_id,code_ids,coder_name,
             memo,char_start,char_end,created_at,updated_at)
             VALUES (?1,?2,(SELECT id FROM transcript_segments WHERE interview_id=?2 LIMIT 1),
             '[]','C',NULL,NULL,NULL,?3,?3)",
            params![Uuid::new_v4().to_string(), iv.id, now_iso()],
        )
        .unwrap();

        let n = import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("c", "00:00:05.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        assert_eq!(n, 1);

        let seg_count = count(
            &conn,
            "SELECT COUNT(*) FROM transcript_segments WHERE interview_id=?1",
            &[&iv.id as &dyn rusqlite::ToSql],
        );
        assert_eq!(seg_count, 1);
        let coded_count = count(
            &conn,
            "SELECT COUNT(*) FROM coded_segments WHERE interview_id=?1",
            &[&iv.id as &dyn rusqlite::ToSql],
        );
        assert_eq!(coded_count, 0);
    }

    #[test]
    fn import_segments_rolls_back_on_fk_violation() {
        let (_dir, _path, conn) = make_project();
        let res = import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: "nonexistent".into(),
                segments: vec![make_segment("x", "00:00:00.000")],
                raw_vtt_path: None,
            },
        );
        assert!(res.is_err(), "FK violation should reject the import");
        let total = count(&conn, "SELECT COUNT(*) FROM transcript_segments", &[]);
        assert_eq!(total, 0, "no segments should remain after rollback");
    }

    #[test]
    fn clear_workspace_failure_does_not_leave_open_transaction() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "A");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("a", "00:00:00.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();

        conn.execute("DROP TABLE coder_activity_log", []).unwrap();
        let res = clear_workspace(
            &conn,
            &ClearWorkspaceInput {
                interview_id: Some(iv.id.clone()),
                clear_hub_memos: false,
                clear_activity_log: false,
                coder_name: "Alice".into(),
            },
        );
        assert!(res.is_err(), "first call should fail mid-transaction");

        conn.execute(
            "CREATE TABLE coder_activity_log (id TEXT PRIMARY KEY, coder_name TEXT NOT NULL,
             action TEXT NOT NULL, detail TEXT, created_at TEXT NOT NULL)",
            [],
        )
        .unwrap();
        clear_workspace(
            &conn,
            &ClearWorkspaceInput {
                interview_id: Some(iv.id.clone()),
                clear_hub_memos: false,
                clear_activity_log: false,
                coder_name: "Alice".into(),
            },
        )
        .expect("second call must succeed — connection must not be stuck mid-transaction");
    }

    #[test]
    fn salvage_keys_coding_per_code_like_normal_edits() {
        let (_dir, _path, conn) = make_project();

        // Activate v2 exactly as sync_v2_activate does.
        set_sync_state_value(&conn, "project_id", "project").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-0000000000aa",
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(
            &conn,
            "sync_generation",
            "00000000-0000-4000-8000-0000000000bb",
        )
        .unwrap();

        let now = now_iso();
        conn.execute(
            "INSERT INTO codebook (id, name, definition, color, sort_order, is_retired, created_at, updated_at)
             VALUES ('cA', 'Alpha', NULL, '#111111', 0, 0, ?1, ?1),
                    ('cB', 'Beta',  NULL, '#222222', 1, 0, ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO interviews (id, participant_label, created_at, updated_at)
             VALUES ('ivS', 'P01', ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcript_segments (id, interview_id, segment_index, speaker, timestamp_start, text)
             VALUES ('segS', 'ivS', 0, 'Participant', '00:00:00.000', 'A shared span carrying two codes.')",
            [],
        )
        .unwrap();
        // One legacy dirty row holding TWO distinct codes on the SAME span —
        // the exact shape that historically collapsed into one mis-keyed
        // span-only entity.
        conn.execute(
            "INSERT INTO coded_segments (id, interview_id, segment_id, code_ids, coder_name,
                                         char_start, char_end, created_at, updated_at)
             VALUES ('csS', 'ivS', 'segS', '[\"cA\",\"cB\"]', 'Ada Lovelace',
                     2, 20, ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "UPDATE coded_segments SET sync_dirty = 1 WHERE id = 'csS'",
            [],
        )
        .unwrap();
        // Fresh-project rows default dirty; scope the salvage to the one
        // coding row so this test asserts on coding keying alone.
        conn.execute("UPDATE codebook SET sync_dirty = 0", [])
            .unwrap();
        conn.execute("UPDATE interviews SET sync_dirty = 0", [])
            .unwrap();

        let salvaged = salvage_legacy_v1_state_for_v2(&conn).unwrap();

        let mut entities: Vec<(String, String)> = conn
            .prepare("SELECT entity_id, payload_json FROM sync_outbox WHERE entity_type = 'coding'")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(salvaged, 2, "two codes on one span must emit two ops");
        assert_eq!(entities.len(), 2);
        entities.sort();
        assert_eq!(
            entities.iter().map(|(e, _)| e.as_str()).collect::<Vec<_>>(),
            vec!["ivS:segS:span:2:20:cA", "ivS:segS:span:2:20:cB"],
            "salvaged coding must be keyed per code, matching normal edits"
        );
        for (entity_id, payload) in &entities {
            let json: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert!(
                entity_id.ends_with(":cA") || entity_id.ends_with(":cB"),
                "unexpected entity {entity_id}"
            );
            let adds = json["adds"].as_array().unwrap();
            assert_eq!(adds.len(), 1, "{entity_id} must carry exactly one add");
            assert_eq!(adds[0]["code_id"], entity_id.rsplit(':').next().unwrap());
        }
    }

    #[test]
    fn build_v2_coding_edge_omits_null_spans_and_rejects_mixed() {
        let whole = build_v2_coding_edge("iv1", "seg1", "c1", None, None).unwrap();
        assert_eq!(whole["interview_id"], "iv1");
        assert_eq!(whole["segment_id"], "seg1");
        assert_eq!(whole["code_id"], "c1");
        assert!(whole.get("char_start").is_none());
        assert!(whole.get("char_end").is_none());

        let span = build_v2_coding_edge("iv1", "seg1", "c1", Some(12), Some(34)).unwrap();
        assert_eq!(span["interview_id"], "iv1");
        assert_eq!(span["segment_id"], "seg1");
        assert_eq!(span["code_id"], "c1");
        assert_eq!(span["char_start"], 12);
        assert_eq!(span["char_end"], 34);

        assert!(build_v2_coding_edge("iv1", "seg1", "c1", Some(12), None).is_err());
        assert!(build_v2_coding_edge("iv1", "seg1", "c1", None, Some(34)).is_err());
    }

    #[test]
    fn v2_live_enqueue_omits_null_spans_and_rejects_mixed() {
        let (_dir, _path, conn) = make_project();
        set_sync_state_value(&conn, "project_id", "project").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-0000000000aa",
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(
            &conn,
            "sync_generation",
            "00000000-0000-4000-8000-0000000000bb",
        )
        .unwrap();

        let iv = make_interview(&conn, "P01");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment(
                    "Hello world transcript content",
                    "00:00:00.000",
                )],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segments = get_segments(&conn, &iv.id).unwrap();
        let seg_id = &segments[0].id;
        let code = create_code(
            &conn,
            &CreateCodeInput {
                name: "Alpha".into(),
                definition: None,
                color: Some("#111111".into()),
                parent_id: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
            },
        )
        .unwrap();

        // 1. Whole-turn live enqueue
        mutate_coding_edge(
            &conn,
            &MutateCodingEdgeInput {
                interview_id: iv.id.clone(),
                segment_id: seg_id.clone(),
                code_id: code.id.clone(),
                coder_name: "Coder Alice".into(),
                char_start: None,
                char_end: None,
                present: true,
            },
        )
        .unwrap();

        let outbox_rows: Vec<(String, String)> = conn
            .prepare("SELECT entity_id, payload_json FROM sync_outbox WHERE entity_type = 'coding'")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(outbox_rows.len(), 1);
        let whole_payload: serde_json::Value = serde_json::from_str(&outbox_rows[0].1).unwrap();
        let whole_edge = &whole_payload["adds"][0];
        assert_eq!(whole_edge["code_id"], code.id);
        assert!(whole_edge.get("char_start").is_none());
        assert!(whole_edge.get("char_end").is_none());

        // 2. Span live enqueue
        mutate_coding_edge(
            &conn,
            &MutateCodingEdgeInput {
                interview_id: iv.id.clone(),
                segment_id: seg_id.clone(),
                code_id: code.id.clone(),
                coder_name: "Coder Alice".into(),
                char_start: Some(0),
                char_end: Some(5),
                present: true,
            },
        )
        .unwrap();

        let outbox_rows: Vec<(String, String)> = conn
            .prepare("SELECT entity_id, payload_json FROM sync_outbox WHERE entity_type = 'coding' ORDER BY client_seq ASC")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(outbox_rows.len(), 2);
        let span_payload: serde_json::Value = serde_json::from_str(&outbox_rows[1].1).unwrap();
        let span_edge = &span_payload["adds"][0];
        assert_eq!(span_edge["char_start"], 0);
        assert_eq!(span_edge["char_end"], 5);

        // 3. Mixed span live enqueue returns error and does not enqueue
        let mixed_res = mutate_coding_edge(
            &conn,
            &MutateCodingEdgeInput {
                interview_id: iv.id.clone(),
                segment_id: seg_id.clone(),
                code_id: code.id.clone(),
                coder_name: "Coder Alice".into(),
                char_start: Some(0),
                char_end: None,
                present: true,
            },
        );
        assert!(mixed_res.is_err());

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE entity_type = 'coding'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn v2_salvage_omits_null_spans_and_rejects_mixed() {
        let (_dir, _path, conn) = make_project();
        set_sync_state_value(&conn, "project_id", "project").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-0000000000aa",
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(
            &conn,
            "sync_generation",
            "00000000-0000-4000-8000-0000000000bb",
        )
        .unwrap();

        let now = now_iso();
        conn.execute(
            "INSERT INTO codebook (id, name, definition, color, sort_order, is_retired, created_at, updated_at)
             VALUES ('cA', 'Alpha', NULL, '#111111', 0, 0, ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO interviews (id, participant_label, created_at, updated_at)
             VALUES ('iv1', 'P01', ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcript_segments (id, interview_id, segment_index, speaker, timestamp_start, timestamp_end, text)
             VALUES ('seg1', 'iv1', 0, 'Alice', '00:00:00.000', '00:00:05.000', 'Hello world')",
            [],
        )
        .unwrap();

        // Insert whole-turn and span legacy dirty rows
        conn.execute(
            "INSERT INTO coded_segments (id, interview_id, segment_id, code_ids, coder_name, char_start, char_end, created_at, updated_at, sync_dirty)
             VALUES ('cs_whole', 'iv1', 'seg1', '[\"cA\"]', 'Coder', NULL, NULL, ?1, ?1, 1),
                    ('cs_span', 'iv1', 'seg1', '[\"cA\"]', 'Coder', 10, 25, ?1, ?1, 1)",
            [&now],
        )
        .unwrap();
        conn.execute("UPDATE codebook SET sync_dirty = 0", [])
            .unwrap();
        conn.execute("UPDATE interviews SET sync_dirty = 0", [])
            .unwrap();

        let salvaged = salvage_legacy_v1_state_for_v2(&conn).unwrap();
        assert_eq!(salvaged, 2);

        let outbox_rows: Vec<(String, String)> = conn
            .prepare("SELECT entity_id, payload_json FROM sync_outbox WHERE entity_type = 'coding' ORDER BY entity_id ASC")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(outbox_rows.len(), 2);
        let whole_payload: serde_json::Value = serde_json::from_str(&outbox_rows[1].1).unwrap();
        assert_eq!(outbox_rows[1].0, "iv1:seg1:whole:cA");
        let whole_edge = &whole_payload["adds"][0];
        assert!(whole_edge.get("char_start").is_none());
        assert!(whole_edge.get("char_end").is_none());

        let span_payload: serde_json::Value = serde_json::from_str(&outbox_rows[0].1).unwrap();
        assert_eq!(outbox_rows[0].0, "iv1:seg1:span:10:25:cA");
        let span_edge = &span_payload["adds"][0];
        assert_eq!(span_edge["char_start"], 10);
        assert_eq!(span_edge["char_end"], 25);

        // Mixed legacy row fails salvage
        conn.execute(
            "INSERT INTO coded_segments (id, interview_id, segment_id, code_ids, coder_name, char_start, char_end, created_at, updated_at, sync_dirty)
             VALUES ('cs_mixed', 'iv1', 'seg1', '[\"cA\"]', 'Coder', 10, NULL, ?1, ?1, 1)",
            [&now],
        )
        .unwrap();
        let mixed_err = salvage_legacy_v1_state_for_v2(&conn);
        assert!(mixed_err.is_err());
    }

    #[test]
    fn block_id_is_monotonic_across_workspace_reset() {
        let (_dir, _path, conn) = make_project();
        let iv = make_interview(&conn, "A");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    make_segment("a", "00:00:00.000"),
                    make_segment("b", "00:00:01.000"),
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(
            apply_one(&conn, &iv.id, &segs[0].id).block_id.as_deref(),
            Some("q001")
        );
        assert_eq!(
            apply_one(&conn, &iv.id, &segs[1].id).block_id.as_deref(),
            Some("q002")
        );

        clear_workspace(
            &conn,
            &ClearWorkspaceInput {
                interview_id: None,
                clear_hub_memos: false,
                clear_activity_log: false,
                coder_name: "Alice".into(),
            },
        )
        .unwrap();

        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    make_segment("c", "00:00:05.000"),
                    make_segment("d", "00:00:06.000"),
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segs2 = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(
            apply_one(&conn, &iv.id, &segs2[0].id).block_id.as_deref(),
            Some("q003"),
            "block_id must keep climbing after a workspace reset"
        );
    }

    #[test]
    fn csv_export_starts_with_utf8_bom() {
        let (_dir, project_path, conn) = make_project();
        let iv = make_interview(&conn, "A");
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("a", "00:00:00.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segs = get_segments(&conn, &iv.id).unwrap();
        apply_one(&conn, &iv.id, &segs[0].id);

        let config = ExportConfigInput {
            preset: "reflexive-ta".to_string(),
            items: vec!["report-html".to_string(), "coded-segments".to_string()],
            include_participant_scope: "all".to_string(),
            selected_participant_ids: None,
            include_coder_scope: "all".to_string(),
        };

        let target_dir = project_path.join("exports_out");
        let res1 = export_with_config(
            &conn,
            &target_dir,
            &config,
            Some("<html>Report</html>"),
            None,
            "Alice",
            &std::collections::HashSet::new(),
        )
        .unwrap();

        let out_dir1 = std::path::PathBuf::from(&res1.exports_dir);
        let csv = std::fs::read(out_dir1.join("coded-segments.csv")).unwrap();
        assert!(csv.len() >= 3, "CSV should have content");
        assert_eq!(
            &csv[..3],
            &[0xEF, 0xBB, 0xBF],
            "CSV must start with UTF-8 BOM"
        );
        assert!(out_dir1.join("report.html").exists());

        // Export a second time: must create a new folder with suffix -2 without overwriting or deleting out_dir1
        let res2 = export_with_config(
            &conn,
            &target_dir,
            &config,
            Some("<html>Report 2</html>"),
            None,
            "Alice",
            &std::collections::HashSet::new(),
        )
        .unwrap();
        let out_dir2 = std::path::PathBuf::from(&res2.exports_dir);
        assert_ne!(out_dir1, out_dir2, "Export directories must be distinct");
        assert!(
            out_dir1.exists(),
            "Previous export folder must not be deleted"
        );
        assert!(out_dir2.exists(), "Second export folder must exist");
    }

    #[test]
    fn csv_export_redacts_speakers_for_toggled_interviews() {
        let (_dir, project_path, conn) = make_project();
        let iv = make_interview(&conn, "A");
        let seg = |speaker: &str, text: &str| crate::models::SegmentInput {
            speaker: speaker.into(),
            timestamp_start: "00:00:00.000".into(),
            timestamp_end: None,
            text: text.into(),
            section_tag: None,
        };
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    seg("Ada Lovelace", "first"),
                    seg("Interviewer", "prompt"),
                    seg("Ada Lovelace", "again"),
                    seg("Bo", "last"),
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        for s in get_segments(&conn, &iv.id).unwrap() {
            apply_one(&conn, &iv.id, &s.id);
        }

        let config = ExportConfigInput {
            preset: "reflexive-ta".to_string(),
            items: vec!["coded-segments".to_string()],
            include_participant_scope: "all".to_string(),
            selected_participant_ids: None,
            include_coder_scope: "all".to_string(),
        };
        let export_once = |redacted: &std::collections::HashSet<String>| {
            let dir = project_path.join(format!("exports_{}", redacted.len()));
            let res =
                export_with_config(&conn, &dir, &config, None, None, "Alice", redacted).unwrap();
            std::fs::read_to_string(
                std::path::PathBuf::from(&res.exports_dir).join("coded-segments.csv"),
            )
            .unwrap()
        };

        // Unredacted: real names in the speaker column.
        let plain = export_once(&std::collections::HashSet::new());
        assert!(plain.contains("Ada Lovelace"), "plain export keeps names");
        assert!(plain.contains("Interviewer"), "Interviewer always literal");

        // Redacted: first-appearance numbering, Interviewer untouched.
        let mut redacted = std::collections::HashSet::new();
        redacted.insert(iv.id.clone());
        let masked = export_once(&redacted);
        assert!(
            !masked.contains("Ada Lovelace"),
            "alias replaces Ada Lovelace"
        );
        assert!(!masked.contains("Bo"), "alias replaces Bo");
        assert!(masked.contains("Speaker 1"), "first speaker is Speaker 1");
        assert!(masked.contains("Speaker 2"), "second speaker is Speaker 2");
        assert!(masked.contains("Interviewer"), "Interviewer stays literal");
    }

    #[test]
    fn export_manifest_records_conflict_count_without_conflict_content() {
        let (_dir, project_path, conn) = make_project();
        let proposal_sentinel = "proposal-content-sentinel";
        conn.execute(
            "INSERT INTO sync_conflicts (
                conflict_id, project_id, generation, entity_type, entity_id, field_name,
                current_json, proposed_json, originating_op_id, status, created_at, proposer_label
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'unresolved', ?10, ?11)",
            params![
                "conflict-export",
                "project",
                "generation",
                "code",
                "code-1",
                "definition",
                "\"canonical definition\"",
                format!("\"{proposal_sentinel}\""),
                "op-export",
                now_iso(),
                "Coder",
            ],
        )
        .unwrap();

        let config = ExportConfigInput {
            preset: "reflexive-ta".to_string(),
            items: vec![],
            include_participant_scope: "all".to_string(),
            selected_participant_ids: None,
            include_coder_scope: "all".to_string(),
        };
        let result = export_with_config(
            &conn,
            &project_path.join("exports_out"),
            &config,
            None,
            None,
            "Alice",
            &std::collections::HashSet::new(),
        )
        .unwrap();

        assert_eq!(result.unresolved_conflict_count, 1);
        let manifest =
            fs::read_to_string(Path::new(&result.exports_dir).join("export-manifest.json"))
                .unwrap();
        let manifest_json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest_json["unresolved_conflict_count"], 1);
        assert_eq!(manifest_json["shared_state_policy"], "canonical_only");
        assert!(!manifest.contains(proposal_sentinel));
    }

    /// One coded segment, so the project has something a lost WAL would lose.
    fn seed_one_coded_segment(conn: &Connection) {
        let iv = make_interview(conn, "A");
        import_segments(
            conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![make_segment("a", "00:00:00.000")],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segs = get_segments(conn, &iv.id).unwrap();
        apply_one(conn, &iv.id, &segs[0].id);
    }

    fn coded_count_of_db_file(db_path: &Path) -> i32 {
        let conn = Connection::open(db_path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM coded_segments", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn checkpoint_leaves_db_file_complete_on_its_own() {
        let (dir, project_path, conn) = make_project();
        seed_one_coded_segment(&conn);

        let wal = project_path.join("project.db-wal");
        assert!(
            std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0) > 0,
            "precondition: the work should be sitting in the WAL, not the db"
        );

        checkpoint(&conn).unwrap();

        assert_eq!(
            std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0),
            0,
            "checkpoint(TRUNCATE) must empty the WAL"
        );

        // The real test: copy project.db the way a partial Drive sync would —
        // alone, without its sidecars — and confirm nothing was left behind.
        let lone = dir.path().join("lone.db");
        std::fs::copy(project_path.join("project.db"), &lone).unwrap();
        assert_eq!(
            coded_count_of_db_file(&lone),
            1,
            "project.db must carry the coding on its own after a checkpoint"
        );
    }

    // ---- project files ---------------------------------------------------

    // ---- codebook editing / deletion -----------------------------------

    fn make_code(conn: &Connection, name: &str) -> Code {
        create_code(
            conn,
            &CreateCodeInput {
                name: name.into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap()
    }

    fn seeded_interview(conn: &Connection, n: usize) -> (Interview, Vec<String>) {
        let iv = make_interview(conn, "A");
        let segs: Vec<SegmentInput> = (0..n)
            .map(|i| make_segment(&format!("line {i}"), &format!("00:00:{i:02}.000")))
            .collect();
        import_segments(
            conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: segs,
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let ids = get_segments(conn, &iv.id)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        (iv, ids)
    }

    fn code_with(conn: &Connection, iv: &str, seg: &str, codes: &[&str], coder: &str) {
        apply_codes(
            conn,
            &ApplyCodesInput {
                interview_id: iv.to_string(),
                segment_id: seg.to_string(),
                code_ids: codes.iter().map(|c| c.to_string()).collect(),
                coder_name: coder.to_string(),
                memo: None,
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn reapplying_codes_revises_the_row_instead_of_duplicating_it() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 2);
        let a = make_code(&conn, "A");
        let b = make_code(&conn, "B");

        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");
        code_with(&conn, &iv.id, &segs[0], &[&a.id, &b.id], "Alice");

        let rows = count(
            &conn,
            "SELECT COUNT(*) FROM coded_segments WHERE segment_id=?1",
            &[&segs[0] as &dyn rusqlite::ToSql],
        );
        assert_eq!(rows, 1, "re-coding a passage must not stack rows");

        let stored = list_coded_segments(&conn, Some(&iv.id), None).unwrap();
        assert_eq!(stored[0].code_ids.len(), 2);
    }

    #[test]
    fn a_second_coder_gets_their_own_row_on_the_same_passage() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 1);
        let a = make_code(&conn, "A");

        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Bob");

        let rows = count(
            &conn,
            "SELECT COUNT(*) FROM coded_segments WHERE segment_id=?1",
            &[&segs[0] as &dyn rusqlite::ToSql],
        );
        assert_eq!(
            rows, 2,
            "divergence between coders is data, not a duplicate"
        );
    }

    #[test]
    fn usage_count_counts_passages_not_rows() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 2);
        let a = make_code(&conn, "A");

        // Same passage, two coders, plus one other passage => 2 passages.
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Bob");
        code_with(&conn, &iv.id, &segs[1], &[&a.id], "Alice");

        let listed = list_codes(&conn).unwrap();
        let found = listed.iter().find(|c| c.id == a.id).unwrap();
        assert_eq!(found.usage_count, 2);
    }

    #[test]
    fn retiring_a_code_hides_it_but_keeps_the_coding() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 1);
        let a = make_code(&conn, "A");
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");

        let result = delete_code(&conn, &a.id, DeleteCodeMode::Retire).unwrap();
        assert_eq!(result.segments_updated, 0);

        assert!(list_codes(&conn).unwrap().is_empty());
        assert_eq!(list_retired_codes(&conn).unwrap().len(), 1);
        assert_eq!(
            list_coded_segments(&conn, Some(&iv.id), None)
                .unwrap()
                .len(),
            1
        );

        // And the name still resolves, so exports keep their label.
        assert_eq!(code_name_map(&conn).unwrap().get(&a.id).unwrap(), "A");

        restore_code(&conn, &a.id).unwrap();
        assert_eq!(list_codes(&conn).unwrap().len(), 1);
        assert!(list_retired_codes(&conn).unwrap().is_empty());
    }

    #[test]
    fn purging_a_code_strips_it_and_drops_rows_it_emptied() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 2);
        let a = make_code(&conn, "A");
        let b = make_code(&conn, "B");

        // seg0 carries both codes; seg1 carries only the doomed one.
        code_with(&conn, &iv.id, &segs[0], &[&a.id, &b.id], "Alice");
        code_with(&conn, &iv.id, &segs[1], &[&a.id], "Alice");

        let result = delete_code(&conn, &a.id, DeleteCodeMode::Purge).unwrap();
        assert_eq!(result.segments_updated, 2);
        assert_eq!(result.coded_segments_removed, 1);

        let remaining = list_coded_segments(&conn, Some(&iv.id), None).unwrap();
        assert_eq!(
            remaining.len(),
            1,
            "the seg1 row had nothing left to record"
        );
        assert_eq!(remaining[0].code_ids, vec![b.id.clone()]);

        assert!(list_codes(&conn).unwrap().iter().all(|c| c.id != a.id));
        assert!(list_retired_codes(&conn).unwrap().is_empty());
    }

    #[test]
    fn updating_a_code_rewrites_it_in_place() {
        let (_dir, _path, conn) = make_project();
        let a = make_code(&conn, "Old name");

        let updated = update_code(
            &conn,
            &UpdateCodeInput {
                id: a.id.clone(),
                name: "New name".into(),
                definition: Some("what counts".into()),
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: "#0d8a5f".into(),
            },
        )
        .unwrap();

        assert_eq!(updated.name, "New name");
        assert_eq!(updated.color, "#0d8a5f");
        assert_eq!(
            list_codes(&conn).unwrap().len(),
            1,
            "update must not insert"
        );
    }

    #[test]
    fn updating_a_code_preserves_criteria_and_example_and_parent() {
        let (_dir, _path, conn) = make_project();
        let parent = make_code(&conn, "Parent");
        let child = make_code(&conn, "Child");

        let updated = update_code(
            &conn,
            &UpdateCodeInput {
                id: child.id.clone(),
                name: "Child renamed".into(),
                definition: Some("detailed definition".into()),
                inclusion_criteria: Some("must match phenotype".into()),
                exclusion_criteria: Some("not general anxiety".into()),
                example: Some("practicing mirror smiles".into()),
                parent_id: Some(parent.id.clone()),
                color: "#8a6410".into(),
            },
        )
        .unwrap();

        assert_eq!(updated.name, "Child renamed");
        assert_eq!(updated.definition.as_deref(), Some("detailed definition"));
        assert_eq!(
            updated.inclusion_criteria.as_deref(),
            Some("must match phenotype")
        );
        assert_eq!(
            updated.exclusion_criteria.as_deref(),
            Some("not general anxiety")
        );
        assert_eq!(updated.example.as_deref(), Some("practicing mirror smiles"));
        assert_eq!(updated.parent_id, Some(parent.id));

        let codes = list_codes(&conn).unwrap();
        let fetched_child = codes.into_iter().find(|c| c.id == child.id).unwrap();
        assert_eq!(
            fetched_child.inclusion_criteria.as_deref(),
            Some("must match phenotype")
        );
        assert_eq!(
            fetched_child.exclusion_criteria.as_deref(),
            Some("not general anxiety")
        );
        assert_eq!(
            fetched_child.example.as_deref(),
            Some("practicing mirror smiles")
        );
    }

    #[test]
    fn deleting_a_coded_segment_leaves_the_transcript_alone() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 2);
        let a = make_code(&conn, "A");
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");

        let row = list_coded_segments(&conn, Some(&iv.id), None)
            .unwrap()
            .remove(0);
        delete_coded_segment(&conn, &row.id).unwrap();

        assert!(list_coded_segments(&conn, Some(&iv.id), None)
            .unwrap()
            .is_empty());
        assert_eq!(get_segments(&conn, &iv.id).unwrap().len(), 2);
        assert_eq!(
            list_codes(&conn).unwrap().len(),
            1,
            "the code itself survives"
        );
    }

    // ---- interviews ------------------------------------------------------

    #[test]
    fn deleting_an_interview_cascades_to_its_segments_and_coding() {
        let (_dir, _path, conn) = make_project();
        let (iv, segs) = seeded_interview(&conn, 2);
        let keep = make_interview(&conn, "Keep");
        let a = make_code(&conn, "A");
        code_with(&conn, &iv.id, &segs[0], &[&a.id], "Alice");

        let impact = interview_delete_impact(&conn, &iv.id).unwrap();
        assert_eq!(impact.segment_count, 2);
        assert_eq!(impact.coded_segment_count, 1);

        delete_interview(&conn, &iv.id).unwrap();

        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM transcript_segments", &[]),
            0
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM coded_segments", &[]), 0);
        let left = list_interviews(&conn).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, keep.id);
    }

    #[test]
    fn renaming_an_interview_keeps_its_transcript() {
        let (_dir, _path, conn) = make_project();
        let (iv, _segs) = seeded_interview(&conn, 3);

        let updated = update_interview(
            &conn,
            &UpdateInterviewInput {
                id: iv.id.clone(),
                participant_label: "betty-2026-04-28".into(),
                interview_date: Some("2026-04-28".into()),
                modality: None,
                diagnosis_notes: None,
            },
        )
        .unwrap();

        assert_eq!(updated.participant_label, "betty-2026-04-28");
        assert_eq!(updated.segment_count, 3);
        assert_eq!(list_interviews(&conn).unwrap().len(), 1);
    }

    #[test]
    fn open_snapshot_stress_budget() {
        let (dir, path, conn) = make_project();

        let iv = create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P04-stress".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec!["Ada".into()],
            },
        )
        .unwrap();

        let segments: Vec<SegmentInput> = (0..320)
            .map(|i| SegmentInput {
                speaker: if i % 2 == 0 {
                    "Interviewer".into()
                } else {
                    "P04".into()
                },
                timestamp_start: format!("00:{:02}:{:02}.000", i / 60, i % 60),
                timestamp_end: None,
                text: format!(
                    "Segment {} with long qualitative response data for reflexive thematic analysis stress testing.",
                    i
                ),
                section_tag: None,
            })
            .collect();

        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments,
                raw_vtt_path: None,
            },
        )
        .unwrap();

        let mut code_ids = Vec::new();
        for i in 1..=60 {
            let c = create_code(
                &conn,
                &CreateCodeInput {
                    name: format!("Code {:02} Long Concept Title", i),
                    definition: Some("Detailed code definition text".into()),
                    inclusion_criteria: None,
                    exclusion_criteria: None,
                    example: None,
                    parent_id: None,
                    color: Some("#8a6410".into()),
                },
            )
            .unwrap();
            code_ids.push(c.id);
        }

        let segs = get_segments(&conn, &iv.id).unwrap();
        for i in 0..160 {
            let seg = &segs[i * 2 % segs.len()];
            let code_id = &code_ids[i % code_ids.len()];
            apply_codes(
                &conn,
                &ApplyCodesInput {
                    interview_id: iv.id.clone(),
                    segment_id: seg.id.clone(),
                    code_ids: vec![code_id.clone()],
                    coder_name: if i % 2 == 0 {
                        "Ada".into()
                    } else {
                        "Luci".into()
                    },
                    memo: if i % 4 == 0 {
                        Some(format!("Analytic note for item {}", i))
                    } else {
                        None
                    },
                    char_start: None,
                    char_end: None,
                },
            )
            .unwrap();
        }

        let mut durations = Vec::new();
        for iter in 0..5 {
            let start = std::time::Instant::now();
            let (_new_conn, snapshot) =
                open_project_snapshot_inner(path.to_str().unwrap()).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(snapshot.segments.len(), 320);
            assert_eq!(snapshot.codes.len(), 60);
            assert!(snapshot.coded_segments.len() >= 80);
            if iter > 0 {
                durations.push(elapsed);
            }
        }

        durations.sort();
        let median = durations[durations.len() / 2];
        let max = *durations.last().unwrap();
        println!(
            "open_snapshot_stress_budget: median={:?}, max={:?}",
            median, max
        );
        assert!(
            median <= std::time::Duration::from_millis(750),
            "median {:?} exceeded 750ms",
            median
        );
        assert!(
            max <= std::time::Duration::from_millis(1500),
            "max {:?} exceeded 1.5s",
            max
        );
        let _ = dir;
    }

    #[test]
    fn update_interview_leaves_id_untouched() {
        let (_dir, _path, conn) = make_project();
        let created = create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P07".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        let original_id = created.id.clone();
        let updated = update_interview(
            &conn,
            &UpdateInterviewInput {
                id: original_id.clone(),
                participant_label: "P08".into(),
                interview_date: Some("2026-08-21".into()),
                modality: None,
                diagnosis_notes: None,
            },
        )
        .unwrap();
        assert_eq!(updated.id, original_id);
    }

    #[test]
    fn code_hierarchy_two_level_constraint_and_promotion() {
        let (_dir, _path, conn) = make_project();
        let parent = create_code(
            &conn,
            &CreateCodeInput {
                name: "Parent Code".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: Some("#8a6410".into()),
            },
        )
        .unwrap();

        let child = create_code(
            &conn,
            &CreateCodeInput {
                name: "Child Code".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(parent.id.clone()),
                color: Some("#8a6410".into()),
            },
        )
        .unwrap();

        // Attempting to create a grandchild (3rd level) fails
        let grandchild_res = create_code(
            &conn,
            &CreateCodeInput {
                name: "Grandchild Code".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(child.id.clone()),
                color: Some("#8a6410".into()),
            },
        );
        assert!(grandchild_res.is_err());

        // Self-parenting fails
        let self_parent_res = update_code(
            &conn,
            &UpdateCodeInput {
                id: parent.id.clone(),
                name: parent.name.clone(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(parent.id.clone()),
                color: "#8a6410".into(),
            },
        );
        assert!(self_parent_res.is_err());

        // Moving a parent that has children under another code fails
        let other_top = create_code(
            &conn,
            &CreateCodeInput {
                name: "Other Top Code".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: Some("#8a6410".into()),
            },
        )
        .unwrap();

        let reparent_parent_res = update_code(
            &conn,
            &UpdateCodeInput {
                id: parent.id.clone(),
                name: parent.name.clone(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(other_top.id.clone()),
                color: "#8a6410".into(),
            },
        );
        assert!(reparent_parent_res.is_err());

        // Deleting the parent promotes the child (parent_id becomes None)
        delete_code(&conn, &parent.id, DeleteCodeMode::Purge).unwrap();
        let codes = list_codes(&conn).unwrap();
        let updated_child = codes.iter().find(|c| c.id == child.id).unwrap();
        assert_eq!(updated_child.parent_id, None);
    }

    #[test]
    fn rollup_code_filtering_matches_parent_and_child_codes() {
        let (_dir, _path, conn) = make_project();
        let parent = create_code(
            &conn,
            &CreateCodeInput {
                name: "Parent".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: Some("#8a6410".into()),
            },
        )
        .unwrap();

        let child = create_code(
            &conn,
            &CreateCodeInput {
                name: "Child".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(parent.id.clone()),
                color: Some("#8a6410".into()),
            },
        )
        .unwrap();

        let iv = create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P01".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();

        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    SegmentInput {
                        speaker: "P01".into(),
                        timestamp_start: "00:00:01.000".into(),
                        timestamp_end: None,
                        text: "Turn 1".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "P01".into(),
                        timestamp_start: "00:00:05.000".into(),
                        timestamp_end: None,
                        text: "Turn 2".into(),
                        section_tag: None,
                    },
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();

        let segs = get_segments(&conn, &iv.id).unwrap();
        // Code turn 1 with parent, turn 2 with child
        apply_codes(
            &conn,
            &ApplyCodesInput {
                interview_id: iv.id.clone(),
                segment_id: segs[0].id.clone(),
                code_ids: vec![parent.id.clone()],
                coder_name: "Ada".into(),
                memo: None,
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();

        apply_codes(
            &conn,
            &ApplyCodesInput {
                interview_id: iv.id.clone(),
                segment_id: segs[1].id.clone(),
                code_ids: vec![child.id.clone()],
                coder_name: "Ada".into(),
                memo: None,
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();

        // Filtering by child returns only turn 2
        let child_results = list_coded_segments(&conn, None, Some(&child.id)).unwrap();
        assert_eq!(child_results.len(), 1);
        assert_eq!(child_results[0].segment_id, segs[1].id);

        // Filtering by parent returns BOTH turn 1 and turn 2 (rollup)
        let parent_results = list_coded_segments(&conn, None, Some(&parent.id)).unwrap();
        assert_eq!(parent_results.len(), 2);
    }

    #[test]
    fn protocol_2_code_purge_promotes_children_and_queues_every_shared_mutation() {
        let (_dir, _path, conn) = make_project();
        let parent = make_code(&conn, "Parent");
        let child = create_code(
            &conn,
            &CreateCodeInput {
                name: "Child".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: Some(parent.id.clone()),
                color: None,
            },
        )
        .unwrap();
        set_sync_state_value(&conn, "sync_protocol", "2").unwrap();
        set_sync_state_value(&conn, "project_id", "project-synthetic").unwrap();
        set_sync_state_value(&conn, "sync_generation", "generation-synthetic").unwrap();
        set_sync_state_value(
            &conn,
            "sync_device_id",
            "00000000-0000-4000-8000-000000000001",
        )
        .unwrap();

        delete_code(&conn, &parent.id, DeleteCodeMode::Purge).unwrap();

        let operations: Vec<(String, String, String)> = conn
            .prepare("SELECT entity_id, op_kind, payload_json FROM sync_outbox ORDER BY client_seq")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(operations.iter().any(|(entity_id, kind, payload)| {
            entity_id == &child.id && kind == "code.patch" && payload.contains("\"parent_id\":null")
        }));
        assert!(operations
            .iter()
            .any(|(entity_id, kind, _)| entity_id == &parent.id && kind == "code.purge"));
    }

    #[test]
    fn project_deletion_summary_counts_accurately() {
        let (_dir, path, conn) = make_project();
        let iv = create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P01".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();

        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![SegmentInput {
                    speaker: "P01".into(),
                    timestamp_start: "00:00:01.000".into(),
                    timestamp_end: None,
                    text: "Turn 1".into(),
                    section_tag: None,
                }],
                raw_vtt_path: None,
            },
        )
        .unwrap();

        let segs = get_segments(&conn, &iv.id).unwrap();
        apply_codes(
            &conn,
            &ApplyCodesInput {
                interview_id: iv.id.clone(),
                segment_id: segs[0].id.clone(),
                code_ids: vec!["code1".into()],
                coder_name: "Ada".into(),
                memo: Some("Important note".into()),
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();

        update_hub_memo(&conn, &iv.id, "Interview level memo").unwrap();

        let summary = get_project_deletion_summary(&path.to_string_lossy()).unwrap();
        assert_eq!(summary.interview_count, 1);
        assert_eq!(summary.coded_segment_count, 1);
        assert_eq!(summary.memo_count, 2); // 1 coded memo + 1 hub memo
    }

    #[test]
    fn delete_project_folder_nonexistent_is_idempotent_ok() {
        let res = delete_project_folder_impl("/nonexistent/fleuron/study/folder");
        assert!(res.is_ok());
    }

    #[test]
    fn three_way_merge_truth_table_all_8_combinations() {
        // Elements: "A"
        // 8 combinations of (in base, in local, in remote)
        // 1. (0, 0, 0) -> not in merged
        assert!(!three_way_merge_codes(Some(&[]), &[], &[]).contains(&"A".to_string()));
        // 2. (0, 0, 1) -> remote added -> in merged
        assert!(three_way_merge_codes(Some(&[]), &[], &["A".into()]).contains(&"A".to_string()));
        // 3. (0, 1, 0) -> local added -> in merged
        assert!(three_way_merge_codes(Some(&[]), &["A".into()], &[]).contains(&"A".to_string()));
        // 4. (0, 1, 1) -> both added -> in merged
        assert!(
            three_way_merge_codes(Some(&[]), &["A".into()], &["A".into()])
                .contains(&"A".to_string())
        );
        // 5. (1, 0, 0) -> both removed -> not in merged
        assert!(!three_way_merge_codes(Some(&["A".into()]), &[], &[]).contains(&"A".to_string()));
        // 6. (1, 0, 1) -> local removed, remote kept -> not in merged (local removal honoured)
        assert!(
            !three_way_merge_codes(Some(&["A".into()]), &[], &["A".into()])
                .contains(&"A".to_string())
        );
        // 7. (1, 1, 0) -> local kept, remote removed -> not in merged (remote removal honoured)
        assert!(
            !three_way_merge_codes(Some(&["A".into()]), &["A".into()], &[])
                .contains(&"A".to_string())
        );
        // 8. (1, 1, 1) -> all kept -> in merged
        assert!(
            three_way_merge_codes(Some(&["A".into()]), &["A".into()], &["A".into()])
                .contains(&"A".to_string())
        );
    }

    #[test]
    fn three_way_merge_keeps_both_additions() {
        let base = vec!["codeA".to_string()];
        let local = vec!["codeA".to_string(), "codeB".to_string()];
        let remote = vec!["codeA".to_string(), "codeC".to_string()];
        let merged = three_way_merge_codes(Some(&base), &local, &remote);
        assert_eq!(
            merged,
            vec![
                "codeA".to_string(),
                "codeB".to_string(),
                "codeC".to_string()
            ]
        );
    }

    #[test]
    fn three_way_merge_honours_local_removal() {
        let base = vec!["codeA".to_string(), "codeB".to_string()];
        let local = vec!["codeB".to_string()]; // removed A
        let remote = vec![
            "codeA".to_string(),
            "codeB".to_string(),
            "codeC".to_string(),
        ]; // added C
        let merged = three_way_merge_codes(Some(&base), &local, &remote);
        assert_eq!(merged, vec!["codeB".to_string(), "codeC".to_string()]);
    }

    #[test]
    fn three_way_merge_without_base_falls_back_to_union() {
        let local = vec!["codeA".to_string(), "codeB".to_string()];
        let remote = vec!["codeB".to_string(), "codeC".to_string()];
        let merged = three_way_merge_codes(None, &local, &remote);
        assert_eq!(
            merged,
            vec![
                "codeA".to_string(),
                "codeB".to_string(),
                "codeC".to_string()
            ]
        );
    }

    #[test]
    fn emptied_merge_becomes_tombstone_and_clears_memo() {
        let (_dir, _path, conn) = make_project();
        let (interview_id, seg_id) = seed_segment(&conn);

        // 1. Initial insert from server with codeA
        let remote1 = RemoteCoded {
            id: "seg_code_1",
            interview_id: &interview_id,
            segment_id: &seg_id,
            code_ids_json: r#"["codeA"]"#,
            coder_name: "Coder1",
            revision: 1,
            deleted: false,
            updated_at: "2026-08-22T00:00:00Z",
            char_start: None,
            char_end: None,
        };
        assert_eq!(
            merge_remote_coded(&conn, &remote1).unwrap(),
            CodedMerge::Applied
        );

        // Add a local memo and remove codeA locally (dirty)
        conn.execute(
            "UPDATE coded_segments SET code_ids = '[]', memo = 'Local notes', sync_dirty = 1 WHERE id = 'seg_code_1'",
            [],
        ).unwrap();

        // 2. Incoming remote update at revision 2 that also removed codeA or modified something
        let remote2 = RemoteCoded {
            id: "seg_code_1",
            interview_id: &interview_id,
            segment_id: &seg_id,
            code_ids_json: r#"["codeA"]"#,
            coder_name: "Coder1",
            revision: 2,
            deleted: false,
            updated_at: "2026-08-22T00:01:00Z",
            char_start: None,
            char_end: None,
        };
        let merge_res = merge_remote_coded(&conn, &remote2).unwrap();
        assert_eq!(merge_res, CodedMerge::Deferred);

        let (deleted, memo, code_ids): (i64, Option<String>, String) = conn
            .query_row(
                "SELECT deleted, memo, code_ids FROM coded_segments WHERE id = 'seg_code_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();

        assert_eq!(deleted, 1);
        assert_eq!(memo, None);
        assert_eq!(code_ids, "[]");
    }

    #[test]
    fn delete_project_folder_guards_and_fallback() {
        // (d) already-absent path returns Ok
        let temp_parent = tempfile::tempdir().unwrap();
        let non_existent = temp_parent.path().join("does_not_exist");
        assert!(
            delete_project_folder_impl(&non_existent.to_string_lossy()).is_ok(),
            "already absent path must return Ok(())"
        );

        // (b) temp dir without marker is refused
        let bare_dir = tempfile::tempdir().unwrap();
        let bare_path = bare_dir.path().to_path_buf();
        let res = delete_project_folder_impl(&bare_path.to_string_lossy());
        assert!(
            res.is_err(),
            "folder without project.db or project.json must be refused"
        );
        assert!(bare_path.exists(), "refused folder must not be deleted");

        // (a) temp dir with project.db deletes to success
        let proj_dir = tempfile::tempdir().unwrap();
        let proj_path = proj_dir.path().to_path_buf();
        std::fs::File::create(proj_path.join("project.db")).unwrap();
        let res = delete_project_folder_impl(&proj_path.to_string_lossy());
        assert!(
            res.is_ok(),
            "delete project folder with project.db failed: {:?}",
            res.err()
        );
        assert!(!proj_path.exists(), "project folder should be gone");

        // (c) Box path refusal guard
        assert!(is_box_path_str(
            "/Users/test/Library/CloudStorage/Box-Box/MyStudy"
        ));
        assert!(is_box_path_str("/Users/test/Box Sync/MyStudy"));
        assert!(!is_box_path_str("/Users/test/Documents/Fleuron/MyStudy"));
    }

    #[test]
    fn inspect_join_target_covers_all_verdicts() {
        use crate::models::JoinTargetVerdict;

        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path();
        let parent_str = parent.to_string_lossy();

        // 1. Available
        let v1 = inspect_join_target(&parent_str, "alpha", "p1").unwrap();
        match v1 {
            JoinTargetVerdict::Available {
                suggested_name,
                suggested_path,
            } => {
                assert_eq!(suggested_name, "alpha.fleuron");
                assert_eq!(
                    suggested_path,
                    parent.join("alpha.fleuron").to_string_lossy()
                );
            }
            other => panic!("expected Available, got {:?}", other),
        }

        // 2. Occupied: folder exists without project.db
        let occ_folder = parent.join("alpha.fleuron");
        std::fs::create_dir_all(&occ_folder).unwrap();
        let v2 = inspect_join_target(&parent_str, "alpha", "p1").unwrap();
        match v2 {
            JoinTargetVerdict::Occupied {
                path,
                suggested_name,
                suggested_path,
            } => {
                assert_eq!(path, occ_folder.to_string_lossy());
                assert_eq!(suggested_name, "alpha-2.fleuron");
                assert_eq!(
                    suggested_path,
                    parent.join("alpha-2.fleuron").to_string_lossy()
                );
            }
            other => panic!("expected Occupied, got {:?}", other),
        }

        // 3. AdoptableUnbound: project.db exists, sync_state has group_bound = 0
        let unbound_folder = parent.join("beta.fleuron");
        std::fs::create_dir_all(&unbound_folder).unwrap();
        let db_path = unbound_folder.join("project.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE sync_state (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sync_state (key, value) VALUES ('group_bound', '0')",
                [],
            )
            .unwrap();
        }
        let v3 = inspect_join_target(&parent_str, "beta", "p1").unwrap();
        match v3 {
            JoinTargetVerdict::AdoptableUnbound { path } => {
                assert_eq!(path, unbound_folder.to_string_lossy());
            }
            other => panic!("expected AdoptableUnbound, got {:?}", other),
        }

        // 4. AlreadySetUpHere: bound = 1 and project_id matches
        let bound_folder = parent.join("gamma.fleuron");
        std::fs::create_dir_all(&bound_folder).unwrap();
        let db_path = bound_folder.join("project.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE sync_state (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sync_state (key, value) VALUES ('group_bound', '1'), ('project_id', 'group-123')",
                [],
            )
            .unwrap();
        }
        let v4 = inspect_join_target(&parent_str, "gamma", "group-123").unwrap();
        match v4 {
            JoinTargetVerdict::AlreadySetUpHere { path } => {
                assert_eq!(path, bound_folder.to_string_lossy());
            }
            other => panic!("expected AlreadySetUpHere, got {:?}", other),
        }

        // 5. BoundElsewhere: bound = 1 and project_id differs -> suggests gamma-2.fleuron
        let v5 = inspect_join_target(&parent_str, "gamma", "different-group").unwrap();
        match v5 {
            JoinTargetVerdict::BoundElsewhere {
                path,
                suggested_name,
                suggested_path,
            } => {
                assert_eq!(path, bound_folder.to_string_lossy());
                assert_eq!(suggested_name, "gamma-2.fleuron");
                assert_eq!(
                    suggested_path,
                    parent.join("gamma-2.fleuron").to_string_lossy()
                );
            }
            other => panic!("expected BoundElsewhere, got {:?}", other),
        }
    }

    #[test]
    fn speaker_reassign_single_turn_and_scope_and_atomic_restore() {
        let (_dir, _path, conn) = make_project();
        create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P-Speaker".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        let iv = &list_interviews(&conn).unwrap()[0];

        // Sequence: A, A, Interviewer, A
        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    SegmentInput {
                        speaker: "A".into(),
                        timestamp_start: "00:00:01".into(),
                        timestamp_end: None,
                        text: "Turn 0".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "A".into(),
                        timestamp_start: "00:00:05".into(),
                        timestamp_end: None,
                        text: "Turn 1".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "Interviewer".into(),
                        timestamp_start: "00:00:10".into(),
                        timestamp_end: None,
                        text: "Turn 2".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "A".into(),
                        timestamp_start: "00:00:15".into(),
                        timestamp_end: None,
                        text: "Turn 3".into(),
                        section_tag: None,
                    },
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();

        let segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(segs.len(), 4);

        // 1. Trimming and empty rejection
        assert!(set_segment_speaker(&conn, &segs[0].id, "   ", false).is_err());

        // 2. Single turn mode on turn 1
        let single_changes = set_segment_speaker(&conn, &segs[1].id, "  B  ", false).unwrap();
        assert_eq!(single_changes.len(), 1);
        assert_eq!(single_changes[0].segment_id, segs[1].id);
        assert_eq!(single_changes[0].old_speaker, "A");
        assert_eq!(single_changes[0].new_speaker, "B");

        let cur_segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(cur_segs[0].speaker, "A");
        assert_eq!(cur_segs[1].speaker, "B");
        assert_eq!(cur_segs[2].speaker, "Interviewer");
        assert_eq!(cur_segs[3].speaker, "A");

        // Restore single turn
        restore_segment_speakers(&conn, &single_changes).unwrap();
        let cur_segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(cur_segs[1].speaker, "A");

        // 3. "And following" on turn 0: should affect turn 0, 1, 3 (all "A"), but leave Interviewer untouched
        let multi_changes = set_segment_speaker(&conn, &segs[0].id, "Participant 1", true).unwrap();
        assert_eq!(multi_changes.len(), 3);
        assert_eq!(multi_changes[0].segment_id, segs[0].id);
        assert_eq!(multi_changes[1].segment_id, segs[1].id);
        assert_eq!(multi_changes[2].segment_id, segs[3].id);

        let cur_segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(cur_segs[0].speaker, "Participant 1");
        assert_eq!(cur_segs[1].speaker, "Participant 1");
        assert_eq!(cur_segs[2].speaker, "Interviewer");
        assert_eq!(cur_segs[3].speaker, "Participant 1");

        // 4. Stale-expected rollback test
        // Modify segs[1] to something else behind the scenes
        conn.execute(
            "UPDATE transcript_segments SET speaker = 'Tampered' WHERE id = ?1",
            params![segs[1].id],
        )
        .unwrap();

        let restore_res = restore_segment_speakers(&conn, &multi_changes);
        assert!(restore_res.is_err(), "Expected rollback on stale speaker");

        // Check that segs[0] and segs[3] were NOT restored because the transaction rolled back
        let after_rollback = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(after_rollback[0].speaker, "Participant 1");
        assert_eq!(after_rollback[1].speaker, "Tampered");
        assert_eq!(after_rollback[3].speaker, "Participant 1");

        // Fix the tampered segment back to "Participant 1", then restore succeeds
        conn.execute(
            "UPDATE transcript_segments SET speaker = 'Participant 1' WHERE id = ?1",
            params![segs[1].id],
        )
        .unwrap();
        restore_segment_speakers(&conn, &multi_changes).unwrap();
        let restored_segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(restored_segs[0].speaker, "A");
        assert_eq!(restored_segs[1].speaker, "A");
        assert_eq!(restored_segs[2].speaker, "Interviewer");
        assert_eq!(restored_segs[3].speaker, "A");

        // 5. No sync side effect: outbox is empty
        let outbox_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outbox_count, 0);
    }

    #[test]
    fn segment_reviews_set_clear_cascade_list() {
        let (_dir, _path, conn) = make_project();
        create_interview(
            &conn,
            &CreateInterviewInput {
                participant_label: "P-Review".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        let iv = &list_interviews(&conn).unwrap()[0];

        import_segments(
            &conn,
            &ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    SegmentInput {
                        speaker: "S1".into(),
                        timestamp_start: "00:00:01".into(),
                        timestamp_end: None,
                        text: "Turn 0".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "S2".into(),
                        timestamp_start: "00:00:05".into(),
                        timestamp_end: None,
                        text: "Turn 1".into(),
                        section_tag: None,
                    },
                    SegmentInput {
                        speaker: "S1".into(),
                        timestamp_start: "00:00:10".into(),
                        timestamp_end: None,
                        text: "Turn 2".into(),
                        section_tag: None,
                    },
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();

        let segs = get_segments(&conn, &iv.id).unwrap();
        assert_eq!(segs.len(), 3);

        // Initial list is empty
        assert!(list_segment_reviews(&conn, &iv.id).unwrap().is_empty());

        // Error on non-existent segment
        assert!(set_segment_reviewed(&conn, "non-existent-seg", true).is_err());

        // Mark turn 2 reviewed, then turn 0 reviewed
        set_segment_reviewed(&conn, &segs[2].id, true).unwrap();
        set_segment_reviewed(&conn, &segs[0].id, true).unwrap();

        // List must return in segment_index order: segs[0], segs[2]
        let reviewed = list_segment_reviews(&conn, &iv.id).unwrap();
        assert_eq!(reviewed, vec![segs[0].id.clone(), segs[2].id.clone()]);

        // Duplicate set true updates timestamp and succeeds
        set_segment_reviewed(&conn, &segs[0].id, true).unwrap();

        // Clear turn 0
        set_segment_reviewed(&conn, &segs[0].id, false).unwrap();
        let reviewed = list_segment_reviews(&conn, &iv.id).unwrap();
        assert_eq!(reviewed, vec![segs[2].id.clone()]);

        // Cascades on delete segment
        conn.execute(
            "DELETE FROM transcript_segments WHERE id = ?1",
            params![segs[2].id],
        )
        .unwrap();
        assert!(list_segment_reviews(&conn, &iv.id).unwrap().is_empty());

        // No sync side effects
        let outbox_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outbox_count, 0);
    }
}
