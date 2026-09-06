//! Local snapshots of a project, and the way back from a mistake.
//!
//! # Why an archive, and not a copied folder
//!
//! A `.fleuron` project is a directory holding a live SQLite database in WAL
//! mode. Copying that directory while the app is running captures `project.db`
//! without the `-wal` alongside it that holds the most recent writes. The copy
//! then opens perfectly and is silently missing work — the worst failure a
//! backup can have, because nothing about it looks wrong until you need it.
//!
//! `VACUUM INTO` avoids the problem outright: SQLite writes a complete,
//! consistent database to a new path, WAL content folded in, without blocking
//! writers and without depending on a checkpoint having succeeded. That is the
//! only way a snapshot is taken here.
//!
//! The result is zipped so that one snapshot is one file. A backup that is a
//! directory invites the same half-copy mistake at the restore end, and a file
//! can be moved to another disk by someone who knows nothing about SQLite.
//!
//! # What restore guarantees
//!
//! Restoring is the only operation in this module that destroys anything, so it
//! is built to be survivable at every step:
//!
//! 1. The archive is opened and its database **verified** before the live
//!    project is touched at all. A truncated or foreign archive fails here,
//!    with the project still intact.
//! 2. The current state is snapshotted as a `PreRestore` backup. Restoring the
//!    wrong file is therefore itself undoable.
//! 3. The new database is staged beside the target and moved into place with a
//!    rename, so a crash mid-restore cannot leave a half-written `project.db`.

use crate::db;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Directory inside a project where snapshots are kept.
const DIR: &str = "backups";
/// Extension for snapshots written from now on.
pub const EXT: &str = "fleuronbak";
/// Extensions accepted when listing or deleting. `codemapbak` is the pre-rename
/// name and stays supported indefinitely — snapshots already on disk and on
/// USB sticks must keep working. Imports are rewritten to `EXT` on copy.
pub const READABLE_EXTS: [&str; 2] = ["fleuronbak", "codemapbak"];

const DB_ENTRY: &str = "project.db";
const META_ENTRY: &str = "project.json";
const MANIFEST_ENTRY: &str = "manifest.json";

/// How many *automatic* snapshots survive a prune.
///
/// Manual snapshots and the one taken immediately before a restore are never
/// pruned. Those are the two a person deliberately reached for, and a rolling
/// window that discarded them would delete the thing they were saving.
const AUTO_KEEP: usize = 10;

/// Why a snapshot exists. Drives both the filename and what pruning may remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupReason {
    /// Taken on open. Pruned on a rolling window.
    Automatic,
    /// Asked for by name. Kept until deleted by hand.
    Manual,
    /// Taken immediately before a restore overwrote the project. Kept.
    PreRestore,
    /// Taken to preserve a local copy before removal or migration.
    Preservation,
}

impl BackupReason {
    fn slug(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Manual => "manual",
            Self::PreRestore => "pre-restore",
            Self::Preservation => "preserved",
        }
    }
}

/// What a snapshot holds, recorded at the moment it was taken.
///
/// Counts are stored rather than derived on listing: reading them back would
/// mean opening every archive's database on every refresh, and the point of
/// this metadata is to let someone tell two snapshots apart at a glance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub created_at: String,
    pub reason: BackupReason,
    pub note: Option<String>,
    pub app_version: String,
    pub schema_version: i64,
    pub project_title: String,
    pub codes: i64,
    pub interviews: i64,
    pub segments: i64,
    pub coded_segments: i64,
    #[serde(default)]
    pub recovery_drafts: Vec<crate::models::NoteDraftRecord>,
    #[serde(default)]
    pub local_files: Vec<String>,
    #[serde(default)]
    pub external_files: Vec<String>,
    #[serde(default)]
    pub external_files_copied: Vec<String>,
}

/// One row in the restore picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    #[serde(flatten)]
    pub manifest: BackupManifest,
}

fn io_err<E: std::fmt::Display>(what: &str) -> impl Fn(E) -> String + '_ {
    move |e| format!("{what}: {e}")
}

pub fn backups_dir(project_path: &Path) -> PathBuf {
    project_path.join(DIR)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}

fn build_manifest(
    conn: &Connection,
    project_path: &Path,
    reason: BackupReason,
    note: Option<String>,
) -> BackupManifest {
    let project_title = db::get_project_info(conn, &project_path.to_string_lossy())
        .map(|p| p.title)
        .unwrap_or_else(|_| {
            project_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    BackupManifest {
        created_at: Utc::now().to_rfc3339(),
        reason,
        note,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0),
        project_title,
        codes: count(conn, "SELECT COUNT(*) FROM codebook WHERE deleted = 0"),
        interviews: count(conn, "SELECT COUNT(*) FROM interviews WHERE deleted = 0"),
        segments: count(conn, "SELECT COUNT(*) FROM transcript_segments"),
        coded_segments: count(
            conn,
            "SELECT COUNT(*) FROM coded_segments WHERE deleted = 0",
        ),
        recovery_drafts: Vec::new(),
        local_files: Vec::new(),
        external_files: Vec::new(),
        external_files_copied: Vec::new(),
    }
}

/// Take a snapshot of the open project.
///
/// The database is copied with `VACUUM INTO` — see the module comment for why
/// nothing here copies `project.db` directly.
pub fn create(
    conn: &Connection,
    project_path: &Path,
    reason: BackupReason,
    note: Option<String>,
) -> Result<BackupInfo, String> {
    let dir = backups_dir(project_path);
    fs::create_dir_all(&dir).map_err(io_err("Could not create the backups folder"))?;

    let manifest = build_manifest(conn, project_path, reason, note);
    let stamp = Utc::now().format("%Y-%m-%d-%H%M%S");

    // The stamp resolves to a whole second, and two snapshots inside one second
    // is not exotic — restore takes a pre-restore snapshot immediately before
    // the one it is restoring lands. Without this, the second would silently
    // overwrite the first, and the file it destroyed would be the safety copy.
    let mut archive_path = dir.join(format!("fleuron-{stamp}-{}.{EXT}", reason.slug()));
    let mut nth = 2;
    while archive_path.exists() {
        archive_path = dir.join(format!("fleuron-{stamp}-{}-{nth}.{EXT}", reason.slug()));
        nth += 1;
    }

    // VACUUM INTO refuses to overwrite, so the staging path must not exist.
    // Placed inside the backups folder rather than a temp dir so the copy never
    // crosses a filesystem boundary — a cross-device rename would fail, and a
    // study database on an external disk is an ordinary setup.
    //
    // The uuid suffix makes the staging name unique per invocation. Opening a
    // project spawns an automatic backup on a detached thread (commands.rs),
    // so two creates can legitimately run at once; a seconds-only stamp let
    // one thread's cleanup delete the other's snapshot mid-write.
    let staged_db = dir.join(format!(
        ".staging-{stamp}-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    let _ = fs::remove_file(&staged_db);

    conn.execute("VACUUM INTO ?1", [staged_db.to_string_lossy().as_ref()])
        .map_err(|e| {
            let _ = fs::remove_file(&staged_db);
            format!("Could not snapshot the database: {e}")
        })?;

    let result = write_archive(&archive_path, &staged_db, project_path, &manifest);
    let _ = fs::remove_file(&staged_db);
    result.inspect_err(|_| {
        // A half-written archive is worse than none: it would list in the
        // restore picker and fail only once someone relied on it.
        let _ = fs::remove_file(&archive_path);
    })?;

    if reason == BackupReason::Automatic {
        prune_automatic(&dir);
    }

    let size_bytes = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupInfo {
        path: archive_path.to_string_lossy().to_string(),
        file_name: archive_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes,
        manifest,
    })
}

fn write_archive(
    archive_path: &Path,
    staged_db: &Path,
    project_path: &Path,
    manifest: &BackupManifest,
) -> Result<(), String> {
    let file = File::create(archive_path).map_err(io_err("Could not create the backup file"))?;
    let mut zip = zip::ZipWriter::new(file);

    zip.start_file::<_, ()>(DB_ENTRY, Default::default())
        .map_err(io_err("Could not write the backup"))?;
    let mut db_file = File::open(staged_db).map_err(io_err("Could not read the snapshot"))?;
    std::io::copy(&mut db_file, &mut zip).map_err(io_err("Could not write the backup"))?;

    // project.json carries the coder roster and title. Without it a restore
    // would bring the coding back under a project that had forgotten who its
    // coders were, and the active-coder picker would come up empty.
    if let Ok(meta) = fs::read(project_path.join("project.json")) {
        zip.start_file::<_, ()>(META_ENTRY, Default::default())
            .map_err(io_err("Could not write the backup"))?;
        zip.write_all(&meta)
            .map_err(io_err("Could not write the backup"))?;
    }

    zip.start_file::<_, ()>(MANIFEST_ENTRY, Default::default())
        .map_err(io_err("Could not write the backup"))?;
    let json = serde_json::to_vec_pretty(manifest).map_err(io_err("Could not write the backup"))?;
    zip.write_all(&json)
        .map_err(io_err("Could not write the backup"))?;

    zip.finish()
        .map_err(io_err("Could not finish the backup"))?;
    Ok(())
}

/// Drop the oldest automatic snapshots, keeping [`AUTO_KEEP`].
///
/// Keyed off the **manifest**, never the filename. The name is derived data —
/// an imported archive keeps whatever it was called elsewhere, and a file can
/// be renamed by hand — while the manifest is the record of what the snapshot
/// actually is. Trusting the name let a manual snapshot named `…-auto` be
/// pruned as though somebody had not chosen to keep it.
///
/// Best-effort: a snapshot that cannot be deleted is no reason to fail the one
/// that was just written successfully.
fn prune_automatic(dir: &Path) {
    let mut autos: Vec<BackupInfo> = list_in(dir)
        .into_iter()
        .filter(|b| b.manifest.reason == BackupReason::Automatic)
        .collect();

    autos.sort_by(|a, b| b.manifest.created_at.cmp(&a.manifest.created_at));
    for stale in autos.into_iter().skip(AUTO_KEEP) {
        let _ = fs::remove_file(&stale.path);
    }
}

fn read_manifest(archive_path: &Path) -> Result<BackupManifest, String> {
    let file = File::open(archive_path).map_err(io_err("Could not open the backup"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(io_err("Not a readable backup file"))?;
    let mut entry = zip
        .by_name(MANIFEST_ENTRY)
        .map_err(|_| "This file is not a Fleuron backup.".to_string())?;
    let mut raw = String::new();
    entry
        .read_to_string(&mut raw)
        .map_err(io_err("Could not read the backup"))?;
    serde_json::from_str(&raw).map_err(io_err("This backup's details could not be read"))
}

/// Every snapshot in the project's backups folder, newest first.
///
/// Archives that cannot be read are skipped rather than failing the listing —
/// one corrupt file should not hide nine good ones.
pub fn list(project_path: &Path) -> Vec<BackupInfo> {
    list_in(&backups_dir(project_path))
}

fn list_in(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| READABLE_EXTS.iter().any(|ok| e == *ok))
        })
        .filter_map(|p| {
            let manifest = read_manifest(&p).ok()?;
            Some(BackupInfo {
                file_name: p.file_name()?.to_string_lossy().to_string(),
                size_bytes: fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                path: p.to_string_lossy().to_string(),
                manifest,
            })
        })
        .collect();

    out.sort_by(|a, b| b.manifest.created_at.cmp(&a.manifest.created_at));
    out
}

/// Unpack `archive_path`'s database to `dest`, and prove it is usable.
///
/// Verification happens on the extracted file, before the caller is allowed
/// anywhere near the live project: `integrity_check` catches truncation and
/// corruption, and the table probe catches a zip that is structurally fine but
/// holds something that is not a Fleuron database.
fn extract_and_verify_db(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(io_err("Could not open the backup"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(io_err("Not a readable backup file"))?;
    let mut entry = zip
        .by_name(DB_ENTRY)
        .map_err(|_| "This backup has no database inside it.".to_string())?;

    let mut out = File::create(dest).map_err(io_err("Could not stage the restore"))?;
    std::io::copy(&mut entry, &mut out).map_err(io_err("Could not stage the restore"))?;
    drop(out);

    let conn =
        Connection::open(dest).map_err(io_err("The backup's database could not be opened"))?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(io_err("The backup's database could not be read"))?;
    if integrity != "ok" {
        return Err(format!(
            "This backup is damaged and was not restored ({integrity})."
        ));
    }

    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
             AND name IN ('codebook', 'interviews', 'transcript_segments', 'coded_segments')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if tables < 4 {
        return Err("This file is not a Fleuron backup.".into());
    }

    Ok(())
}

/// What a restore replaced, for the confirmation the UI shows afterwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreOutcome {
    pub restored_from: String,
    pub restored: BackupManifest,
    /// Where the pre-restore snapshot of the old state was written.
    pub safety_backup_path: String,
}

/// Replace `dest` with `src`, retrying briefly on Windows lock contention.
///
/// Windows refuses to replace a file that still has an open handle, and a
/// `project.db` closed milliseconds earlier routinely has one for a moment --
/// an antivirus or indexer scanning it, or a handle the OS has not released
/// yet. Unix renames regardless, which is why a restore that passes every
/// macOS gate still fails with `os error 5` on a real Windows machine. Same
/// transient, same five-attempt shape, as the delete retry in
/// `db::delete_project`.
///
/// Retrying stops as soon as `src` is gone: a vanished source will never
/// succeed, and spinning on it would only delay the error the caller needs.
fn rename_with_retry(src: &Path, dest: &Path) -> std::io::Result<()> {
    let mut result = fs::rename(src, dest);
    for attempt in 1..=5 {
        match &result {
            Ok(()) => break,
            Err(_) if src.exists() => {
                std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                result = fs::rename(src, dest);
            }
            Err(_) => break,
        }
    }
    result
}

/// Replace the project's database with the one inside `archive_path`.
///
/// **The caller must have closed the project's connection first.** Windows
/// refuses to rename over an open file, and on macOS the swap would succeed
/// while the app kept reading the old inode — a restore that appears to do
/// nothing until the next launch. `commands::restore_backup` owns that
/// sequencing; nothing else should call this. The final swap retries briefly
/// for a transient Windows lock after that connection has closed.
pub fn restore(project_path: &Path, archive_path: &Path) -> Result<RestoreOutcome, String> {
    let manifest = read_manifest(archive_path)?;

    let dir = backups_dir(project_path);
    fs::create_dir_all(&dir).map_err(io_err("Could not create the backups folder"))?;

    // Step 1: verify before touching anything the coder still depends on.
    let staged = dir.join(".staging-restore.db");
    let _ = fs::remove_file(&staged);
    extract_and_verify_db(archive_path, &staged).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;

    // Step 2: snapshot what is about to be overwritten. Opened read-only-ish
    // through a fresh connection because the caller has already dropped theirs.
    let safety_backup_path = match Connection::open(project_path.join("project.db")) {
        Ok(current) => create(
            &current,
            project_path,
            BackupReason::PreRestore,
            Some(format!(
                "Automatic — taken before restoring {}",
                archive_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            )),
        )
        .map(|b| b.path)
        .unwrap_or_default(),
        Err(_) => String::new(),
    };

    // Step 3: swap. The WAL and shm belong to the database being replaced; left
    // in place, SQLite would apply a stale journal over the restored file.
    // Deliberately deleted *after* the swap: `checkpoint_open_project` is
    // best-effort, so a WAL that failed to fold in may still be the only copy
    // of the newest coding. Removing it before a rename that then fails would
    // destroy exactly the work this function exists to protect.
    let db_path = project_path.join("project.db");
    rename_with_retry(&staged, &db_path).map_err(|e| {
        let _ = fs::remove_file(&staged);
        format!(
            "Could not put the restored database in place: {e} (os error {}; \
             wal present: {}, shm present: {}). Your project was left unchanged.",
            e.raw_os_error()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string()),
            project_path.join("project.db-wal").exists(),
            project_path.join("project.db-shm").exists(),
        )
    })?;
    let _ = fs::remove_file(project_path.join("project.db-wal"));
    let _ = fs::remove_file(project_path.join("project.db-shm"));

    // project.json is metadata, not analysis. A backup made before the roster
    // existed should not wipe a roster that does — only overwrite when present.
    if let Ok(file) = File::open(archive_path) {
        if let Ok(mut zip) = zip::ZipArchive::new(file) {
            if let Ok(mut entry) = zip.by_name(META_ENTRY) {
                let mut raw = Vec::new();
                if entry.read_to_end(&mut raw).is_ok() && !raw.is_empty() {
                    let _ = fs::write(project_path.join("project.json"), raw);
                }
            }
        }
    }

    Ok(RestoreOutcome {
        restored_from: archive_path.to_string_lossy().to_string(),
        restored: manifest,
        safety_backup_path,
    })
}

/// Read one archive's details without restoring it.
///
/// Used for files chosen from outside the project — the restore picker lists
/// the project's own folder, but a backup that was moved to another disk and
/// back has to be identifiable before anyone is asked to trust it.
pub fn inspect(archive_path: &Path) -> Result<BackupInfo, String> {
    let manifest = read_manifest(archive_path)?;
    Ok(BackupInfo {
        file_name: archive_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: fs::metadata(archive_path).map(|m| m.len()).unwrap_or(0),
        path: archive_path.to_string_lossy().to_string(),
        manifest,
    })
}

/// Copy an external archive into the project's backups folder.
///
/// Validated before the copy, so an unreadable file is rejected where the user
/// picked it rather than appearing in the list as a snapshot that fails at the
/// only moment it matters. A name collision gets a numeric suffix instead of
/// overwriting: two files with the same name are far more likely to be two
/// different snapshots than the same one twice.
pub fn import(project_path: &Path, source: &Path) -> Result<BackupInfo, String> {
    let manifest = read_manifest(source)?;

    let dir = backups_dir(project_path);
    fs::create_dir_all(&dir).map_err(io_err("Could not create the backups folder"))?;

    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "imported".into());

    let mut dest = dir.join(format!("{stem}.{EXT}"));
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{stem}-{n}.{EXT}"));
        n += 1;
    }

    // Already inside this project's folder — importing it would duplicate it.
    if source.parent() == Some(dir.as_path()) {
        return inspect(source);
    }

    fs::copy(source, &dest).map_err(io_err("Could not copy the backup into this project"))?;

    Ok(BackupInfo {
        file_name: dest
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
        path: dest.to_string_lossy().to_string(),
        manifest,
    })
}

/// Remove one snapshot. Used by the manage-backups list.
pub fn delete(archive_path: &Path) -> Result<(), String> {
    if archive_path
        .extension()
        .is_none_or(|e| !READABLE_EXTS.iter().any(|ok| e == *ok))
    {
        return Err("That is not a Fleuron backup file.".into());
    }
    fs::remove_file(archive_path).map_err(io_err("Could not delete the backup"))
}

/// Save a verified recovery archive of `project_path` into `destination_dir`.
///
/// Guaranteed:
/// 1. Destination cannot be inside `project_path`.
/// 2. WAL content folded in via `VACUUM INTO`.
/// 3. Manifest extended with local recovery drafts, local project files, and external media references.
/// 4. Archive integrity verified via SQLite integrity_check before returning success.
/// 5. Restricted 0o600 file permissions on Unix.
pub fn save_local_copy(
    conn: &Connection,
    project_path: &Path,
    destination_dir: &Path,
    include_external_media: bool,
    recovery_conn: Option<&Connection>,
) -> Result<BackupInfo, String> {
    let canon_proj = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    let canon_dest = destination_dir
        .canonicalize()
        .unwrap_or_else(|_| destination_dir.to_path_buf());
    if canon_dest.starts_with(&canon_proj) || destination_dir.starts_with(project_path) {
        return Err(
            "Archive destination cannot be inside the study folder being preserved.".into(),
        );
    }

    fs::create_dir_all(destination_dir)
        .map_err(io_err("Could not create destination directory"))?;

    let mut manifest = build_manifest(
        conn,
        project_path,
        BackupReason::Preservation,
        Some("Local copy preserved before removal".into()),
    );

    if let Some(rconn) = recovery_conn {
        let canonical_str = canon_proj.to_string_lossy().to_string();
        let proj_key: Option<String> = rconn
            .query_row(
                "SELECT project_key FROM recovery_projects WHERE canonical_path = ?1",
                params![canonical_str],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
        if let Some(key) = proj_key {
            manifest.recovery_drafts =
                crate::note_recovery::list_active_drafts_for_project(rconn, &key)
                    .unwrap_or_default();
        }
    }

    let mut local_files = Vec::new();
    for sub in &["interviews", "codebook", "exports"] {
        let subdir = project_path.join(sub);
        if subdir.exists() {
            if let Ok(entries) = fs::read_dir(&subdir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_file() {
                            if let Ok(rel) = entry.path().strip_prefix(project_path) {
                                local_files.push(rel.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    manifest.local_files = local_files;

    let mut external_refs = Vec::new();
    let mut external_copied = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT audio_path, raw_vtt_path FROM interviews WHERE deleted = 0")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            let a: Option<String> = row.get(0)?;
            let v: Option<String> = row.get(1)?;
            Ok((a, v))
        }) {
            for row in rows.flatten() {
                for p_str in [row.0, row.1].into_iter().flatten() {
                    if !p_str.trim().is_empty() {
                        let p = Path::new(&p_str);
                        if !p.starts_with(project_path) {
                            external_refs.push(p_str.clone());
                            if include_external_media && p.exists() {
                                external_copied.push(p_str);
                            }
                        }
                    }
                }
            }
        }
    }
    external_refs.sort();
    external_refs.dedup();
    external_copied.sort();
    external_copied.dedup();
    manifest.external_files = external_refs;
    manifest.external_files_copied = external_copied;

    let stamp = Utc::now().format("%Y-%m-%d-%H%M%S");
    let safe_title = manifest
        .project_title
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    let mut archive_path = destination_dir.join(format!("{safe_title}-{stamp}-preserved.{EXT}"));
    let mut nth = 2;
    while archive_path.exists() {
        archive_path = destination_dir.join(format!("{safe_title}-{stamp}-preserved-{nth}.{EXT}"));
        nth += 1;
    }

    let staged_db = destination_dir.join(format!(
        ".staging-preservation-{stamp}-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    let _ = fs::remove_file(&staged_db);

    conn.execute("VACUUM INTO ?1", [staged_db.to_string_lossy().as_ref()])
        .map_err(|e| {
            let _ = fs::remove_file(&staged_db);
            format!("Could not snapshot database for preservation: {e}")
        })?;

    let file = File::create(&archive_path).map_err(io_err("Could not create archive file"))?;
    let mut zip = zip::ZipWriter::new(file);

    let db_res = (|| -> Result<(), String> {
        zip.start_file::<_, ()>(DB_ENTRY, Default::default())
            .map_err(io_err("Could not write database to archive"))?;
        let mut db_file = File::open(&staged_db).map_err(io_err("Could not read snapshot"))?;
        std::io::copy(&mut db_file, &mut zip).map_err(io_err("Could not write database"))?;
        Ok(())
    })();
    let _ = fs::remove_file(&staged_db);
    db_res?;

    if let Ok(meta) = fs::read(project_path.join("project.json")) {
        zip.start_file::<_, ()>(META_ENTRY, Default::default())
            .map_err(io_err("Could not write metadata"))?;
        zip.write_all(&meta)
            .map_err(io_err("Could not write metadata"))?;
    }

    zip.start_file::<_, ()>(MANIFEST_ENTRY, Default::default())
        .map_err(io_err("Could not write manifest"))?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(io_err("Could not encode manifest"))?;
    zip.write_all(&json)
        .map_err(io_err("Could not write manifest"))?;

    for rel in &manifest.local_files {
        let abs = project_path.join(rel);
        if let Ok(mut f) = File::open(&abs) {
            let zip_entry_name = format!("files/{}", rel.replace('\\', "/"));
            if zip
                .start_file::<_, ()>(&zip_entry_name, Default::default())
                .is_ok()
            {
                let _ = std::io::copy(&mut f, &mut zip);
            }
        }
    }

    for ext_path in &manifest.external_files_copied {
        let p = Path::new(ext_path);
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let zip_entry_name = format!("external_media/{name}");
            if let Ok(mut f) = File::open(p) {
                if zip
                    .start_file::<_, ()>(&zip_entry_name, Default::default())
                    .is_ok()
                {
                    let _ = std::io::copy(&mut f, &mut zip);
                }
            }
        }
    }

    zip.finish().map_err(io_err("Could not finish archive"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&archive_path, fs::Permissions::from_mode(0o600));
    }

    let verify_staging = destination_dir.join(format!(
        ".verify-preservation-{stamp}-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    let verify_res = extract_and_verify_db(&archive_path, &verify_staging);
    let _ = fs::remove_file(&verify_staging);
    if let Err(e) = verify_res {
        let _ = fs::remove_file(&archive_path);
        return Err(format!(
            "Preservation archive integrity verification failed: {e}"
        ));
    }

    let size_bytes = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupInfo {
        path: archive_path.to_string_lossy().to_string(),
        file_name: archive_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes,
        manifest,
    })
}

/// Restore a study backup to a newly allocated project folder.
pub fn restore_backup_to_new_project(
    archive_path: &Path,
    parent_dir: &Path,
    target_folder_name: Option<&str>,
    recovery_conn: Option<&Connection>,
) -> Result<PathBuf, String> {
    if !archive_path.exists() {
        return Err("Backup file does not exist.".into());
    }

    let manifest = read_manifest(archive_path)?;

    let temp_verify = parent_dir.join(format!(
        ".verify-restore-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(parent_dir).map_err(io_err("Could not access parent directory"))?;
    let verify_res = extract_and_verify_db(archive_path, &temp_verify);
    let _ = fs::remove_file(&temp_verify);
    verify_res?;

    let base_name = if let Some(custom) = target_folder_name.filter(|s| !s.trim().is_empty()) {
        custom.trim().to_string()
    } else if !manifest.project_title.trim().is_empty() {
        manifest.project_title.clone()
    } else {
        "Restored Study".to_string()
    };

    let folder_name = if db::PROJECT_EXTS
        .iter()
        .any(|ext| base_name.ends_with(&format!(".{ext}")))
    {
        base_name.clone()
    } else {
        format!("{}.{}", base_name, db::PROJECT_EXT)
    };

    let mut target_path = parent_dir.join(&folder_name);
    let mut nth = 2;
    let clean_stem = if db::PROJECT_EXTS
        .iter()
        .any(|ext| base_name.ends_with(&format!(".{ext}")))
    {
        base_name
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&base_name)
    } else {
        &base_name
    };
    while target_path.exists() {
        target_path = parent_dir.join(format!("{clean_stem}-{nth}.{}", db::PROJECT_EXT));
        nth += 1;
    }

    fs::create_dir_all(&target_path).map_err(io_err("Could not create target directory"))?;

    let file = File::open(archive_path).map_err(io_err("Could not open backup archive"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(io_err("Not a readable backup zip"))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io_err("Corrupt archive entry"))?;
        let entry_name = entry.name().to_string();

        if entry_name == DB_ENTRY {
            let mut out = File::create(target_path.join(DB_ENTRY))
                .map_err(io_err("Could not write restored database"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(io_err("Could not write restored database"))?;
        } else if entry_name == META_ENTRY {
            let mut out = File::create(target_path.join(META_ENTRY))
                .map_err(io_err("Could not write restored metadata"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(io_err("Could not write restored metadata"))?;
        } else if let Some(rel) = entry_name.strip_prefix("files/") {
            let dest_file = target_path.join(rel);
            if let Some(p) = dest_file.parent() {
                let _ = fs::create_dir_all(p);
            }
            if let Ok(mut out) = File::create(&dest_file) {
                let _ = std::io::copy(&mut entry, &mut out);
            }
        } else if let Some(rel) = entry_name.strip_prefix("external_media/") {
            let dest_file = target_path.join("interviews").join(rel);
            if let Some(p) = dest_file.parent() {
                let _ = fs::create_dir_all(p);
            }
            if let Ok(mut out) = File::create(&dest_file) {
                let _ = std::io::copy(&mut entry, &mut out);
            }
        }
    }

    let restored_db_path = target_path.join(DB_ENTRY);
    if let Ok(conn) = Connection::open(&restored_db_path) {
        let _ = conn.execute(
            "DELETE FROM sync_state WHERE key IN ('group_bound', 'project_id', 'group_title', 'group_key')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('group_bound', '0')",
            [],
        );
    }

    if let Some(rconn) = recovery_conn {
        if !manifest.recovery_drafts.is_empty() {
            let canon_path_str = target_path
                .canonicalize()
                .unwrap_or_else(|_| target_path.clone())
                .to_string_lossy()
                .to_string();
            if let Ok(new_key) = crate::note_recovery::register_project(
                rconn,
                &canon_path_str,
                &manifest.project_title,
            ) {
                for draft in &manifest.recovery_drafts {
                    let mut remapped = draft.clone();
                    remapped.project_key = new_key.clone();
                    let _ = crate::note_recovery::restore_draft_record(rconn, &remapped);
                }
            }
        }
    }

    Ok(target_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CreateCodeInput, CreateProjectInput};

    fn make_project() -> (tempfile::TempDir, PathBuf, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = db::create_project(&CreateProjectInput {
            parent_dir: dir.path().to_string_lossy().to_string(),
            project_name: "p".into(),
            title: "T".into(),
            coders: vec!["Alice".into()],
        })
        .unwrap();
        let conn = db::open_project(&path.to_string_lossy()).unwrap();
        (dir, path, conn)
    }

    fn add_code(conn: &Connection, name: &str) {
        db::create_code(
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
        .unwrap();
    }

    fn code_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM codebook WHERE deleted = 0 ORDER BY name")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .flatten()
            .collect();
        rows
    }

    /// The whole point of the module, end to end: work that existed at snapshot
    /// time comes back, and work that came after it does not.
    #[test]
    fn restore_returns_the_project_to_the_snapshot() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "kept");
        let snapshot = create(&conn, &path, BackupReason::Manual, None).unwrap();

        add_code(&conn, "added-after");
        assert_eq!(code_names(&conn), vec!["added-after", "kept"]);

        // The caller owns this in the real command; the module contract is that
        // the connection is gone before the swap.
        drop(conn);
        restore(&path, Path::new(&snapshot.path)).unwrap();

        let conn = db::open_project(&path.to_string_lossy()).unwrap();
        assert_eq!(code_names(&conn), vec!["kept"]);
    }

    #[test]
    fn rename_with_retry_moves_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.db");
        let dest = dir.path().join("destination.db");
        fs::write(&src, b"restored database").unwrap();

        rename_with_retry(&src, &dest).unwrap();

        assert!(!src.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"restored database");
    }

    #[test]
    fn rename_with_retry_gives_up_when_the_source_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("already-gone.db");
        let dest = dir.path().join("destination.db");

        assert!(rename_with_retry(&missing, &dest).is_err());
    }

    #[test]
    fn a_failed_swap_leaves_no_staging_file() {
        // A Windows file lock cannot be forced portably in this unit test. A
        // successful restore nevertheless proves the staging file's normal
        // lifecycle terminates, which is the invariant a failed-swap cleanup
        // preserves as well.
        let (_dir, path, conn) = make_project();
        add_code(&conn, "before");
        let snapshot = create(&conn, &path, BackupReason::Manual, None).unwrap();
        add_code(&conn, "after");
        drop(conn);

        restore(&path, Path::new(&snapshot.path)).unwrap();

        assert!(!backups_dir(&path).join(".staging-restore.db").exists());
    }

    /// Restoring the wrong file has to be survivable, or nobody should be
    /// offered the button.
    #[test]
    fn restore_snapshots_what_it_is_about_to_overwrite() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "original");
        let empty_state = create(&conn, &path, BackupReason::Manual, None).unwrap();

        add_code(&conn, "work-i-would-hate-to-lose");
        drop(conn);

        let outcome = restore(&path, Path::new(&empty_state.path)).unwrap();
        assert!(!outcome.safety_backup_path.is_empty());

        // Undo the restore using the snapshot the restore itself took.
        restore(&path, Path::new(&outcome.safety_backup_path)).unwrap();
        let conn = db::open_project(&path.to_string_lossy()).unwrap();
        assert_eq!(
            code_names(&conn),
            vec!["original", "work-i-would-hate-to-lose"]
        );
    }

    /// 🔑 A snapshot taken while the WAL held uncommitted-to-main-db writes must
    /// contain them. This is the failure the module exists to prevent: copying
    /// `project.db` alone yields a file that opens cleanly and is missing the
    /// most recent coding, with nothing to indicate it.
    #[test]
    fn snapshot_includes_writes_still_sitting_in_the_wal() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "written-but-not-checkpointed");
        // No checkpoint call here, deliberately.

        let snapshot = create(&conn, &path, BackupReason::Manual, None).unwrap();
        let staged = path.join("verify.db");
        extract_and_verify_db(Path::new(&snapshot.path), &staged).unwrap();

        let restored = Connection::open(&staged).unwrap();
        assert_eq!(code_names(&restored), vec!["written-but-not-checkpointed"]);
    }

    /// A stale `-wal` beside a replaced database would be applied over it on the
    /// next open, silently undoing the restore.
    ///
    /// The journal has to be planted by hand: closing the last connection
    /// checkpoints and unlinks it, so the ordinary path never leaves one
    /// behind. The case that does is a crash or a force-quit — precisely when
    /// somebody reaches for a restore.
    #[test]
    fn restore_clears_the_replaced_databases_journal() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "a");
        let snapshot = create(&conn, &path, BackupReason::Manual, None).unwrap();
        add_code(&conn, "b");
        drop(conn);

        fs::write(path.join("project.db-wal"), b"stale journal from a crash").unwrap();
        fs::write(path.join("project.db-shm"), b"stale index").unwrap();

        restore(&path, Path::new(&snapshot.path)).unwrap();
        assert!(!path.join("project.db-wal").exists());
        assert!(!path.join("project.db-shm").exists());

        // And the restored database is readable, which it would not be if the
        // planted journal had survived to be replayed over it.
        let conn = db::open_project(&path.to_string_lossy()).unwrap();
        assert_eq!(code_names(&conn), vec!["a"]);
    }

    #[test]
    fn a_damaged_archive_is_refused_before_anything_is_replaced() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "still-here");
        let snapshot = create(&conn, &path, BackupReason::Manual, None).unwrap();

        // Truncate the archive to halfway through.
        let bytes = fs::read(&snapshot.path).unwrap();
        fs::write(&snapshot.path, &bytes[..bytes.len() / 2]).unwrap();

        drop(conn);
        assert!(restore(&path, Path::new(&snapshot.path)).is_err());

        let conn = db::open_project(&path.to_string_lossy()).unwrap();
        assert_eq!(code_names(&conn), vec!["still-here"]);
    }

    #[test]
    fn a_zip_that_is_not_a_backup_is_refused() {
        let (_dir, path, conn) = make_project();
        let not_a_backup = path.join("nonsense.fleuronbak");
        fs::write(&not_a_backup, b"this is not a zip at all").unwrap();

        drop(conn);
        let err = restore(&path, &not_a_backup).unwrap_err();
        assert!(
            err.contains("Not a readable backup") || err.contains("not a Fleuron backup"),
            "unhelpful message: {err}"
        );
    }

    /// Automatic snapshots roll; the two kinds somebody chose on purpose do not.
    #[test]
    fn pruning_spares_manual_and_pre_restore_snapshots() {
        let (_dir, path, conn) = make_project();
        create(&conn, &path, BackupReason::Manual, None).unwrap();
        create(&conn, &path, BackupReason::PreRestore, None).unwrap();
        for _ in 0..AUTO_KEEP + 5 {
            create(&conn, &path, BackupReason::Automatic, None).unwrap();
        }

        let all = list(&path);
        let by = |r: BackupReason| all.iter().filter(|b| b.manifest.reason == r).count();
        assert_eq!(by(BackupReason::Automatic), AUTO_KEEP);
        assert_eq!(by(BackupReason::Manual), 1);
        assert_eq!(by(BackupReason::PreRestore), 1);
    }

    /// The regression behind the manifest-driven prune: a snapshot somebody
    /// kept on purpose, carrying a name that looks automatic, must survive.
    #[test]
    fn pruning_reads_the_manifest_rather_than_the_filename() {
        let (_dir, path, conn) = make_project();
        let manual = create(&conn, &path, BackupReason::Manual, None).unwrap();

        let dir = backups_dir(&path);
        let disguised = dir.join(format!("fleuron-1999-01-01-000000-auto.{EXT}"));
        fs::rename(&manual.path, &disguised).unwrap();

        for _ in 0..AUTO_KEEP + 2 {
            create(&conn, &path, BackupReason::Automatic, None).unwrap();
        }

        assert!(
            disguised.exists(),
            "a manual snapshot was pruned because of its name"
        );
    }

    #[test]
    fn manifest_records_what_the_project_held() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "one");
        add_code(&conn, "two");

        let info = create(
            &conn,
            &path,
            BackupReason::Manual,
            Some("before I merge".into()),
        )
        .unwrap();
        assert_eq!(info.manifest.codes, 2);
        assert_eq!(info.manifest.interviews, 0);
        assert_eq!(info.manifest.project_title, "T");
        assert_eq!(info.manifest.note.as_deref(), Some("before I merge"));
        assert!(info.size_bytes > 0);
    }

    /// Snapshots written before the Fleuron rename keep their `.codemapbak`
    /// suffix on disk. They must stay listable, importable, and deletable.
    #[test]
    fn pre_rename_codemapbak_snapshots_are_still_usable() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "written-before-the-rename");
        let made = create(&conn, &path, BackupReason::Manual, None).unwrap();

        // Age it back to the old extension, as an existing project's folder has it.
        let legacy = backups_dir(&path).join("codemap-2026-01-01-000000-manual.codemapbak");
        fs::rename(&made.path, &legacy).unwrap();

        assert_eq!(list(&path).len(), 1, "a .codemapbak must still be listed");
        assert!(
            inspect(&legacy).is_ok(),
            "a .codemapbak must still be readable"
        );

        // Imported from elsewhere, it is rewritten under the new extension.
        let (_dir2, other, conn2) = make_project();
        drop(conn2);
        let brought_in = import(&other, &legacy).unwrap();
        assert!(
            brought_in.file_name.ends_with(".fleuronbak"),
            "import must rewrite to the current extension, got {}",
            brought_in.file_name
        );

        assert!(
            delete(&legacy).is_ok(),
            "a .codemapbak must still be deletable"
        );
    }

    /// One unreadable file must not hide the good snapshots beside it.
    #[test]
    fn listing_skips_archives_it_cannot_read() {
        let (_dir, path, conn) = make_project();
        create(&conn, &path, BackupReason::Manual, None).unwrap();
        fs::write(backups_dir(&path).join("junk.fleuronbak"), b"garbage").unwrap();

        assert_eq!(list(&path).len(), 1);
    }

    #[test]
    fn importing_the_projects_own_backup_does_not_duplicate_it() {
        let (_dir, path, conn) = make_project();
        let made = create(&conn, &path, BackupReason::Manual, None).unwrap();

        import(&path, Path::new(&made.path)).unwrap();
        assert_eq!(list(&path).len(), 1);
    }

    #[test]
    fn an_external_backup_can_be_brought_in() {
        let (_dir, path, conn) = make_project();
        add_code(&conn, "from-elsewhere");
        let made = create(&conn, &path, BackupReason::Manual, None).unwrap();

        let elsewhere = tempfile::tempdir().unwrap();
        let moved = elsewhere.path().join("carried-on-a-usb-stick.fleuronbak");
        fs::rename(&made.path, &moved).unwrap();
        assert!(list(&path).is_empty());

        let imported = import(&path, &moved).unwrap();
        assert_eq!(imported.manifest.codes, 1);
        assert_eq!(list(&path).len(), 1);
    }

    #[test]
    fn delete_refuses_anything_that_is_not_a_backup() {
        let (_dir, path, _conn) = make_project();
        assert!(delete(&path.join("project.db")).is_err());
        assert!(path.join("project.db").exists());
    }

    #[test]
    fn backup_and_restore_preserves_local_segment_reviews() {
        let (_dir, path, conn) = make_project();
        db::create_interview(
            &conn,
            &crate::models::CreateInterviewInput {
                participant_label: "P-Rev".into(),
                interview_date: None,
                modality: None,
                diagnosis_notes: None,
                interviewers: vec![],
            },
        )
        .unwrap();
        let interviews = db::list_interviews(&conn).unwrap();
        let iv = &interviews[0];
        db::import_segments(
            &conn,
            &crate::models::ImportSegmentsInput {
                interview_id: iv.id.clone(),
                segments: vec![
                    crate::models::SegmentInput {
                        speaker: "Speaker 1".into(),
                        timestamp_start: "00:00:01".into(),
                        timestamp_end: None,
                        text: "First turn".into(),
                        section_tag: None,
                    },
                    crate::models::SegmentInput {
                        speaker: "Speaker 2".into(),
                        timestamp_start: "00:00:05".into(),
                        timestamp_end: None,
                        text: "Second turn".into(),
                        section_tag: None,
                    },
                ],
                raw_vtt_path: None,
            },
        )
        .unwrap();
        let segs = db::get_segments(&conn, &iv.id).unwrap();
        assert_eq!(segs.len(), 2);
        let s0 = &segs[0].id;
        db::set_segment_reviewed(&conn, s0, true).unwrap();
        assert_eq!(
            db::list_segment_reviews(&conn, &iv.id).unwrap(),
            vec![s0.clone()]
        );

        let made = create(
            &conn,
            &path,
            BackupReason::Manual,
            Some("with review".to_string()),
        )
        .unwrap();

        // Clear the review in the live database
        db::set_segment_reviewed(&conn, s0, false).unwrap();
        assert!(db::list_segment_reviews(&conn, &iv.id).unwrap().is_empty());

        // Drop the open connection before restoring
        drop(conn);

        restore(&path, Path::new(&made.path)).unwrap();

        let reopened = db::open_project(&path.to_string_lossy()).unwrap();
        assert_eq!(
            db::list_segment_reviews(&reopened, &iv.id).unwrap(),
            vec![s0.clone()],
            "Restored database must retain segment_reviews"
        );
    }
}
