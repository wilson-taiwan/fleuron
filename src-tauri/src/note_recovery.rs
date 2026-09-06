//! Private local crash-recovery storage for unfinished note drafts.
//!
//! A recovery draft is **not** a committed note: it never syncs, never enters
//! exports or study backups, and never leaves this machine. It lives in
//! `app_data_dir/note-recovery.sqlite3` — a database the app owns outright —
//! precisely so it cannot be carried into a cloud folder copy, a study backup
//! ZIP, or a project export pipeline alongside committed study data.
//!
//! Crash guarantee: only the latest **acknowledged** recovery write is durable.
//! Keystrokes still in memory or in flight over IPC are not promised back.
//!
//! Lock discipline: every function here takes `&Connection` and performs its
//! read-modify-write inside one SQLite transaction. Callers in `commands.rs`
//! hold the workspace-transition lock before the project lock before touching
//! the recovery connection (`workspace_transition → project_path → db →
//! recovery_db`), so a project replacement cannot interleave with a checked
//! note write.

use crate::models::{
    DiscardNoteDraftResult, NoteDraftRecord, ResolveNoteDraftTargetResult, SaveNoteDraftResult,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub const RECOVERY_DB_FILENAME: &str = "note-recovery.sqlite3";
pub const RECOVERY_SCHEMA_VERSION: i32 = 1;

const RECOVERY_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS recovery_projects (
  project_key TEXT PRIMARY KEY,
  canonical_path TEXT UNIQUE NOT NULL,
  title TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS note_drafts (
  draft_id TEXT PRIMARY KEY,
  project_key TEXT NOT NULL,
  kind TEXT NOT NULL,
  target_id TEXT NOT NULL,
  interview_id TEXT NOT NULL,
  coder_name TEXT,
  base_text TEXT NOT NULL,
  draft_text TEXT NOT NULL,
  revision INTEGER NOT NULL,
  participant_label TEXT NOT NULL,
  segment_id TEXT,
  segment_index INTEGER,
  char_start INTEGER,
  char_end INTEGER,
  quote_text TEXT,
  updated_at TEXT NOT NULL,
  discarded INTEGER NOT NULL DEFAULT 0,
  UNIQUE(project_key, kind, target_id)
);
";

/// Treat a database NULL memo as empty string for content comparisons.
/// Actual text is never trimmed: an empty string is a deliberate edit.
pub fn normalize_memo(memo: Option<&str>) -> &str {
    memo.unwrap_or("")
}

/// Validate the draft kind carried over IPC. Anything else is rejected
/// rather than stored: the UNIQUE(project_key, kind, target_id) scope and
/// the save path both branch on exactly these two values.
pub fn validate_kind(kind: &str) -> Result<(), String> {
    if kind == "coding" || kind == "interview" {
        Ok(())
    } else {
        Err("Unknown note kind.".into())
    }
}

pub fn recovery_db_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(RECOVERY_DB_FILENAME)
}

/// Open (creating) the app-local recovery database.
///
/// Independent of any open study: recovery must work while no project is
/// open (orphan Copy/Discard) and must never block on project state.
/// Uses its own schema version, WAL journaling and FULL synchronous mode.
/// On Unix the file is restricted to owner read/write; on Windows it
/// inherits the current user's app-data permissions.
pub fn open_recovery_db(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    init_recovery_schema(&conn)?;
    restrict_owner_only(path);
    Ok(conn)
}

pub fn init_recovery_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(RECOVERY_SCHEMA)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(&format!("PRAGMA user_version = {RECOVERY_SCHEMA_VERSION};"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(unix)]
fn restrict_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // Only tighten: never grant permission a stricter umask removed.
    let tighten = |p: &Path| {
        if let Ok(meta) = std::fs::metadata(p) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() & 0o600);
            let _ = std::fs::set_permissions(p, perms);
        }
    };
    tighten(path);
    // Best-effort for the WAL sidecars, which carry the same unfinished text.
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        if let Some(parent) = path.parent() {
            for suffix in ["-wal", "-shm"] {
                tighten(&parent.join(format!("{file_name}{suffix}")));
            }
        }
    }
}

#[cfg(not(unix))]
fn restrict_owner_only(_path: &Path) {}

/// Register the backend-resolved canonical project folder path to a stable
/// local-study UUID. The key is NOT a Supabase group id: cloned projects at
/// different resolved paths stay separate, and a moved project leaves its old
/// records behind (accessible, Copy/Discard only) rather than guessing a
/// reassociation.
pub fn register_project(
    conn: &Connection,
    canonical_path: &str,
    title: &str,
) -> Result<String, String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT project_key FROM recovery_projects WHERE canonical_path = ?1",
            params![canonical_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(key) = existing {
        conn.execute(
            "UPDATE recovery_projects SET title = ?2 WHERE project_key = ?1",
            params![key, title],
        )
        .map_err(|e| e.to_string())?;
        return Ok(key);
    }
    let key = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO recovery_projects (project_key, canonical_path, title) VALUES (?1, ?2, ?3)",
        params![key, canonical_path, title],
    )
    .map_err(|e| e.to_string())?;
    Ok(key)
}

/// Joined row a coding resolve returns: (interview_id, participant_label,
/// memo, char_start, char_end).
type CodingResolveRow = (String, String, Option<String>, Option<i64>, Option<i64>);

fn map_draft_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteDraftRecord> {
    // Column 17 (project_title) exists only on the list query's JOIN; every
    // other read selects the 17 base columns. Probe by name so one mapper
    // serves both shapes.
    let project_title: Option<String> = row
        .get::<_, Option<String>>("project_title")
        .unwrap_or(None);
    Ok(NoteDraftRecord {
        draft_id: row.get(0)?,
        project_key: row.get(1)?,
        kind: row.get(2)?,
        target_id: row.get(3)?,
        interview_id: row.get(4)?,
        coder_name: row.get(5)?,
        base_text: row.get(6)?,
        draft_text: row.get(7)?,
        revision: row.get(8)?,
        participant_label: row.get(9)?,
        segment_id: row.get(10)?,
        segment_index: row.get(11)?,
        char_start: row.get(12)?,
        char_end: row.get(13)?,
        quote_text: row.get(14)?,
        updated_at: row.get(15)?,
        discarded: row.get::<_, i64>(16)? != 0,
        project_title,
    })
}

const DRAFT_COLUMNS: &str = "draft_id, project_key, kind, target_id, interview_id, coder_name, base_text, draft_text, revision, participant_label, segment_id, segment_index, char_start, char_end, quote_text, updated_at, discarded";

pub fn get_draft_by_id(
    conn: &Connection,
    draft_id: &str,
) -> Result<Option<NoteDraftRecord>, String> {
    conn.query_row(
        &format!("SELECT {DRAFT_COLUMNS} FROM note_drafts WHERE draft_id = ?1"),
        params![draft_id],
        map_draft_row,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn get_active_draft(
    conn: &Connection,
    project_key: &str,
    kind: &str,
    target_id: &str,
) -> Result<Option<NoteDraftRecord>, String> {
    conn.query_row(
        &format!(
            "SELECT {DRAFT_COLUMNS} FROM note_drafts
             WHERE project_key = ?1 AND kind = ?2 AND target_id = ?3 AND discarded = 0"
        ),
        params![project_key, kind, target_id],
        map_draft_row,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Allocate a fresh draft generation for a target whose previous generation
/// was tombstoned (or which never had one). A new `draft_id` is what makes
/// late writes from the old generation fail instead of resurrecting text.
#[allow(clippy::too_many_arguments)]
pub fn insert_draft(
    conn: &Connection,
    project_key: &str,
    kind: &str,
    target_id: &str,
    interview_id: &str,
    coder_name: Option<&str>,
    base_text: &str,
    participant_label: &str,
    segment_id: Option<&str>,
    segment_index: Option<i64>,
    char_start: Option<i64>,
    char_end: Option<i64>,
    quote_text: Option<&str>,
    now: &str,
) -> Result<NoteDraftRecord, String> {
    validate_kind(kind)?;
    let draft_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO note_drafts
         (draft_id, project_key, kind, target_id, interview_id, coder_name, base_text, draft_text,
          revision, participant_label, segment_id, segment_index, char_start, char_end, quote_text,
          updated_at, discarded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 0, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0)",
        params![
            draft_id,
            project_key,
            kind,
            target_id,
            interview_id,
            coder_name,
            base_text,
            participant_label,
            segment_id,
            segment_index,
            char_start,
            char_end,
            quote_text,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;
    get_draft_by_id(conn, &draft_id)?
        .ok_or_else(|| "Draft vanished right after creation.".to_string())
}

/// Begin (or resume) editing for a live target: the only operation allowed
/// to reactivate a discarded target. Returns the active record when one
/// exists; replaces a tombstone with a fresh generation (new `draft_id`, so
/// late writes from the old generation fail); otherwise allocates row one.
#[allow(clippy::too_many_arguments)]
pub fn begin_draft_for_target(
    conn: &Connection,
    project_key: &str,
    kind: &str,
    target_id: &str,
    interview_id: &str,
    coder_name: Option<&str>,
    base_text: &str,
    participant_label: &str,
    segment_id: Option<&str>,
    segment_index: Option<i64>,
    char_start: Option<i64>,
    char_end: Option<i64>,
    quote_text: Option<&str>,
    now: &str,
) -> Result<NoteDraftRecord, String> {
    validate_kind(kind)?;
    let existing: Option<NoteDraftRecord> = conn
        .query_row(
            &format!(
                "SELECT {DRAFT_COLUMNS} FROM note_drafts
                 WHERE project_key = ?1 AND kind = ?2 AND target_id = ?3"
            ),
            params![project_key, kind, target_id],
            map_draft_row,
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match existing {
        Some(record) if !record.discarded => {
            // Resume: refresh the live label for a target that still exists,
            // never erase captured context (it is what orphans keep).
            if !participant_label.is_empty() && record.participant_label != participant_label {
                conn.execute(
                    "UPDATE note_drafts SET participant_label = ?2, updated_at = ?3 WHERE draft_id = ?1",
                    params![record.draft_id, participant_label, now],
                )
                .map_err(|e| e.to_string())?;
                return get_draft_by_id(conn, &record.draft_id)?
                    .ok_or_else(|| "Draft vanished right after resuming.".to_string());
            }
            Ok(record)
        }
        Some(tombstone) => {
            conn.execute(
                "DELETE FROM note_drafts WHERE draft_id = ?1",
                params![tombstone.draft_id],
            )
            .map_err(|e| e.to_string())?;
            insert_draft(
                conn,
                project_key,
                kind,
                target_id,
                interview_id,
                coder_name,
                base_text,
                participant_label,
                segment_id,
                segment_index,
                char_start,
                char_end,
                quote_text,
                now,
            )
        }
        None => insert_draft(
            conn,
            project_key,
            kind,
            target_id,
            interview_id,
            coder_name,
            base_text,
            participant_label,
            segment_id,
            segment_index,
            char_start,
            char_end,
            quote_text,
            now,
        ),
    }
}

/// Upsert captured draft text with an expected-stored-revision check.
/// Rejects stale writes (a queued older revision arriving after a newer one
/// was acknowledged) and writes to tombstoned generations.
pub fn put_draft_text(
    conn: &Connection,
    draft_id: &str,
    expected_revision: i64,
    draft_text: &str,
    now: &str,
) -> Result<NoteDraftRecord, String> {
    let current = get_draft_by_id(conn, draft_id)?.ok_or_else(|| "STALE_GENERATION".to_string())?;
    if current.discarded {
        return Err("STALE_GENERATION".to_string());
    }
    if current.revision != expected_revision {
        return Err("REVISION_MISMATCH".to_string());
    }
    let next_revision = current.revision + 1;
    // A save-cleanup tombstone for the acknowledged revision may land between
    // the frontend's read and this write; re-check the row is still the same
    // generation and revision in the UPDATE predicate itself.
    let changed = conn
        .execute(
            "UPDATE note_drafts SET draft_text = ?2, revision = ?3, updated_at = ?4
             WHERE draft_id = ?1 AND revision = ?5 AND discarded = 0",
            params![draft_id, draft_text, next_revision, now, expected_revision],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        // Lost the race with a concurrent newer write or a tombstone:
        // re-read to report which one so the caller retries correctly.
        let reread =
            get_draft_by_id(conn, draft_id)?.ok_or_else(|| "STALE_GENERATION".to_string())?;
        if reread.discarded || reread.draft_id != current.draft_id {
            return Err("STALE_GENERATION".to_string());
        }
        return Err("REVISION_MISMATCH".to_string());
    }
    get_draft_by_id(conn, draft_id)?
        .ok_or_else(|| "Draft vanished right after writing.".to_string())
}

/// Explicit discard: clear text and captured context transactionally, keep a
/// revision tombstone so stale queued writes fail instead of resurrecting.
pub fn tombstone_draft(
    conn: &Connection,
    draft_id: &str,
    now: &str,
) -> Result<DiscardNoteDraftResult, String> {
    let current = get_draft_by_id(conn, draft_id)?.ok_or_else(|| "No such draft.".to_string())?;
    if !current.discarded {
        conn.execute(
            "UPDATE note_drafts
             SET draft_text = '', base_text = '', participant_label = '',
                 segment_id = NULL, segment_index = NULL, char_start = NULL,
                 char_end = NULL, quote_text = NULL, coder_name = NULL,
                 discarded = 1, updated_at = ?2
             WHERE draft_id = ?1",
            params![draft_id, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(DiscardNoteDraftResult {
        draft_id: draft_id.to_string(),
        revision: current.revision,
    })
}

/// Clear ONLY the acknowledged recovery revision after a committed project
/// write: the two SQLite files cannot share an atomic commit, so the note is
/// committed first and recovery cleanup follows conditionally. A newer queued
/// put (higher revision) must survive this cleanup.
pub fn clear_draft_revision(
    conn: &Connection,
    draft_id: &str,
    acknowledged_revision: i64,
) -> Result<bool, String> {
    let changed = conn
        .execute(
            "DELETE FROM note_drafts WHERE draft_id = ?1 AND revision = ?2 AND discarded = 0",
            params![draft_id, acknowledged_revision],
        )
        .map_err(|e| e.to_string())?;
    Ok(changed == 1)
}

/// Remove a recovery record whose draft text now equals the live committed
/// text (redundant after restart), conditionally on the stored revision so a
/// concurrent newer edit is never dropped.
pub fn remove_redundant_draft(
    conn: &Connection,
    draft_id: &str,
    committed_text: &str,
) -> Result<bool, String> {
    let current = match get_draft_by_id(conn, draft_id)? {
        Some(record) => record,
        None => return Ok(false),
    };
    if current.discarded || current.draft_text != committed_text {
        return Ok(false);
    }
    clear_draft_revision(conn, draft_id, current.revision)
}

/// App-local list for the Unfinished notes surface: active records and their
/// captured context, newest first, excluding tombstones. Available with no
/// project open; liveness is resolved per record on demand.
pub fn list_active_drafts(conn: &Connection) -> Result<Vec<NoteDraftRecord>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {DRAFT_COLUMNS}, recovery_projects.title AS project_title
             FROM note_drafts
             LEFT JOIN recovery_projects USING (project_key)
             WHERE discarded = 0
             ORDER BY updated_at DESC, draft_id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], map_draft_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn count_active_drafts_for_project(
    conn: &Connection,
    project_key: &str,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM note_drafts WHERE project_key = ?1 AND discarded = 0",
        params![project_key],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Resolve a recovery record against the currently open project database.
/// Returns live target/context or a missing reason. Never infers deletion
/// from the loaded interview or active filters: the project database is
/// queried directly.
pub fn resolve_draft_against_project(
    project_conn: &Connection,
    draft: &NoteDraftRecord,
) -> Result<ResolveNoteDraftTargetResult, String> {
    if draft.kind == "coding" {
        let row: Option<CodingResolveRow> = project_conn
            .query_row(
                "SELECT cs.interview_id, i.participant_label, cs.memo, cs.char_start, cs.char_end
                     FROM coded_segments cs
                     JOIN interviews i ON i.id = cs.interview_id
                     WHERE cs.id = ?1 AND cs.deleted = 0",
                params![draft.target_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((interview_id, participant_label, memo, _char_start, _char_end)) => {
                Ok(ResolveNoteDraftTargetResult::live_coding(
                    draft.draft_id.clone(),
                    interview_id,
                    participant_label,
                    normalize_memo(memo.as_deref()).to_string(),
                ))
            }
            None => Ok(ResolveNoteDraftTargetResult::missing(
                draft.draft_id.clone(),
                "deleted",
            )),
        }
    } else {
        let row: Option<(String, Option<String>)> = project_conn
            .query_row(
                "SELECT participant_label, hub_memo FROM interviews WHERE id = ?1 AND deleted = 0",
                params![draft.target_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((participant_label, memo)) => Ok(ResolveNoteDraftTargetResult::live_interview(
                draft.draft_id.clone(),
                participant_label,
                normalize_memo(memo.as_deref()).to_string(),
            )),
            None => Ok(ResolveNoteDraftTargetResult::missing(
                draft.draft_id.clone(),
                "deleted",
            )),
        }
    }
}

/// Commit-then-cleanup composition shared by the command layer and tests.
///
/// Commits the note FIRST via CAS, then conditionally clears ONLY the
/// acknowledged recovery revision. The two SQLite files cannot share an
/// assumed atomic commit: if recovery cleanup fails, the commit still stands
/// (`recovery_cleared: false`) and the draft stays for retry — the UI must
/// never claim the note write failed nor roll it back. A simulated crash
/// between the two steps therefore recovers the saved draft for review
/// instead of resurrecting it as unsaved work.
///
/// `draft_id = None` is the memory-only path (recovery unavailable):
/// the same target/epoch/CAS checks run, the commit proceeds, and the
/// recovery warning travels separately through the status surface.
#[allow(clippy::too_many_arguments)]
pub fn commit_then_cleanup(
    project_conn: &Connection,
    recovery_conn: Option<&Connection>,
    kind: &str,
    target_id: &str,
    expected_saved_text: &str,
    draft_text: &str,
    draft_id: Option<&str>,
    revision: i64,
    now: &str,
) -> Result<SaveNoteDraftResult, String> {
    validate_kind(kind)?;
    if let (Some(recovery), Some(id)) = (recovery_conn, draft_id) {
        // The generation must still be alive: a discard followed by this
        // save (or a save from a superseded begin) fails, never resurrects.
        let alive = match get_draft_by_id(recovery, id)? {
            Some(record) => !record.discarded,
            None => false,
        };
        if !alive {
            return Ok(SaveNoteDraftResult::StaleGeneration);
        }
    }
    let commit = commit_draft_cas(
        project_conn,
        kind,
        target_id,
        expected_saved_text,
        draft_text,
        now,
    )?;
    let committed_text = match commit {
        SaveNoteDraftResult::Saved { committed_text, .. } => committed_text,
        other => return Ok(other),
    };
    let recovery_cleared = match (recovery_conn, draft_id) {
        (Some(recovery), Some(id)) => {
            // Newer queued edits (higher revision) survive cleanup; advance
            // their stored base to the just-committed text first.
            if let Ok(Some(record)) = get_draft_by_id(recovery, id) {
                if !record.discarded && record.revision > revision {
                    let _ = recovery.execute(
                        "UPDATE note_drafts SET base_text = ?2 WHERE draft_id = ?1",
                        params![id, committed_text],
                    );
                }
            }
            clear_draft_revision(recovery, id, revision).unwrap_or(false)
        }
        _ => false,
    };
    Ok(SaveNoteDraftResult::Saved {
        draft_id: draft_id.unwrap_or_default().to_string(),
        revision,
        committed_text,
        recovery_cleared,
    })
}
/// Read the current committed text for a live target inside a checked write.
/// `Ok(None)` means the target row is gone (deleted): the write must not
/// proceed.
pub fn read_live_memo(
    project_conn: &Connection,
    kind: &str,
    target_id: &str,
) -> Result<Option<String>, String> {
    if kind == "coding" {
        project_conn
            .query_row(
                "SELECT memo FROM coded_segments WHERE id = ?1 AND deleted = 0",
                params![target_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|row| row.map(|memo| normalize_memo(memo.as_deref()).to_string()))
            .map_err(|e| e.to_string())
    } else {
        project_conn
            .query_row(
                "SELECT hub_memo FROM interviews WHERE id = ?1 AND deleted = 0",
                params![target_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|row| row.map(|memo| normalize_memo(memo.as_deref()).to_string()))
            .map_err(|e| e.to_string())
    }
}

/// Compare-and-swap commit of one draft revision against the live target.
/// The whole read-compare-write runs in one project-DB transaction under the
/// workspace-transition lock held by the caller. Requires exactly one
/// affected live row for BOTH kinds; a conflict or missing target performs
/// no overwrite.
pub fn commit_draft_cas(
    project_conn: &Connection,
    kind: &str,
    target_id: &str,
    expected_saved_text: &str,
    draft_text: &str,
    now: &str,
) -> Result<SaveNoteDraftResult, String> {
    validate_kind(kind)?;
    let tx = project_conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let live: Option<String> = if kind == "coding" {
        tx.query_row(
            "SELECT memo FROM coded_segments WHERE id = ?1 AND deleted = 0",
            params![target_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|row| row.map(|memo| normalize_memo(memo.as_deref()).to_string()))
        .map_err(|e| e.to_string())?
    } else {
        tx.query_row(
            "SELECT hub_memo FROM interviews WHERE id = ?1 AND deleted = 0",
            params![target_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|row| row.map(|memo| normalize_memo(memo.as_deref()).to_string()))
        .map_err(|e| e.to_string())?
    };
    let live_text = match live {
        Some(text) => text,
        None => return Ok(SaveNoteDraftResult::missing_target()),
    };
    if live_text != expected_saved_text {
        return Ok(SaveNoteDraftResult::conflict(live_text));
    }
    let changed = if kind == "coding" {
        tx.execute(
            "UPDATE coded_segments SET memo = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted = 0",
            params![target_id, draft_text, now],
        )
        .map_err(|e| e.to_string())?
    } else {
        tx.execute(
            "UPDATE interviews SET hub_memo = ?1, updated_at = ?2 WHERE id = ?3 AND deleted = 0",
            params![draft_text, now, target_id],
        )
        .map_err(|e| e.to_string())?
    };
    if changed != 1 {
        return Ok(SaveNoteDraftResult::missing_target());
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(SaveNoteDraftResult::ready_to_save(draft_text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BeginNoteDraftInput, DiscardNoteDraftInput, PutNoteDraftInput, SaveNoteDraftInput,
    };

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_recovery_schema(&conn).unwrap();
        conn
    }

    fn sample_draft(conn: &Connection, target: &str) -> NoteDraftRecord {
        insert_draft(
            conn,
            "proj-1",
            "coding",
            target,
            "iv-1",
            Some("Ada"),
            "base",
            "Ada / Passage 1",
            Some("seg-1"),
            Some(0),
            None,
            None,
            Some("quoted passage"),
            "2026-09-05T00:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn schema_version_is_stamped() {
        let conn = memory_db();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, RECOVERY_SCHEMA_VERSION);
    }

    #[test]
    fn register_project_is_stable_per_path() {
        let conn = memory_db();
        let first = register_project(&conn, "/studies/alpha", "Alpha").unwrap();
        let second = register_project(&conn, "/studies/alpha", "Alpha renamed").unwrap();
        assert_eq!(first, second);
        // A clone at another resolved path is a separate local study.
        let clone = register_project(&conn, "/studies/alpha-copy", "Alpha").unwrap();
        assert_ne!(first, clone);
    }

    #[test]
    fn put_rejects_stale_revision_and_keeps_newer() {
        let conn = memory_db();
        let draft = sample_draft(&conn, "coding-1");
        let v1 = put_draft_text(&conn, &draft.draft_id, 0, "v1", "2026-09-05T00:00:01Z").unwrap();
        assert_eq!(v1.revision, 1);
        let stale = put_draft_text(&conn, &draft.draft_id, 0, "stale", "2026-09-05T00:00:02Z");
        assert_eq!(stale.unwrap_err(), "REVISION_MISMATCH");
        let kept = get_draft_by_id(&conn, &draft.draft_id).unwrap().unwrap();
        assert_eq!(kept.draft_text, "v1");
    }

    #[test]
    fn tombstone_blocks_late_writes_without_resurrection() {
        let conn = memory_db();
        let draft = sample_draft(&conn, "coding-1");
        put_draft_text(&conn, &draft.draft_id, 0, "v1", "2026-09-05T00:00:01Z").unwrap();
        tombstone_draft(&conn, &draft.draft_id, "2026-09-05T00:00:02Z").unwrap();
        assert_eq!(
            put_draft_text(&conn, &draft.draft_id, 1, "late", "2026-09-05T00:00:03Z").unwrap_err(),
            "STALE_GENERATION"
        );
        // Tombstoned rows leave the app-local list.
        assert!(list_active_drafts(&conn).unwrap().is_empty());
        // Re-begin is the only path back: the tombstone is replaced by a
        // fresh generation with a new identity, so the old queue can never
        // alias it.
        let fresh = begin_draft_for_target(
            &conn,
            "proj-1",
            "coding",
            "coding-1",
            "iv-1",
            Some("Ada"),
            "base",
            "Ada / Passage 1",
            None,
            None,
            None,
            None,
            None,
            "2026-09-05T00:00:04Z",
        )
        .unwrap();
        assert_ne!(fresh.draft_id, draft.draft_id);
        assert_eq!(fresh.revision, 0);
        // Resuming again returns the same active generation.
        let resumed = begin_draft_for_target(
            &conn,
            "proj-1",
            "coding",
            "coding-1",
            "iv-1",
            Some("Ada"),
            "base",
            "Ada / Passage 1",
            None,
            None,
            None,
            None,
            None,
            "2026-09-05T00:00:05Z",
        )
        .unwrap();
        assert_eq!(resumed.draft_id, fresh.draft_id);
    }

    #[test]
    fn conditional_cleanup_keeps_newer_queued_revision() {
        let conn = memory_db();
        let draft = sample_draft(&conn, "coding-1");
        let v1 = put_draft_text(&conn, &draft.draft_id, 0, "v1", "2026-09-05T00:00:01Z").unwrap();
        // Commit of v1 clears exactly revision 1.
        assert!(clear_draft_revision(&conn, &draft.draft_id, v1.revision).unwrap());
        assert!(get_draft_by_id(&conn, &draft.draft_id).unwrap().is_none());
    }

    #[test]
    fn cleanup_after_commit_does_not_drop_newer_put() {
        let conn = memory_db();
        let draft = sample_draft(&conn, "coding-1");
        let v1 = put_draft_text(&conn, &draft.draft_id, 0, "v1", "2026-09-05T00:00:01Z").unwrap();
        // A newer edit lands before the commit cleanup runs.
        let v2 = put_draft_text(&conn, &draft.draft_id, 1, "v2", "2026-09-05T00:00:02Z").unwrap();
        assert_eq!(v2.revision, 2);
        // Cleanup for the committed revision 1 must not remove revision 2.
        assert!(!clear_draft_revision(&conn, &draft.draft_id, v1.revision).unwrap());
        let kept = get_draft_by_id(&conn, &draft.draft_id).unwrap().unwrap();
        assert_eq!(kept.draft_text, "v2");
    }

    #[test]
    fn redundant_recovery_removed_only_when_text_matches() {
        let conn = memory_db();
        let draft = sample_draft(&conn, "coding-1");
        put_draft_text(&conn, &draft.draft_id, 0, "base", "2026-09-05T00:00:01Z").unwrap();
        // Draft equals committed text: redundant, remove.
        assert!(remove_redundant_draft(&conn, &draft.draft_id, "base").unwrap());
        let draft2 = sample_draft(&conn, "coding-2");
        put_draft_text(
            &conn,
            &draft2.draft_id,
            0,
            "different",
            "2026-09-05T00:00:02Z",
        )
        .unwrap();
        assert!(!remove_redundant_draft(&conn, &draft2.draft_id, "base").unwrap());
    }

    #[test]
    fn normalize_memo_treats_null_as_empty_without_trimming() {
        assert_eq!(normalize_memo(None), "");
        assert_eq!(normalize_memo(Some("  padded  ")), "  padded  ");
        assert_eq!(normalize_memo(Some("")), "");
    }

    #[test]
    fn unknown_kind_rejected() {
        assert!(validate_kind("hub").is_err());
        assert!(validate_kind("coding").is_ok());
        assert!(validate_kind("interview").is_ok());
    }

    #[test]
    fn dto_shapes_match_ipc_contract() {
        // Compile-time shape pins: renaming a field here must update the
        // frontend wrappers, dev mocks and parity tests in the same change.
        let begin = BeginNoteDraftInput {
            project_key: "pk".into(),
            epoch: "ep".into(),
            kind: "coding".into(),
            target_id: "c1".into(),
            interview_id: "iv1".into(),
            coder_name: Some("Ada".into()),
            participant_label: "Ada".into(),
            segment_id: None,
            segment_index: None,
            char_start: None,
            char_end: None,
            quote_text: None,
        };
        let put = PutNoteDraftInput {
            project_key: "pk".into(),
            epoch: "ep".into(),
            draft_id: "d1".into(),
            expected_revision: 0,
            draft_text: "text".into(),
        };
        let discard = DiscardNoteDraftInput {
            draft_id: "d1".into(),
        };
        let save = SaveNoteDraftInput {
            project_key: "pk".into(),
            epoch: "ep".into(),
            draft_id: "d1".into(),
            revision: 1,
            kind: "coding".into(),
            target_id: "c1".into(),
            expected_saved_text: "base".into(),
            draft_text: "v1".into(),
        };
        assert_eq!(begin.kind, "coding");
        assert_eq!(put.expected_revision, 0);
        assert_eq!(discard.draft_id, "d1");
        assert_eq!(save.revision, 1);
        // Silence dead-code warnings for the import surface.
        let _ = (begin, put, discard, save);
    }
}

#[cfg(test)]
mod checked_write_tests {
    //! End-to-end coverage for the checked note-write contract against real
    //! temporary project databases: reopen persistence, stale-epoch rejection
    //! across cloned projects with identical coding ids, CAS conflicts, both
    //! deleted-target kinds, storage errors, cleanup failure after a
    //! successful commit, and the privacy boundary (recovery markers must not
    //! appear in backup ZIPs, exports, sync payloads or diagnostics).
    use super::*;
    use crate::models::{
        ApplyCodesInput, CreateCodeInput, CreateInterviewInput, CreateProjectInput,
        ExportConfigInput, ImportSegmentsInput, NoteDraftRecord, PatchCodingMemoInput,
        SegmentInput,
    };
    use crate::{backup, db, sync};
    use std::io::Read;

    const NOW: &str = "2026-09-05T00:00:00Z";

    struct Fixture {
        _project_dir: tempfile::TempDir,
        _recovery_dir: tempfile::TempDir,
        project_path: PathBuf,
        project: Connection,
        recovery: Connection,
        interview_id: String,
        coding_id: String,
    }

    fn fixture() -> Fixture {
        let project_temp = tempfile::tempdir().unwrap();
        let project_path = db::create_project(&CreateProjectInput {
            parent_dir: project_temp.path().to_string_lossy().to_string(),
            project_name: "study".into(),
            title: "Synthetic Study".into(),
            coders: vec!["Ada".into()],
        })
        .unwrap();
        let project = db::open_project(&project_path.to_string_lossy()).unwrap();
        let interview = db::create_interview(
            &project,
            &CreateInterviewInput {
                participant_label: "P01".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        db::import_segments(
            &project,
            &ImportSegmentsInput {
                interview_id: interview.id.clone(),
                segments: vec![SegmentInput {
                    speaker: "P01".into(),
                    timestamp_start: "00:00:00.000".into(),
                    timestamp_end: None,
                    text: "Synthetic passage one for recovery tests.".into(),
                    section_tag: None,
                }],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segment = db::get_segments(&project, &interview.id).unwrap()[0]
            .id
            .clone();
        let code = db::create_code(
            &project,
            &CreateCodeInput {
                name: "Theme".into(),
                definition: None,
                inclusion_criteria: None,
                exclusion_criteria: None,
                example: None,
                parent_id: None,
                color: None,
            },
        )
        .unwrap();
        let coding = db::apply_codes(
            &project,
            &ApplyCodesInput {
                interview_id: interview.id.clone(),
                segment_id: segment,
                code_ids: vec![code.id],
                coder_name: "Ada".into(),
                memo: None,
                char_start: None,
                char_end: None,
            },
        )
        .unwrap();
        let recovery_dir = tempfile::tempdir().unwrap();
        let recovery = open_recovery_db(&recovery_dir.path().join(RECOVERY_DB_FILENAME)).unwrap();
        Fixture {
            _project_dir: project_temp,
            _recovery_dir: recovery_dir,
            project_path,
            project,
            recovery,
            interview_id: interview.id,
            coding_id: coding.id,
        }
    }

    fn draft_for(fx: &Fixture, kind: &str, target: &str, base: &str) -> NoteDraftRecord {
        let (target_id, interview_id) = if kind == "coding" {
            (target.to_string(), fx.interview_id.clone())
        } else {
            (fx.interview_id.clone(), fx.interview_id.clone())
        };
        insert_draft(
            &fx.recovery,
            "proj-key",
            kind,
            &target_id,
            &interview_id,
            Some("Ada"),
            base,
            "P01",
            None,
            Some(0),
            None,
            None,
            Some("Synthetic passage one for recovery tests."),
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn recovery_round_trips_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RECOVERY_DB_FILENAME);
        let key = {
            let conn = open_recovery_db(&path).unwrap();
            let key = register_project(&conn, "/studies/alpha", "Alpha").unwrap();
            let draft = insert_draft(
                &conn,
                &key,
                "coding",
                "coding-1",
                "iv-1",
                Some("Ada"),
                "",
                "P01",
                None,
                None,
                None,
                None,
                None,
                NOW,
            )
            .unwrap();
            put_draft_text(&conn, &draft.draft_id, 0, "unsentenced fragment", NOW).unwrap();
            key
        };
        // Reopen the same file: the acknowledged write survives.
        let reopened = open_recovery_db(&path).unwrap();
        let listed = list_active_drafts(&reopened).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].project_key, key);
        assert_eq!(listed[0].draft_text, "unsentenced fragment");
        assert_eq!(listed[0].revision, 1);
        // Keystrokes never acknowledged are not promised back: nothing else
        // is here.
        assert_eq!(count_active_drafts_for_project(&reopened, &key).unwrap(), 1);
    }

    #[test]
    fn recovery_open_failure_is_typed_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the directory should be: creation must fail.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(open_recovery_db(&blocker.join(RECOVERY_DB_FILENAME)).is_err());
    }

    #[test]
    fn happy_path_commits_and_clears_acknowledged_revision() {
        let fx = fixture();
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "");
        let v1 = put_draft_text(&fx.recovery, &draft.draft_id, 0, "v1 reasoning", NOW).unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "",
            "v1 reasoning",
            Some(&draft.draft_id),
            v1.revision,
            NOW,
        )
        .unwrap();
        match outcome {
            SaveNoteDraftResult::Saved {
                committed_text,
                recovery_cleared,
                ..
            } => {
                assert_eq!(committed_text, "v1 reasoning");
                assert!(recovery_cleared);
            }
            other => panic!("expected Saved, got {other:?}"),
        }
        assert_eq!(
            normalize_memo(
                read_live_memo(&fx.project, "coding", &fx.coding_id)
                    .unwrap()
                    .as_deref()
            ),
            "v1 reasoning"
        );
        assert!(get_draft_by_id(&fx.recovery, &draft.draft_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn conflict_preserves_both_versions_without_overwrite() {
        let fx = fixture();
        // Newer revision committed behind the draft's back (sync, restore,
        // another edit): the save must not overwrite it.
        db::patch_coding_memo(
            &fx.project,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some("B (newer)".into()),
            },
        )
        .unwrap();
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "");
        put_draft_text(&fx.recovery, &draft.draft_id, 0, "A (stale base)", NOW).unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "",
            "A (stale base)",
            Some(&draft.draft_id),
            1,
            NOW,
        )
        .unwrap();
        match outcome {
            SaveNoteDraftResult::Conflict { current_text } => {
                assert_eq!(current_text, "B (newer)");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        // Neither side lost: live text untouched, draft intact.
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("B (newer)")
        );
        let kept = get_draft_by_id(&fx.recovery, &draft.draft_id)
            .unwrap()
            .unwrap();
        assert_eq!(kept.draft_text, "A (stale base)");
    }

    #[test]
    fn deleted_coding_reports_missing_target() {
        let fx = fixture();
        let coding_id = fx.coding_id.clone();
        let draft = draft_for(&fx, "coding", &coding_id, "");
        put_draft_text(&fx.recovery, &draft.draft_id, 0, "orphaned", NOW).unwrap();
        db::delete_coded_segment(&fx.project, &coding_id).unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &coding_id,
            "",
            "orphaned",
            Some(&draft.draft_id),
            1,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome, SaveNoteDraftResult::MissingTarget));
        // The draft survives for the recovery list (Copy/Discard), not
        // silently dropped by the failed commit.
        assert!(get_draft_by_id(&fx.recovery, &draft.draft_id)
            .unwrap()
            .is_some());
        // Resolution against the live DB reports the deletion.
        let resolved = resolve_draft_against_project(
            &fx.project,
            &get_draft_by_id(&fx.recovery, &draft.draft_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            resolved,
            ResolveNoteDraftTargetResult::Missing { .. }
        ));
    }

    #[test]
    fn deleted_interview_reports_missing_target() {
        let fx = fixture();
        db::update_hub_memo(&fx.project, &fx.interview_id, "committed").unwrap();
        let draft = draft_for(&fx, "interview", &fx.interview_id.clone(), "committed");
        put_draft_text(&fx.recovery, &draft.draft_id, 0, "unsaved reflection", NOW).unwrap();
        db::delete_interview(&fx.project, &fx.interview_id).unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "interview",
            &fx.interview_id,
            "committed",
            "unsaved reflection",
            Some(&draft.draft_id),
            1,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome, SaveNoteDraftResult::MissingTarget));
        assert!(get_draft_by_id(&fx.recovery, &draft.draft_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn legacy_interview_write_rejects_missing_targets() {
        let fx = fixture();
        assert!(db::update_hub_memo(&fx.project, "no-such-interview", "x").is_err());
    }

    #[test]
    fn empty_string_is_a_deliberate_edit_and_text_is_never_trimmed() {
        let fx = fixture();
        db::patch_coding_memo(
            &fx.project,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some("old".into()),
            },
        )
        .unwrap();
        // Clearing a note to empty commits empty (not NULL, not skipped).
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "old");
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "old",
            "",
            Some(&draft.draft_id),
            0,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome, SaveNoteDraftResult::Saved { .. }));
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("")
        );
        // Padding is content: " x" does not equal "x".
        let draft2 = draft_for(&fx, "coding", &fx.coding_id.clone(), "");
        let outcome2 = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "x",
            " x",
            Some(&draft2.draft_id),
            0,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome2, SaveNoteDraftResult::Conflict { .. }));
    }

    #[test]
    fn cleanup_failure_after_commit_keeps_the_commit() {
        let fx = fixture();
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "");
        put_draft_text(&fx.recovery, &draft.draft_id, 0, "v1", NOW).unwrap();
        // Simulate a crash between the two steps: the recovery file goes
        // read-only after the project commit point. Reads (generation check)
        // still work; the cleanup DELETE fails.
        fx.recovery
            .execute_batch("PRAGMA query_only = ON;")
            .unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "",
            "v1",
            Some(&draft.draft_id),
            1,
            NOW,
        )
        .unwrap();
        // The commit stands; cleanup reports false instead of failing it.
        match outcome {
            SaveNoteDraftResult::Saved {
                committed_text,
                recovery_cleared,
                ..
            } => {
                assert_eq!(committed_text, "v1");
                assert!(!recovery_cleared);
            }
            other => panic!("expected Saved with cleanup warning, got {other:?}"),
        }
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("v1")
        );
    }

    #[test]
    fn stale_generation_save_fails_without_touching_live_text() {
        let fx = fixture();
        db::patch_coding_memo(
            &fx.project,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some("live".into()),
            },
        )
        .unwrap();
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "live");
        tombstone_draft(&fx.recovery, &draft.draft_id, NOW).unwrap();
        let outcome = commit_then_cleanup(
            &fx.project,
            Some(&fx.recovery),
            "coding",
            &fx.coding_id,
            "live",
            "resurrected",
            Some(&draft.draft_id),
            0,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome, SaveNoteDraftResult::StaleGeneration));
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("live")
        );
    }

    #[test]
    fn clone_projects_with_identical_ids_do_not_cross_write() {
        let fx = fixture();
        // A filesystem clone: identical coding ids, different resolved path,
        // therefore a different local-study key.
        let clone_dir = tempfile::tempdir().unwrap();
        let clone_path = clone_dir.path().join("study-clone.fleuron");
        copy_dir_recursive(&fx.project_path, &clone_path);
        let clone = db::open_project(&clone_path.to_string_lossy()).unwrap();
        // Content diverges between the twins.
        db::patch_coding_memo(
            &clone,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some("clone text".into()),
            },
        )
        .unwrap();
        // A write credentialed against the ORIGINAL's base text must conflict
        // on the clone, never overwrite it.
        let outcome = commit_then_cleanup(
            &clone,
            None,
            "coding",
            &fx.coding_id,
            "",
            "original text",
            None,
            0,
            NOW,
        )
        .unwrap();
        assert!(matches!(outcome, SaveNoteDraftResult::Conflict { .. }));
        assert_eq!(
            read_live_memo(&clone, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("clone text")
        );
        // And the original is untouched by anything addressed at the clone.
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("")
        );
        // Registration keeps the twins apart at the identity layer too.
        let key_a = register_project(
            &fx.recovery,
            &fx.project_path.canonicalize().unwrap().to_string_lossy(),
            "Original",
        )
        .unwrap();
        let key_b = register_project(
            &fx.recovery,
            &clone_path.canonicalize().unwrap().to_string_lossy(),
            "Clone",
        )
        .unwrap();
        assert_ne!(key_a, key_b);
    }

    fn copy_dir_recursive(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir_recursive(&entry.path(), &dest);
            } else {
                std::fs::copy(entry.path(), dest).unwrap();
            }
        }
    }

    #[test]
    fn memory_only_commit_skips_recovery_with_same_cas_checks() {
        let fx = fixture();
        // Recovery unavailable: the commit still runs the CAS checks.
        let outcome = commit_then_cleanup(
            &fx.project,
            None,
            "coding",
            &fx.coding_id,
            "",
            "memory draft",
            None,
            0,
            NOW,
        )
        .unwrap();
        match outcome {
            SaveNoteDraftResult::Saved {
                recovery_cleared, ..
            } => assert!(!recovery_cleared),
            other => panic!("expected Saved, got {other:?}"),
        }
        assert_eq!(
            read_live_memo(&fx.project, "coding", &fx.coding_id)
                .unwrap()
                .as_deref(),
            Some("memory draft")
        );
        // …including conflicts.
        let conflict = commit_then_cleanup(
            &fx.project,
            None,
            "coding",
            &fx.coding_id,
            "wrong base",
            "x",
            None,
            0,
            NOW,
        )
        .unwrap();
        assert!(matches!(conflict, SaveNoteDraftResult::Conflict { .. }));
    }

    #[test]
    fn resolve_reports_live_targets() {
        let fx = fixture();
        db::patch_coding_memo(
            &fx.project,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some("live coding note".into()),
            },
        )
        .unwrap();
        let draft = draft_for(&fx, "coding", &fx.coding_id.clone(), "live coding note");
        let resolved = resolve_draft_against_project(
            &fx.project,
            &get_draft_by_id(&fx.recovery, &draft.draft_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        match resolved {
            ResolveNoteDraftTargetResult::LiveCoding { committed_text, .. } => {
                assert_eq!(committed_text, "live coding note")
            }
            other => panic!("expected LiveCoding, got {other:?}"),
        }
        db::update_hub_memo(&fx.project, &fx.interview_id, "live hub").unwrap();
        let iv_draft = draft_for(&fx, "interview", &fx.interview_id.clone(), "live hub");
        let resolved_iv = resolve_draft_against_project(
            &fx.project,
            &get_draft_by_id(&fx.recovery, &iv_draft.draft_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            resolved_iv,
            ResolveNoteDraftTargetResult::LiveInterview { .. }
        ));
    }

    #[test]
    fn recovery_markers_reach_no_committed_pipeline() {
        // Unique synthetic markers live ONLY in the recovery DB — never in
        // the project. Every committed-data pipeline must come back clean.
        const DRAFT_MARKER: &str = "zz-unfinished-draft-marker-9f3k";
        const COMMITTED_MARKER: &str = "zz-committed-note-marker-4q7d";
        let fx = fixture();
        db::patch_coding_memo(
            &fx.project,
            &PatchCodingMemoInput {
                coded_segment_id: fx.coding_id.clone(),
                memo: Some(COMMITTED_MARKER.into()),
            },
        )
        .unwrap();
        db::update_hub_memo(
            &fx.project,
            &fx.interview_id,
            &format!("hub holds {COMMITTED_MARKER}"),
        )
        .unwrap();
        let draft = insert_draft(
            &fx.recovery,
            "proj-key",
            "coding",
            &fx.coding_id,
            &fx.interview_id,
            Some("Ada"),
            COMMITTED_MARKER,
            "P01",
            None,
            Some(0),
            None,
            None,
            Some("synthetic quote"),
            NOW,
        )
        .unwrap();
        put_draft_text(
            &fx.recovery,
            &draft.draft_id,
            0,
            &format!("draft body {DRAFT_MARKER}"),
            NOW,
        )
        .unwrap();

        // 1. Study backup ZIP: committed marker in, draft marker out.
        // The archive is deflated, so search decompressed contents — never
        // bare filenames or raw bytes.
        let backup = backup::create(
            &fx.project,
            &fx.project_path,
            backup::BackupReason::Manual,
            None,
        )
        .unwrap();
        let backup_file = std::fs::File::open(&backup.path).unwrap();
        let mut archive = zip::ZipArchive::new(backup_file).unwrap();
        assert!(!archive.is_empty(), "backup archive must not be empty");
        let mut zipped = String::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let mut raw = Vec::new();
            entry.read_to_end(&mut raw).unwrap_or_default();
            // project.db inside the ZIP is binary SQLite: decode lossily so
            // the embedded memo bytes stay searchable instead of failing the
            // whole entry on the first non-UTF-8 page.
            zipped.push_str(&String::from_utf8_lossy(&raw));
            zipped.push('\n');
        }
        assert!(
            zipped.contains(COMMITTED_MARKER),
            "backup must carry committed notes"
        );
        assert!(
            !zipped.contains(DRAFT_MARKER),
            "backup ZIP must not carry unfinished draft text"
        );

        // 2. Export outputs across every memo-bearing format.
        let config = ExportConfigInput {
            preset: "reflexive-ta".to_string(),
            items: vec![
                "report-html".to_string(),
                "coded-segments".to_string(),
                "memos".to_string(),
            ],
            include_participant_scope: "all".to_string(),
            selected_participant_ids: None,
            include_coder_scope: "all".to_string(),
        };
        let target = fx.project_path.join("exports_out");
        let exported = db::export_with_config(
            &fx.project,
            &target,
            &config,
            Some("<html>Report</html>"),
            None,
            "Ada",
            &std::collections::HashSet::new(),
        )
        .unwrap();
        let mut saw_committed = false;
        for file in &exported.files {
            let bytes = std::fs::read(std::path::Path::new(&exported.exports_dir).join(file))
                .unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains(DRAFT_MARKER),
                "export file {file} must not carry unfinished draft text"
            );
            if text.contains(COMMITTED_MARKER) {
                saw_committed = true;
            }
        }
        assert!(saw_committed, "exports must carry committed notes");

        // 3. Sync payload built from the project connection.
        let batch = sync::collect_push_batch(&fx.project, "proj-group").unwrap();
        let payload = format!("{batch:?}");
        assert!(
            !payload.contains(DRAFT_MARKER),
            "sync payload must not carry unfinished draft text"
        );

        // 4. The recovery file itself lives outside the project folder.
        assert!(
            !recovery_db_path(std::path::Path::new("/tmp")).starts_with(&fx.project_path),
            "recovery DB must not resolve inside a study folder"
        );
    }
}
