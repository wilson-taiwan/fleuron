use crate::commands::AppState;
use crate::db;
use crate::models::{AppPreferences, CreateProjectInput};
use crate::sync::SyncConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, State};

static SELFTEST_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_selftest_active(active: bool) {
    SELFTEST_ACTIVE.store(active, Ordering::SeqCst);
}

pub fn is_selftest_active() -> bool {
    SELFTEST_ACTIVE.load(Ordering::SeqCst)
}

#[derive(Debug, Clone)]
pub struct SelftestStagingConfig {
    pub url: String,
    pub anon_key: String,
    pub owner_email: String,
    pub owner_password: String,
    pub joiner_email: String,
    pub joiner_password: String,
}

impl SelftestStagingConfig {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("FLEURON_STAGING_SUPABASE_URL").ok()?;
        let anon_key = std::env::var("FLEURON_STAGING_SUPABASE_ANON_KEY").ok()?;
        let owner_email = std::env::var("FLEURON_STAGING_OWNER_EMAIL").ok()?;
        let owner_password = std::env::var("FLEURON_STAGING_OWNER_PASSWORD").ok()?;
        let joiner_email = std::env::var("FLEURON_STAGING_JOINER_EMAIL").ok()?;
        let joiner_password = std::env::var("FLEURON_STAGING_JOINER_PASSWORD").ok()?;
        if url.trim().is_empty()
            || anon_key.trim().is_empty()
            || owner_email.trim().is_empty()
            || owner_password.trim().is_empty()
            || joiner_email.trim().is_empty()
            || joiner_password.trim().is_empty()
        {
            return None;
        }
        Some(Self {
            url,
            anon_key,
            owner_email,
            owner_password,
            joiner_email,
            joiner_password,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SelftestSuiteResult {
    pub suite: String,
    #[serde(default)]
    pub passed: bool,
    #[serde(default)]
    pub status: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
}

#[tauri::command]
pub fn is_selftest() -> bool {
    is_selftest_active()
}

#[tauri::command]
pub fn selftest_online_status() -> Result<bool, String> {
    Ok(SelftestStagingConfig::from_env().is_some())
}

#[tauri::command]
pub async fn selftest_sign_in_as(
    app: AppHandle,
    state: State<'_, AppState>,
    role: String,
) -> Result<(), String> {
    let staging = SelftestStagingConfig::from_env()
        .ok_or_else(|| "Staging configuration environment variables are missing".to_string())?;

    let (email, password) = match role.as_str() {
        "owner" => (staging.owner_email, staging.owner_password),
        "joiner" => (staging.joiner_email, staging.joiner_password),
        _ => return Err(format!("Unknown selftest role: {role}")),
    };

    // Update app preferences so commands point at staging
    let prefs = AppPreferences {
        sync_url: Some(staging.url.clone()),
        sync_anon_key: Some(staging.anon_key.clone()),
        ..crate::app_data::get_app_preferences(&app)?
    };
    crate::app_data::set_app_preferences(&app, prefs)?;

    let cfg = SyncConfig {
        url: staging.url,
        anon_key: staging.anon_key,
        project_id: String::new(),
    };

    let client = crate::sync::client().map_err(|e| e.to_string())?;
    let session = crate::sync::sign_in(&client, &cfg, &email, &password)
        .await
        .map_err(|e| e.to_string())?;

    crate::sync::remember_session(&app, &session);
    *state.sync_session.lock().map_err(|e| e.to_string())? = Some(session);
    state.maybe_start_realtime(&app);

    Ok(())
}

#[tauri::command]
pub fn selftest_seed_unbound(
    app: AppHandle,
    state: State<'_, AppState>,
    coder_name: String,
) -> Result<String, String> {
    let proj_dir = db::create_project(&CreateProjectInput {
        parent_dir: std::env::temp_dir().to_string_lossy().to_string(),
        project_name: format!("fleuron_selftest_unbound_{}", uuid::Uuid::new_v4()),
        title: "Selftest Study Unbound".to_string(),
        coders: vec![coder_name],
    })
    .map_err(|e| e.to_string())?;

    let path_str = proj_dir.to_string_lossy().to_string();
    let (opened_conn, _) = db::open_project_snapshot_inner(&path_str).map_err(|e| e.to_string())?;

    *state.project_path.lock().map_err(|e| e.to_string())? = Some(PathBuf::from(&path_str));
    *state.db.lock().map_err(|e| e.to_string())? = Some(opened_conn);
    // Synthetic projects rotate like real opens, so checked-write tests run
    // against a live epoch rather than a stale one.
    let _ = state.rotate_workspace(&app, &proj_dir, "Selftest Study Unbound");

    Ok(path_str)
}

#[tauri::command]
pub fn selftest_seed(
    app: AppHandle,
    state: State<'_, AppState>,
    _suite: String,
) -> Result<serde_json::Value, String> {
    let proj_dir = db::create_project(&CreateProjectInput {
        parent_dir: std::env::temp_dir().to_string_lossy().to_string(),
        project_name: format!("fleuron_selftest_{}", uuid::Uuid::new_v4()),
        title: "Selftest Study".to_string(),
        coders: vec!["Ada Lovelace".to_string()],
    })
    .map_err(|e| e.to_string())?;

    let db_path = proj_dir.join("project.db");
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO codebook (id, name, definition, color, sort_order, is_retired, created_at, updated_at)
         VALUES ('c1', 'Theme A', 'Parent theme definition', '#3b82f6', 0, 0, ?1, ?1),
                ('c2', 'Sub-theme B', 'Child theme definition', '#10b981', 1, 0, ?1, ?1)",
        [&now],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO interviews (id, participant_label, created_at, updated_at)
         VALUES ('iv1', 'P01', ?1, ?1)",
        [&now],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO transcript_segments (id, interview_id, segment_index, speaker, timestamp_start, text)
         VALUES ('seg1', 'iv1', 0, 'Participant', '00:00:00.000', 'This is a sample qualitative transcript passage for selftest.')",
        [],
    ).map_err(|e| e.to_string())?;

    let _ = crate::sync::bind_to_group(&conn, "selftest-group");

    drop(conn);

    let path_str = proj_dir.to_string_lossy().to_string();
    let (opened_conn, snapshot) =
        db::open_project_snapshot_inner(&path_str).map_err(|e| e.to_string())?;

    *state.project_path.lock().map_err(|e| e.to_string())? = Some(PathBuf::from(&path_str));
    *state.db.lock().map_err(|e| e.to_string())? = Some(opened_conn);
    *state.sync_session.lock().map_err(|e| e.to_string())? = Some(crate::sync::SyncSession {
        user_id: "selftest-user".to_string(),
        email: "selftest@example.com".to_string(),
        access_token: "selftest-token".to_string(),
        refresh_token: "selftest-refresh-token".to_string(),
    });
    let _ = state.rotate_workspace(&app, &proj_dir, "Selftest Study");

    Ok(serde_json::json!({
        "project_path": path_str,
        "snapshot": snapshot,
    }))
}

#[tauri::command]
pub fn selftest_report(results: Vec<SelftestSuiteResult>) -> Result<(), String> {
    println!("\n==========================================");
    println!("        FLEURON SELFTEST REPORT           ");
    println!("==========================================");
    let require_online = std::env::args().any(|a| a == "--require-online");
    let mut all_passed = true;
    for r in &results {
        let dur = r
            .duration_ms
            .map(|d| format!(" ({d}ms)"))
            .unwrap_or_default();
        let status = r
            .status
            .as_deref()
            .unwrap_or(if r.passed { "passed" } else { "failed" });

        if status == "passed" {
            println!("  ✓ [PASS] {}{}", r.suite, dur);
        } else if status == "skipped" {
            let reason = r.error.as_deref().unwrap_or("Preconditions not met");
            if require_online {
                all_passed = false;
                eprintln!(
                    "  ❌ [FAIL] {}{} (skipped while --require-online active): {}",
                    r.suite, dur, reason
                );
            } else {
                println!("  ↷ [SKIP] {}{}: {}", r.suite, dur, reason);
            }
        } else {
            all_passed = false;
            let err = r.error.as_deref().unwrap_or("Unknown error");
            eprintln!("  ❌ [FAIL] {}{}: {}", r.suite, dur, err);
        }
    }
    println!("==========================================\n");

    if all_passed {
        println!("All selftest suites PASSED (or cleanly SKIPPED)! Exiting process 0.");
        std::process::exit(0);
    } else {
        eprintln!("Selftest failures detected! Exiting process 1.");
        std::process::exit(1);
    }
}
