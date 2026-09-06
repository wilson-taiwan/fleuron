#![deny(clippy::disallowed_methods)]

// Windows unit-test binaries carry no application manifest: tauri-build embeds
// one for the app, but `cargo test` links a plain executable. Without the
// Common-Controls v6 assembly the process loads comctl32 v5 from System32,
// which does not export TaskDialogIndirect (imported via rfd's Windows
// dialogs through tauri-plugin-dialog), and dies at load with
// STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before a single test runs.
//
// A `.drectve` section is handed to the MSVC linker as command-line
// directives, so this declares the dependency for whatever binary it is linked
// into. It is `cfg(test)`, so it exists ONLY in the unit-test executable and
// can never affect the shipped app or its tauri-generated manifest.
//
// `cargo:rustc-link-arg-tests` cannot do this job: it applies only to
// `[[test]]` integration targets, and this package's tests live in the lib.
#[cfg(all(test, windows, target_env = "msvc"))]
#[used]
#[link_section = ".drectve"]
static COMMON_CONTROLS_V6_MANIFEST: [u8; 167] =
    *b" /manifestdependency:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\"";

mod app_data;
mod backup;
mod commands;
pub mod crash_report;
mod db;
pub mod diagnostics;
mod docx;
mod file_error;
mod ids;
#[cfg(test)]
mod lifecycle_harness;
pub mod location;
mod menu;
mod models;
pub mod note_recovery;
pub mod open_marker;
pub mod pdf;
pub mod readiness;
mod realtime;
pub mod run_marker;
#[cfg(test)]
mod schema_lint;
pub mod selftest;
mod session_store;
mod sync;

#[cfg(windows)]
pub mod dpapi;
mod sync_coordinator;
mod sync_defaults;
#[cfg(test)]
mod sync_harness;
mod sync_v2;
#[cfg(test)]
mod sync_v2_harness;
mod update_coordinator;
mod window_guard;

use commands::{AppState, PendingOpen};
use tauri::{Emitter, Listener, Manager, RunEvent};

/// Route a double-clicked project to the UI, however it arrived.
///
/// Both platform paths land here so the extension check happens once. Neither
/// source is trustworthy on its own: argv[1] is whatever the shell passed, and
/// the Apple Event carries any URL the association matched.
///
/// Delivery is queued rather than emitted while the frontend is still loading.
/// This is the fix for the bug where double-clicking a `.fleuron` opened the
/// app on its home screen: a cold launch delivers the path during `setup` (or
/// as an Apple Event moments later), both of which land well before the webview
/// has mounted its listener, and an emit with nobody listening is simply lost.
fn deliver_open_project(app: &tauri::AppHandle, path: &str) {
    if !db::is_project_path(std::path::Path::new(path)) {
        return;
    }

    // Canonicalize separators so a Windows path with mixed slashes normalises
    // before it reaches state or the frontend.
    let path: String = std::path::Path::new(path)
        .components()
        .collect::<std::path::PathBuf>()
        .to_string_lossy()
        .into_owned();

    if !app.state::<PendingOpen>().queue_unless_ready(path.clone()) {
        let _ = app.emit("open-project-path", path);
    }
}

#[cfg(target_os = "macos")]
fn activate_macos_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.activate();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let is_selftest_flag = std::env::args().any(|a| a == "--selftest");
    if is_selftest_flag {
        crate::selftest::set_selftest_active(true);
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(120));
            eprintln!("[selftest] ❌ Watchdog timeout (120s) exceeded without report");
            std::process::exit(1);
        });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            for argument in args {
                deliver_open_project(app, &argument);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .manage(PendingOpen::new())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            if update_coordinator::startup_install_guard(app.handle())
                == update_coordinator::StartupInstallGuard::InstallerActive
            {
                app.handle().exit(0);
                return Ok(());
            }

            let menu = menu::build_menu(app.handle())?;
            app.set_menu(menu)?;

            if crate::selftest::is_selftest_active() {
                let _ = app_data::clear_recent_projects_for_selftest(app.handle());
            }

            if let Ok(app_dir) = app_data::app_data_dir(app.handle()) {
                crash_report::init_crash_handler(app_dir.clone());
                run_marker::init_run_marker(&app_dir);
            }
            app.handle()
                .state::<AppState>()
                .update_coordinator
                .reconcile_startup(app.handle());
            let _ = app_data::sync_device_id(app.handle());

            let realtime_hint_app = app.handle().clone();
            app.listen("sync://remote-change", move |event| {
                let state = realtime_hint_app.state::<AppState>();
                let hint =
                    serde_json::from_str::<realtime::RemoteChangeEvent>(event.payload()).ok();
                state.record_realtime_hint(
                    hint.as_ref().and_then(|value| value.generation.as_deref()),
                    hint.as_ref().and_then(|value| value.head),
                );
                state.request_background_sync(
                    realtime_hint_app.clone(),
                    sync_coordinator::SyncTrigger::RealtimeHeadChanged,
                );
            });
            let reconnect_app = app.handle().clone();
            app.listen("sync://realtime-connected", move |_| {
                let state = reconnect_app.state::<AppState>();
                state.request_background_sync(
                    reconnect_app.clone(),
                    sync_coordinator::SyncTrigger::NetworkRecovered,
                );
            });
            app.handle()
                .state::<AppState>()
                .start_sync_polling(app.handle().clone());
            window_guard::clamp_main_window_to_work_area(app.handle());

            if !crate::selftest::is_selftest_active() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    #[cfg(target_os = "macos")]
                    activate_macos_app();
                    let _ = window.set_focus();
                }
            }

            if let Some(path) = std::env::args().nth(1) {
                if path != "--selftest" {
                    deliver_open_project(app.handle(), &path);
                }
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            menu::handle_menu_event(app, event.id().as_ref());
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::open_project,
            commands::close_project,
            commands::get_project_info,
            commands::get_live_workspace_snapshot,
            commands::list_sync_conflicts,
            commands::adopt_project_coder,
            commands::adopt_project_title,
            commands::list_activity,
            commands::list_codes,
            commands::list_retired_codes,
            commands::create_code,
            commands::ensure_code_and_apply,
            commands::update_code,
            commands::delete_code,
            commands::restore_code,
            commands::create_backup,
            commands::list_backups,
            commands::restore_backup,
            commands::delete_backup,
            commands::inspect_backup,
            commands::import_backup,
            commands::pending_coded_count,
            commands::list_interviews,
            commands::create_interview,
            commands::update_interview,
            commands::interview_delete_impact,
            commands::delete_interview,
            commands::import_segments,
            commands::get_segments,
            commands::set_segment_speaker,
            commands::restore_segment_speakers,
            commands::set_segment_reviewed,
            commands::list_segment_reviews,
            commands::apply_codes,
            commands::mutate_coding_edge,
            commands::patch_coding_memo,
            commands::delete_coded_segment,
            commands::list_coded_segments,
            commands::update_hub_memo,
            commands::list_note_drafts,
            commands::begin_note_draft,
            commands::put_note_draft,
            commands::discard_note_draft,
            commands::save_note_draft,
            commands::resolve_note_draft_target,
            commands::note_recovery_status,
            commands::set_recovery_root_for_selftest,
            commands::approve_update_departure,
            commands::complete_note_departure,
            commands::clear_workspace,
            commands::export_with_config,
            commands::consume_pending_open,
            commands::get_projects_library_dir,
            commands::cloud_provider_for_path,
            commands::library_sync_warning,
            commands::get_workspace_state,
            commands::save_workspace_state,
            commands::read_text_file,
            commands::read_transcript_file,
            commands::scan_transcript_folder,
            commands::hash_candidate_segments,
            commands::list_recent_projects,
            commands::record_recent_project,
            commands::remove_recent_project,
            commands::get_app_preferences,
            commands::set_app_preferences,
            commands::get_app_version,
            commands::list_project_files,
            commands::copy_project_file,
            commands::sync_status,
            commands::sync_sign_in,
            commands::sync_restore_session,
            commands::sync_sign_out,
            commands::sync_now,
            commands::sync_list_memberships,
            commands::list_cached_memberships,
            commands::sync_join_project,
            commands::sync_create_project,
            commands::sync_join_group,
            commands::sync_group_info,
            commands::sync_v2_readiness,
            commands::sync_v2_activate,
            commands::sync_v2_resolve_conflict,
            commands::sync_reset_group_key,
            commands::sync_set_member_role,
            commands::sync_remove_member,
            commands::sync_delete_group,
            commands::sync_leave_group,
            commands::sync_detach_local,
            commands::project_deletion_summary,
            commands::delete_project_folder,
            commands::inspect_join_target,
            commands::list_left_studies,
            commands::resolve_study_location,
            commands::auto_relink_study,
            commands::study_readiness,
            commands::check_open_marker,
            commands::heartbeat_open_marker,
            commands::sync_set_my_coder_name,
            commands::sync_sign_up,
            commands::sync_request_password_reset,
            commands::sync_complete_password_reset,
            commands::sync_redeem_invite,
            commands::sync_deep_verify,
            commands::sync_repair,
            commands::sync_diagnostics_dump,
            commands::sync_refresh_server_schema,
            commands::sync_reconcile_pending_unbind,
            commands::generate_diagnostic_report,
            commands::read_crash_log,
            commands::take_unclean_exit_notice,
            commands::render_report_pdf,
            commands::get_update_status,
            commands::update_check,
            commands::update_download,
            commands::update_cancel_download,
            commands::update_install,
            selftest::is_selftest,
            selftest::selftest_seed,
            selftest::selftest_report,
            selftest::selftest_online_status,
            selftest::selftest_sign_in_as,
            selftest::selftest_seed_unbound,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| match event {
            // How a double-clicked project reaches the app on macOS — cold or
            // warm. Apple delivers it as an Apple Event, surfaced here; Windows
            // and Linux pass the path as argv[1], which `setup` above reads.
            // On a cold launch this arrives before the webview is ready, which
            // is why `deliver_open_project` queues rather than emits.
            //
            // The gate has to be on the match arm, not just the behaviour:
            // `RunEvent::Opened` is itself `#[cfg(any(macos, ios, android))]`
            // in tauri, so on Windows this arm names a variant that does not
            // exist and the crate fails to compile. Mirror tauri's own cfg
            // exactly — a narrower one here would break Android later.
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
            RunEvent::Opened { urls } => {
                for url in urls {
                    if url.scheme() == "file" {
                        if let Ok(path) = url.to_file_path() {
                            deliver_open_project(app_handle, &path.to_string_lossy());
                        }
                    }
                }
            }
            // Quitting never routed through `close_project`, so the write-ahead
            // log survived every Cmd-Q with the session's work still in it.
            // Locally that reads fine; it only shows up as data loss once the
            // folder is copied to Drive without its sidecars.
            RunEvent::WindowEvent {
                event: tauri::WindowEvent::Focused(focused),
                ..
            } => {
                let state = app_handle.state::<AppState>();
                state.sync_coordinator.set_foreground(focused);
                if focused {
                    window_guard::clamp_main_window_to_work_area(app_handle);
                    state.request_background_sync(
                        app_handle.clone(),
                        sync_coordinator::SyncTrigger::WindowFocused,
                    );
                }
            }
            RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::ScaleFactorChanged { .. },
                ..
            } if label == "main" => {
                window_guard::clamp_main_window_to_work_area(app_handle);
            }
            RunEvent::Resumed => {
                window_guard::clamp_main_window_to_work_area(app_handle);
                let state = app_handle.state::<AppState>();
                state.sync_coordinator.set_foreground(true);
                state.request_background_sync(
                    app_handle.clone(),
                    sync_coordinator::SyncTrigger::WindowFocused,
                );
            }
            RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                // Native close guard: unapproved closes hold for the frontend
                // draft preflight instead of checkpointing past unsaved work.
                // Menu quit / Cmd+Q / Alt+F4 arrive as ExitRequested below;
                // the window X button arrives here. Both funnel to the one
                // typed choice controller in the frontend.
                let state = app_handle.state::<AppState>();
                if state.take_any_departure_approval().is_none() {
                    let intent_id = state.request_native_departure("close");
                    api.prevent_close();
                    let _ = app_handle.emit(
                        "notes://departure-requested",
                        serde_json::json!({ "intent_id": intent_id, "kind": "close" }),
                    );
                } else {
                    // Approved replay: fall through to normal close cleanup.
                    let state = app_handle.state::<AppState>();
                    state.checkpoint_open_project();
                    if let Ok(p) = state.project_path_str() {
                        open_marker::remove_marker(std::path::Path::new(&p));
                    }
                    if let Ok(app_dir) = app_data::app_data_dir(app_handle) {
                        run_marker::remove_current_run_marker(&app_dir);
                    }
                }
            }
            RunEvent::ExitRequested { api, .. } => {
                // Native quit guard: same preflight as window close. Forced OS
                // termination (SIGKILL / RunEvent::Exit) cannot be held — it
                // stays covered by acknowledged recovery writes only.
                let state = app_handle.state::<AppState>();
                if state.take_any_departure_approval().is_none() {
                    let intent_id = state.request_native_departure("quit");
                    api.prevent_exit();
                    let _ = app_handle.emit(
                        "notes://departure-requested",
                        serde_json::json!({ "intent_id": intent_id, "kind": "quit" }),
                    );
                } else {
                    let state = app_handle.state::<AppState>();
                    state.checkpoint_open_project();
                    if let Ok(p) = state.project_path_str() {
                        open_marker::remove_marker(std::path::Path::new(&p));
                    }
                    if let Ok(app_dir) = app_data::app_data_dir(app_handle) {
                        run_marker::remove_current_run_marker(&app_dir);
                    }
                }
            }
            RunEvent::Exit => {
                let state = app_handle.state::<AppState>();
                state.checkpoint_open_project();
                if let Ok(p) = state.project_path_str() {
                    open_marker::remove_marker(std::path::Path::new(&p));
                }
                if let Ok(app_dir) = app_data::app_data_dir(app_handle) {
                    run_marker::remove_current_run_marker(&app_dir);
                }
            }
            _ => {}
        });
}
