use crate::app_data;
use crate::backup;
use crate::db;
use crate::models::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, State};

type CachedSchemaVersion = std::sync::Arc<Mutex<Option<(String, Option<i32>)>>>;

fn initialize_local_v2_metadata(app: &tauri::AppHandle, conn: &Connection) -> Result<(), String> {
    let device_id = app_data::sync_device_id(app)?;
    db::set_sync_state_value(conn, "sync_device_id", &device_id).map_err(|e| e.to_string())
}

fn schedule_committed_shared_mutation(app: tauri::AppHandle, state: &AppState) {
    state.request_background_sync(app, crate::sync_coordinator::SyncTrigger::LocalMutation);
}

pub struct AppState {
    pub project_path: Mutex<Option<PathBuf>>,
    pub db: Mutex<Option<Connection>>,
    /// Local-study UUID bound to the backend-resolved canonical project
    /// folder path (NOT a Supabase group id). Recovery drafts are scoped to
    /// it; clones at different paths stay separate.
    pub project_key: Mutex<Option<String>>,
    /// Workspace epoch, rotated on every open/replacement. Checked note
    /// writes carry it; a write from a superseded connection fails closed
    /// instead of landing in the wrong database.
    pub workspace_epoch: Mutex<Option<String>>,
    /// Serializes workspace replacement (open, close, restore, updater
    /// connection swap) against checked note writes. Lock order everywhere:
    /// `workspace_transition → project_path → db → recovery_db`. Checking an
    /// epoch under any other lock is not sufficient, because the replacement
    /// paths swap the connection this guard protects.
    pub(crate) workspace_transition: Mutex<()>,
    /// App-local recovery database (`note-recovery.sqlite3`). Separate from
    /// every project folder, so unfinished text never rides along in a cloud
    /// copy, backup ZIP, export or sync payload.
    pub(crate) recovery_db: Mutex<Option<Connection>>,
    /// Set once the recovery database fails to open or write: editing
    /// continues in memory with a persistent warning, never a crash and never
    /// silent loss of the recovery promise.
    pub(crate) recovery_unavailable: AtomicBool,
    pub(crate) recovery_error: Mutex<Option<String>>,
    /// Selftest-only injection: a temporary recovery root so the suites never
    /// touch (or clear) real recovery records. Refused outside selftest mode.
    pub(crate) recovery_path_override: Mutex<Option<PathBuf>>,
    /// Bumped on every acknowledged recovery write and every checked commit.
    /// Update-install approvals bind to it, so an edit landing between
    /// approval and install invalidates the approval.
    pub(crate) draft_write_seq: std::sync::atomic::AtomicU64,
    /// Native close/quit intent waiting on the frontend preflight (one-shot).
    pub(crate) pending_departure: Mutex<Option<PendingDeparture>>,
    /// One-use approval consumed when replaying an approved native action,
    /// so the replay does not prompt recursively.
    pub(crate) departure_approved: Mutex<Option<ApprovedDeparture>>,
    /// One-use updater approval minted after the draft preflight.
    pub(crate) update_departure_approval: Mutex<Option<StoredUpdateApproval>>,
    /// The live sync session for this run of the app.
    ///
    /// The access token exists only here and dies with the process. The
    /// *refresh* token is kept in the OS credential store so a sign-in survives
    /// a relaunch — see `sync::remember_session`. Plain app data was rejected
    /// for it; the keychain encrypts at rest and unlocks with the user's login.
    ///
    /// Persisting it at all is a reversal, and a considered one: while nothing
    /// was stored, background sync could not begin until somebody signed in,
    /// which turned an automatic process into a chore once per launch. A
    /// feature people must remember to switch on does not run.
    pub sync_session: Mutex<Option<crate::sync::SyncSession>>,
    pub realtime: crate::realtime::RealtimeManager,
    pub sync_coordinator: crate::sync_coordinator::SyncCoordinator,
    pub update_coordinator: crate::update_coordinator::UpdateCoordinator,
    pub cached_schema_version: CachedSchemaVersion,
    update_write_blocked: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct SyncRunTarget {
    pub(crate) generation: u64,
    pub(crate) project_path: PathBuf,
}

/// Projects that were double-clicked before the UI could receive them.
///
/// A cold launch always delivers the path first and loads the webview second,
/// so the obvious implementation — emit and hope — drops every double-click
/// that starts the app, on every platform. The path waits here instead until
/// the frontend says it is listening. See `deliver_open_project` in `lib.rs`.
pub struct PendingOpen {
    inner: Mutex<PendingOpenInner>,
}

#[derive(Default)]
struct PendingOpenInner {
    paths: Vec<String>,
    frontend_ready: bool,
}

impl PendingOpen {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PendingOpenInner::default()),
        }
    }

    /// Queue `path`, or report that the caller should emit it instead.
    ///
    /// Returns `true` when the path was queued. The readiness check and the
    /// push share one lock deliberately: split them, and a path arriving during
    /// the handover can read "not ready", then get pushed onto a queue that was
    /// already drained, where it sits for the rest of the session.
    pub fn queue_unless_ready(&self, path: String) -> bool {
        match self.inner.lock() {
            Ok(mut inner) => {
                if inner.frontend_ready {
                    return false;
                }
                inner.paths.push(path);
                true
            }
            // A poisoned lock means some other thread panicked mid-queue. Emit
            // and let the frontend miss it rather than panicking the launch.
            Err(_) => false,
        }
    }
}

/// A native close/quit event held for the frontend draft preflight.
///
/// The OS asked once; the frontend answers once. The intent survives until it
/// is approved or cancelled — a timeout auto-approving it would discard drafts
/// the user never saw, and a timeout auto-cancelling it would swallow quits.
#[derive(Debug, Clone)]
pub(crate) struct PendingDeparture {
    pub(crate) intent_id: String,
    pub(crate) kind: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ApprovedDeparture {
    pub(crate) intent_id: String,
    pub(crate) kind: String,
}

/// A minted update-install approval: one use, tied to the workspace epoch
/// and the draft-write sequence observed when the preflight passed.
#[derive(Debug, Clone)]
pub(crate) struct StoredUpdateApproval {
    pub(crate) token: String,
    pub(crate) project_key: Option<String>,
    pub(crate) epoch: Option<String>,
    pub(crate) draft_write_seq: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            db: Mutex::new(None),
            project_key: Mutex::new(None),
            workspace_epoch: Mutex::new(None),
            workspace_transition: Mutex::new(()),
            recovery_db: Mutex::new(None),
            recovery_unavailable: AtomicBool::new(false),
            recovery_error: Mutex::new(None),
            recovery_path_override: Mutex::new(None),
            draft_write_seq: std::sync::atomic::AtomicU64::new(0),
            pending_departure: Mutex::new(None),
            departure_approved: Mutex::new(None),
            update_departure_approval: Mutex::new(None),
            sync_session: Mutex::new(None),
            realtime: crate::realtime::RealtimeManager::new(),
            sync_coordinator: crate::sync_coordinator::SyncCoordinator::new(),
            update_coordinator: crate::update_coordinator::UpdateCoordinator::new(),
            cached_schema_version: std::sync::Arc::new(Mutex::new(None)),
            update_write_blocked: AtomicBool::new(false),
        }
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let guard = self.db.lock().map_err(|e| e.to_string())?;
        if self.update_write_blocked.load(Ordering::SeqCst) {
            return Err("Preparing an update—your work is saved locally.".into());
        }
        let conn = guard.as_ref().ok_or("No project is open")?;
        f(conn)
    }

    pub(crate) fn sync_run_target(&self, generation: u64) -> Result<SyncRunTarget, String> {
        self.sync_coordinator.with_active(generation, || {
            let project_path = self
                .project_path
                .lock()
                .map_err(|e| e.to_string())?
                .clone()
                .ok_or_else(|| "No project is open".to_string())?;
            Ok(SyncRunTarget {
                generation,
                project_path,
            })
        })
    }

    pub(crate) fn with_sync_target<F, T>(&self, target: &SyncRunTarget, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        self.sync_coordinator.with_active(target.generation, || {
            let current_path = self
                .project_path
                .lock()
                .map_err(|e| e.to_string())?
                .clone()
                .ok_or_else(|| "No project is open".to_string())?;
            if current_path != target.project_path {
                return Err("Sync was cancelled because the project changed.".into());
            }
            let guard = self.db.lock().map_err(|e| e.to_string())?;
            let conn = guard.as_ref().ok_or("No project is open")?;
            f(conn)
        })
    }

    /// Fold the WAL into `project.db` so the folder on disk is complete.
    ///
    /// Called on close and on app exit. Best-effort and deliberately silent:
    /// there is no open project during most of the app's life, a checkpoint can
    /// return SQLITE_BUSY for ordinary reasons, and neither case is worth
    /// blocking a quit over. Failing to checkpoint leaves the WAL in place,
    /// which is still correct locally — it only matters once the folder moves.
    pub fn checkpoint_open_project(&self) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                let _ = db::checkpoint(conn);
            }
        }
    }

    pub fn close_project_connection(&self) -> Result<(), String> {
        self.close_project_connection_for_update()?;
        let mut path_guard = self.project_path.lock().map_err(|e| e.to_string())?;
        *path_guard = None;
        Ok(())
    }

    pub(crate) fn close_project_connection_for_update(&self) -> Result<Option<PathBuf>, String> {
        // Updater connection replacement: under the transition lock, so a
        // checked note write cannot validate its epoch against a connection
        // that is being pulled out from under it.
        let _transition = self
            .workspace_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let project_path = self.project_path.lock().map_err(|e| e.to_string())?.clone();
        let mut db_guard = self.db.lock().map_err(|e| e.to_string())?;
        if let Some(conn) = db_guard.as_ref() {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        *db_guard = None;
        Ok(project_path)
    }

    pub(crate) fn reopen_project_connection_after_update_failure(&self) -> Result<(), String> {
        // Same lock as the teardown above: the reopen is part of the same
        // replacement. The epoch is deliberately NOT rotated — the same
        // database file is back, so in-flight drafts are still addressed
        // correctly and CAS still guards their content.
        let _transition = self
            .workspace_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let project_path = self
            .project_path
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("No project is open")?;
        let connection =
            db::open_project(&project_path.to_string_lossy()).map_err(|error| error.to_string())?;
        *self.db.lock().map_err(|error| error.to_string())? = Some(connection);
        Ok(())
    }

    pub(crate) fn set_update_write_blocked(&self, blocked: bool) {
        self.update_write_blocked.store(blocked, Ordering::SeqCst);
    }

    pub(crate) fn ensure_update_writable(&self) -> Result<(), String> {
        if self.update_write_blocked.load(Ordering::SeqCst) {
            return Err("Preparing an update—your work is saved locally.".into());
        }
        Ok(())
    }

    /// Take a backup of whatever project is currently open.
    ///
    /// Holds the db lock for the duration, which is what makes the snapshot
    /// coherent with respect to this process: no command can commit a write
    /// between the manifest counts being read and `VACUUM INTO` running.
    pub fn snapshot_open_project(
        &self,
        reason: backup::BackupReason,
        note: Option<String>,
    ) -> Result<backup::BackupInfo, String> {
        let path_guard = self.project_path.lock().map_err(|e| e.to_string())?;
        let path = path_guard.as_ref().ok_or("No project is open")?;
        let db_guard = self.db.lock().map_err(|e| e.to_string())?;
        let conn = db_guard.as_ref().ok_or("No project is open")?;
        backup::create(conn, path, reason, note)
    }

    pub(crate) fn restore_sync_repair_backup(&self, backup_path: &Path) -> Result<(), String> {
        let project_path = self
            .project_path
            .lock()
            .map_err(|error| error.to_string())?
            .clone()
            .ok_or("No project is open")?;
        self.checkpoint_open_project();
        // Replacement under the transition lock (see
        // close_project_connection_for_update). No epoch rotation: the repair
        // restores the same study, and CAS on note content still guards
        // in-flight drafts against the swapped rows.
        let _transition = self
            .workspace_transition
            .lock()
            .map_err(|e| e.to_string())?;
        *self.db.lock().map_err(|error| error.to_string())? = None;
        let restored = backup::restore(&project_path, backup_path);
        let reopened =
            db::open_project(&project_path.to_string_lossy()).map_err(|error| error.to_string());
        if let Ok(connection) = reopened {
            *self.db.lock().map_err(|error| error.to_string())? = Some(connection);
        }
        restored.map(|_| ())
    }

    pub fn project_path_str(&self) -> Result<String, String> {
        let guard = self.project_path.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| "No project is open".into())
    }

    // ── Workspace epoch + recovery ──────────────────────────────────────────

    /// Current workspace identity (project key + epoch), cloned under lock.
    pub(crate) fn current_workspace(&self) -> (Option<String>, Option<String>) {
        let key = self.project_key.lock().ok().and_then(|g| g.clone());
        let epoch = self.workspace_epoch.lock().ok().and_then(|g| g.clone());
        (key, epoch)
    }

    /// Validate a checked-write credential pair. Must be called with the
    /// workspace-transition lock held (the caller holds it for the whole
    /// checked operation), so a replacement path cannot swap the connection
    /// between this check and the write it guards.
    pub(crate) fn check_workspace_under_transition(
        &self,
        project_key: &str,
        epoch: &str,
    ) -> Result<(), String> {
        let (key, current_epoch) = self.current_workspace();
        match (key, current_epoch) {
            (Some(k), Some(e)) if k == project_key && e == epoch => Ok(()),
            _ => Err("STALE_WORKSPACE".into()),
        }
    }

    /// Bind a freshly swapped-in connection to a project key + epoch.
    /// Call AFTER the swap, holding the transition lock (taken here), so no
    /// checked write can observe a new connection with an old epoch.
    pub(crate) fn rotate_workspace(
        &self,
        app: &tauri::AppHandle,
        project_path: &Path,
        title: &str,
    ) -> Result<(String, String), String> {
        let _transition = self
            .workspace_transition
            .lock()
            .map_err(|e| e.to_string())?;
        // Canonicalize through filesystem resolution on the backend; do not
        // lowercase (case-sensitive paths) and do not trust the frontend path.
        let canonical = project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());
        let canonical_str = canonical.to_string_lossy().to_string();
        let key = self.with_recovery(app, |recovery| {
            crate::note_recovery::register_project(recovery, &canonical_str, title)
        })?;
        let epoch = uuid::Uuid::new_v4().to_string();
        *self.project_key.lock().map_err(|e| e.to_string())? = Some(key.clone());
        *self.workspace_epoch.lock().map_err(|e| e.to_string())? = Some(epoch.clone());
        Ok((key, epoch))
    }

    /// Unbind the workspace (close). Checked writes fail closed afterwards.
    /// Takes the transition lock so a checked write cannot interleave with
    /// the teardown it is validated against.
    pub(crate) fn clear_workspace_binding(&self) {
        // Order: transition first, per the module lock discipline.
        if let Ok(_transition) = self.workspace_transition.lock() {
            if let Ok(mut key) = self.project_key.lock() {
                *key = None;
            }
            if let Ok(mut epoch) = self.workspace_epoch.lock() {
                *epoch = None;
            }
        }
    }

    /// Recovery database path: the selftest override wins when set, so the
    /// suites operate on a temporary root and can never clear real records.
    pub(crate) fn recovery_db_path_for(&self, app: &tauri::AppHandle) -> Result<PathBuf, String> {
        if let Ok(guard) = self.recovery_path_override.lock() {
            if let Some(path) = guard.as_ref() {
                return Ok(path.join(crate::note_recovery::RECOVERY_DB_FILENAME));
            }
        }
        Ok(crate::note_recovery::recovery_db_path(
            &crate::app_data::app_data_dir(app)?,
        ))
    }

    /// Open the recovery database on first use and hand out scoped access.
    /// A failure marks recovery unavailable (sticky, with the message kept
    /// for the UI's Retry surface) and returns Err — but never panics, never
    /// deletes an unreadable file, and never blocks editing itself.
    pub(crate) fn with_recovery<F, T>(&self, app: &tauri::AppHandle, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let mut guard = self.recovery_db.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let path = self.recovery_db_path_for(app)?;
            match crate::note_recovery::open_recovery_db(&path) {
                Ok(conn) => {
                    *guard = Some(conn);
                    self.recovery_unavailable.store(false, Ordering::SeqCst);
                    if let Ok(mut err) = self.recovery_error.lock() {
                        *err = None;
                    }
                }
                Err(error) => {
                    self.recovery_unavailable.store(true, Ordering::SeqCst);
                    if let Ok(mut err) = self.recovery_error.lock() {
                        *err = Some(error.clone());
                    }
                    return Err(format!(
                        "Draft recovery is unavailable ({error}). Editing still works in memory; retry to restore local backup."
                    ));
                }
            }
        }
        let conn = guard.as_ref().ok_or_else(|| {
            "Draft recovery is unavailable. Editing still works in memory.".to_string()
        })?;
        match f(conn) {
            Ok(value) => Ok(value),
            Err(error) => {
                if is_recovery_io_failure(&error) {
                    self.recovery_unavailable.store(true, Ordering::SeqCst);
                    if let Ok(mut slot) = self.recovery_error.lock() {
                        *slot = Some(error.clone());
                    }
                    // Drop the poisoned connection so the next call reopens.
                    *guard = None;
                }
                Err(error)
            }
        }
    }

    pub(crate) fn bump_draft_seq(&self) {
        self.draft_write_seq.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn draft_seq(&self) -> u64 {
        self.draft_write_seq.load(Ordering::SeqCst)
    }

    pub(crate) fn recovery_status(&self) -> (bool, Option<String>) {
        let unavailable = self.recovery_unavailable.load(Ordering::SeqCst);
        let error = self.recovery_error.lock().ok().and_then(|g| g.clone());
        (unavailable, error)
    }

    // ── Native departure intents ────────────────────────────────────────────

    /// Hold a native close/quit event for the frontend preflight. Returns the
    /// one-use intent id the frontend must answer via `complete_note_departure`.
    /// A second competing native intent while one is pending reuses the
    /// pending id: two prompts for one decision is how answers get attached
    /// to the wrong action.
    pub(crate) fn request_native_departure(&self, kind: &str) -> String {
        if let Ok(guard) = self.pending_departure.lock() {
            if let Some(pending) = guard.as_ref() {
                if pending.kind == kind {
                    return pending.intent_id.clone();
                }
            }
        }
        let intent_id = uuid::Uuid::new_v4().to_string();
        if let Ok(mut guard) = self.pending_departure.lock() {
            *guard = Some(PendingDeparture {
                intent_id: intent_id.clone(),
                kind: kind.to_string(),
            });
        }
        intent_id
    }

    /// Consume a replay approval matching the expected action kind (close vs quit),
    /// ensuring a window close cannot approve process exit.
    pub(crate) fn take_departure_approval(&self, expected_kind: &str) -> Option<String> {
        let mut guard = self.departure_approved.lock().ok()?;
        if let Some(ref approved) = *guard {
            if approved.kind == expected_kind {
                return guard.take().map(|a| a.intent_id);
            }
        }
        None
    }

    /// Backwards-compatible consumption for tests.
    #[allow(dead_code)]
    pub(crate) fn take_any_departure_approval(&self) -> Option<String> {
        self.departure_approved
            .lock()
            .ok()
            .and_then(|mut guard| guard.take().map(|a| a.intent_id))
    }

    /// Clear any pending departure intent and approved departure.
    #[allow(dead_code)]
    pub(crate) fn clear_departure_state(&self) {
        if let Ok(mut g) = self.pending_departure.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.departure_approved.lock() {
            *g = None;
        }
    }
}

/// True when a recovery-layer error means the database itself is unusable
/// (as opposed to a content/protocol outcome like a stale generation, which
/// says nothing about the health of the store). Only IO failures poison the
/// cached connection and raise the persistent `Draft recovery unavailable`
/// surface; protocol outcomes leave recovery running.
fn is_recovery_io_failure(error: &str) -> bool {
    let lower = error.to_lowercase();
    [
        "disk",
        "i/o",
        "locked",
        "corrupt",
        "unable to open",
        "readonly",
        "read-only",
        "read only",
        "permission",
        "sqlite",
        "database is",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[tauri::command]
pub fn create_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> Result<ProjectInfo, String> {
    state.ensure_update_writable()?;
    let path = db::create_project(&input).map_err(|e| {
        crate::file_error::classified(
            "Could not create the study folder. Check that you have permission to write to this location.",
            &e.to_string(),
        )
    })?;
    let conn = db::open_project(&path.to_string_lossy()).map_err(|e| {
        crate::file_error::classified(
            "The study was created but could not be opened. Try reopening it from Recents.",
            &e.to_string(),
        )
    })?;

    *state.project_path.lock().map_err(|e| e.to_string())? = Some(path.clone());
    *state.db.lock().map_err(|e| e.to_string())? = Some(conn);

    let info = state.with_conn(|conn| {
        initialize_local_v2_metadata(&app, conn)?;
        db::get_project_info(conn, &path.to_string_lossy()).map_err(|e| e.to_string())
    })?;
    // A fresh connection means a fresh epoch: any checked write still
    // carrying the previous study's credential fails closed.
    let _ = state.rotate_workspace(&app, &path, &info.title);

    let path_str = path.to_string_lossy().to_string();
    let _ = app_data::record_recent_project(
        &app,
        RecordRecentProjectInput {
            path: path_str,
            title: info.title.clone(),
            group_id: None,
            group_title: None,
            coder_name: None,
            former_group_id: None,
            former_group_title: None,
            readiness: None,
        },
    );
    Ok(info)
}

#[tauri::command]
pub async fn open_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectOpenSnapshot, String> {
    state.ensure_update_writable()?;
    let path_clone = path.clone();
    let (conn, mut snapshot) =
        tauri::async_runtime::spawn_blocking(move || db::open_project_snapshot_inner(&path_clone))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| {
                crate::file_error::classified(
                    "Could not open this study. It may have moved, be unavailable, or access may be restricted.",
                    &e.to_string(),
                )
            })?;

    // Validate candidate database before swapping state
    let is_bound = sync::is_bound(&conn).unwrap_or(false);
    let group_id = sync::get_state(&conn, sync::KEY_PROJECT_ID).ok().flatten();
    if is_bound {
        if let Some(ref gid) = group_id {
            if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
                for r in recents {
                    if r.group_id.as_ref() == Some(gid)
                        && r.path != path
                        && Path::new(&r.path).exists()
                    {
                        return Err(format!("TWO_FOLDERS_ONE_GROUP|{}|{}", r.path, gid));
                    }
                }
            }
        }
    }

    // Stop and drain previous project sync before replacing connection
    state.sync_coordinator.cancel();
    state.realtime.stop(&app);
    state.checkpoint_open_project();

    *state.project_path.lock().map_err(|e| e.to_string())? = Some(PathBuf::from(&path));
    *state.db.lock().map_err(|e| e.to_string())? = Some(conn);

    state.with_conn(|conn| initialize_local_v2_metadata(&app, conn))?;
    // Bind the swapped-in connection to a fresh project key + epoch BEFORE
    // any checked write can run against it. Rotation takes the transition
    // lock, so a write validating the old epoch cannot slip between the swap
    // and the rotation.
    match state.rotate_workspace(&app, Path::new(&path), &snapshot.project.title) {
        Ok((project_key, epoch)) => {
            snapshot.project_key = Some(project_key);
            snapshot.workspace_epoch = Some(epoch);
        }
        Err(error) => {
            // Recovery registration failing must not block opening the study:
            // editing continues in memory with the persistent warning raised
            // by with_recovery on first draft use.
            let _ = error;
        }
    }

    // Task 11: Write open marker
    let marker_path = PathBuf::from(&path);
    let coder_name = snapshot
        .workspace
        .active_coder
        .clone()
        .unwrap_or_else(|| "Me".into());
    let machine_name = crate::open_marker::get_machine_name();
    let _ = crate::open_marker::write_marker(&marker_path, &machine_name, &coder_name);

    state.maybe_start_realtime(&app);
    state.request_background_sync(
        app.clone(),
        crate::sync_coordinator::SyncTrigger::ProjectOpened,
    );

    let backup_path = PathBuf::from(&path);
    std::thread::spawn(move || {
        if let Ok(conn) = db::open_project(&backup_path.to_string_lossy()) {
            let _ = backup::create(
                &conn,
                Path::new(&backup_path),
                backup::BackupReason::Automatic,
                None,
            );
        }
    });

    Ok(snapshot)
}

#[tauri::command]
pub fn close_project(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.sync_coordinator.cancel();
    state.realtime.stop(&app);
    state.checkpoint_open_project();
    if let Ok(p) = state.project_path_str() {
        crate::open_marker::remove_marker(Path::new(&p));
    }
    // Unbind first (transition lock): checked writes fail closed from here,
    // and the teardown below cannot interleave with one mid-validation.
    state.clear_workspace_binding();
    *state.project_path.lock().map_err(|e| e.to_string())? = None;
    *state.db.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

// ── Backups ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn create_backup(
    state: State<'_, AppState>,
    note: Option<String>,
) -> Result<backup::BackupInfo, String> {
    let note = note.filter(|n| !n.trim().is_empty());
    state.snapshot_open_project(backup::BackupReason::Manual, note)
}

#[tauri::command]
pub fn list_backups(state: State<'_, AppState>) -> Result<Vec<backup::BackupInfo>, String> {
    let path = state.project_path_str()?;
    Ok(backup::list(std::path::Path::new(&path)))
}

/// Replace the open project with the contents of a backup.
///
/// The ordering here is the whole point, and it is not the obvious one. The
/// connection must be **dropped before** the file swap: Windows will not rename
/// over an open handle at all, and macOS would happily let the rename succeed
/// while this process kept reading the old inode — a restore that silently does
/// nothing until the next launch. So the project is closed, restored, and
/// reopened, and the frontend reloads everything from the reopened connection.
///
/// If the swap fails the project is reopened regardless, so a failed restore
/// leaves the app usable rather than holding a closed project it will not
/// explain.
#[tauri::command]
pub fn restore_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    backup_path: String,
) -> Result<backup::RestoreOutcome, String> {
    let project_path = PathBuf::from(state.project_path_str()?);
    let archive = PathBuf::from(&backup_path);

    // Fold the WAL in before letting go: the pre-restore safety snapshot opens
    // its own connection to project.db, and an uncheckpointed WAL would make
    // that snapshot miss the most recent coding — exactly the work most worth
    // keeping when somebody is about to overwrite it.
    state.checkpoint_open_project();
    // Replacement: drop the connection under the transition lock so checked
    // writes cannot validate against the pre-restore database.
    {
        let _transition = state
            .workspace_transition
            .lock()
            .map_err(|e| e.to_string())?;
        *state.db.lock().map_err(|e| e.to_string())? = None;
    }

    let outcome = backup::restore(&project_path, &archive);

    let reopened = db::open_project(&project_path.to_string_lossy()).map_err(|e| e.to_string());
    match reopened {
        Ok(conn) => *state.db.lock().map_err(|e| e.to_string())? = Some(conn),
        Err(e) => {
            return Err(match outcome {
                Ok(_) => format!(
                    "The backup was restored, but the project could not be reopened: {e}. \
                     Close and reopen it from the home screen."
                ),
                Err(restore_err) => {
                    format!("{restore_err} The project could not be reopened: {e}.")
                }
            })
        }
    }
    // The restored database replaces every row: rotate the epoch so in-flight
    // drafts re-resolve against the restored content instead of writing
    // against pre-restore base text. The recovery DB itself is untouched.
    if let Ok(info) = state.with_conn(|conn| {
        db::get_project_info(conn, &project_path.to_string_lossy()).map_err(|e| e.to_string())
    }) {
        let _ = state.rotate_workspace(&app, &project_path, &info.title);
    }

    outcome
}

#[tauri::command]
pub fn delete_backup(state: State<'_, AppState>, backup_path: String) -> Result<(), String> {
    // Scoped to this project's own backups folder. Without the check this
    // command would delete any `.fleuronbak` on the disk that the frontend
    // named, and the frontend is not where that authority belongs.
    let project_path = PathBuf::from(state.project_path_str()?);
    let archive = PathBuf::from(&backup_path);
    let expected = backup::backups_dir(&project_path);
    if archive.parent() != Some(expected.as_path()) {
        return Err("That backup does not belong to this project.".into());
    }
    backup::delete(&archive)
}

/// Read a backup file from anywhere on disk, so one can be restored after
/// being moved off the machine and back.
#[tauri::command]
pub fn inspect_backup(backup_path: String) -> Result<backup::BackupInfo, String> {
    backup::inspect(std::path::Path::new(&backup_path))
}

/// Copy an external backup file into this project's backups folder.
///
/// Restoring reads only from the project's own folder, so an archive that was
/// emailed or pulled off a USB stick has to land there first. Doing the copy
/// here rather than letting restore take an arbitrary path keeps one rule true:
/// everything this project can restore from is something it can also list.
#[tauri::command]
pub fn import_backup(
    state: State<'_, AppState>,
    source_path: String,
) -> Result<backup::BackupInfo, String> {
    let project_path = PathBuf::from(state.project_path_str()?);
    backup::import(&project_path, std::path::Path::new(&source_path))
}

/// Save a local verified recovery archive of a study outside its project directory.
#[tauri::command]
pub fn save_local_copy(
    state: State<'_, AppState>,
    source_path: String,
    destination_dir: String,
    include_external_media: bool,
) -> Result<backup::BackupInfo, String> {
    let source = Path::new(&source_path);
    let dest = Path::new(&destination_dir);

    let recovery_guard = state.recovery_db.lock().ok();
    let recovery_conn = recovery_guard.as_ref().and_then(|g| g.as_ref());

    let is_current = if let Ok(current) = state.project_path_str() {
        let current_path = Path::new(&current);
        current_path == source
            || current_path
                .canonicalize()
                .ok()
                .zip(source.canonicalize().ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false)
    } else {
        false
    };

    if is_current {
        state.with_conn(|conn| {
            backup::save_local_copy(conn, source, dest, include_external_media, recovery_conn)
        })
    } else {
        let db_path = source.join("project.db");
        if !db_path.exists() {
            return Err("Not a valid Fleuron project database".into());
        }
        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        backup::save_local_copy(&conn, source, dest, include_external_media, recovery_conn)
    }
}

/// Restore a study backup to a newly allocated project folder.
#[tauri::command]
pub fn restore_study_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    archive_path: String,
    parent_dir: Option<String>,
    target_title: Option<String>,
) -> Result<String, String> {
    let default_parent = if let Some(p) = parent_dir {
        PathBuf::from(p)
    } else {
        crate::app_data::projects_library_dir(&app).unwrap_or_else(|_| PathBuf::from("."))
    };

    let recovery_guard = state.recovery_db.lock().ok();
    let recovery_conn = recovery_guard.as_ref().and_then(|g| g.as_ref());

    let restored_path = backup::restore_backup_to_new_project(
        Path::new(&archive_path),
        &default_parent,
        target_title.as_deref(),
        recovery_conn,
    )?;

    Ok(restored_path.to_string_lossy().to_string())
}

/// Coding pulled from a colleague that is still waiting for its transcript.
///
/// Zero for anyone who imported before syncing. Non-zero is the joining
/// coder's normal state, and worth saying out loud — otherwise the app shows an
/// empty transcript and gives no sign that the work is already on the machine.
#[tauri::command]
pub fn pending_coded_count(state: State<'_, AppState>) -> Result<i64, String> {
    state.with_conn(|conn| db::pending_coded_count(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn get_project_info(state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    let path = state.project_path_str()?;
    state.with_conn(|conn| db::get_project_info(conn, &path).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn get_live_workspace_snapshot(
    state: State<'_, AppState>,
    active_interview_id: Option<String>,
) -> Result<LiveWorkspaceSnapshot, String> {
    let path = state.project_path_str()?;
    let mut snapshot = state.with_conn(|conn| {
        db::get_live_workspace_snapshot(conn, &path, active_interview_id.as_deref())
            .map_err(|e| e.to_string())
    })?;
    let (project_key, epoch) = state.current_workspace();
    snapshot.project_key = project_key;
    snapshot.workspace_epoch = epoch;
    Ok(snapshot)
}

#[tauri::command]
pub fn list_sync_conflicts(state: State<'_, AppState>) -> Result<Vec<SyncConflictDetail>, String> {
    state.with_conn(|conn| db::list_sync_conflict_details(conn).map_err(|error| error.to_string()))
}

/// Align this folder with the name the group knows you by.
///
/// `from` is the local name currently in use. Coded rows under it are rewritten
/// so one person does not split into two identities, and the coder list is
/// collapsed to the group name.
#[tauri::command]
pub fn adopt_project_coder(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<ProjectInfo, String> {
    let to = to.trim().to_string();
    if to.is_empty() {
        return Err("Name is required.".into());
    }
    let path = state.project_path_str()?;
    state.with_conn(|conn| {
        db::rename_project_coder(conn, from.trim(), &to).map_err(|e| e.to_string())?;
        let mut ws = db::get_workspace_state(conn).map_err(|e| e.to_string())?;
        ws.active_coder = Some(to.clone());
        db::save_workspace_state(conn, &ws).map_err(|e| e.to_string())?;
        db::get_project_info(conn, &path).map_err(|e| e.to_string())
    })
}

/// Align this local project with the group title from the server.
#[tauri::command]
pub fn adopt_project_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    title: String,
) -> Result<ProjectInfo, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Title is required.".into());
    }
    let path = state.project_path_str()?;
    let project = state.with_conn(|conn| {
        db::set_meta(conn, "title", &title).map_err(|e| e.to_string())?;
        db::get_project_info(conn, &path).map_err(|e| e.to_string())
    })?;
    let group_id = state
        .with_conn(|conn| db::get_meta(conn, "group_id").map_err(|e| e.to_string()))
        .unwrap_or(None);
    let coder_name = state
        .with_conn(|conn| db::get_workspace_state(conn).map_err(|e| e.to_string()))
        .map(|ws| ws.active_coder)
        .unwrap_or(None);
    let (former_group_id, former_group_title) = state
        .with_conn(|conn| {
            let id = sync::get_state(conn, sync::KEY_FORMER_GROUP_ID).map_err(|e| e.to_string())?;
            let title =
                sync::get_state(conn, sync::KEY_FORMER_GROUP_TITLE).map_err(|e| e.to_string())?;
            Ok((id, title))
        })
        .unwrap_or((None, None));
    let _ = app_data::record_recent_project(
        &app,
        RecordRecentProjectInput {
            path,
            title: title.clone(),
            group_id,
            group_title: Some(title),
            coder_name,
            former_group_id,
            former_group_title,
            readiness: None,
        },
    );
    Ok(project)
}

#[tauri::command]
pub fn list_activity(state: State<'_, AppState>) -> Result<Vec<ActivityLogEntry>, String> {
    state.with_conn(|conn| db::list_activity(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn list_codes(state: State<'_, AppState>) -> Result<Vec<Code>, String> {
    state.with_conn(|conn| db::list_codes(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn list_retired_codes(state: State<'_, AppState>) -> Result<Vec<Code>, String> {
    state.with_conn(|conn| db::list_retired_codes(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn create_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CreateCodeInput,
) -> Result<Code, String> {
    let code = state.with_conn(|conn| db::create_code(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(code)
}

#[tauri::command]
pub fn ensure_code_and_apply(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: EnsureCodeAndApplyInput,
) -> Result<EnsureCodeAndApplyResult, String> {
    let result = state
        .with_conn(|conn| db::ensure_code_and_apply(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(result)
}

#[tauri::command]
pub fn update_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: UpdateCodeInput,
) -> Result<Code, String> {
    let code = state.with_conn(|conn| db::update_code(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(code)
}

#[tauri::command]
pub fn delete_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    code_id: String,
    mode: DeleteCodeMode,
    coder_name: String,
) -> Result<DeleteCodeResult, String> {
    let result = state.with_conn(|conn| {
        let result = db::delete_code(conn, &code_id, mode).map_err(|e| e.to_string())?;
        // Codebook changes are analytic decisions, not housekeeping — a
        // teammate reading the log should see that a code stopped existing.
        let detail = match result.mode {
            DeleteCodeMode::Retire => format!("retired code \"{}\"", result.code_name),
            DeleteCodeMode::Purge => format!(
                "deleted code \"{}\" from {} passage(s)",
                result.code_name, result.segments_updated
            ),
        };
        let _ = db::log_activity(conn, &coder_name, "codebook", Some(&detail));
        Ok(result)
    })?;
    schedule_committed_shared_mutation(app, &state);
    Ok(result)
}

#[tauri::command]
pub fn restore_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    code_id: String,
) -> Result<(), String> {
    state.with_conn(|conn| db::restore_code(conn, &code_id).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(())
}

#[tauri::command]
pub fn list_interviews(state: State<'_, AppState>) -> Result<Vec<Interview>, String> {
    state.with_conn(|conn| db::list_interviews(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn create_interview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CreateInterviewInput,
) -> Result<Interview, String> {
    let interview =
        state.with_conn(|conn| db::create_interview(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(interview)
}

#[tauri::command]
pub fn update_interview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: UpdateInterviewInput,
) -> Result<Interview, String> {
    let interview =
        state.with_conn(|conn| db::update_interview(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(interview)
}

#[tauri::command]
pub fn interview_delete_impact(
    state: State<'_, AppState>,
    interview_id: String,
) -> Result<InterviewDeleteImpact, String> {
    state.with_conn(|conn| {
        db::interview_delete_impact(conn, &interview_id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn delete_interview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    interview_id: String,
    coder_name: String,
) -> Result<(), String> {
    state.with_conn(|conn| {
        let impact = db::interview_delete_impact(conn, &interview_id).map_err(|e| e.to_string())?;
        db::delete_interview(conn, &interview_id).map_err(|e| e.to_string())?;
        let detail = format!(
            "deleted interview \"{}\" ({} segments, {} coded)",
            impact.participant_label, impact.segment_count, impact.coded_segment_count
        );
        let _ = db::log_activity(conn, &coder_name, "interview", Some(&detail));
        Ok(())
    })?;
    schedule_committed_shared_mutation(app, &state);
    Ok(())
}

#[tauri::command]
pub fn import_segments(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ImportSegmentsInput,
) -> Result<i32, String> {
    let imported =
        state.with_conn(|conn| db::import_segments(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(imported)
}

#[tauri::command]
pub fn get_segments(
    state: State<'_, AppState>,
    interview_id: String,
) -> Result<Vec<TranscriptSegment>, String> {
    state.with_conn(|conn| db::get_segments(conn, &interview_id).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn set_segment_speaker(
    state: State<'_, AppState>,
    segment_id: String,
    new_speaker: String,
    include_following: bool,
) -> Result<Vec<SegmentSpeakerChange>, String> {
    state.with_conn(|conn| {
        db::set_segment_speaker(conn, &segment_id, &new_speaker, include_following)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn restore_segment_speakers(
    state: State<'_, AppState>,
    changes: Vec<SegmentSpeakerChange>,
) -> Result<(), String> {
    state.with_conn(|conn| db::restore_segment_speakers(conn, &changes).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn get_interview_speakers(
    state: State<'_, AppState>,
    interview_id: String,
) -> Result<Vec<crate::models::InterviewSpeakerSummary>, String> {
    state.with_conn(|conn| {
        db::get_interview_speakers(conn, &interview_id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn rename_interview_speaker(
    state: State<'_, AppState>,
    input: crate::models::RenameInterviewSpeakerInput,
) -> Result<Vec<SegmentSpeakerChange>, String> {
    let _transition = state
        .workspace_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if let (Some(k), Some(e)) = (&input.project_key, &input.epoch) {
        state.check_workspace_under_transition(k, e)?;
    }
    state.with_conn(|conn| {
        db::rename_interview_speaker(
            conn,
            &input.interview_id,
            &input.old_speaker,
            &input.new_speaker,
            input.expected_count,
        )
        .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn undo_rename_interview_speaker(
    state: State<'_, AppState>,
    input: crate::models::UndoRenameInterviewSpeakerInput,
) -> Result<(), String> {
    let _transition = state
        .workspace_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if let (Some(k), Some(e)) = (&input.project_key, &input.epoch) {
        state.check_workspace_under_transition(k, e)?;
    }
    state.with_conn(|conn| {
        db::restore_segment_speakers(conn, &input.changes).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn set_segment_reviewed(
    state: State<'_, AppState>,
    segment_id: String,
    reviewed: bool,
) -> Result<(), String> {
    state.with_conn(|conn| {
        db::set_segment_reviewed(conn, &segment_id, reviewed).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn list_segment_reviews(
    state: State<'_, AppState>,
    interview_id: String,
) -> Result<Vec<String>, String> {
    state.with_conn(|conn| db::list_segment_reviews(conn, &interview_id).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn apply_codes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ApplyCodesInput,
) -> Result<CodedSegment, String> {
    let coding =
        state.with_conn(|conn| db::apply_codes(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(coding)
}

#[tauri::command]
pub fn mutate_coding_edge(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: MutateCodingEdgeInput,
) -> Result<MutateCodingEdgeResult, String> {
    let result =
        state.with_conn(|conn| db::mutate_coding_edge(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(result)
}

#[tauri::command]
pub fn patch_coding_memo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: PatchCodingMemoInput,
) -> Result<CodedSegment, String> {
    let coding =
        state.with_conn(|conn| db::patch_coding_memo(conn, &input).map_err(|e| e.to_string()))?;
    schedule_committed_shared_mutation(app, &state);
    Ok(coding)
}

#[tauri::command]
pub fn delete_coded_segment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    coded_segment_id: String,
) -> Result<(), String> {
    state.with_conn(|conn| {
        db::delete_coded_segment(conn, &coded_segment_id).map_err(|e| e.to_string())
    })?;
    schedule_committed_shared_mutation(app, &state);
    Ok(())
}

#[tauri::command]
pub fn list_coded_segments(
    state: State<'_, AppState>,
    interview_id: Option<String>,
    code_id: Option<String>,
) -> Result<Vec<CodedSegment>, String> {
    state.with_conn(|conn| {
        db::list_coded_segments(conn, interview_id.as_deref(), code_id.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn update_hub_memo(
    state: State<'_, AppState>,
    interview_id: String,
    memo: String,
) -> Result<(), String> {
    state.with_conn(|conn| {
        db::update_hub_memo(conn, &interview_id, &memo).map_err(|e| e.to_string())
    })
}

// ── Note drafts: local recovery + checked writes ────────────────────────────
//
// Every checked write validates (project_key, epoch) under the
// workspace-transition lock before touching either database, and the project
// commit runs inside one CAS transaction. Recovery cleanup is conditional on
// the acknowledged revision: commit the note first, then clear recovery.

#[tauri::command]
pub fn note_recovery_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<RecoveryStatus, String> {
    // A status probe that initializes recovery when it has never run: the
    // first paint of the Unfinished notes badge must reflect reality, and a
    // lazy open here is indistinguishable from one on first draft use.
    let probe = state.with_recovery(&app, |conn| {
        crate::note_recovery::list_active_drafts(conn).map(|_| ())
    });
    let (unavailable, error) = state.recovery_status();
    Ok(match probe {
        Ok(()) => RecoveryStatus {
            available: !unavailable,
            error,
        },
        Err(io_error) => RecoveryStatus {
            available: false,
            error: Some(io_error),
        },
    })
}

#[tauri::command]
pub fn list_note_drafts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<NoteDraftRecord>, String> {
    state.with_recovery(&app, crate::note_recovery::list_active_drafts)
}

#[tauri::command]
pub fn begin_note_draft(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: BeginNoteDraftInput,
) -> Result<BeginNoteDraftResult, String> {
    crate::note_recovery::validate_kind(&input.kind)?;
    // Hold the transition lock across validation + begin so the connection
    // this epoch names cannot be replaced underneath us.
    let _transition = state
        .workspace_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state
        .check_workspace_under_transition(&input.project_key, &input.epoch)
        .is_err()
    {
        return Ok(BeginNoteDraftResult::StaleWorkspace);
    }
    let live = state.with_conn(|conn| {
        crate::note_recovery::read_live_memo(conn, &input.kind, &input.target_id)
    })?;
    let committed_text = match live {
        Some(text) => text,
        None => return Ok(BeginNoteDraftResult::MissingTarget),
    };
    let now = db::now_iso();
    let record = state.with_recovery(&app, |recovery| {
        crate::note_recovery::begin_draft_for_target(
            recovery,
            &input.project_key,
            &input.kind,
            &input.target_id,
            &input.interview_id,
            input.coder_name.as_deref(),
            &committed_text,
            &input.participant_label,
            input.segment_id.as_deref(),
            input.segment_index,
            input.char_start,
            input.char_end,
            input.quote_text.as_deref(),
            &now,
        )
    })?;
    state.bump_draft_seq();
    Ok(BeginNoteDraftResult::Active {
        record: Box::new(record),
        committed_text,
    })
}

#[tauri::command]
pub fn put_note_draft(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: PutNoteDraftInput,
) -> Result<PutNoteDraftResult, String> {
    let _transition = state
        .workspace_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state
        .check_workspace_under_transition(&input.project_key, &input.epoch)
        .is_err()
    {
        return Ok(PutNoteDraftResult::StaleWorkspace);
    }
    let now = db::now_iso();
    let stored = state.with_recovery(&app, |recovery| {
        crate::note_recovery::put_draft_text(
            recovery,
            &input.draft_id,
            input.expected_revision,
            &input.draft_text,
            &now,
        )
    });
    match stored {
        Ok(record) => {
            state.bump_draft_seq();
            Ok(PutNoteDraftResult::Stored { record })
        }
        Err(error) if error == "STALE_GENERATION" => Ok(PutNoteDraftResult::StaleGeneration),
        Err(error) if error == "REVISION_MISMATCH" => {
            // Report the winning revision so the caller replays the latest
            // snapshot instead of dropping the newer text.
            let current = state
                .with_recovery(&app, |recovery| {
                    crate::note_recovery::get_draft_by_id(recovery, &input.draft_id)?
                        .ok_or_else(|| "STALE_GENERATION".to_string())
                })
                .map_err(|_| "REVISION_MISMATCH".to_string());
            match current {
                Ok(record) => Ok(PutNoteDraftResult::RevisionMismatch { record }),
                Err(_) => Ok(PutNoteDraftResult::StaleGeneration),
            }
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub fn discard_note_draft(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: DiscardNoteDraftInput,
) -> Result<DiscardNoteDraftResult, String> {
    // No epoch check: orphans are discarded with no project open, and an
    // explicit discard is authoritative over whatever the workspace says.
    let now = db::now_iso();
    let result = state.with_recovery(&app, |recovery| {
        crate::note_recovery::tombstone_draft(recovery, &input.draft_id, &now)
    })?;
    state.bump_draft_seq();
    Ok(result)
}

#[tauri::command]
pub fn save_note_draft(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: SaveNoteDraftInput,
) -> Result<SaveNoteDraftResult, String> {
    crate::note_recovery::validate_kind(&input.kind)?;
    // The whole checked commit — validation, CAS transaction, conditional
    // recovery cleanup — runs under the transition lock, so no open, close,
    // restore or updater swap can interleave with it.
    let _transition = state
        .workspace_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state
        .check_workspace_under_transition(&input.project_key, &input.epoch)
        .is_err()
    {
        return Ok(SaveNoteDraftResult::StaleWorkspace);
    }
    let memory_only = input.draft_id.is_empty();
    let draft_id_opt: Option<&str> = if memory_only {
        None
    } else {
        Some(&input.draft_id)
    };
    let commit = state.with_conn(|project_conn| {
        state
            .with_recovery(&app, |recovery_conn| {
                crate::note_recovery::commit_then_cleanup(
                    project_conn,
                    Some(recovery_conn),
                    &input.kind,
                    &input.target_id,
                    &input.expected_saved_text,
                    &input.draft_text,
                    draft_id_opt,
                    input.revision,
                    &db::now_iso(),
                )
            })
            // Recovery being unavailable must not block an explicit save:
            // commit the memory-only draft through the same CAS checks and
            // surface the recovery warning separately via the status probe.
            .or_else(|recovery_error| {
                if memory_only || recovery_error.contains("Draft recovery is unavailable") {
                    crate::note_recovery::commit_then_cleanup(
                        project_conn,
                        None,
                        &input.kind,
                        &input.target_id,
                        &input.expected_saved_text,
                        &input.draft_text,
                        None,
                        input.revision,
                        &db::now_iso(),
                    )
                } else {
                    Err(recovery_error)
                }
            })
    })?;
    if let SaveNoteDraftResult::Saved { .. } = &commit {
        if input.kind == "coding" {
            schedule_committed_shared_mutation(app.clone(), &state);
        }
        state.bump_draft_seq();
    }
    Ok(commit)
}

#[tauri::command]
pub fn resolve_note_draft_target(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<ResolveNoteDraftTargetResult, String> {
    let draft = state.with_recovery(&app, |recovery| {
        crate::note_recovery::get_draft_by_id(recovery, &draft_id)?
            .ok_or_else(|| "That unfinished note is no longer in local recovery.".to_string())
    })?;
    let (open_key, _) = state.current_workspace();
    match open_key {
        None => Ok(ResolveNoteDraftTargetResult::missing(
            draft.draft_id.clone(),
            "no-project",
        )),
        Some(key) if key != draft.project_key => Ok(ResolveNoteDraftTargetResult::missing(
            draft.draft_id.clone(),
            "other-project",
        )),
        Some(_) => state
            .with_conn(|conn| crate::note_recovery::resolve_draft_against_project(conn, &draft)),
    }
}

/// Selftest-only injection: redirect recovery storage at a temporary root so
/// the suites never read, write, or clear real recovery records. Refused
/// outside `--selftest` mode; passing null restores the real location.
#[tauri::command]
pub fn set_recovery_root_for_selftest(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<(), String> {
    if !crate::selftest::is_selftest_active() {
        return Err("Recovery test injection is only available in selftest mode.".into());
    }
    *state
        .recovery_path_override
        .lock()
        .map_err(|e| e.to_string())? = path.map(PathBuf::from);
    // Drop any cached connection so the next recovery use opens the
    // overridden location, and clear the unavailable latch for the suite.
    *state.recovery_db.lock().map_err(|e| e.to_string())? = None;
    state.recovery_unavailable.store(false, Ordering::SeqCst);
    if let Ok(mut err) = state.recovery_error.lock() {
        *err = None;
    }
    Ok(())
}

/// Mint a one-use updater approval after the frontend draft preflight has
/// passed. Bound to the current workspace identity and draft-write sequence:
/// any draft write landing between approval and install invalidates it.
#[tauri::command]
pub fn approve_update_departure(
    state: State<'_, AppState>,
) -> Result<UpdateDepartureApproval, String> {
    let (project_key, epoch) = state.current_workspace();
    let approval = UpdateDepartureApproval {
        token: uuid::Uuid::new_v4().to_string(),
        project_key: project_key.clone(),
        epoch: epoch.clone(),
        draft_write_seq: state.draft_seq(),
    };
    *state
        .update_departure_approval
        .lock()
        .map_err(|e| e.to_string())? = Some(StoredUpdateApproval {
        token: approval.token.clone(),
        project_key,
        epoch,
        draft_write_seq: approval.draft_write_seq,
    });
    Ok(approval)
}

/// Complete a held native close/quit intent after the frontend preflight.
/// Consumes the intent either way; on approval, replays the original native
/// action once (which the one-use approval lets through without prompting).
#[tauri::command]
pub fn complete_note_departure(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    intent_id: String,
    approved: bool,
) -> Result<DepartureCompletion, String> {
    let pending = state
        .pending_departure
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let pending = match pending {
        Some(intent) if intent.intent_id == intent_id => intent,
        _ => return Err("That close request has already been handled.".into()),
    };
    *state.pending_departure.lock().map_err(|e| e.to_string())? = None;
    if !approved {
        *state.departure_approved.lock().map_err(|e| e.to_string())? = None;
        return Ok(DepartureCompletion {
            intent_id,
            replayed: "cancelled".into(),
        });
    }
    *state.departure_approved.lock().map_err(|e| e.to_string())? = Some(ApprovedDeparture {
        intent_id: intent_id.clone(),
        kind: pending.kind.clone(),
    });
    if pending.kind == "quit" {
        app.exit(0);
        Ok(DepartureCompletion {
            intent_id,
            replayed: "quit".into(),
        })
    } else {
        #[cfg(target_os = "macos")]
        {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            Ok(DepartureCompletion {
                intent_id,
                replayed: "hide".into(),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(window) = app.get_webview_window("main") {
                window.close().map_err(|e| e.to_string())?;
            }
            Ok(DepartureCompletion {
                intent_id,
                replayed: "close".into(),
            })
        }
    }
}

#[tauri::command]
pub fn clear_workspace(
    state: State<'_, AppState>,
    input: ClearWorkspaceInput,
) -> Result<ClearWorkspaceResult, String> {
    state.with_conn(|conn| db::clear_workspace(conn, &input).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn export_with_config(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_dir: String,
    config: ExportConfigInput,
    report_html: Option<String>,
    framework_matrix_csv: Option<String>,
    coder_name: String,
) -> Result<ExportResult, String> {
    let target_path = PathBuf::from(&target_dir);
    // Speaker redaction is a per-machine view/export layer: the toggle lives
    // in app preferences (which have no sync path), so the exporting machine
    // decides and the stored names are never rewritten.
    let redacted: std::collections::HashSet<String> = crate::app_data::get_app_preferences(&app)
        .map(|prefs| {
            prefs
                .speaker_redaction
                .into_iter()
                .filter(|(_, on)| *on)
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default();
    state.with_conn(|conn| {
        db::export_with_config(
            conn,
            &target_path,
            &config,
            report_html.as_deref(),
            framework_matrix_csv.as_deref(),
            &coder_name,
            &redacted,
        )
        .map_err(|e| {
            crate::file_error::classified(
                "Could not write the export there. Choose a different location or free up disk space.",
                &e.to_string(),
            )
        })
    })
}

#[tauri::command]
pub fn render_report_pdf(html: String, out_path: String) -> Result<(), String> {
    crate::pdf::render_report_pdf(&html, Path::new(&out_path))
}

#[tauri::command]
pub fn get_update_status(
    state: State<'_, AppState>,
) -> crate::update_coordinator::UpdateCoordinatorStatus {
    state.update_coordinator.status()
}

#[tauri::command]
pub async fn update_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update_coordinator::UpdateCoordinatorStatus, String> {
    state.update_coordinator.check(app).await
}

#[tauri::command]
pub async fn update_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update_coordinator::UpdateCoordinatorStatus, String> {
    state.update_coordinator.download(app).await
}

#[tauri::command]
pub fn update_cancel_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::update_coordinator::UpdateCoordinatorStatus, String> {
    state.update_coordinator.cancel_download(app)
}

#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    approval: Option<String>,
) -> Result<crate::update_coordinator::UpdateCoordinatorStatus, String> {
    enforce_update_departure_approval(Some(&app), &state, approval)?;
    state.update_coordinator.install(app, &state).await
}

/// Reject an update install that would bypass unsaved draft work.
///
/// The approval minted by `approve_update_departure` (after the frontend ran
/// its Save/Discard/Cancel preflight) is required exactly when there is work
/// to lose: a project is open AND (unfinished drafts exist for it OR recovery
/// itself is unavailable, so the backend cannot prove there is nothing).
/// The approval is one-use and bound to workspace identity + draft sequence;
/// a stale token, a rotated epoch, or an intervening draft write all reject
/// rather than installing over work the user never approved losing.
pub(crate) fn enforce_update_departure_approval(
    app: Option<&tauri::AppHandle>,
    state: &AppState,
    approval: Option<String>,
) -> Result<(), String> {
    let project_open = state
        .project_path
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    if !project_open {
        return Ok(());
    }
    let needs_approval = {
        let (key, _) = state.current_workspace();
        match key {
            None => false,
            Some(project_key) => {
                let (unavailable, _) = state.recovery_status();
                if unavailable {
                    true
                } else {
                    match app {
                        // The store cannot be consulted: fail closed rather
                        // than installing over unknown unfinished work.
                        None => true,
                        Some(handle) => state
                            .with_recovery(handle, |recovery| {
                                crate::note_recovery::count_active_drafts_for_project(
                                    recovery,
                                    &project_key,
                                )
                                .map(|count| count > 0)
                            })
                            .unwrap_or(true),
                    }
                }
            }
        }
    };
    if !needs_approval {
        return Ok(());
    }
    let token = approval.filter(|t| !t.is_empty()).ok_or_else(|| {
        "This study has unfinished notes. Save or discard them before installing the update."
            .to_string()
    })?;
    let mut slot = state
        .update_departure_approval
        .lock()
        .map_err(|e| e.to_string())?;
    let stored = slot.take().ok_or_else(|| {
        "That update approval has already been used. Re-confirm to install.".to_string()
    })?;
    let (current_key, current_epoch) = state.current_workspace();
    if stored.token != token
        || stored.project_key != current_key
        || stored.epoch != current_epoch
        || stored.draft_write_seq != state.draft_seq()
    {
        return Err(
            "Notes changed since the update was approved. Review them and approve again."
                .to_string(),
        );
    }
    Ok(())
}

/// Hand over any project double-clicked before this page could listen, and
/// register that it is listening now.
///
/// Call this exactly once per page load, and only *after* the
/// `open-project-path` listener is registered — that order is the whole point.
/// A path queued before the call comes back in the return value; one that
/// arrives after is emitted to a listener that now exists. Calling it early
/// reopens the gap it closes.
#[tauri::command]
pub fn consume_pending_open(state: State<'_, PendingOpen>) -> Vec<String> {
    match state.inner.lock() {
        Ok(mut inner) => {
            inner.frontend_ready = true;
            std::mem::take(&mut inner.paths)
        }
        Err(_) => Vec::new(),
    }
}

/// Absolute path of the local working library, for display in settings.
#[tauri::command]
pub fn get_projects_library_dir(app: tauri::AppHandle) -> Result<String, String> {
    Ok(crate::app_data::projects_library_dir(&app)?
        .to_string_lossy()
        .to_string())
}

/// Name of the sync service that appears to own this path, if any.
///
/// Feeds an advisory note in the setup wizard. Returns `None` for ordinary
/// local paths. The app never refuses a synced location on the strength of
/// this — see `cloud_provider_for_path`.
#[tauri::command]
pub fn cloud_provider_for_path(app: tauri::AppHandle, path: String) -> Option<String> {
    crate::app_data::cloud_provider_for_path(&app, &PathBuf::from(path))
}

/// Is the app's own default library inside a synced folder?
///
/// `document_dir()` is not guaranteed to be local: Windows Backup redirects
/// `Documents` into OneDrive by default on many Microsoft-account setups, and
/// macOS moves it into iCloud when Desktop & Documents syncing is on. In both
/// cases the location the app treats as safe is silently a sync root, chosen by
/// nobody. Surfaced so the wizard can say so instead of recommending it blindly.
#[tauri::command]
pub fn library_sync_warning(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let lib = crate::app_data::projects_library_dir(&app)?;
    // Canonicalize needs the path to exist; check the nearest parent that does.
    let probe = if lib.exists() {
        lib.clone()
    } else {
        lib.parent().map(|p| p.to_path_buf()).unwrap_or(lib)
    };
    Ok(crate::app_data::cloud_provider_for_path(&app, &probe))
}

#[tauri::command]
pub fn get_workspace_state(state: State<'_, AppState>) -> Result<WorkspaceState, String> {
    state.with_conn(|conn| db::get_workspace_state(conn).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn save_workspace_state(
    state: State<'_, AppState>,
    workspace: WorkspaceState,
) -> Result<(), String> {
    state.with_conn(|conn| db::save_workspace_state(conn, &workspace).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn read_text_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let project_path = state
        .project_path
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    db::read_text_file(project_path.as_deref(), &path)
}

/// Read a transcript in whatever container it arrived in.
///
/// Word documents are unzipped and flattened to text here rather than in the
/// webview: `.docx` is a binary archive, and the app already carries a zip
/// reader for no other reason. Everything else is text and passes through
/// unchanged, to be identified by content in `transcript-parser.ts`.
///
/// Lossy UTF-8 decoding is deliberate. Transcripts come from services that
/// emit stray Windows-1252 bytes in otherwise-clean UTF-8, and refusing the
/// whole file over one bad smart quote would be a worse outcome than replacing
/// that character.
#[tauri::command]
pub fn read_transcript_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let project_path = state
        .project_path
        .lock()
        .map_err(|e| e.to_string())?
        .clone();

    if std::path::Path::new(&path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("docx"))
    {
        let bytes = db::read_binary_file(project_path.as_deref(), &path)?;
        return crate::docx::extract_text(&bytes);
    }

    db::read_text_file(project_path.as_deref(), &path)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScannedTranscriptFile {
    pub path: String,
    pub name: String,
    pub raw_text: String,
}

#[tauri::command]
pub fn scan_transcript_folder(folder_path: String) -> Result<Vec<ScannedTranscriptFile>, String> {
    let dir = std::path::Path::new(&folder_path);
    if !dir.is_dir() {
        return Err(crate::file_error::wrap_file_error(
            crate::file_error::FileAccessCategory::PathUnavailable,
            "That folder is not available. It may have been moved, renamed, or disconnected.",
            "not a directory: scan_transcript_folder",
        ));
    }

    let mut result = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        let category = crate::file_error::classify_io(&e);
        crate::file_error::wrap_file_error(
            category,
            "Fleuron could not read that folder. If access is restricted, choose another folder.",
            &e.to_string(),
        )
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if matches!(
                ext.as_str(),
                "vtt" | "srt" | "txt" | "md" | "csv" | "tsv" | "docx"
            ) {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let path_str = path.to_string_lossy().to_string();
                let raw_text = if ext == "docx" {
                    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    crate::docx::extract_text(&bytes).unwrap_or_default()
                } else {
                    std::fs::read_to_string(&path).unwrap_or_default()
                };
                if !raw_text.trim().is_empty() {
                    result.push(ScannedTranscriptFile {
                        path: path_str,
                        name,
                        raw_text,
                    });
                }
            }
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

#[tauri::command]
pub fn list_recent_projects(app: tauri::AppHandle) -> Result<Vec<RecentProject>, String> {
    crate::app_data::list_recent_projects(&app)
}

#[tauri::command]
pub fn record_recent_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut input: RecordRecentProjectInput,
) -> Result<Vec<RecentProject>, String> {
    let (bound, group_id, group_title, coder_name) = state
        .with_conn(|conn| {
            let bound = sync::is_bound(conn).unwrap_or(false);
            if bound {
                let gid = sync::get_state(conn, sync::KEY_PROJECT_ID).ok().flatten();
                let gtitle = sync::get_state(conn, sync::KEY_GROUP_TITLE).ok().flatten();
                let cname = sync::get_state(conn, sync::KEY_CODER_NAME).ok().flatten();
                Ok((true, gid, gtitle, cname))
            } else {
                Ok((false, None, None, None))
            }
        })
        .unwrap_or((false, None, None, None));

    if bound {
        if input.group_id.is_none() {
            input.group_id = group_id;
        }
        if input.group_title.is_none() {
            input.group_title = group_title;
        }
        if input.coder_name.is_none() {
            input.coder_name = coder_name;
        }
    } else {
        input.group_id = None;
    }

    crate::app_data::record_recent_project(&app, input)
}

#[tauri::command]
pub fn remove_recent_project(
    app: tauri::AppHandle,
    path: String,
) -> Result<Vec<RecentProject>, String> {
    crate::app_data::remove_recent_project(&app, path)
}

#[tauri::command]
pub fn get_app_preferences(app: tauri::AppHandle) -> Result<AppPreferences, String> {
    crate::app_data::get_app_preferences(&app)
}

#[tauri::command]
pub fn set_app_preferences(
    app: tauri::AppHandle,
    prefs: AppPreferences,
) -> Result<AppPreferences, String> {
    crate::app_data::set_app_preferences(&app, prefs)
}

/// Pins the build commit into the binary's data section so release verifiers
/// can find it by byte search (`scripts/verify-mac-bundle.sh`).
///
/// `get_app_version` reads the same value through `env!`, which is always
/// correct at runtime — but at `opt-level=3` a *short* commit string (an
/// abbreviated SHA) is materialized as instruction immediates rather than a
/// `.rodata` memcpy source, which dead-strips the literal out of the binary
/// and makes the verifier fail. A 40-char `github.sha` happens to survive;
/// anything short does not. `#[used]` forces the marker static to be emitted,
/// which in turn forces its string data to exist at any length. The commit is
/// a substring of the marker, so the verifier's plain search still matches.
#[used]
static BUILD_COMMIT_MARKER: &str = concat!("fleuron-build-commit:", env!("FLEURON_BUILD_COMMIT"));

#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> Result<AppVersionInfo, String> {
    let info = app.package_info();
    // Build commit comes from compile time; candidate/release CI sets
    // FLEURON_BUILD_COMMIT to the exact source SHA and the packaged-app
    // verifier fails if About does not report it. Non-CI builds honestly say
    // "development".
    let build_commit = env!("FLEURON_BUILD_COMMIT").to_string();
    Ok(AppVersionInfo {
        name: info.name.to_string(),
        version: info.version.to_string(),
        copyright: Some("Fleuron contributors".into()),
        build_commit: Some(build_commit),
        source_url: Some("https://github.com/wilson-taiwan/fleuron".into()),
        release_url: Some("https://github.com/wilson-taiwan/fleuron/releases".into()),
        install_guide_url: Some(
            "https://github.com/wilson-taiwan/fleuron/blob/main/docs/INSTALLING.md".into(),
        ),
    })
}

#[tauri::command]
pub fn list_project_files(state: State<'_, AppState>) -> Result<Vec<ProjectFileEntry>, String> {
    let path = state.project_path_str()?;
    db::list_project_files(PathBuf::from(&path).as_path())
}

#[tauri::command]
pub fn copy_project_file(
    state: State<'_, AppState>,
    source: String,
    destination: String,
) -> Result<(), String> {
    let path = state.project_path_str()?;
    db::copy_project_file(PathBuf::from(&path).as_path(), &source, &destination)
}

// ── Sync ─────────────────────────────────────────────────────────────────────
//
// Every command here is structured the same way, and it is not stylistic: the
// database lives behind a `std::sync::Mutex` whose guard is not `Send`, so it
// cannot be held across an `.await`. Read from SQLite into owned values, drop
#[tauri::command]
pub fn hash_candidate_segments(segments: Vec<CandidateSegment>) -> Option<String> {
    sync::content_hash_of(segments.into_iter().map(|s| (s.index, s.text)))
}

use crate::sync::{self, SyncConfig, SyncOutcome, SyncSession};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// A server address is configured on this machine.
    pub configured: bool,
    /// Someone has signed in during this app session (or was restored from
    /// the session store).
    pub signed_in: bool,
    /// Email on the signed-in account, when the server sent one.
    pub signed_in_email: Option<String>,
    /// Whether this build carries a compiled-in server.
    ///
    /// Distinct from `configured`, which is also true when someone typed an
    /// address in by hand. This one answers a question only the *inviter* asks:
    /// will the person receiving this invitation need the address and key as
    /// well, or will their copy already know? Their copy is this build.
    pub server_preset: bool,
    /// Present once the open project has been given a sync identity.
    pub project_id: Option<String>,
    /// This folder is a member of a remote group — not merely holding a
    /// locally minted id.
    pub in_group: bool,
    /// Whether the current signed-in account is an admin in this group.
    pub is_group_admin: bool,
    pub last_synced_at: Option<String>,
    pub pending_changes: usize,
    pub never_synced: bool,
    pub realtime_connected: bool,
    pub realtime_health: crate::realtime::RealtimeHealth,
    pub server_schema_version: Option<i32>,
    pub required_server_schema: i32,
    pub pending_unbind_operation: Option<String>,
    pub coordinator_running: bool,
    pub coordinator_rerun_requested: bool,
    pub coordinator_backoff_attempt: u32,
    pub coordinator_last_trigger: Option<String>,
    pub oldest_outbox_age_seconds: Option<i64>,
    pub protocol: i64,
    pub generation_suffix: Option<String>,
    pub device_id_suffix: Option<String>,
    pub local_sequence: i64,
    pub observed_head: i64,
    pub outbox_count: i64,
    pub blocked_outbox_count: i64,
    pub unresolved_conflict_count: i64,
    pub last_realtime_at: Option<String>,
    pub last_success_at: Option<String>,
    pub sequence_lag_age_seconds: Option<i64>,
}

#[derive(Default)]
struct LocalSyncStatus {
    project_id: Option<String>,
    last_synced_at: Option<String>,
    pending_changes: usize,
    in_group: bool,
    is_group_admin: bool,
    never_synced: bool,
    pending_unbind_operation: Option<String>,
    protocol: i64,
    generation_suffix: Option<String>,
    device_id_suffix: Option<String>,
    local_sequence: i64,
    observed_head: i64,
    outbox_count: i64,
    blocked_outbox_count: i64,
    unresolved_conflict_count: i64,
    last_realtime_at: Option<String>,
    last_success_at: Option<String>,
    oldest_outbox_age_seconds: Option<i64>,
    sequence_lag_age_seconds: Option<i64>,
}

struct V2DiagnosticsSnapshot {
    generation_suffix: Option<String>,
    device_id_suffix: Option<String>,
    local_sequence: i64,
    observed_head: i64,
    outbox_count: i64,
    oldest_outbox_age_seconds: Option<i64>,
    blocked_outbox_count: i64,
    unresolved_conflict_count: i64,
    last_realtime_at: Option<String>,
    last_success_at: Option<String>,
    code_count: i64,
    interview_count: i64,
    assignment_count: i64,
}

fn safe_identifier_suffix(value: Option<String>) -> Option<String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| crate::diagnostics::safe_suffix(&value))
}

fn elapsed_seconds_from_rfc3339(value: Option<String>) -> Option<i64> {
    value.and_then(|value| {
        chrono::DateTime::parse_from_rfc3339(&value)
            .ok()
            .map(|timestamp| {
                (chrono::Utc::now() - timestamp.with_timezone(&chrono::Utc))
                    .num_seconds()
                    .max(0)
            })
    })
}

fn format_v2_diagnostics(
    snapshot: &V2DiagnosticsSnapshot,
    coordinator: &crate::sync_coordinator::SyncCoordinatorHealth,
) -> String {
    format!(
        "Fleuron Sync Protocol 2 Diagnostics\n\
         ==================================\n\
         Protocol: 2\n\
         Generation suffix: {}\n\
         Device suffix: {}\n\
         Local sequence: {}\n\
         Observed head: {}\n\
         Outbox: {} (oldest age: {}s)\n\
         Blocked outbox: {}\n\
         Unresolved conflicts: {}\n\
         Last Realtime hint: {}\n\
         Last successful sync: {}\n\
         Last trigger: {}\n\
         Backoff attempt: {}\n\
         Rerun requested: {}\n\
         Entity counts: {} codes, {} interviews, {} assignments\n",
        snapshot.generation_suffix.as_deref().unwrap_or("none"),
        snapshot.device_id_suffix.as_deref().unwrap_or("none"),
        snapshot.local_sequence,
        snapshot.observed_head,
        snapshot.outbox_count,
        snapshot.oldest_outbox_age_seconds.unwrap_or(0),
        snapshot.blocked_outbox_count,
        snapshot.unresolved_conflict_count,
        snapshot.last_realtime_at.as_deref().unwrap_or("never"),
        snapshot.last_success_at.as_deref().unwrap_or("never"),
        coordinator.last_trigger.as_deref().unwrap_or("none"),
        coordinator.backoff_attempt,
        coordinator.rerun_requested,
        snapshot.code_count,
        snapshot.interview_count,
        snapshot.assignment_count,
    )
}

/// Where this machine syncs, without reference to any project.
///
/// Split out because three things happen before a project is open: restoring a
/// remembered sign-in at startup, signing in, and listing the studies available
/// to join. All need the server; none has a project to take an id from.
///
/// Falls back to the address compiled into the build, so a coder joining this
/// study has nothing to paste — see `sync_defaults`. Stored preferences still
/// win, which is what lets one build point at a different study without a
/// rebuild.
/// Resolve sync server settings from preferences and compile-time defaults.
///
/// Extracted so contract tests can verify pre-project commands never need an
/// open project — only this function, not [`AppState::project_sync_config`].
pub(crate) fn resolve_sync_server_config(
    prefs: &crate::models::AppPreferences,
) -> Result<SyncConfig, String> {
    let url = prefs
        .sync_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| crate::sync_defaults::URL.map(str::to_string))
        .ok_or("Sync is not set up on this machine yet.")?;
    let anon_key = prefs
        .sync_anon_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| crate::sync_defaults::ANON_KEY.map(str::to_string))
        .ok_or("Sync is not set up on this machine yet.")?;
    Ok(SyncConfig {
        url,
        anon_key,
        project_id: String::new(),
    })
}

fn sync_server_config(app: &tauri::AppHandle) -> Result<SyncConfig, String> {
    resolve_sync_server_config(&crate::app_data::get_app_preferences(app)?)
}

/// Resolve which project a membership command targets: the explicit id if the
/// caller passed a non-empty one, else the currently-open project, else an error.
/// Shared by leave/detach/delete so "act on a study from the home screen" behaves
/// identically across all three.
fn resolve_target_project_id(
    provided: Option<String>,
    current_open: Option<&str>,
) -> Result<String, String> {
    match provided {
        Some(id) if !id.trim().is_empty() => Ok(id),
        _ => current_open
            .map(|s| s.to_string())
            .ok_or_else(|| "No project is open and no project id was provided.".into()),
    }
}

impl AppState {
    pub fn cached_server_schema(&self, url: &str) -> Option<Option<i32>> {
        if let Ok(guard) = self.cached_schema_version.lock() {
            if let Some((cached_url, v)) = &*guard {
                if cached_url == url {
                    return Some(*v);
                }
            }
        }
        None
    }

    pub fn set_cached_server_schema(&self, url: &str, v: Option<i32>) {
        if let Ok(mut guard) = self.cached_schema_version.lock() {
            *guard = Some((url.to_string(), v));
        }
    }

    pub async fn get_or_probe_server_schema(
        &self,
        _app: &tauri::AppHandle,
        cfg: &SyncConfig,
    ) -> Option<i32> {
        if let Some(cached) = self.cached_server_schema(&cfg.url) {
            return cached;
        }
        let client = reqwest::Client::new();
        let res = sync::server_schema_version_fn(&client, cfg).await;
        let v = match res {
            Ok(ver) => Some(ver),
            Err(sync::SyncError::ServerFunctionUnavailable { .. }) => Some(0),
            Err(_) => None,
        };
        self.set_cached_server_schema(&cfg.url, v);
        v
    }

    pub(crate) async fn require_server_schema(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let cfg = sync_server_config(app)?;
        let version = self.get_or_probe_server_schema(app, &cfg).await;
        match version {
            Some(v) if v >= sync::REQUIRED_SERVER_SCHEMA => Ok(()),
            Some(v) => Err(format!(
                "Server update needed (found version {}; Fleuron needs {}).",
                v, sync::REQUIRED_SERVER_SCHEMA
            )),
            None => Err("Fleuron couldn't verify the server version. Membership and admin changes are temporarily disabled; check again when online.".into()),
        }
    }

    /// Server config *plus* the open project's sync id.
    ///
    /// Named for what it requires, not what it returns: it calls `with_conn`, so
    /// every caller inherits "No project is open". Only use it for operations
    /// that genuinely act on a project — pushing, pulling, inviting someone to
    /// this study. Anything that runs before a project exists wants
    /// `sync_server_config` instead.
    pub(crate) fn project_sync_config(&self, app: &tauri::AppHandle) -> Result<SyncConfig, String> {
        let project_id =
            self.with_conn(|conn| sync::ensure_project_id(conn).map_err(|e| e.to_string()))?;
        Ok(SyncConfig {
            project_id,
            ..sync_server_config(app)?
        })
    }

    pub(crate) fn project_sync_config_for_target(
        &self,
        app: &tauri::AppHandle,
        target: &SyncRunTarget,
    ) -> Result<SyncConfig, String> {
        let project_id = self.with_sync_target(target, |conn| {
            sync::ensure_project_id(conn).map_err(|e| e.to_string())
        })?;
        Ok(SyncConfig {
            project_id,
            ..sync_server_config(app)?
        })
    }

    pub(crate) fn sync_session(&self) -> Result<SyncSession, String> {
        self.sync_session
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "Sign in to sync first.".into())
    }

    pub(crate) fn update_sync_session(
        &self,
        app: &tauri::AppHandle,
        renewed: SyncSession,
    ) -> Result<(), sync::SyncError> {
        sync::remember_session(app, &renewed);
        *self
            .sync_session
            .lock()
            .map_err(|e| sync::SyncError::Db(e.to_string()))? = Some(renewed);
        self.maybe_start_realtime(app);
        Ok(())
    }

    pub(crate) fn maybe_start_realtime(&self, app: &tauri::AppHandle) {
        let (is_bound, sync_protocol) = self
            .with_conn(|conn| {
                Ok((
                    sync::is_bound(conn).map_err(|e| e.to_string())?,
                    db::local_sync_protocol(conn).map_err(|e| e.to_string())?,
                ))
            })
            .unwrap_or((false, 1));
        if !is_bound {
            return;
        }
        if let Ok(cfg) = self.project_sync_config(app) {
            if let Ok(session) = self.sync_session() {
                let my_coder_name = self
                    .with_conn(|conn| {
                        sync::get_state(conn, sync::KEY_CODER_NAME).map_err(|e| e.to_string())
                    })
                    .unwrap_or(None)
                    .unwrap_or_default();
                self.realtime.start(
                    app.clone(),
                    cfg.url,
                    cfg.anon_key,
                    session.access_token,
                    cfg.project_id,
                    my_coder_name,
                    sync_protocol,
                );
            }
        }
    }

    pub(crate) fn record_realtime_hint(&self, generation: Option<&str>, head: Option<i64>) {
        let _ = self.with_conn(|conn| {
            db::note_v2_realtime_hint(conn, generation, head).map_err(|error| error.to_string())
        });
    }

    pub async fn request_sync(
        &self,
        app: tauri::AppHandle,
        trigger: crate::sync_coordinator::SyncTrigger,
    ) -> Result<SyncOutcome, String> {
        let (start, generation, waiter) = self.sync_coordinator.request(trigger);
        if start {
            Self::spawn_sync_runner(app, generation);
        }
        waiter.wait().await
    }

    pub fn request_background_sync(
        &self,
        app: tauri::AppHandle,
        trigger: crate::sync_coordinator::SyncTrigger,
    ) {
        if !self.can_schedule_background_sync() {
            return;
        }
        let (start, generation, _) = self.sync_coordinator.request(trigger);
        if start {
            Self::spawn_sync_runner(app, generation);
        }
    }

    fn can_schedule_background_sync(&self) -> bool {
        let has_session = self
            .sync_session
            .lock()
            .map(|session| session.is_some())
            .unwrap_or(false);
        has_session
            && self
                .with_conn(|conn| sync::is_bound(conn).map_err(|e| e.to_string()))
                .unwrap_or(false)
    }

    pub fn start_sync_polling(&self, app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                let state = app.state::<AppState>();
                let foreground = state.sync_coordinator.is_foreground();
                tokio::time::sleep(crate::sync_coordinator::poll_interval(foreground)).await;
                let state = app.state::<AppState>();
                let trigger = if state.sync_coordinator.is_foreground() {
                    crate::sync_coordinator::SyncTrigger::ForegroundPoll
                } else {
                    crate::sync_coordinator::SyncTrigger::HiddenPoll
                };
                state.request_background_sync(app.clone(), trigger);
            }
        });
    }

    fn spawn_sync_runner(app: tauri::AppHandle, generation: u64) {
        tauri::async_runtime::spawn(async move {
            loop {
                loop {
                    let state = app.state::<AppState>();
                    let Some(debounce) = state.sync_coordinator.debounce_remaining(generation)
                    else {
                        return;
                    };
                    if debounce.is_zero() {
                        break;
                    }
                    tokio::time::sleep(debounce).await;
                }
                let state = app.state::<AppState>();
                if !state.sync_coordinator.begin_pass(generation) {
                    break;
                }
                let result = run_sync_pass(&app, &state, generation).await;
                if !state.sync_coordinator.is_active(generation) {
                    break;
                }
                let rerun = state.sync_coordinator.take_rerun(generation);
                if rerun {
                    continue;
                }
                let retry = result.as_ref().err().is_some_and(|error| {
                    error.starts_with("Could not reach")
                        || error.contains("network")
                        || error.contains("sending request")
                });
                if !state.sync_coordinator.finish(generation, result) {
                    break;
                }
                if retry && state.can_schedule_background_sync() {
                    let retry_delay = state.sync_coordinator.retry_delay();
                    let retry_app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(retry_delay).await;
                        let retry_state = retry_app.state::<AppState>();
                        retry_state.request_background_sync(
                            retry_app.clone(),
                            crate::sync_coordinator::SyncTrigger::NetworkRecovered,
                        );
                    });
                }
                break;
            }
        });
    }
}

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn sync_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncStatus, String> {
    // "Configured" means a server can be reached, from preferences or from the
    // build. Asking preferences alone would have the UI demand setup on a build
    // that already knows where it syncs — and the join screen keys its whole
    // shape off this.
    let configured = sync_server_config(&app).is_ok();
    let (signed_in, signed_in_email) = {
        let guard = state.sync_session.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(s) => (
                true,
                if s.email.is_empty() {
                    None
                } else {
                    Some(s.email.clone())
                },
            ),
            None => (false, None),
        }
    };

    // A project need not be open to ask whether sync is set up.
    let local = state
        .with_conn(|conn| {
            let id = sync::get_state(conn, sync::KEY_PROJECT_ID).map_err(|e| e.to_string())?;
            let last = sync::get_state(conn, sync::KEY_LAST_SYNCED).map_err(|e| e.to_string())?;
            let protocol = db::local_sync_protocol(conn).map_err(|e| e.to_string())?;
            let legacy_pending = if protocol == 2 {
                0
            } else {
                sync::collect_push_batch(conn, "")
                    .map(|b| b.coded.len() + b.codes.len())
                    .map_err(|e| e.to_string())?
            };
            let outbox_count = conn
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|e| e.to_string())?;
            let blocked_outbox_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE blocked_reason IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?;
            let unresolved_conflict_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_conflicts WHERE status = 'unresolved'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?;
            let in_group = sync::is_bound(conn).map_err(|e| e.to_string())?;
            let role = sync::get_state(conn, sync::KEY_ROLE).map_err(|e| e.to_string())?;
            let is_group_admin = in_group && role.as_deref() == Some("admin");
            let last_success_at =
                db::sync_state_value(conn, "sync_last_success_at").map_err(|e| e.to_string())?;
            let never_synced = if protocol == 2 {
                last_success_at.as_deref().is_none_or(str::is_empty)
            } else {
                sync::is_never_synced(conn).unwrap_or(false)
            };
            let pending_unbind =
                sync::get_state(conn, sync::KEY_PENDING_UNBIND).map_err(|e| e.to_string())?;
            let pending_op = pending_unbind.and_then(|s| {
                let v: serde_json::Value = serde_json::from_str(&s).ok()?;
                v.get("operation")
                    .and_then(|op| op.as_str())
                    .map(String::from)
            });
            let local_sequence = db::sync_state_value(conn, "sync_server_seq")
                .map_err(|e| e.to_string())?
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            let observed_head = db::sync_state_value(conn, "sync_observed_head")
                .map_err(|e| e.to_string())?
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            let oldest_outbox_age_seconds = conn
                .query_row("SELECT MIN(created_at) FROM sync_outbox", [], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .map_err(|e| e.to_string())
                .ok()
                .flatten()
                .and_then(|value| elapsed_seconds_from_rfc3339(Some(value)));
            let sequence_lag_age_seconds = if observed_head > local_sequence {
                elapsed_seconds_from_rfc3339(
                    db::sync_state_value(conn, "sync_lag_observed_at")
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            Ok(LocalSyncStatus {
                project_id: id,
                last_synced_at: if protocol == 2 {
                    last_success_at.clone()
                } else {
                    last
                },
                pending_changes: if protocol == 2 {
                    outbox_count.max(0) as usize
                } else {
                    legacy_pending
                },
                in_group,
                is_group_admin,
                never_synced,
                pending_unbind_operation: pending_op,
                protocol,
                generation_suffix: safe_identifier_suffix(
                    db::sync_state_value(conn, "sync_generation").map_err(|e| e.to_string())?,
                ),
                device_id_suffix: safe_identifier_suffix(
                    db::sync_state_value(conn, "sync_device_id").map_err(|e| e.to_string())?,
                ),
                local_sequence,
                observed_head,
                outbox_count,
                blocked_outbox_count,
                unresolved_conflict_count,
                last_realtime_at: db::sync_state_value(conn, "sync_last_realtime_at")
                    .map_err(|e| e.to_string())?
                    .filter(|value| !value.is_empty()),
                last_success_at,
                oldest_outbox_age_seconds,
                sequence_lag_age_seconds,
            })
        })
        .unwrap_or_default();

    let server_schema_version = if let Ok(cfg) = sync_server_config(&app) {
        state.cached_server_schema(&cfg.url).flatten()
    } else {
        None
    };
    let coordinator = state.sync_coordinator.health();
    Ok(SyncStatus {
        configured,
        signed_in,
        signed_in_email,
        server_preset: crate::sync_defaults::URL.is_some()
            && crate::sync_defaults::ANON_KEY.is_some(),
        project_id: local.project_id,
        in_group: local.in_group,
        is_group_admin: local.is_group_admin,
        last_synced_at: local.last_synced_at,
        pending_changes: local.pending_changes,
        never_synced: local.never_synced,
        realtime_connected: state.realtime.is_connected(),
        realtime_health: state.realtime.health(),
        server_schema_version,
        required_server_schema: sync::REQUIRED_SERVER_SCHEMA,
        pending_unbind_operation: local.pending_unbind_operation,
        coordinator_running: coordinator.running,
        coordinator_rerun_requested: coordinator.rerun_requested,
        coordinator_backoff_attempt: coordinator.backoff_attempt,
        coordinator_last_trigger: coordinator.last_trigger,
        oldest_outbox_age_seconds: local.oldest_outbox_age_seconds,
        protocol: local.protocol,
        generation_suffix: local.generation_suffix,
        device_id_suffix: local.device_id_suffix,
        local_sequence: local.local_sequence,
        observed_head: local.observed_head,
        outbox_count: local.outbox_count,
        blocked_outbox_count: local.blocked_outbox_count,
        unresolved_conflict_count: local.unresolved_conflict_count,
        last_realtime_at: local.last_realtime_at,
        last_success_at: local.last_success_at,
        sequence_lag_age_seconds: local.sequence_lag_age_seconds,
    })
}

/// Query the server schema version and update the in-memory cache.
#[tauri::command]
pub async fn sync_refresh_server_schema(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<i32>, String> {
    let cfg = sync_server_config(&app)?;
    if let Ok(mut guard) = state.cached_schema_version.lock() {
        *guard = None;
    }
    let client = sync::client().map_err(|e| e.to_string())?;
    let res = sync::server_schema_version_fn(&client, &cfg).await;
    let v = match res {
        Ok(ver) => Some(ver),
        Err(sync::SyncError::ServerFunctionUnavailable { .. }) => Some(0),
        Err(_) => None,
    };
    state.set_cached_server_schema(&cfg.url, v);
    Ok(v)
}

/// Sign in to the sync server.
///
/// Server config, not project config. Signing in is the *third* thing that
/// happens before a project exists — alongside restoring a session and listing
/// studies — and it was the one that got missed: this took `project_sync_config`
/// for six releases, so it demanded an open project purely to derive a
/// `project_id` that `sync::sign_in` never reads. The second coder, whose whole
/// situation is "fresh machine, no project yet", got "No project is open" from
/// the sign-in form and had no way forward.
#[tauri::command]
pub async fn sync_sign_in(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<(), String> {
    let cfg = sync_server_config(&app)?;
    let client = sync::client().map_err(|e| e.to_string())?;

    let session = sync::sign_in(&client, &cfg, &email, &password)
        .await
        .map_err(|e| e.to_string())?;

    sync::remember_session(&app, &session);
    *state.sync_session.lock().map_err(|e| e.to_string())? = Some(session);
    state.maybe_start_realtime(&app);
    state.request_background_sync(app.clone(), crate::sync_coordinator::SyncTrigger::SignedIn);
    Ok(())
}

/// Bring back a sign-in remembered from a previous launch.
///
/// Called once at startup, before anything else touches sync. Returns whether
/// a session was restored so the UI can tell "signed out" from "still trying".
/// Deliberately forgiving: no server configured, no stored token, or no network
/// are all ordinary states, not errors worth showing anybody.
#[tauri::command]
pub async fn sync_restore_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let Ok(cfg) = sync_server_config(&app) else {
        return Ok(false);
    };
    let Ok(client) = sync::client() else {
        return Ok(false);
    };
    match sync::restore_session(&app, &client, &cfg).await {
        Ok(Some(session)) => {
            *state.sync_session.lock().map_err(|e| e.to_string())? = Some(session);
            state.maybe_start_realtime(&app);
            state.request_background_sync(
                app.clone(),
                crate::sync_coordinator::SyncTrigger::SessionRestored,
            );
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[tauri::command]
pub fn sync_sign_out(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.sync_coordinator.cancel();
    state.realtime.stop(&app);
    sync::forget_session(&app);
    let _ = crate::app_data::clear_memberships_cache(&app);
    *state.sync_session.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

/// Groups this account belongs to, with the name it files under in each.
#[tauri::command]
pub async fn sync_list_memberships(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<sync::MembershipSummary>, String> {
    let cfg = sync_server_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let memberships = sync::list_memberships(&client, &cfg, &session)
        .await
        .map_err(|e| e.to_string())?;
    let _ = crate::app_data::write_memberships_cache(&app, &memberships);
    Ok(memberships)
}

/// Read cached group memberships from app data for offline / fast first-paint.
#[tauri::command]
pub fn list_cached_memberships(
    app: tauri::AppHandle,
) -> Result<crate::app_data::MembershipsCache, String> {
    crate::app_data::read_memberships_cache(&app)
}

/// Bind this local project to a shared one.
///
/// Rebinding a folder that is already in a *different* group is refused —
/// that was Change project, and it mixed two studies. Binding the same group
/// again is a no-op (cursors stay). An unbound folder, including one that
/// only has a locally minted id, takes the shared id and re-offers everything
/// local.
#[tauri::command]
pub fn sync_join_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
) -> Result<(), String> {
    let cfg = sync_server_config(&app)?;
    let cached_v = state.cached_server_schema(&cfg.url);
    match cached_v {
        Some(Some(v)) if v >= sync::REQUIRED_SERVER_SCHEMA => {}
        Some(Some(v)) => {
            return Err(format!(
                "Server update needed (found version {}; Fleuron needs {}).",
                v,
                sync::REQUIRED_SERVER_SCHEMA
            ))
        }
        _ => return Err("Server schema version has not been verified yet.".into()),
    }

    let current_path = state.project_path_str().ok();
    if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
        for r in recents {
            if r.group_id.as_deref() == Some(&project_id)
                && current_path.as_deref() != Some(&r.path)
            {
                if std::path::Path::new(&r.path).exists() {
                    return Err(format!(
                        "This group is already set up on this Mac, at {}. Open that folder instead — a second copy would split your coding across two folders.",
                        r.path
                    ));
                } else {
                    let _ = crate::app_data::remove_recent_project(&app, r.path);
                }
            }
        }
    }
    state.with_conn(|conn| sync::bind_to_group(conn, &project_id).map_err(|e| e.to_string()))?;
    state.maybe_start_realtime(&app);
    state.request_background_sync(
        app.clone(),
        crate::sync_coordinator::SyncTrigger::ProjectJoined,
    );
    if let Some(path) = current_path {
        let title = state
            .with_conn(|conn| {
                db::get_project_info(conn, &path)
                    .map(|p| p.title)
                    .map_err(|e| e.to_string())
            })
            .unwrap_or_default();
        let _ = crate::app_data::record_recent_project(
            &app,
            RecordRecentProjectInput {
                path,
                title,
                group_id: Some(project_id),
                group_title: None,
                coder_name: None,
                former_group_id: None,
                former_group_title: None,
                readiness: None,
            },
        );
    }
    Ok(())
}

/// Create an account on the sync server.
///
/// Returns whether the account is usable straight away. `false` means the
/// project has email confirmation switched on, so the account exists but cannot
/// sign in until the link is clicked — worth saying rather than leaving someone
/// looking at a form that appeared to do nothing.
#[tauri::command]
pub async fn sync_sign_up(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<bool, String> {
    let cfg = sync_server_config(&app)?;
    let client = sync::client().map_err(|e| e.to_string())?;
    match sync::sign_up(&client, &cfg, email.trim(), &password)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(session) => {
            sync::remember_session(&app, &session);
            *state.sync_session.lock().map_err(|e| e.to_string())? = Some(session);
            state.maybe_start_realtime(&app);
            state.request_background_sync(
                app.clone(),
                crate::sync_coordinator::SyncTrigger::SignedIn,
            );
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Email a password-reset code. Succeeds the same way whether or not that
/// inbox has an account — the UI must not claim otherwise.
#[tauri::command]
pub async fn sync_request_password_reset(
    app: tauri::AppHandle,
    email: String,
) -> Result<(), String> {
    let cfg = sync_server_config(&app)?;
    let client = sync::client().map_err(|e| e.to_string())?;
    sync::request_password_reset(&client, &cfg, email.trim())
        .await
        .map_err(|e| e.to_string())
}

/// Verify the email's code and set a new password. Signs in on success.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sync_complete_password_reset(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    email: String,
    password: String,
    token: Option<String>,
    token_hash: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
) -> Result<(), String> {
    let cfg = sync_server_config(&app)?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let session = sync::complete_password_reset(
        &client,
        &cfg,
        email.trim(),
        &password,
        token.as_deref(),
        token_hash.as_deref(),
        access_token.as_deref(),
        refresh_token.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    sync::remember_session(&app, &session);
    *state.sync_session.lock().map_err(|e| e.to_string())? = Some(session);
    state.maybe_start_realtime(&app);
    state.request_background_sync(app.clone(), crate::sync_coordinator::SyncTrigger::SignedIn);
    Ok(())
}

/// Redeem an invitation. Returns the project joined and the assigned name.
///
/// Only the server side of joining — the caller still binds the local project
/// with `sync_join_project`, which needs a project open and this does not.
#[tauri::command]
pub async fn sync_redeem_invite(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<sync::Redeemed, String> {
    state.require_server_schema(&app).await?;
    let cfg = sync_server_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    sync::redeem_invite(&client, &cfg, &session, code.trim())
        .await
        .map_err(|e| e.to_string())
}

/// What a freshly created group came back with.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedGroup {
    pub project_id: String,
    /// Display-formatted (`XXXX-XXXX`), ready to show and share.
    pub group_key: String,
}

/// Turn this local project into a group, minting its key.
///
/// The creator's membership starts out under their email address — the
/// `add_creator_as_member` trigger has nothing better — so the caller follows
/// this with `sync_set_my_coder_name`, which is also why that command exists
/// for everyone else.
#[tauri::command]
pub async fn sync_create_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    title: String,
) -> Result<CreatedGroup, String> {
    state.require_server_schema(&app).await?;
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let key = sync::create_remote_project(&client, &cfg, &session, &title)
        .await
        .map_err(|e| e.to_string())?;

    state.with_conn(|conn| {
        sync::set_state(conn, sync::KEY_BOUND, "1").map_err(|e| e.to_string())?;
        sync::set_state(conn, sync::KEY_ROLE, "admin").map_err(|e| e.to_string())?;
        sync::set_state(conn, sync::KEY_GROUP_TITLE, &title).map_err(|e| e.to_string())?;
        Ok(())
    })?;
    state.maybe_start_realtime(&app);
    state.request_background_sync(
        app.clone(),
        crate::sync_coordinator::SyncTrigger::ProjectJoined,
    );

    Ok(CreatedGroup {
        project_id: cfg.project_id,
        group_key: sync::format_group_key(&key),
    })
}

/// Join a group by its key. Returns what the server settled.
///
/// Only the server side of joining — the caller still binds the local project
/// with `sync_join_project`, which needs a project open and this does not.
#[tauri::command]
pub async fn sync_join_group(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    key: String,
    coder_name: String,
) -> Result<sync::JoinedGroup, String> {
    state.require_server_schema(&app).await?;
    let cfg = sync_server_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    sync::join_group_client_fn(&client, &cfg, &session, key.trim(), coder_name.trim())
        .await
        .map_err(|e| e.to_string())
}

/// The open project's group: key, roster, and per-person activity.
#[tauri::command]
pub async fn sync_group_info(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<sync::GroupInfo, String> {
    let bound = state.with_conn(|conn| sync::is_bound(conn).map_err(|e| e.to_string()))?;
    if !bound {
        return Err("This project is not in a group yet.".into());
    }
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let info = sync::group_info_fetch(&client, &cfg, &session)
        .await
        .map_err(|e| e.to_string())?;

    let _ = state.with_conn(|conn| {
        let _ = sync::set_state(conn, sync::KEY_GROUP_TITLE, &info.title);
        if let Some(you) = info.members.iter().find(|m| m.is_you) {
            if let Some(role) = &you.role {
                let _ = sync::set_state(conn, sync::KEY_ROLE, role);
            }
            let _ = sync::set_state(conn, sync::KEY_CODER_NAME, &you.coder_name);
        }
        Ok(())
    });

    Ok(info)
}

async fn require_sync_v2_server(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let cfg = state.project_sync_config(app)?;
    match state.get_or_probe_server_schema(app, &cfg).await {
        Some(version) if version >= 10 => Ok(()),
        Some(version) => Err(format!(
            "Server update needed (found version {version}; Sync Protocol 2 needs 10)."
        )),
        None => Err(
            "Fleuron could not verify the Sync Protocol 2 server version. Try again when online."
                .into(),
        ),
    }
}

#[tauri::command]
pub async fn sync_v2_readiness(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::sync_v2::Readiness, String> {
    require_sync_v2_server(&app, &state).await?;
    crate::sync_v2::readiness(&app, &state).await
}

#[tauri::command]
pub async fn sync_v2_activate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::sync_v2::Activation, String> {
    require_sync_v2_server(&app, &state).await?;
    crate::sync_v2::activate(&app, &state).await
}

#[tauri::command]
pub async fn sync_v2_resolve_conflict(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conflict_id: String,
    resolution: String,
    custom_value: Option<serde_json::Value>,
) -> Result<(), String> {
    require_sync_v2_server(&app, &state).await?;
    crate::sync_v2::resolve_conflict(&app, &state, &conflict_id, &resolution, custom_value).await?;
    state.request_background_sync(
        app,
        crate::sync_coordinator::SyncTrigger::RealtimeHeadChanged,
    );
    Ok(())
}

/// Mint a fresh group key, retiring the old one. Returns the new key,
/// display-formatted.
#[tauri::command]
pub async fn sync_reset_group_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state.require_server_schema(&app).await?;
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let key = sync::reset_group_key_fn(&client, &cfg, &session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(sync::format_group_key(&key))
}

/// Set a group member's role (admin or coder). Admin-only.
#[tauri::command]
pub async fn sync_set_member_role(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user_id: String,
    role: String,
) -> Result<(), String> {
    state.require_server_schema(&app).await?;
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    sync::set_member_role_fn(&client, &cfg, &session, &user_id, &role)
        .await
        .map_err(|e| e.to_string())
}

/// Remove a member from the group. Admin-only.
#[tauri::command]
pub async fn sync_remove_member(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user_id: String,
) -> Result<(), String> {
    state.require_server_schema(&app).await?;
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    sync::remove_member_fn(&client, &cfg, &session, &user_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete the entire group. Admin-only. Requires exact confirmation title.
/// Works with no project open when `project_id` is provided (e.g. from home screen).
#[tauri::command]
pub async fn sync_delete_group(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    confirm_title: String,
    project_id: Option<String>,
) -> Result<(), String> {
    state.require_server_schema(&app).await?;

    let current_project_id = state
        .with_conn(|conn| sync::ensure_project_id(conn).map_err(|e| e.to_string()))
        .ok();

    let target_id = resolve_target_project_id(project_id, current_project_id.as_deref())?;
    let is_open_project = current_project_id.as_deref() == Some(&target_id);

    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let cfg = SyncConfig {
        project_id: target_id.clone(),
        ..sync_server_config(&app)?
    };

    if is_open_project {
        let journal_val = serde_json::json!({
            "operation": "delete",
            "project_id": target_id,
        });
        state.with_conn(|conn| {
            sync::set_state(conn, sync::KEY_PENDING_UNBIND, &journal_val.to_string())
                .map_err(|e| e.to_string())
        })?;
    }

    let res = sync::delete_group_fn(&client, &cfg, &session, &confirm_title).await;

    if is_open_project {
        let outcome = state.with_conn(|conn| {
            sync::apply_delete_result(conn, res.as_ref().map(|_| ())).map_err(|e| e.to_string())
        })?;

        match outcome {
            sync::LocalDeleteOutcome::UnboundCleanly => {
                state.realtime.stop(&app);
                Ok(())
            }
            sync::LocalDeleteOutcome::DeleteRejected(msg) => Err(msg),
            sync::LocalDeleteOutcome::DeletePendingRetry(msg) => {
                Err(format!("Could not reach server: {msg}"))
            }
        }
    } else {
        if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
            if let Some(target_recent) = recents
                .iter()
                .find(|r| r.group_id.as_deref() == Some(&target_id))
            {
                if let Ok(conn) = db::open_project(&target_recent.path) {
                    let _ = sync::apply_delete_result(&conn, res.as_ref().map(|_| ()));
                }
                let _ = crate::app_data::record_recent_project(
                    &app,
                    crate::models::RecordRecentProjectInput {
                        path: target_recent.path.clone(),
                        title: target_recent.title.clone(),
                        group_id: None,
                        group_title: None,
                        coder_name: None,
                        former_group_id: Some(target_id.clone()),
                        former_group_title: target_recent.group_title.clone(),
                        readiness: None,
                    },
                );
            }
        }
        res.map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn sync_leave_group(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<(), String> {
    let current_project_id = state
        .with_conn(|conn| sync::ensure_project_id(conn).map_err(|e| e.to_string()))
        .ok();

    let target_id = resolve_target_project_id(project_id, current_project_id.as_deref())?;

    let is_open_project = current_project_id.as_deref() == Some(&target_id);

    if is_open_project {
        let journal_val = serde_json::json!({
            "operation": "leave",
            "project_id": target_id,
        });
        state.with_conn(|conn| {
            sync::set_state(conn, sync::KEY_PENDING_UNBIND, &journal_val.to_string())
                .map_err(|e| e.to_string())
        })?;
    }

    if let (Ok(cfg), Ok(session), Ok(client)) = (
        sync_server_config(&app),
        state.sync_session(),
        sync::client(),
    ) {
        if let Ok(info) = sync::group_info_fetch(&client, &cfg, &session).await {
            let my_coder_name = state
                .with_conn(|conn| {
                    sync::get_state(conn, sync::KEY_CODER_NAME).map_err(|e| e.to_string())
                })
                .ok()
                .flatten()
                .or_else(|| {
                    info.members
                        .iter()
                        .find(|m| m.user_id.as_deref() == Some(&session.user_id))
                        .map(|m| m.coder_name.clone())
                })
                .unwrap_or_else(|| "Me".into());
            let _ = crate::app_data::record_left_study(
                &app,
                crate::models::LeftStudy {
                    project_id: target_id.clone(),
                    title: info.title,
                    group_key: info.group_key,
                    coder_name: my_coder_name,
                    left_at: chrono::Utc::now().to_rfc3339(),
                },
            );
        }
    }

    let server_res = match (
        sync_server_config(&app),
        state.sync_session(),
        sync::client(),
    ) {
        (Ok(cfg), Ok(session), Ok(client)) => {
            sync::leave_group_fn(&client, &cfg, &session, &target_id).await
        }
        _ => Err(sync::SyncError::Network(
            "Could not configure sync client".into(),
        )),
    };

    if is_open_project {
        let outcome = state.with_conn(|conn| {
            sync::apply_leave_result(conn, server_res.as_ref().map(|_| ()), &target_id)
                .map_err(|e| e.to_string())
        })?;
        match outcome {
            sync::LocalLeaveOutcome::UnboundCleanly
            | sync::LocalLeaveOutcome::UnboundWithPendingReconcile => {
                state.realtime.stop(&app);
                Ok(())
            }
            sync::LocalLeaveOutcome::UnboundServerRejected(msg) => {
                state.realtime.stop(&app);
                Err(format!("Could not remove membership on server: {msg}"))
            }
        }
    } else {
        if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
            if let Some(target_recent) = recents
                .iter()
                .find(|r| r.group_id.as_deref() == Some(&target_id))
            {
                if let Ok(conn) = db::open_project(&target_recent.path) {
                    let _ = sync::apply_leave_result(
                        &conn,
                        server_res.as_ref().map(|_| ()),
                        &target_id,
                    );
                }
                let _ = crate::app_data::record_recent_project(
                    &app,
                    crate::models::RecordRecentProjectInput {
                        path: target_recent.path.clone(),
                        title: target_recent.title.clone(),
                        group_id: None,
                        group_title: None,
                        coder_name: None,
                        former_group_id: Some(target_id.clone()),
                        former_group_title: target_recent.group_title.clone(),
                        readiness: None,
                    },
                );
            }
        }
        server_res.map_err(|e| e.to_string())
    }
}

/// Detach/unbind a study locally without touching the server.
///
/// Stops syncing on this computer, resets sync_dirty and cursors, and marks the
/// study unbound. Works completely offline; requires no server schema or network.
#[tauri::command]
pub fn sync_detach_local(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<(), String> {
    let current_project_id = state
        .with_conn(|conn| sync::ensure_project_id(conn).map_err(|e| e.to_string()))
        .ok();

    let target_id = resolve_target_project_id(project_id, current_project_id.as_deref())?;

    let is_open_project = current_project_id.as_deref() == Some(&target_id);
    if is_open_project {
        state.with_conn(|conn| sync::unbind_from_group(conn).map_err(|e| e.to_string()))?;
        state.realtime.stop(&app);
        if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
            for r in recents {
                if r.group_id.as_deref() == Some(&target_id) {
                    let _ = crate::app_data::record_recent_project(
                        &app,
                        crate::models::RecordRecentProjectInput {
                            path: r.path,
                            title: r.title,
                            group_id: None,
                            group_title: None,
                            coder_name: None,
                            former_group_id: Some(target_id.clone()),
                            former_group_title: r.group_title.clone(),
                            readiness: None,
                        },
                    );
                }
            }
        }
        Ok(())
    } else {
        if let Ok(recents) = crate::app_data::list_recent_projects(&app) {
            if let Some(target_recent) = recents
                .iter()
                .find(|r| r.group_id.as_deref() == Some(&target_id))
            {
                let conn = db::open_project(&target_recent.path).map_err(|e| e.to_string())?;
                sync::unbind_from_group(&conn).map_err(|e| e.to_string())?;
                let _ = crate::app_data::record_recent_project(
                    &app,
                    crate::models::RecordRecentProjectInput {
                        path: target_recent.path.clone(),
                        title: target_recent.title.clone(),
                        group_id: None,
                        group_title: None,
                        coder_name: None,
                        former_group_id: Some(target_id.clone()),
                        former_group_title: target_recent.group_title.clone(),
                        readiness: None,
                    },
                );
                return Ok(());
            }
        }
        Err(format!("Could not find project with id '{target_id}'"))
    }
}

/// Reconciles an in-flight unbind operation recorded in the intent journal.
#[tauri::command]
pub async fn sync_reconcile_pending_unbind(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let pending = state.with_conn(|conn| {
        sync::get_state(conn, sync::KEY_PENDING_UNBIND).map_err(|e| e.to_string())
    })?;
    let pending_str = match pending {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Ok(()),
    };

    let parsed: serde_json::Value = serde_json::from_str(&pending_str).unwrap_or_default();
    let target_id = parsed
        .get("project_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let cfg = sync_server_config(&app)?;
    let session = match state.sync_session() {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let client = sync::client().map_err(|e| e.to_string())?;

    let memberships = sync::list_memberships(&client, &cfg, &session).await;
    let outcome = state.with_conn(|conn| {
        sync::apply_reconcile_result(conn, memberships.as_deref(), target_id)
            .map_err(|e| e.to_string())
    })?;

    if outcome == sync::LocalReconcileOutcome::UnboundAndDrained {
        state.realtime.stop(&app);
    }
    Ok(())
}

#[tauri::command]
pub fn project_deletion_summary(
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectDeletionSummary, String> {
    let recovery_guard = state.recovery_db.lock().ok();
    let recovery_conn = recovery_guard.as_ref().and_then(|g| g.as_ref());
    db::get_project_deletion_summary_with_recovery(&path, recovery_conn)
}

#[tauri::command]
pub fn delete_project_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let target = Path::new(&path);
    let is_current = if let Ok(current) = state.project_path_str() {
        let current_path = Path::new(&current);
        current_path == target
            || current_path
                .canonicalize()
                .ok()
                .zip(target.canonicalize().ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false)
    } else {
        false
    };
    if is_current {
        close_project(app, state)?;
    }
    db::delete_project_folder_impl(&path)
}

#[tauri::command]
pub fn inspect_join_target(
    parent_dir: String,
    slug: String,
    project_id: String,
) -> Result<crate::models::JoinTargetVerdict, String> {
    db::inspect_join_target(&parent_dir, &slug, &project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_left_studies(app: tauri::AppHandle) -> Result<Vec<crate::models::LeftStudy>, String> {
    app_data::list_left_studies(&app)
}

#[tauri::command]
pub fn resolve_study_location(
    app: tauri::AppHandle,
    path: String,
) -> Result<crate::models::StudyLocation, String> {
    Ok(crate::location::resolve_study_location(&app, &path))
}

#[tauri::command]
pub fn auto_relink_study(
    app: tauri::AppHandle,
    path: String,
    expected_project_id: Option<String>,
) -> Result<crate::models::RelinkOutcome, String> {
    Ok(crate::location::auto_relink_study(
        &app,
        &path,
        expected_project_id.as_deref(),
    ))
}

#[tauri::command]
pub fn study_readiness(path: String) -> Result<crate::models::StudyReadiness, String> {
    Ok(crate::readiness::study_readiness(&path))
}

#[tauri::command]
pub fn check_open_marker(path: String) -> Result<crate::open_marker::OpenMarkerStatus, String> {
    let machine_name = crate::open_marker::get_machine_name();
    Ok(crate::open_marker::check_marker(
        Path::new(&path),
        &machine_name,
    ))
}

#[tauri::command]
pub fn heartbeat_open_marker(state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(p) = state.project_path_str() {
        crate::open_marker::heartbeat_marker(Path::new(&p))?;
    }
    Ok(())
}

/// Change the name the signed-in member files under in this group.
///
/// The server rewrites their coded rows to the new name in the same
/// transaction; this side mirrors the rename into the local coder list so the
/// picker and the group roster tell the same story.
#[tauri::command]
pub async fn sync_set_my_coder_name(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    coder_name: String,
) -> Result<sync::RenamedSelf, String> {
    state.require_server_schema(&app).await?;
    let cfg = state.project_sync_config(&app)?;
    let session = state.sync_session()?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let renamed = sync::set_my_coder_name_fn(&client, &cfg, &session, coder_name.trim())
        .await
        .map_err(|e| e.to_string())?;
    state.with_conn(|conn| {
        db::rename_project_coder(conn, &renamed.previous_name, &renamed.coder_name)
            .map_err(|e| e.to_string())
    })?;
    Ok(renamed)
}

/// The whole network half of a sync run, in one place so it can be retried
/// wholesale after a token refresh.
///
/// The corpus check comes first and gates everything: if the two machines are
/// not holding the same transcripts, segment ids do not refer to the same
/// passages and nothing may be exchanged in either direction.
///
/// The exception is a machine holding *no* transcripts — a coder who has just
/// joined. It pulls, so the app can name the transcripts they still need, and
/// pushes nothing, so it cannot pin an empty corpus onto the study. See
/// `sync::plan_corpus`.
/// Push dirty rows to the server with one token refresh on 401.
async fn push_with_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
    client: &reqwest::Client,
    cfg: &SyncConfig,
    batch: &sync::PushBatch,
    local_hash: &str,
) -> Result<bool, sync::SyncError> {
    let session = state.sync_session().map_err(sync::SyncError::Db)?;
    let corpus = sync::reconcile_corpus(client, cfg, &session, local_hash).await;
    let corpus = match corpus {
        Err(sync::SyncError::Unauthorized) => {
            let renewed = sync::refresh(client, cfg, &session).await?;
            state.update_sync_session(app, renewed.clone())?;
            sync::reconcile_corpus(client, cfg, &renewed, local_hash).await?
        }
        other => other?,
    };

    let pushed = corpus == sync::CorpusState::Ready && !batch.is_empty();
    if pushed {
        let session = state.sync_session().map_err(sync::SyncError::Db)?;
        match sync::push(client, cfg, &session, batch).await {
            Err(sync::SyncError::Unauthorized) => {
                let renewed = sync::refresh(client, cfg, &session).await?;
                state.update_sync_session(app, renewed.clone())?;
                sync::push(client, cfg, &renewed, batch).await?;
            }
            other => other?,
        }
    }
    Ok(pushed)
}

/// Pull updated rows from the server with one token refresh on 401.
async fn pull_with_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
    client: &reqwest::Client,
    cfg: &SyncConfig,
    coded_cursor: Option<&str>,
    codebook_cursor: Option<&str>,
    interview_cursor: Option<&str>,
) -> Result<sync::PullBatch, sync::SyncError> {
    let session = state.sync_session().map_err(sync::SyncError::Db)?;
    match sync::pull(
        client,
        cfg,
        &session,
        coded_cursor,
        codebook_cursor,
        interview_cursor,
    )
    .await
    {
        Err(sync::SyncError::Unauthorized) => {
            let renewed = sync::refresh(client, cfg, &session).await?;
            state.update_sync_session(app, renewed.clone())?;
            sync::pull(
                client,
                cfg,
                &renewed,
                coded_cursor,
                codebook_cursor,
                interview_cursor,
            )
            .await
        }
        other => other,
    }
}

/// One full sync attempt: push, mark pushed, pull, apply.
///
/// Factored out of `sync_now` because of the duplicate-coding rescue below:
/// when a push collides with the server's (passage, coder, span) constraint,
/// the fix is to reconcile locally and then run this whole thing once more.
///
/// Errors stay as [`sync::SyncError`] rather than flattening to strings so the
/// caller can still tell a rescuable conflict from everything else.
async fn sync_attempt(
    app: &tauri::AppHandle,
    state: &AppState,
    target: &SyncRunTarget,
    client: &reqwest::Client,
    cfg: &SyncConfig,
    batch: &sync::PushBatch,
) -> Result<SyncOutcome, sync::SyncError> {
    // ── Phase 1: the batch comes from the caller; cursors and the
    // corpus hash are read here, then the DB guard is dropped.
    let (local_hash, coded_cursor, codebook_cursor, interview_cursor) = state
        .with_sync_target(target, |conn| {
            let hash = sync::corpus_hash(conn).map_err(|e| e.to_string())?;
            let coded = sync::get_state(conn, sync::KEY_CODED_CURSOR).map_err(|e| e.to_string())?;
            let codebook =
                sync::get_state(conn, sync::KEY_CODEBOOK_CURSOR).map_err(|e| e.to_string())?;
            let interview =
                sync::get_state(conn, sync::KEY_INTERVIEW_CURSOR).map_err(|e| e.to_string())?;
            Ok((hash, coded, codebook, interview))
        })
        .map_err(sync::SyncError::Db)?;

    // ── Phase 2: Push first, mark pushed, then pull.
    //
    // Push and pull are sequential on purpose. Overlapping them means the pull's
    // response predates the push's effect, and applying it writes stale remote state
    // over work that has just been marked clean — v0.23–v0.24 lost coding exactly
    // this way. One extra round trip is the price of the pulled rows meaning what
    // they say.
    let pushed = push_with_refresh(app, state, client, cfg, batch, &local_hash).await?;

    if pushed {
        state
            .with_sync_target(target, |conn| {
                sync::mark_pushed(conn, batch).map_err(|e| e.to_string())
            })
            .map_err(sync::SyncError::Db)?;
    }

    let pulled = pull_with_refresh(
        app,
        state,
        client,
        cfg,
        coded_cursor.as_deref(),
        codebook_cursor.as_deref(),
        interview_cursor.as_deref(),
    )
    .await?;

    // Task 8: When pull is empty and idle for > 60s, check if membership or group was removed
    let empty_pull =
        pulled.coded.is_empty() && pulled.codes.is_empty() && pulled.interviews.is_empty();
    if empty_pull {
        let last_synced = state
            .with_sync_target(target, |conn| {
                sync::get_state(conn, sync::KEY_LAST_SYNCED).map_err(|e| e.to_string())
            })
            .unwrap_or(None);

        let should_check_membership = match last_synced {
            None => true,
            Some(ts) => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&ts) {
                    chrono::Utc::now()
                        .signed_duration_since(dt.with_timezone(&chrono::Utc))
                        .num_seconds()
                        > 60
                } else {
                    true
                }
            }
        };

        if should_check_membership {
            if let Ok(session) = state.sync_session() {
                if let Ok(Some(reason)) = sync::check_access_lost_fn(client, cfg, &session).await {
                    let target_path_str = target.project_path.to_string_lossy().to_string();

                    let _ = state.with_sync_target(target, |conn| {
                        sync::unbind_from_group(conn).map_err(|e| e.to_string())
                    });

                    if let Ok(recents) = crate::app_data::list_recent_projects(app) {
                        if let Some(r) = recents.iter().find(|r| r.path == target_path_str) {
                            let _ = crate::app_data::record_recent_project(
                                app,
                                crate::models::RecordRecentProjectInput {
                                    path: r.path.clone(),
                                    title: r.title.clone(),
                                    group_id: None,
                                    group_title: None,
                                    coder_name: None,
                                    former_group_id: Some(cfg.project_id.clone()),
                                    former_group_title: r.group_title.clone(),
                                    readiness: None,
                                },
                            );
                        }
                    }
                    state.realtime.stop(app);
                    return Err(sync::SyncError::Network(reason.user_message().to_string()));
                }
            }
        }
    }

    // ── Phase 3: apply, in one transaction.
    let synced_at = db::now_iso();
    let mut outcome = state
        .with_sync_target(target, |conn| {
            let mut outcome = sync::apply_pull(conn, &pulled).map_err(|e| e.to_string())?;

            if let Some(cursor) = sync::next_cursor(
                pulled.coded.iter().map(|r| &r.updated_at),
                &outcome.coded_receipt,
            ) {
                sync::set_state(conn, sync::KEY_CODED_CURSOR, &cursor)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(cursor) = sync::next_cursor(
                pulled.codes.iter().map(|r| &r.updated_at),
                &outcome.codes_receipt,
            ) {
                sync::set_state(conn, sync::KEY_CODEBOOK_CURSOR, &cursor)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(cursor) = sync::next_cursor(
                pulled.interviews.iter().map(|r| &r.updated_at),
                &outcome.interviews_receipt,
            ) {
                sync::set_state(conn, sync::KEY_INTERVIEW_CURSOR, &cursor)
                    .map_err(|e| e.to_string())?;
            }
            sync::set_state(conn, sync::KEY_LAST_SYNCED, &synced_at).map_err(|e| e.to_string())?;

            outcome.pushed_coded = if pushed { batch.coded.len() } else { 0 };
            outcome.pushed_codes = if pushed { batch.codes.len() } else { 0 };
            outcome.missing_transcripts =
                sync::missing_transcripts(conn).map_err(|e| e.to_string())?;
            Ok(outcome)
        })
        .map_err(sync::SyncError::Db)?;

    outcome.synced_at = synced_at;
    Ok(outcome)
}

/// Push this machine's coding, then pull the other coder's.
///
/// Push first so that a run which fails halfway has still got this machine's
/// work off the laptop — the direction that loses data if it never happens.
///
/// One coder working from two machines is an ordinary case, and it produces a
/// specific failure: each machine mints its own id for the same (passage,
/// coder, span), and the server's unique constraint rejects the second one.
/// The rescue below answers it — pull the server's rows for the names this
/// batch touched (which re-keys the local twins onto the server's ids through
/// `apply_pull`), then run the whole attempt once more. A collision that
/// survives the rescue is surfaced; some things should be loud.
pub(crate) async fn run_sync_pass(
    app: &tauri::AppHandle,
    state: &AppState,
    generation: u64,
) -> Result<SyncOutcome, String> {
    let target = state.sync_run_target(generation)?;
    let bound = state.with_sync_target(&target, |conn| {
        sync::is_bound(conn).map_err(|e| e.to_string())
    })?;
    if !bound {
        return Err("This project is not in a group yet.".into());
    }
    let cfg = state.project_sync_config_for_target(app, &target)?;
    let client = sync::client().map_err(|e| e.to_string())?;

    let server_supports_v2 = matches!(
        state.get_or_probe_server_schema(app, &cfg).await,
        Some(version) if version >= 10
    );
    if server_supports_v2 {
        let prior_protocol = state.with_sync_target(&target, |conn| {
            db::local_sync_protocol(conn).map_err(|error| error.to_string())
        })?;
        let registration = crate::sync_v2::register_device(app, state, &target, &client, &cfg)
            .await
            .map_err(|error| error.to_string())?;
        if registration.protocol == 2 {
            if prior_protocol != 2 {
                state.with_sync_target(&target, |conn| {
                    db::salvage_legacy_v1_state_for_v2(conn).map_err(|error| error.to_string())
                })?;
                state.maybe_start_realtime(app);
            }
            let force_repair = state.with_sync_target(&target, |conn| {
                Ok(db::sync_state_value(conn, "sync_v2_force_repair")
                    .map_err(|error| error.to_string())?
                    .as_deref()
                    == Some("1"))
            })?;
            let result = crate::sync_v2::run_registered(
                app,
                state,
                &target,
                &client,
                &cfg,
                registration,
                force_repair,
            )
            .await;
            if result.is_ok() && force_repair {
                let _ = state.with_sync_target(&target, |conn| {
                    db::set_sync_state_value(conn, "sync_v2_force_repair", "0")
                        .map_err(|error| error.to_string())
                });
            }
            if result.is_ok() && state.sync_coordinator.is_active(generation) {
                let _ = app.emit("sync://workspace-changed", ());
            }
            return result;
        }
    }

    // ── Phase 1: the batch, then let go of the DB.
    let batch = state.with_sync_target(&target, |conn| {
        sync::collect_push_batch(conn, &cfg.project_id).map_err(|e| e.to_string())
    })?;

    let result = match sync_attempt(app, state, &target, &client, &cfg, &batch).await {
        Ok(outcome) => Ok(outcome),
        Err(sync::SyncError::DuplicateCoding(_)) if !batch.coded.is_empty() => {
            let mut names: Vec<String> = batch.coded.iter().map(|c| c.coder_name.clone()).collect();
            names.sort();
            names.dedup();

            let rows = {
                let session = state.sync_session()?;
                match sync::fetch_coded_for_coders(&client, &cfg, &session, &names).await {
                    Err(sync::SyncError::Unauthorized) => {
                        let renewed = sync::refresh(&client, &cfg, &session)
                            .await
                            .map_err(|e| e.to_string())?;
                        sync::remember_session(app, &renewed);
                        *state.sync_session.lock().map_err(|e| e.to_string())? =
                            Some(renewed.clone());
                        sync::fetch_coded_for_coders(&client, &cfg, &renewed, &names)
                            .await
                            .map_err(|e| e.to_string())?
                    }
                    other => other.map_err(|e| e.to_string())?,
                }
            };

            // Reconcile under the DB guard, re-collect (the reconciled rows now
            // carry the server's ids), and try the whole thing once more.
            state.with_sync_target(&target, |conn| {
                sync::apply_pull(
                    conn,
                    &sync::PullBatch {
                        coded: rows,
                        codes: Vec::new(),
                        interviews: Vec::new(),
                        truncated: false,
                    },
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })?;
            let retry_batch = state.with_sync_target(&target, |conn| {
                sync::collect_push_batch(conn, &cfg.project_id).map_err(|e| e.to_string())
            })?;
            sync_attempt(app, state, &target, &client, &cfg, &retry_batch)
                .await
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    let outcome = result?;
    if server_supports_v2 {
        if let Ok(registration) =
            crate::sync_v2::register_device(app, state, &target, &client, &cfg).await
        {
            if registration.protocol == 2 {
                state.with_sync_target(&target, |conn| {
                    db::salvage_legacy_v1_state_for_v2(conn).map_err(|error| error.to_string())
                })?;
                state.maybe_start_realtime(app);
                let cutover = crate::sync_v2::run_registered(
                    app,
                    state,
                    &target,
                    &client,
                    &cfg,
                    registration,
                    false,
                )
                .await;
                if cutover.is_ok() && state.sync_coordinator.is_active(generation) {
                    let _ = app.emit("sync://workspace-changed", ());
                }
                return cutover;
            }
        }
    }
    if state.sync_coordinator.is_active(generation) {
        let _ = app.emit("sync://workspace-changed", ());
    }
    Ok(outcome)
}

#[tauri::command]
pub async fn sync_now(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncOutcome, String> {
    state
        .request_sync(app, crate::sync_coordinator::SyncTrigger::Manual)
        .await
}

#[tauri::command]
pub async fn sync_deep_verify(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<sync::DiagnosticsReport, String> {
    let bound = state.with_conn(|conn| sync::is_bound(conn).map_err(|e| e.to_string()))?;
    if !bound {
        return Err("This project is not in a group yet.".into());
    }
    let cfg = state.project_sync_config(&app)?;
    let client = sync::client().map_err(|e| e.to_string())?;
    let session = state.sync_session()?;

    let local = state.with_conn(|conn| {
        sync::LocalDiagnosticsState::collect(conn, &cfg.project_id).map_err(|e| e.to_string())
    })?;

    sync::deep_verify(&local, &client, &cfg, &session)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_repair(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncOutcome, String> {
    let protocol =
        state.with_conn(|conn| db::local_sync_protocol(conn).map_err(|error| error.to_string()))?;
    if protocol == 2 {
        state.with_conn(|conn| {
            db::set_sync_state_value(conn, "sync_v2_force_repair", "1")
                .map_err(|error| error.to_string())
        })?;
        return state
            .request_sync(app, crate::sync_coordinator::SyncTrigger::Repair)
            .await;
    }
    state.with_conn(|conn| sync::reset_cursors_for_repair(conn).map_err(|e| e.to_string()))?;
    sync_now(app, state).await
}

#[tauri::command]
pub async fn sync_diagnostics_dump(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let v2 = state.with_conn(|conn| {
        if db::local_sync_protocol(conn).map_err(|error| error.to_string())? != 2 {
            return Ok(None);
        }
        let read_i64 = |key: &str| -> Result<i64, String> {
            Ok(db::sync_state_value(conn, key)
                .map_err(|error| error.to_string())?
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0))
        };
        let oldest = conn
            .query_row("SELECT MIN(created_at) FROM sync_outbox", [], |row| {
                row.get::<_, Option<String>>(0)
            })
            .map_err(|error| error.to_string())?;
        Ok(Some(V2DiagnosticsSnapshot {
            generation_suffix: safe_identifier_suffix(
                db::sync_state_value(conn, "sync_generation").map_err(|error| error.to_string())?,
            ),
            device_id_suffix: safe_identifier_suffix(
                db::sync_state_value(conn, "sync_device_id").map_err(|error| error.to_string())?,
            ),
            local_sequence: read_i64("sync_server_seq")?,
            observed_head: read_i64("sync_observed_head")?,
            outbox_count: conn
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
                .map_err(|error| error.to_string())?,
            oldest_outbox_age_seconds: elapsed_seconds_from_rfc3339(oldest),
            blocked_outbox_count: conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE blocked_reason IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?,
            unresolved_conflict_count: conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_conflicts WHERE status = 'unresolved'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?,
            last_realtime_at: db::sync_state_value(conn, "sync_last_realtime_at")
                .map_err(|error| error.to_string())?
                .filter(|value| !value.is_empty()),
            last_success_at: db::sync_state_value(conn, "sync_last_success_at")
                .map_err(|error| error.to_string())?
                .filter(|value| !value.is_empty()),
            code_count: conn
                .query_row(
                    "SELECT COUNT(*) FROM codebook WHERE deleted = 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?,
            interview_count: conn
                .query_row(
                    "SELECT COUNT(*) FROM interviews WHERE deleted = 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?,
            assignment_count: conn
                .query_row(
                    "SELECT COUNT(*) FROM coding_assignments WHERE present != 0",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?,
        }))
    })?;
    if let Some(snapshot) = v2 {
        return Ok(format_v2_diagnostics(
            &snapshot,
            &state.sync_coordinator.health(),
        ));
    }
    let report = sync_deep_verify(app, state).await?;
    Ok(sync::diagnostics_summary_text(&report))
}

#[tauri::command]
pub async fn generate_diagnostic_report(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Header
    let header = format!(
        "=== Fleuron Diagnostic Report ===\nGenerated: {}\nNotice: This report contains no transcript text, participant labels, or code names.",
        now
    );

    // 2. Application
    let app_version = get_app_version(app.clone())?;
    let app_section = format!(
        "--- Application ---\nName: {}\nVersion: {}\nBuild commit: {}\nSource URL: {}\nOS: {}\nArch: {}",
        app_version.name,
        app_version.version,
        app_version.build_commit.unwrap_or_else(|| "unknown".into()),
        app_version.source_url.unwrap_or_else(|| "unknown".into()),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    // 3. Install
    let install_section = match std::env::current_exe() {
        Ok(exe_path) => {
            let exe_dir = exe_path.parent().unwrap_or(&exe_path);
            let redacted = crate::diagnostics::redact_path(exe_dir);
            let per_user = {
                let s = exe_dir.to_string_lossy();
                #[cfg(windows)]
                {
                    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
                    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
                    (!localappdata.is_empty() && s.starts_with(&localappdata))
                        || (!userprofile.is_empty() && s.starts_with(&userprofile))
                }
                #[cfg(target_os = "macos")]
                {
                    let home = std::env::var("HOME").unwrap_or_default();
                    s.starts_with("/Applications") || (!home.is_empty() && s.starts_with(&home))
                }
                #[cfg(not(any(windows, target_os = "macos")))]
                {
                    true
                }
            };
            let (_, poisoned_status) = crate::diagnostics::check_poisoned_install(exe_dir);
            format!(
                "--- Install ---\nExecutable directory: {}\nPer-user install: {}\nPoisoned nested directory: {}",
                redacted, per_user, poisoned_status
            )
        }
        Err(e) => format!("--- Install ---\nunavailable: {}", e),
    };

    // 4. Library
    let library_section = match crate::app_data::projects_library_dir(&app) {
        Ok(lib_dir) => {
            let redacted = crate::diagnostics::redact_path(&lib_dir);
            let exists = lib_dir.exists();
            match crate::diagnostics::scan_library_counts(&lib_dir) {
                Ok((count, writable)) => format!(
                    "--- Library ---\nLibrary directory: {}\nExists: {}\nWritable: {}\nProject folders count: {}",
                    redacted, exists, writable, count
                ),
                Err(err) => format!(
                    "--- Library ---\nLibrary directory: {}\nExists: {}\nunavailable: {}",
                    redacted, exists, err
                ),
            }
        }
        Err(e) => format!("--- Library ---\nunavailable: {}", e),
    };

    // 5. Sync
    let sync_content = match sync_diagnostics_dump(app.clone(), state.clone()).await {
        Ok(dump) => dump,
        Err(e) => format!("unavailable: {}", e),
    };
    let sync_section = format!("--- Sync ---\n{}", sync_content.trim());

    // 6. Storage
    let storage_section = {
        let guard = state.project_path.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(p) => {
                let diag = crate::diagnostics::check_storage_status(p);
                format!(
                    "--- Storage ---\n{}",
                    crate::diagnostics::format_storage_diagnostics(&diag)
                )
            }
            None => "--- Storage ---\nunavailable: No project is open".to_string(),
        }
    };

    // 7. Updater
    let updater_section = {
        let update_status = state.update_coordinator.status();
        let status_summary = match update_status.phase {
            crate::update_coordinator::UpdatePhase::Idle => "idle",
            crate::update_coordinator::UpdatePhase::Checking => "checking",
            crate::update_coordinator::UpdatePhase::Available => "available",
            crate::update_coordinator::UpdatePhase::Downloading => "downloading",
            crate::update_coordinator::UpdatePhase::ReadyToInstall => "ready-to-install",
            crate::update_coordinator::UpdatePhase::Preparing => "preparing",
            crate::update_coordinator::UpdatePhase::Installing => "installing",
            crate::update_coordinator::UpdatePhase::Failed => "failed",
        };

        let residue = match crate::app_data::app_data_dir(&app) {
            Ok(dir) => {
                let (_, status) = crate::diagnostics::check_staged_updater_residue(&dir);
                status
            }
            Err(e) => format!("unavailable: {}", e),
        };

        format!(
            "--- Updater ---\nStatus: {}\nStaged update residue: {}",
            status_summary, residue
        )
    };

    // 8. Crashes
    let crashes_section = match crate::crash_report::read_crash_log_file(&app) {
        Ok(raw_log) => format!(
            "--- Crashes ---\n{}",
            crate::diagnostics::parse_and_redact_crash_log(&raw_log).trim()
        ),
        Err(e) => format!("--- Crashes ---\nunavailable: {}", e),
    };

    let report = [
        header,
        app_section,
        install_section,
        library_section,
        sync_section,
        storage_section,
        updater_section,
        crashes_section,
    ]
    .join("\n\n");

    Ok(report)
}

#[tauri::command]
pub fn read_crash_log(app: tauri::AppHandle) -> Result<String, String> {
    crate::crash_report::read_crash_log_file(&app)
}

#[tauri::command]
pub fn take_unclean_exit_notice() -> Result<bool, String> {
    Ok(crate::run_marker::take_unclean_exit_notice())
}

#[cfg(test)]
mod command_contract_tests {
    use super::*;
    use crate::models::AppPreferences;

    /// Commands that must call [`resolve_sync_server_config`], not
    /// [`AppState::project_sync_config`]. The v0.12.0 sign-in bug was one of
    /// these accidentally requiring an open project.
    const PRE_PROJECT_SYNC_COMMANDS: &[&str] = &[
        "sync_sign_in",
        "sync_restore_session",
        "sync_list_memberships",
        "sync_request_password_reset",
        "sync_join_group",
        "sync_redeem_invite",
        "sync_refresh_server_schema",
    ];

    #[test]
    fn with_conn_fails_when_no_project_is_open() {
        let state = AppState::new();
        let err = state.with_conn(|_| Ok(())).unwrap_err();
        assert_eq!(err, "No project is open");
    }

    #[test]
    fn project_sync_config_fails_without_open_project() {
        let state = AppState::new();
        let err = state
            .with_conn(|conn| sync::ensure_project_id(conn).map_err(|e| e.to_string()))
            .unwrap_err();
        assert_eq!(err, "No project is open");
    }

    #[test]
    fn resolve_sync_server_config_needs_no_open_project() {
        let prefs = AppPreferences {
            sync_url: Some("https://example.supabase.co".into()),
            sync_anon_key: Some("test-anon-key".into()),
            ..AppPreferences::default()
        };
        let cfg = resolve_sync_server_config(&prefs).expect("prefs carry a server");
        assert_eq!(cfg.url, "https://example.supabase.co");
        assert_eq!(cfg.anon_key, "test-anon-key");
        assert!(cfg.project_id.is_empty());
    }

    #[test]
    fn resolve_target_project_id_prefers_provided() {
        let res = resolve_target_project_id(Some("P123".into()), Some("OPEN")).unwrap();
        assert_eq!(res, "P123");
    }

    #[test]
    fn resolve_target_project_id_trims_empty_to_open() {
        let res = resolve_target_project_id(Some("  ".into()), Some("OPEN")).unwrap();
        assert_eq!(res, "OPEN");
    }

    #[test]
    fn resolve_target_project_id_none_open_none_provided_errors() {
        let err = resolve_target_project_id(None, None).unwrap_err();
        assert_eq!(err, "No project is open and no project id was provided.");
    }

    #[test]
    fn pre_project_sync_commands_are_documented() {
        assert!(
            PRE_PROJECT_SYNC_COMMANDS.contains(&"sync_sign_in"),
            "keep the regression list current"
        );
        assert_eq!(PRE_PROJECT_SYNC_COMMANDS.len(), 7);
    }

    #[test]
    fn v2_diagnostics_never_copy_content_sentinels() {
        let coordinator = crate::sync_coordinator::SyncCoordinator::new();
        let sentinels = [
            "TRANSCRIPT-SENTINEL-7e0d",
            "MEMO-SENTINEL-028c",
            "CODE-SENTINEL-5b71",
            "email@example.invalid",
            "/private/study/path",
            "token-sentinel-9c3f",
        ];
        let output = format_v2_diagnostics(
            &V2DiagnosticsSnapshot {
                generation_suffix: Some("a1b2c3d4".into()),
                device_id_suffix: Some("e5f60718".into()),
                local_sequence: 10,
                observed_head: 12,
                outbox_count: 2,
                oldest_outbox_age_seconds: Some(61),
                blocked_outbox_count: 0,
                unresolved_conflict_count: 1,
                last_realtime_at: Some("2026-08-23T12:00:00Z".into()),
                last_success_at: Some("2026-08-23T11:59:59Z".into()),
                code_count: 3,
                interview_count: 2,
                assignment_count: 5,
            },
            &coordinator.health(),
        );
        for sentinel in sentinels {
            assert!(!output.contains(sentinel));
        }
        assert!(output.contains("Generation suffix: a1b2c3d4"));
        assert!(output.contains("Outbox: 2"));
    }

    #[test]
    fn diagnostic_report_seeded_identifiers_are_redacted() {
        let temp = tempfile::tempdir().unwrap();
        let lib_dir = temp.path().join("P07-Grief-Study");
        std::fs::create_dir_all(&lib_dir).unwrap();
        let study_dir = lib_dir.join("P07-Interview-Study.fleuron");
        std::fs::create_dir_all(&study_dir).unwrap();
        std::fs::write(study_dir.join("project.db"), b"sqlite").unwrap();

        // 1. Library scan count only
        let (count, writable) = crate::diagnostics::scan_library_counts(&lib_dir).unwrap();
        assert_eq!(count, 1);
        assert!(writable);

        // 2. Storage status
        let storage = crate::diagnostics::check_storage_status(&study_dir);
        let formatted = crate::diagnostics::format_storage_diagnostics(&storage);

        assert!(!formatted.contains("P07"));
        assert!(!formatted.contains("Grief"));
        assert!(!formatted.contains("Interview-Study"));
        assert!(formatted.contains("<name>.fleuron"));
        assert!(formatted.contains("project.db"));
    }

    #[test]
    fn diagnostic_report_stranded_staging_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let study_dir = temp.path();
        std::fs::write(study_dir.join("project.db"), b"sqlite").unwrap();
        std::fs::write(study_dir.join(".staging-restore.db"), b"stranded").unwrap();

        let storage = crate::diagnostics::check_storage_status(study_dir);
        assert!(storage.stranded_staging_restore);
        let formatted = crate::diagnostics::format_storage_diagnostics(&storage);
        assert!(formatted.contains("detected (.staging-restore.db is stranded)"));
    }

    #[test]
    fn diagnostic_report_poisoned_install_detected() {
        let temp = tempfile::tempdir().unwrap();
        let exe_dir = temp.path();
        let (poisoned, status) = crate::diagnostics::check_poisoned_install(exe_dir);
        assert!(!poisoned);
        assert_eq!(status, "clean");

        let nested = exe_dir.join("Fleuron.exe");
        std::fs::create_dir(&nested).unwrap();
        let (poisoned, status) = crate::diagnostics::check_poisoned_install(exe_dir);
        assert!(poisoned);
        assert!(status.contains("directory"));
    }

    #[test]
    fn stale_epoch_rejected_after_switching_to_clone() {
        // Two opens of different folders (a clone carries identical coding
        // ids): the second rotation invalidates the first epoch, so a write
        // credentialed for the original fails closed on the clone.
        let state = AppState::new();
        *state.project_key.lock().unwrap() = Some("key-original".into());
        *state.workspace_epoch.lock().unwrap() = Some("epoch-1".into());
        assert!(state
            .check_workspace_under_transition("key-original", "epoch-1")
            .is_ok());
        // Switch to the clone: new key AND new epoch.
        *state.project_key.lock().unwrap() = Some("key-clone".into());
        *state.workspace_epoch.lock().unwrap() = Some("epoch-2".into());
        assert_eq!(
            state
                .check_workspace_under_transition("key-original", "epoch-1")
                .unwrap_err(),
            "STALE_WORKSPACE"
        );
        // Same key but rotated epoch (restore/reopen) also fails closed.
        assert_eq!(
            state
                .check_workspace_under_transition("key-clone", "epoch-1")
                .unwrap_err(),
            "STALE_WORKSPACE"
        );
        assert!(state
            .check_workspace_under_transition("key-clone", "epoch-2")
            .is_ok());
        // Close unbinds everything.
        state.clear_workspace_binding();
        assert_eq!(
            state
                .check_workspace_under_transition("key-clone", "epoch-2")
                .unwrap_err(),
            "STALE_WORKSPACE"
        );
    }

    #[test]
    fn update_approval_gates_only_when_work_is_open() {
        let state = AppState::new();
        // No project open: nothing to lose, no approval needed.
        assert!(super::enforce_update_departure_approval(None, &state, None).is_ok());
        // Project open but the store cannot be consulted: fail closed and
        // require an approval rather than installing over unknown drafts.
        *state.project_path.lock().unwrap() = Some(PathBuf::from("/studies/alpha"));
        *state.project_key.lock().unwrap() = Some("key-1".into());
        *state.workspace_epoch.lock().unwrap() = Some("epoch-1".into());
        assert!(super::enforce_update_departure_approval(None, &state, None).is_err());
    }
}
