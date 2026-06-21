#![cfg_attr(test, allow(dead_code))]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use crc32fast::Hasher as Crc32Hasher;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::UpdaterExt;

#[cfg(all(not(test), target_os = "macos"))]
use tauri::menu::{Menu, MenuItem, Submenu};

const RUN_EVENT: &str = "redumper://event";
const REDUMP_INFO_DISCS_URL: &str = "https://redump.info/discs";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Default)]
struct AppState {
    active: Mutex<Option<ActiveRun>>,
}

#[derive(Clone, Debug)]
struct ActiveRun {
    id: String,
    pid: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRequest {
    command: String,
    options: Vec<RunOption>,
    drive_mode: DriveMode,
    drive: Option<String>,
    image_path: Option<String>,
    image_name: Option<String>,
    working_directory: Option<String>,
    manual_command: Option<String>,
    #[serde(default = "default_output_subfolder")]
    output_subfolder: bool,
    archive_tool_path: Option<String>,
    #[serde(default = "default_compress_log_files")]
    compress_log_files: bool,
    #[serde(default)]
    archive_format: ArchiveFormat,
    #[serde(default)]
    dump_twice_compare_hashes: bool,
    danger_confirmed: bool,
}

fn default_compress_log_files() -> bool {
    true
}

fn default_output_subfolder() -> bool {
    true
}

#[derive(Clone, Debug)]
struct ArchiveRequest {
    output_directory: PathBuf,
    image_name: Option<String>,
    archive_tool_path: Option<String>,
    archive_format: ArchiveFormat,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum ArchiveFormat {
    SevenZip,
    Zip,
}

impl Default for ArchiveFormat {
    fn default() -> Self {
        Self::SevenZip
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunOption {
    flag: String,
    value: Option<String>,
    enabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum DriveMode {
    Auto,
    Manual,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    app_version: String,
    upstream_tag: String,
    upstream_app_version: String,
    platform: String,
    arch: String,
    default_output_dir: String,
    redumper_path: String,
    redumper_available: bool,
    resource_dir: String,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckResult {
    available: bool,
    current_version: String,
    latest_version: Option<String>,
    body: Option<String>,
    date: Option<String>,
    download_url: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostic {
    level: String,
    message: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DriveCandidate {
    path: String,
    label: String,
    source: String,
    volume_name: Option<String>,
    redump_compliant: bool,
    generic_mode_required: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ExistingImageCandidate {
    directory: String,
    image_name: String,
    files: Vec<String>,
    supports_refine: bool,
    supports_split: bool,
    supports_hash: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingOutputConflict {
    exists: bool,
    directory: String,
    matches: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct ExistingImageMatchContext {
    volume_key: Option<String>,
    no_volume_title: bool,
}

impl ExistingImageMatchContext {
    fn from_drive(drive_volume_name: Option<String>, drive_label: Option<String>) -> Option<Self> {
        let mut volume_candidates = Vec::new();
        if let Some(value) = drive_volume_name
            .as_deref()
            .and_then(non_empty_trimmed_string)
        {
            volume_candidates.push(value);
        }
        if let Some(value) = drive_label
            .as_deref()
            .and_then(volume_name_from_drive_label)
        {
            volume_candidates.push(value);
        }
        let volume_name = volume_candidates
            .iter()
            .find(|value| !is_no_volume_title(value))
            .cloned()
            .or_else(|| volume_candidates.first().cloned());

        let no_volume_title = volume_name
            .as_deref()
            .map(is_no_volume_title)
            .unwrap_or_else(|| {
                drive_label
                    .as_deref()
                    .is_none_or(drive_label_has_no_volume_title)
            });

        Some(Self {
            volume_key: volume_name
                .as_deref()
                .filter(|value| !is_no_volume_title(value))
                .map(normalize_existing_image_match_text)
                .filter(|value| !value.is_empty()),
            no_volume_title,
        })
    }

    fn matches(&self, directory: &Path, image_name: &str, files: &[String]) -> bool {
        if let Some(volume_key) = &self.volume_key {
            return existing_image_match_tokens(directory, image_name, files)
                .into_iter()
                .any(|token| token.contains(volume_key));
        }

        if self.no_volume_title {
            return existing_image_match_tokens(directory, image_name, files)
                .into_iter()
                .any(|token| token_is_no_volume_title(&token));
        }

        true
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RunEvent {
    run_id: String,
    kind: String,
    stream: Option<String>,
    line: Option<String>,
    stage: Option<String>,
    progress: Option<ProgressEvent>,
    exit_code: Option<i32>,
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_iso_path: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    percentage: Option<u32>,
    lba_current: Option<i64>,
    lba_total: Option<i64>,
    scsi_errors: Option<u64>,
    c2_errors: Option<u64>,
    q_errors: Option<u64>,
    edc_errors: Option<u64>,
}

#[cfg(not(test))]
fn main() {
    let builder = tauri::Builder::default().manage(AppState::default());

    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(|app| {
            let settings =
                MenuItem::with_id(app, "settings", "Settings", true, Some("CmdOrCtrl+,"))?;
            let close = MenuItem::with_id(app, "close", "Close", true, Some("CmdOrCtrl+W"))?;
            let file = Submenu::with_items(app, "File", true, &[&settings, &close])?;
            Menu::with_items(app, &[&file])
        })
        .on_menu_event(|app, event| {
            if event.id() == "settings" {
                let _ = open_settings_window(app);
            } else if event.id() == "close" {
                close_focused_window(app);
            }
        });

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            list_drives,
            find_existing_image_candidate,
            check_output_conflict,
            run_redumper,
            cancel_redumper,
            delete_duplicate_iso,
            save_log_file,
            show_settings_window,
            check_for_updates,
            install_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running Redumper UI");
}

#[cfg(test)]
fn main() {}

fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window
            .show()
            .map_err(|e| format!("Unable to show Settings window: {e}"))?;
        let _ = window.set_size(tauri::LogicalSize::new(820.0, 640.0));
        window
            .set_focus()
            .map_err(|e| format!("Unable to focus Settings window: {e}"))?;
        return Ok(());
    }

    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("?window=settings".into()))
        .title("Settings")
        .inner_size(820.0, 640.0)
        .min_inner_size(720.0, 520.0)
        .resizable(true)
        .build()
        .map_err(|e| format!("Unable to open Settings window: {e}"))?;

    Ok(())
}

#[tauri::command]
fn show_settings_window(app: AppHandle) -> Result<(), String> {
    open_settings_window(&app)
}

#[cfg(all(not(test), target_os = "macos"))]
fn close_focused_window(app: &AppHandle) {
    if let Some(window) = app
        .webview_windows()
        .into_values()
        .find(|window| window.is_focused().unwrap_or(false))
    {
        let _ = window.close();
    } else if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
}

#[tauri::command]
fn get_app_info(app: AppHandle) -> Result<AppInfo, String> {
    let manifest = upstream_manifest(&app)?;
    let redumper_path = redumper_executable_path(&app)?;
    let resource_dir = resource_root(&app)?;
    let default_output_dir = default_output_root(&app);

    let mut diagnostics = Vec::new();
    if !redumper_path.exists() {
        diagnostics.push(Diagnostic {
            level: "warning".to_string(),
            message: "Bundled redumper binary is missing. Run npm run prepare-redumper before launching commands.".to_string(),
        });
    }

    if cfg!(target_os = "macos") {
        if let Ok(exe) = std::env::current_exe() {
            let path = exe.to_string_lossy();
            if path.contains("/Desktop/")
                || path.contains("/Downloads/")
                || path.contains("/Documents/")
            {
                diagnostics.push(Diagnostic {
                    level: "warning".to_string(),
                    message: "macOS may block SCSI access when the app runs from Desktop, Downloads, or Documents. Move the app to Applications or another unrestricted folder.".to_string(),
                });
            }
        }
    }

    if cfg!(target_os = "linux") {
        diagnostics.push(Diagnostic {
            level: "info".to_string(),
            message: "Linux dumping requires a generic SCSI device such as /dev/sg0 and the disc should be unmounted before running redumper.".to_string(),
        });
    }

    Ok(AppInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        upstream_tag: manifest
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        upstream_app_version: manifest
            .get("appVersion")
            .and_then(|v| v.as_str())
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        default_output_dir: default_output_dir.to_string_lossy().to_string(),
        redumper_path: redumper_path.to_string_lossy().to_string(),
        redumper_available: redumper_path.exists(),
        resource_dir: resource_dir.to_string_lossy().to_string(),
        diagnostics,
    })
}

#[tauri::command]
fn list_drives(app: AppHandle) -> Result<Vec<DriveCandidate>, String> {
    let mut candidates = platform_drive_candidates();
    let recommended_drives = recommended_drive_signatures(&app).unwrap_or_default();
    mark_drive_compliance(&mut candidates, &recommended_drives);
    Ok(candidates)
}

#[tauri::command]
fn find_existing_image_candidate(
    directory: String,
    drive_volume_name: Option<String>,
    drive_label: Option<String>,
) -> Result<Option<ExistingImageCandidate>, String> {
    image_candidate_for_directory(
        Path::new(&directory),
        ExistingImageMatchContext::from_drive(drive_volume_name, drive_label),
    )
}

#[tauri::command]
fn delete_duplicate_iso(path: String) -> Result<String, String> {
    let candidate = PathBuf::from(path.trim());
    if candidate.as_os_str().is_empty() {
        return Err("Duplicate ISO path is empty.".to_string());
    }
    if deletable_duplicate_iso_path(&candidate).is_none() {
        return Err("Only duplicate _verify.iso files can be deleted.".to_string());
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|e| {
        format!(
            "Unable to inspect duplicate ISO {}: {e}",
            candidate.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Duplicate ISO is not a regular file: {}",
            candidate.display()
        ));
    }

    fs::remove_file(&candidate).map_err(|e| {
        format!(
            "Unable to delete duplicate ISO {}: {e}",
            candidate.display()
        )
    })?;
    let deleted_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| candidate.to_string_lossy().to_string());
    Ok(format!("Deleted duplicate ISO {deleted_name}."))
}

#[tauri::command]
fn save_log_file(path: String, contents: String) -> Result<String, String> {
    let destination = PathBuf::from(path.trim());
    if destination.as_os_str().is_empty() {
        return Err("Log path is empty.".to_string());
    }
    if destination.is_dir() {
        return Err(format!(
            "Log path points to a directory: {}",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(format!(
                "Log directory does not exist: {}",
                parent.display()
            ));
        }
    }

    fs::write(&destination, contents)
        .map_err(|e| format!("Unable to save log to {}: {e}", destination.display()))?;
    Ok(destination.to_string_lossy().to_string())
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<UpdateCheckResult, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let updater = app
        .updater()
        .map_err(|e| format!("Updater is not configured correctly: {e}"))?;

    match updater.check().await {
        Ok(Some(update)) => {
            let latest_version = update.version.clone();
            Ok(UpdateCheckResult {
                available: true,
                current_version,
                latest_version: Some(latest_version.clone()),
                body: update.body.clone(),
                date: update.date.map(|date| date.to_string()),
                download_url: Some(update.download_url.to_string()),
                message: format!("Update {latest_version} is available."),
            })
        }
        Ok(None) => Ok(UpdateCheckResult {
            available: false,
            current_version,
            latest_version: None,
            body: None,
            date: None,
            download_url: None,
            message: "Redumper UI is up to date.".to_string(),
        }),
        Err(error) => Err(format!("Update check failed: {error}")),
    }
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<String, String> {
    let updater = app
        .updater()
        .map_err(|e| format!("Updater is not configured correctly: {e}"))?;

    let Some(update) = updater
        .check()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?
    else {
        return Ok("Redumper UI is already up to date.".to_string());
    };

    let version = update.version.clone();
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("Update install failed: {e}"))?;

    Ok(format!(
        "Update {version} was installed. Restart Redumper UI to finish."
    ))
}

#[tauri::command]
fn run_redumper(
    app: AppHandle,
    state: State<AppState>,
    request: RunRequest,
) -> Result<String, String> {
    validate_request(&request)?;

    let redumper_path = redumper_launch_path(&app)?;
    if !redumper_path.exists() {
        return Err(format!(
            "Bundled redumper binary was not found at {}",
            redumper_path.display()
        ));
    }

    let effective_command = effective_request_command(&request)?;
    let image_path = resolve_output_directory(&app, &request, &effective_command);
    if command_writes_files(&effective_command) {
        fs::create_dir_all(&image_path)
            .map_err(|e| format!("Unable to create output directory: {e}"))?;
    }

    let working_dir = request
        .working_directory
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| image_path.clone());

    if request.dump_twice_compare_hashes {
        return run_dump_twice_compare_workflow(
            app,
            &state,
            request,
            redumper_path,
            image_path,
            working_dir,
        );
    }

    let args = build_args(&request, &image_path)?;

    let run_id = new_run_id();
    let mut command = Command::new(&redumper_path);
    command.args(&args);
    configure_redumper_command_environment(&mut command, &redumper_path);
    command.current_dir(&working_dir);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start redumper: {e}"))?;
    let pid = child.id();

    {
        let mut active = state
            .active
            .lock()
            .map_err(|_| "Run state is poisoned".to_string())?;
        if active.is_some() {
            let _ = kill_pid(pid, true);
            return Err("A redumper command is already running".to_string());
        }
        *active = Some(ActiveRun {
            id: run_id.clone(),
            pid: Some(pid),
        });
    }

    emit_event(
        &app,
        RunEvent {
            run_id: run_id.clone(),
            kind: "started".to_string(),
            stream: None,
            line: None,
            stage: None,
            progress: None,
            exit_code: None,
            message: Some(format!("redumper {}", shell_preview(&args))),
            duplicate_iso_path: None,
        },
    );

    if let Some(stdout) = child.stdout.take() {
        spawn_reader(app.clone(), run_id.clone(), "stdout", stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(app.clone(), run_id.clone(), "stderr", stderr);
    }

    let archive_request = if request.compress_log_files && command_writes_files(&effective_command)
    {
        Some(ArchiveRequest {
            output_directory: image_path.clone(),
            image_name: request.image_name.clone(),
            archive_tool_path: request.archive_tool_path.clone(),
            archive_format: request.archive_format,
        })
    } else {
        None
    };

    let app_for_wait = app.clone();
    let run_id_for_wait = run_id.clone();
    thread::spawn(move || {
        let exit = child.wait();
        let exit_code = match &exit {
            Ok(status) => status.code(),
            Err(_) => None,
        };

        if exit
            .as_ref()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            if let Some(archive_request) = archive_request {
                emit_event(
                    &app_for_wait,
                    RunEvent {
                        run_id: run_id_for_wait.clone(),
                        kind: "stage".to_string(),
                        stream: None,
                        line: None,
                        stage: Some("ARCHIVE".to_string()),
                        progress: None,
                        exit_code: None,
                        message: Some("Archiving auxiliary dump files...".to_string()),
                        duplicate_iso_path: None,
                    },
                );

                match compress_log_files_into_archive(&archive_request) {
                    Ok(message) => emit_event(
                        &app_for_wait,
                        RunEvent {
                            run_id: run_id_for_wait.clone(),
                            kind: "stage".to_string(),
                            stream: None,
                            line: None,
                            stage: Some("ARCHIVE".to_string()),
                            progress: None,
                            exit_code: None,
                            message: Some(message),
                            duplicate_iso_path: None,
                        },
                    ),
                    Err(message) => emit_event(
                        &app_for_wait,
                        RunEvent {
                            run_id: run_id_for_wait.clone(),
                            kind: "warning".to_string(),
                            stream: None,
                            line: None,
                            stage: Some("ARCHIVE".to_string()),
                            progress: None,
                            exit_code: None,
                            message: Some(message),
                            duplicate_iso_path: None,
                        },
                    ),
                }
            }
        }

        emit_event(
            &app_for_wait,
            RunEvent {
                run_id: run_id_for_wait.clone(),
                kind: "exit".to_string(),
                stream: None,
                line: None,
                stage: Some("END".to_string()),
                progress: None,
                exit_code,
                message: exit.err().map(|e| format!("redumper wait failed: {e}")),
                duplicate_iso_path: None,
            },
        );

        let state = app_for_wait.state::<AppState>();
        if let Ok(mut active) = state.active.lock() {
            if active
                .as_ref()
                .map(|run| run.id.as_str() == run_id_for_wait.as_str())
                .unwrap_or(false)
            {
                *active = None;
            }
        };
    });

    Ok(run_id)
}

#[tauri::command]
fn check_output_conflict(
    app: AppHandle,
    request: RunRequest,
) -> Result<ExistingOutputConflict, String> {
    validate_request(&request)?;

    let effective_command = effective_request_command(&request)?;
    let output_directory = resolve_output_directory(&app, &request, &effective_command);
    if !command_writes_files(&effective_command) || !command_uses_image_path(&effective_command) {
        return Ok(ExistingOutputConflict {
            exists: false,
            directory: output_directory.to_string_lossy().to_string(),
            matches: Vec::new(),
        });
    }

    let output_subfolder =
        request.output_subfolder && command_uses_output_subfolder(&effective_command);
    let matches = existing_output_matches(
        &output_directory,
        request.image_name.as_deref(),
        output_subfolder,
    )?;

    Ok(ExistingOutputConflict {
        exists: !matches.is_empty(),
        directory: output_directory.to_string_lossy().to_string(),
        matches,
    })
}

fn run_dump_twice_compare_workflow(
    app: AppHandle,
    state: &State<AppState>,
    request: RunRequest,
    redumper_path: PathBuf,
    image_path: PathBuf,
    working_dir: PathBuf,
) -> Result<String, String> {
    let base_image_name = request
        .image_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Dump Twice if No Match requires an image name".to_string())?
        .to_string();
    let verify_image_name = format!("{base_image_name}_verify");

    let mut first_request = request.clone();
    first_request.dump_twice_compare_hashes = false;
    first_request.image_name = Some(base_image_name.clone());

    let mut second_request = first_request.clone();
    second_request.image_name = Some(verify_image_name.clone());

    let first_args = build_args(&first_request, &image_path)?;
    let second_args = build_args(&second_request, &image_path)?;
    let run_id = new_run_id();

    {
        let mut active = state
            .active
            .lock()
            .map_err(|_| "Run state is poisoned".to_string())?;
        if active.is_some() {
            return Err("A redumper command is already running".to_string());
        }
        *active = Some(ActiveRun {
            id: run_id.clone(),
            pid: None,
        });
    }

    let app_for_wait = app.clone();
    let run_id_for_wait = run_id.clone();
    thread::spawn(move || {
        emit_stage(
            &app_for_wait,
            &run_id_for_wait,
            "DUMP 1",
            format!("First dump will use image name {base_image_name}."),
        );

        let first_exit = match run_redumper_child(
            &app_for_wait,
            &run_id_for_wait,
            &redumper_path,
            &first_args,
            &working_dir,
        ) {
            Ok(code) => code,
            Err(message) => {
                emit_error(&app_for_wait, &run_id_for_wait, message);
                emit_exit(&app_for_wait, &run_id_for_wait, Some(1), None);
                clear_active_run(&app_for_wait, &run_id_for_wait);
                return;
            }
        };

        if first_exit.unwrap_or(1) != 0 {
            emit_exit(&app_for_wait, &run_id_for_wait, first_exit, None);
            clear_active_run(&app_for_wait, &run_id_for_wait);
            return;
        }

        emit_stage(
            &app_for_wait,
            &run_id_for_wait,
            "REDUMP",
            "Checking redump.info for a CRC32 match...".to_string(),
        );

        match redump_info_lookup_for_dump(&image_path, &base_image_name) {
            Ok(lookup) if lookup.matched => {
                emit_stage(
                    &app_for_wait,
                    &run_id_for_wait,
                    "REDUMP",
                    format!(
                        "redump.info already has a match for CRC32 {}; skipping second dump.",
                        lookup.crc32
                    ),
                );
                archive_successful_dump(
                    &app_for_wait,
                    &run_id_for_wait,
                    &first_request,
                    &image_path,
                    &base_image_name,
                );
                emit_exit(&app_for_wait, &run_id_for_wait, Some(0), None);
                clear_active_run(&app_for_wait, &run_id_for_wait);
                return;
            }
            Ok(lookup) => emit_stage(
                &app_for_wait,
                &run_id_for_wait,
                "REDUMP",
                format!(
                    "No redump.info match for CRC32 {}; running second dump.",
                    lookup.crc32
                ),
            ),
            Err(message) => emit_warning(
                &app_for_wait,
                &run_id_for_wait,
                "REDUMP",
                format!("Unable to check redump.info ({message}); running second dump."),
            ),
        }

        emit_stage(
            &app_for_wait,
            &run_id_for_wait,
            "DUMP 2",
            format!("Second dump will use image name {verify_image_name}."),
        );

        let second_exit = match run_redumper_child(
            &app_for_wait,
            &run_id_for_wait,
            &redumper_path,
            &second_args,
            &working_dir,
        ) {
            Ok(code) => code,
            Err(message) => {
                emit_error(&app_for_wait, &run_id_for_wait, message);
                emit_exit(&app_for_wait, &run_id_for_wait, Some(1), None);
                clear_active_run(&app_for_wait, &run_id_for_wait);
                return;
            }
        };

        if second_exit.unwrap_or(1) != 0 {
            emit_exit(&app_for_wait, &run_id_for_wait, second_exit, None);
            clear_active_run(&app_for_wait, &run_id_for_wait);
            return;
        }

        emit_stage(
            &app_for_wait,
            &run_id_for_wait,
            "VERIFY",
            "Comparing dump SHA-256 hashes...".to_string(),
        );

        let mut exit_code = Some(0);
        match compare_dump_hashes(&image_path, &base_image_name, &verify_image_name) {
            Ok(comparison) => emit_stage_with_duplicate_iso(
                &app_for_wait,
                &run_id_for_wait,
                "VERIFY",
                comparison.message,
                comparison.duplicate_iso_path,
            ),
            Err(message) => {
                exit_code = Some(1);
                emit_error(&app_for_wait, &run_id_for_wait, message);
            }
        }

        if exit_code == Some(0) {
            archive_successful_dump(
                &app_for_wait,
                &run_id_for_wait,
                &first_request,
                &image_path,
                &base_image_name,
            );
        }

        emit_exit(&app_for_wait, &run_id_for_wait, exit_code, None);
        clear_active_run(&app_for_wait, &run_id_for_wait);
    });

    Ok(run_id)
}

#[derive(Debug, PartialEq, Eq)]
struct RedumpInfoLookup {
    crc32: String,
    matched: bool,
}

fn redump_info_lookup_for_dump(
    output_directory: &Path,
    image_name: &str,
) -> Result<RedumpInfoLookup, String> {
    let dump_file = primary_dump_file(output_directory, image_name)?;
    let crc32 = crc32_file(&dump_file)?;
    let matched = redump_info_has_crc_match(&crc32)?;
    Ok(RedumpInfoLookup { crc32, matched })
}

fn redump_info_has_crc_match(crc32: &str) -> Result<bool, String> {
    let crc32 = normalize_crc32_query(crc32)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(format!("Redumper UI/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("unable to prepare redump.info request: {e}"))?;
    let html = client
        .get(REDUMP_INFO_DISCS_URL)
        .query(&[("q", crc32.as_str())])
        .send()
        .map_err(|e| format!("redump.info request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("redump.info returned an error: {e}"))?
        .text()
        .map_err(|e| format!("unable to read redump.info response: {e}"))?;
    parse_redump_info_disc_count(&html)
        .map(|count| count > 0)
        .ok_or_else(|| "redump.info response did not include a recognizable disc count".to_string())
}

fn normalize_crc32_query(crc32: &str) -> Result<String, String> {
    let normalized = crc32.trim().trim_start_matches("0x").to_ascii_lowercase();
    if normalized.len() == 8 && normalized.chars().all(|value| value.is_ascii_hexdigit()) {
        return Ok(normalized);
    }

    Err(format!("invalid CRC32 value: {crc32}"))
}

fn parse_redump_info_disc_count(html: &str) -> Option<u64> {
    for line in html.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(found_at) = lower.find("discs found") else {
            continue;
        };
        let before = &lower[..found_at];
        let digits_reversed: String = before
            .chars()
            .rev()
            .skip_while(|value| !value.is_ascii_digit())
            .take_while(|value| value.is_ascii_digit())
            .collect();
        if digits_reversed.is_empty() {
            continue;
        }
        let digits: String = digits_reversed.chars().rev().collect();
        if let Ok(count) = digits.parse() {
            return Some(count);
        }
    }

    let lower = html.to_ascii_lowercase();
    if lower.contains("no discs found") {
        Some(0)
    } else if lower.contains("data-href=\"/disc/") || lower.contains("href=\"/disc/") {
        Some(1)
    } else {
        None
    }
}

fn archive_successful_dump(
    app: &AppHandle,
    run_id: &str,
    request: &RunRequest,
    image_path: &Path,
    image_name: &str,
) {
    if !request.compress_log_files {
        return;
    }

    emit_stage(
        app,
        run_id,
        "ARCHIVE",
        "Archiving auxiliary dump files...".to_string(),
    );

    let archive_request = ArchiveRequest {
        output_directory: image_path.to_path_buf(),
        image_name: Some(image_name.to_string()),
        archive_tool_path: request.archive_tool_path.clone(),
        archive_format: request.archive_format,
    };
    match compress_log_files_into_archive(&archive_request) {
        Ok(message) => emit_stage(app, run_id, "ARCHIVE", message),
        Err(message) => emit_warning(app, run_id, "ARCHIVE", message),
    }
}

fn run_redumper_child(
    app: &AppHandle,
    run_id: &str,
    redumper_path: &Path,
    args: &[String],
    working_dir: &Path,
) -> Result<Option<i32>, String> {
    let mut command = Command::new(redumper_path);
    command.args(args);
    configure_redumper_command_environment(&mut command, redumper_path);
    command.current_dir(working_dir);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start redumper: {e}"))?;
    set_active_pid(app, run_id, Some(child.id()))?;

    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind: "started".to_string(),
            stream: None,
            line: None,
            stage: None,
            progress: None,
            exit_code: None,
            message: Some(format!("redumper {}", shell_preview(args))),
            duplicate_iso_path: None,
        },
    );

    if let Some(stdout) = child.stdout.take() {
        spawn_reader(app.clone(), run_id.to_string(), "stdout", stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(app.clone(), run_id.to_string(), "stderr", stderr);
    }

    let exit = child.wait();
    set_active_pid(app, run_id, None)?;

    match exit {
        Ok(status) => Ok(status.code()),
        Err(e) => Err(format!("redumper wait failed: {e}")),
    }
}

fn set_active_pid(app: &AppHandle, run_id: &str, pid: Option<u32>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut active = state
        .active
        .lock()
        .map_err(|_| "Run state is poisoned".to_string())?;
    let Some(active_run) = active.as_mut() else {
        return Err("Run was cancelled before redumper started.".to_string());
    };
    if active_run.id != run_id {
        return Err("Run state changed before redumper started.".to_string());
    }
    active_run.pid = pid;
    Ok(())
}

fn clear_active_run(app: &AppHandle, run_id: &str) {
    let state = app.state::<AppState>();
    if let Ok(mut active) = state.active.lock() {
        if active
            .as_ref()
            .map(|run| run.id.as_str() == run_id)
            .unwrap_or(false)
        {
            *active = None;
        }
    };
}

fn emit_stage(app: &AppHandle, run_id: &str, stage: &str, message: String) {
    emit_stage_with_duplicate_iso(app, run_id, stage, message, None);
}

fn emit_stage_with_duplicate_iso(
    app: &AppHandle,
    run_id: &str,
    stage: &str,
    message: String,
    duplicate_iso_path: Option<PathBuf>,
) {
    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind: "stage".to_string(),
            stream: None,
            line: None,
            stage: Some(stage.to_string()),
            progress: None,
            exit_code: None,
            message: Some(message),
            duplicate_iso_path: duplicate_iso_path.map(|path| path.to_string_lossy().to_string()),
        },
    );
}

fn emit_warning(app: &AppHandle, run_id: &str, stage: &str, message: String) {
    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind: "warning".to_string(),
            stream: None,
            line: None,
            stage: Some(stage.to_string()),
            progress: None,
            exit_code: None,
            message: Some(message),
            duplicate_iso_path: None,
        },
    );
}

fn emit_error(app: &AppHandle, run_id: &str, message: String) {
    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind: "error".to_string(),
            stream: None,
            line: None,
            stage: None,
            progress: None,
            exit_code: None,
            message: Some(message),
            duplicate_iso_path: None,
        },
    );
}

fn emit_exit(app: &AppHandle, run_id: &str, exit_code: Option<i32>, message: Option<String>) {
    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind: "exit".to_string(),
            stream: None,
            line: None,
            stage: Some("END".to_string()),
            progress: None,
            exit_code,
            message,
            duplicate_iso_path: None,
        },
    );
}

#[tauri::command]
fn cancel_redumper(state: State<AppState>) -> Result<(), String> {
    let active = state
        .active
        .lock()
        .map_err(|_| "Run state is poisoned".to_string())?
        .clone();

    let Some(run) = active else {
        return Ok(());
    };

    let Some(pid) = run.pid else {
        return Ok(());
    };

    kill_pid(pid, false)?;
    thread::sleep(Duration::from_millis(1800));

    let still_active = state
        .active
        .lock()
        .map_err(|_| "Run state is poisoned".to_string())?
        .as_ref()
        .map(|active| active.id == run.id)
        .unwrap_or(false);

    if still_active {
        kill_pid(pid, true)?;
    }

    Ok(())
}

fn upstream_manifest(app: &AppHandle) -> Result<serde_json::Value, String> {
    let fallback_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.redumper/upstream.json");
    let resolved_path = app
        .path()
        .resolve("redumper-upstream.json", BaseDirectory::Resource)
        .unwrap_or_else(|_| fallback_path.clone());
    let manifest_path = if resolved_path.exists() {
        resolved_path
    } else {
        fallback_path
    };
    let text = fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "Unable to read upstream manifest at {}: {e}",
            manifest_path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|e| format!("Invalid upstream manifest: {e}"))
}

fn resource_root(app: &AppHandle) -> Result<PathBuf, String> {
    let fallback_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/redumper");
    let resolved_path = app
        .path()
        .resolve("redumper", BaseDirectory::Resource)
        .unwrap_or_else(|_| fallback_path.clone());
    let resource_path = if resolved_path.exists() {
        resolved_path
    } else {
        fallback_path
    };
    Ok(resource_path)
}

fn redumper_executable_path(app: &AppHandle) -> Result<PathBuf, String> {
    let mut path = resource_root(app)?;
    path.push("bin");
    path.push(if cfg!(target_os = "windows") {
        "redumper.exe"
    } else {
        "redumper"
    });
    Ok(path)
}

fn default_output_root(app: &AppHandle) -> PathBuf {
    app.path()
        .home_dir()
        .map(|home| home.join("Downloads"))
        .or_else(|_| app.path().download_dir())
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_output_directory(app: &AppHandle, request: &RunRequest, command: &str) -> PathBuf {
    let base = request
        .image_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_output_root(app));

    resolve_output_directory_from_base(base, request, command)
}

fn resolve_output_directory_from_base(
    base: PathBuf,
    request: &RunRequest,
    command: &str,
) -> PathBuf {
    if !request.output_subfolder || !command_uses_output_subfolder(command) {
        return base;
    }

    let Some(folder_name) = request
        .image_name
        .as_deref()
        .map(safe_output_folder_name)
        .filter(|name| !name.is_empty())
    else {
        return base;
    };

    base.join(folder_name)
}

fn safe_output_folder_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' => '_',
            _ if ch.is_control() => '_',
            _ => ch,
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_string()
}

fn existing_output_matches(
    output_directory: &Path,
    image_name: Option<&str>,
    output_subfolder: bool,
) -> Result<Vec<String>, String> {
    if !output_directory.exists() {
        return Ok(Vec::new());
    }

    if !output_directory.is_dir() {
        let name = output_directory
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| output_directory.to_string_lossy().to_string());
        return Ok(vec![name]);
    }

    let image_name = image_name.map(str::trim).filter(|name| !name.is_empty());
    let mut matches = Vec::new();

    for entry in fs::read_dir(output_directory).map_err(|e| {
        format!(
            "Unable to inspect output directory {}: {e}",
            output_directory.display()
        )
    })? {
        let entry = entry.map_err(|e| format!("Unable to inspect output entry: {e}"))?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let is_match = if output_subfolder {
            true
        } else if let Some(image_name) = image_name {
            file_name == image_name
                || file_name.starts_with(&format!("{image_name}."))
                || file_name.starts_with(&format!("{image_name}_"))
        } else {
            false
        };

        if is_match {
            matches.push(file_name);
        }
        if matches.len() >= 8 {
            break;
        }
    }

    matches.sort();
    Ok(matches)
}

fn redumper_launch_path(app: &AppHandle) -> Result<PathBuf, String> {
    let bundled_path = redumper_executable_path(app)?;
    if !cfg!(target_os = "macos") {
        return Ok(bundled_path);
    }

    let staged_root = std::env::temp_dir()
        .join("redumper-ui")
        .join(format!("redumper-runtime-{}", env!("CARGO_PKG_VERSION")));
    let source_root = resource_root(app)?;

    if staged_root.exists() {
        fs::remove_dir_all(&staged_root).map_err(|e| {
            format!(
                "Unable to clear staged redumper runtime {}: {e}",
                staged_root.display()
            )
        })?;
    }
    copy_directory(&source_root, &staged_root)?;

    let mut staged_path = staged_root;
    staged_path.push("bin");
    staged_path.push("redumper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&staged_path)
            .map_err(|e| format!("Unable to inspect staged redumper binary: {e}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&staged_path, permissions)
            .map_err(|e| format!("Unable to mark staged redumper executable: {e}"))?;
    }
    Ok(staged_path)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| {
        format!(
            "Unable to create staged redumper directory {}: {e}",
            destination.display()
        )
    })?;

    for entry in fs::read_dir(source)
        .map_err(|e| format!("Unable to read redumper resource {}: {e}", source.display()))?
    {
        let entry = entry.map_err(|e| format!("Unable to read redumper resource entry: {e}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Unable to inspect redumper resource entry: {e}"))?;

        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_symlink() {
            copy_symlink(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|e| {
                format!(
                    "Unable to stage redumper resource {}: {e}",
                    source_path.display()
                )
            })?;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let target = fs::read_link(source)
        .map_err(|e| format!("Unable to read redumper symlink {}: {e}", source.display()))?;
    symlink(&target, destination).map_err(|e| {
        format!(
            "Unable to stage redumper symlink {} -> {}: {e}",
            destination.display(),
            target.display()
        )
    })
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    let target = fs::read_link(source)
        .map_err(|e| format!("Unable to read redumper symlink {}: {e}", source.display()))?;
    let resolved = source
        .parent()
        .map(|parent| parent.join(&target))
        .unwrap_or(target);
    fs::copy(&resolved, destination).map_err(|e| {
        format!(
            "Unable to stage redumper symlink target {}: {e}",
            resolved.display()
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DriveSignature {
    vendor: String,
    model: String,
}

fn recommended_drive_signatures(app: &AppHandle) -> Result<Vec<DriveSignature>, String> {
    let redumper_path = redumper_executable_path(app)?;
    let mut command = Command::new(&redumper_path);
    command.arg("--list-recommended-drives");
    configure_redumper_command_environment(&mut command, &redumper_path);
    let output = command
        .output()
        .map_err(|e| format!("Unable to query recommended redumper drives: {e}"))?;
    if !output.status.success() {
        return Err("redumper did not return recommended drives.".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_recommended_drive_signatures(&text))
}

fn configure_redumper_command_environment(command: &mut Command, redumper_path: &Path) {
    suppress_child_console(command);
    if let Some(resource_dir) = redumper_path.parent().and_then(|bin| bin.parent()) {
        let lib_dir = resource_dir.join("lib");
        if lib_dir.exists() {
            command.env("DYLD_LIBRARY_PATH", &lib_dir);
            command.env("LD_LIBRARY_PATH", &lib_dir);
        }
    }
}

fn suppress_child_console(_command: &mut Command) {
    #[cfg(windows)]
    {
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn parse_recommended_drive_signatures(text: &str) -> Vec<DriveSignature> {
    text.lines()
        .filter_map(parse_recommended_drive_signature)
        .collect()
}

fn parse_recommended_drive_signature(line: &str) -> Option<DriveSignature> {
    let (vendor, rest) = line.split_once(" - ")?;
    let model = rest
        .split_once("(revision level:")
        .map(|(model, _)| model)
        .unwrap_or(rest)
        .trim();
    let vendor = vendor.trim();
    if vendor.is_empty() || model.is_empty() {
        None
    } else {
        Some(DriveSignature {
            vendor: normalize_drive_match_text(vendor),
            model: normalize_drive_match_text(model),
        })
    }
}

fn mark_drive_compliance(candidates: &mut [DriveCandidate], recommended_drives: &[DriveSignature]) {
    for candidate in candidates {
        let compliant = drive_matches_recommended_list(&candidate.label, recommended_drives);
        candidate.redump_compliant = compliant;
        candidate.generic_mode_required = !compliant;
    }
}

fn drive_matches_recommended_list(label: &str, recommended_drives: &[DriveSignature]) -> bool {
    let normalized_label = normalize_drive_match_text(label);
    recommended_drives.iter().any(|drive| {
        !drive.vendor.is_empty()
            && !drive.model.is_empty()
            && normalized_label.contains(&drive.vendor)
            && normalized_label.contains(&drive.model)
    })
}

fn normalize_drive_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn validate_request(request: &RunRequest) -> Result<(), String> {
    if let Some(args) = parse_manual_command(request)? {
        if request.dump_twice_compare_hashes {
            return Err(
                "Manual command editing is not available with Dump Twice if No Match".to_string(),
            );
        }
        if args
            .first()
            .is_some_and(|command| command.starts_with("flash::"))
            && !request.danger_confirmed
        {
            return Err("Firmware flashing commands require explicit confirmation".to_string());
        }
        return Ok(());
    }

    if !allowed_commands().contains(request.command.as_str()) {
        return Err(format!("Unsupported redumper command: {}", request.command));
    }

    if request.command.starts_with("flash::") && !request.danger_confirmed {
        return Err("Firmware flashing commands require explicit confirmation".to_string());
    }

    if request.dump_twice_compare_hashes && !matches!(request.command.as_str(), "disc" | "dump") {
        return Err(
            "Dump Twice if No Match is only available for disc and dump commands".to_string(),
        );
    }

    if request.dump_twice_compare_hashes
        && request
            .image_name
            .as_deref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    {
        return Err("Dump Twice if No Match requires an image name".to_string());
    }

    if image_name_required(&request.command)
        && request
            .image_name
            .as_deref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(format!(
            "The {} command requires an image name",
            request.command
        ));
    }

    if request.drive_mode == DriveMode::Manual
        && request
            .drive
            .as_deref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        && command_uses_drive(&request.command)
    {
        return Err("Manual drive mode requires a drive path".to_string());
    }

    let flags = allowed_options();
    for option in &request.options {
        if option.enabled && !flags.contains(option.flag.as_str()) {
            return Err(format!("Unsupported redumper option: {}", option.flag));
        }
    }

    Ok(())
}

fn build_args(request: &RunRequest, image_path: &Path) -> Result<Vec<String>, String> {
    if let Some(args) = parse_manual_command(request)? {
        return Ok(args);
    }

    let mut args = vec![request.command.clone()];

    if request.drive_mode == DriveMode::Manual {
        if let Some(drive) = request.drive.as_deref().filter(|v| !v.trim().is_empty()) {
            args.push(format!("--drive={}", drive.trim()));
        }
    }

    if command_uses_image_path(&request.command) {
        args.push(format!("--image-path={}", image_path.to_string_lossy()));
    }
    if let Some(name) = request
        .image_name
        .as_deref()
        .filter(|v| !v.trim().is_empty())
    {
        args.push(format!("--image-name={}", name.trim()));
    }

    for option in &request.options {
        if !option.enabled {
            continue;
        }
        if option.flag == "--drive"
            || option.flag == "--image-path"
            || option.flag == "--image-name"
        {
            continue;
        }
        match option
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(value) => args.push(format!("{}={}", option.flag, value)),
            None => args.push(option.flag.clone()),
        }
    }

    Ok(args)
}

fn effective_request_command(request: &RunRequest) -> Result<String, String> {
    if let Some(args) = parse_manual_command(request)? {
        return args
            .first()
            .cloned()
            .ok_or_else(|| "Manual command must include a redumper command".to_string());
    }

    Ok(request.command.clone())
}

fn parse_manual_command(request: &RunRequest) -> Result<Option<Vec<String>>, String> {
    let Some(command) = request
        .manual_command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let mut tokens = split_command_line(command)?;
    if tokens
        .first()
        .is_some_and(|token| token.eq_ignore_ascii_case("redumper"))
    {
        tokens.remove(0);
    }
    if tokens.is_empty() {
        return Err("Manual command must include a redumper command".to_string());
    }

    let command = tokens[0].as_str();
    if !allowed_commands().contains(command) {
        return Err(format!("Unsupported redumper command: {command}"));
    }

    let flags = allowed_options();
    for token in tokens.iter().skip(1) {
        if !token.starts_with("--") {
            return Err(format!(
                "Manual command arguments must be redumper options: {token}"
            ));
        }
        let flag = token.split_once('=').map(|(flag, _)| flag).unwrap_or(token);
        if !flags.contains(flag) {
            return Err(format!("Unsupported redumper option: {flag}"));
        }
    }

    Ok(Some(tokens))
}

fn split_command_line(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                token.push(ch);
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(ch),
        }
    }

    if quote.is_some() {
        return Err("Manual command has an unterminated quote".to_string());
    }
    if !token.is_empty() {
        tokens.push(token);
    }

    Ok(tokens)
}

fn image_candidate_for_directory(
    directory: &Path,
    match_context: Option<ExistingImageMatchContext>,
) -> Result<Option<ExistingImageCandidate>, String> {
    if !directory.is_dir() {
        return Ok(None);
    }

    let mut log_files = Vec::new();
    let mut scram_files = Vec::new();
    let mut has_bin = false;
    for entry in fs::read_dir(directory)
        .map_err(|e| {
            format!(
                "Unable to read output directory {}: {e}",
                directory.display()
            )
        })?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match file_extension(&path).as_deref() {
            Some("log") => log_files.push(path),
            Some("scram") => scram_files.push(path),
            Some("bin") => has_bin = true,
            _ => {}
        }
    }

    if has_bin || log_files.is_empty() || scram_files.is_empty() {
        return Ok(None);
    }

    let has_read_errors = log_files.iter().try_fold(false, |found, path| {
        if found {
            return Ok(true);
        }
        log_file_has_refinable_read_errors(path)
    })?;
    if !has_read_errors {
        return Ok(None);
    }

    let mut files = Vec::new();
    for path in log_files.iter().chain(scram_files.iter()) {
        files.push(file_name(path)?);
    }
    files.sort();
    files.dedup();

    let image_name = scram_files
        .iter()
        .filter_map(|path| path.file_stem().and_then(|value| value.to_str()))
        .min()
        .unwrap_or_default()
        .to_string();

    if let Some(context) = match_context {
        if !context.matches(directory, &image_name, &files) {
            return Ok(None);
        }
    }

    Ok(Some(ExistingImageCandidate {
        directory: directory.to_string_lossy().to_string(),
        image_name,
        files,
        supports_refine: true,
        supports_split: false,
        supports_hash: false,
    }))
}

fn log_file_has_refinable_read_errors(path: &Path) -> Result<bool, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("Unable to read log file {}: {e}", path.display()))?;
    Ok(text.lines().any(line_has_refinable_read_errors))
}

fn line_has_refinable_read_errors(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("c2 shift") {
        return false;
    }
    if (lower.contains("c2 error") || lower.contains("scsi error"))
        && line_has_positive_number(&lower)
    {
        return true;
    }

    line_has_positive_metric(&lower, "c2") || line_has_positive_metric(&lower, "scsi")
}

fn line_has_positive_metric(line: &str, metric: &str) -> bool {
    let mut search_start = 0usize;
    while let Some(relative_index) = line[search_start..].find(metric) {
        let index = search_start + relative_index + metric.len();
        let after = &line[index..];
        let after = after.trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, ':' | 's' | '=' | '/' | ',' | '{' | '[' | '(')
        });
        let digits: String = after.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.parse::<u64>().unwrap_or(0) > 0 {
            return true;
        }
        search_start = index;
    }

    false
}

fn line_has_positive_number(line: &str) -> bool {
    line.split(|ch: char| !ch.is_ascii_digit())
        .filter(|value| !value.is_empty())
        .any(|value| value.parse::<u64>().unwrap_or(0) > 0)
}

fn existing_image_match_tokens(
    directory: &Path,
    image_name: &str,
    files: &[String],
) -> Vec<String> {
    let mut tokens = vec![normalize_existing_image_match_text(image_name)];
    if let Some(name) = directory.file_name().and_then(|value| value.to_str()) {
        tokens.push(normalize_existing_image_match_text(name));
    }
    tokens.extend(
        files
            .iter()
            .map(|file| normalize_existing_image_match_text(file)),
    );
    tokens
        .into_iter()
        .filter(|token| !token.is_empty())
        .collect()
}

fn normalize_existing_image_match_text(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn non_empty_trimmed_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn volume_name_from_drive_label(label: &str) -> Option<String> {
    let start = label.rfind('(')?;
    let end = label[start + 1..].find(')')? + start + 1;
    let value = label[start + 1..end].trim();
    if value.is_empty() || looks_like_drive_model_label(value) {
        None
    } else {
        Some(value.to_string())
    }
}

fn looks_like_drive_model_label(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("bd-")
        || lower.contains("dvd")
        || lower.contains("cd")
        || lower.contains("optical")
        || lower.contains("drive")
        || lower.contains("hl-dt-st")
        || lower.contains("matshita")
        || lower.contains("plextor")
        || lower.contains("pioneer")
        || lower.contains("asus")
        || lower.contains("tsstcorp")
}

fn drive_label_has_no_volume_title(label: &str) -> bool {
    volume_name_from_drive_label(label)
        .as_deref()
        .map(is_no_volume_title)
        .unwrap_or(true)
}

fn is_no_volume_title(value: &str) -> bool {
    token_is_no_volume_title(&normalize_existing_image_match_text(value))
}

fn token_is_no_volume_title(token: &str) -> bool {
    token.is_empty()
        || token == "not_applicable"
        || token == "no_file_system"
        || token.contains("not_applicable_no_file_system")
        || token.contains("no_volume")
        || token.contains("untitled")
}

fn file_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Unable to read file name for {}", path.display()))
}

#[derive(Debug, PartialEq, Eq)]
struct DumpHashComparison {
    message: String,
    duplicate_iso_path: Option<PathBuf>,
}

fn compare_dump_hashes(
    output_directory: &Path,
    first_image_name: &str,
    second_image_name: &str,
) -> Result<DumpHashComparison, String> {
    let first_file = primary_dump_file(output_directory, first_image_name)?;
    let second_file = primary_dump_file(output_directory, second_image_name)?;
    let first_hash = sha256_file(&first_file)?;
    let second_hash = sha256_file(&second_file)?;

    if first_hash == second_hash {
        return Ok(DumpHashComparison {
            message: format!(
                "Dump hashes match: {} and {} share SHA-256 {first_hash}.",
                first_file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(first_image_name),
                second_file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(second_image_name)
            ),
            duplicate_iso_path: deletable_duplicate_iso_path(&second_file),
        });
    }

    Err(format!(
        "Dump hash mismatch: {} = {first_hash}, {} = {second_hash}.",
        first_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(first_image_name),
        second_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(second_image_name)
    ))
}

fn deletable_duplicate_iso_path(path: &Path) -> Option<PathBuf> {
    if file_extension(path).as_deref() != Some("iso") {
        return None;
    }
    let stem = path.file_stem().and_then(|value| value.to_str())?;
    stem.ends_with("_verify").then(|| path.to_path_buf())
}

fn primary_dump_file(output_directory: &Path, image_name: &str) -> Result<PathBuf, String> {
    let image_name = image_name.trim();
    for extension in comparable_image_extensions() {
        let candidate = output_directory.join(format!("{image_name}.{extension}"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let entries = fs::read_dir(output_directory).map_err(|e| {
        format!(
            "Unable to find dump file for {image_name}: could not read {}: {e}",
            output_directory.display()
        )
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let stem_matches = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem == image_name)
            .unwrap_or(false);
        if stem_matches && is_comparable_image_file(&path) {
            return Ok(path);
        }
    }

    Err(format!(
        "Unable to compare hashes: no primary dump image was found for {image_name} in {}.",
        output_directory.display()
    ))
}

fn comparable_image_extensions() -> &'static [&'static str] {
    &["iso", "bin", "img", "raw", "scram", "sdram", "sbram"]
}

fn is_comparable_image_file(path: &Path) -> bool {
    file_extension(path)
        .map(|extension| comparable_image_extensions().contains(&extension.as_str()))
        .unwrap_or(false)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Unable to hash {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 64];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Unable to hash {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn crc32_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Unable to hash {}: {e}", path.display()))?;
    let mut hasher = Crc32Hasher::new();
    let mut buffer = [0_u8; 1024 * 64];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Unable to hash {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:08x}", hasher.finalize()))
}

fn compress_log_files_into_archive(request: &ArchiveRequest) -> Result<String, String> {
    let (image_prefix, candidates) = archive_candidates(request)?;

    if candidates.is_empty() {
        return Ok("No log files were available to compress.".to_string());
    }

    if request.archive_format == ArchiveFormat::Zip {
        let (archive_name, deleted, kept_logs) =
            compress_candidates_with_zip(request, &image_prefix, &candidates)?;
        return Ok(format!(
            "Created {archive_name}; archived {} file(s) as ZIP, deleted {deleted}, kept {kept_logs} .log file(s) outside the archive.",
            candidates.len()
        ));
    }

    if let Some(seven_zip) = find_7z_executable(request.archive_tool_path.as_deref()) {
        match compress_candidates_with_7z(request, &image_prefix, &candidates, &seven_zip) {
            Ok((archive_name, deleted, kept_logs)) => {
                return Ok(format!(
                    "Created {archive_name}; archived {} file(s) with 7z, deleted {deleted}, kept {kept_logs} .log file(s) outside the archive.",
                    candidates.len()
                ));
            }
            Err(message) => {
                let (archive_name, deleted, kept_logs) =
                    compress_candidates_with_zip(request, &image_prefix, &candidates)?;
                return Ok(format!(
                    "{message} Falling back to ZIP. Created {archive_name}; archived {} file(s), deleted {deleted}, kept {kept_logs} .log file(s) outside the archive.",
                    candidates.len()
                ));
            }
        }
    }

    let (archive_name, deleted, kept_logs) =
        compress_candidates_with_zip(request, &image_prefix, &candidates)?;
    Ok(format!(
        "7z was not found; created {archive_name} as ZIP fallback. Archived {} file(s), deleted {deleted}, kept {kept_logs} .log file(s) outside the archive.",
        candidates.len()
    ))
}

fn archive_candidates(request: &ArchiveRequest) -> Result<(String, Vec<String>), String> {
    let image_prefix = request
        .image_name
        .as_deref()
        .and_then(archive_prefix)
        .ok_or_else(|| {
            "Log compression skipped: no image name was available to limit the file set."
                .to_string()
        })?;

    let entries = fs::read_dir(&request.output_directory)
        .map_err(|e| format!("Log compression failed: unable to read output directory: {e}"))?;

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with(&image_prefix) || should_exclude_from_log_archive(file_name) {
            continue;
        }

        candidates.push(file_name.to_string());
    }

    candidates.sort();
    candidates.dedup();
    Ok((image_prefix, candidates))
}

fn archive_file_name(output_directory: &Path, image_prefix: &str, extension: &str) -> String {
    let mut archive_name = format!("{image_prefix}_logs.{extension}");
    if output_directory.join(&archive_name).exists() {
        archive_name = format!("{image_prefix}_logs_{}.{}", archive_timestamp(), extension);
    }
    archive_name
}

fn compress_candidates_with_7z(
    request: &ArchiveRequest,
    image_prefix: &str,
    candidates: &[String],
    seven_zip: &Path,
) -> Result<(String, usize, usize), String> {
    let archive_name = archive_file_name(&request.output_directory, image_prefix, "7z");
    let mut command = Command::new(seven_zip);
    suppress_child_console(&mut command);
    let output = command
        .current_dir(&request.output_directory)
        .arg("a")
        .arg("-t7z")
        .arg("-mx=9")
        .arg("-y")
        .arg(&archive_name)
        .args(candidates)
        .output()
        .map_err(|e| {
            format!(
                "Log compression failed: unable to run {}: {e}.",
                seven_zip.display()
            )
        })?;

    if !output.status.success() {
        let _ = fs::remove_file(request.output_directory.join(&archive_name));
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr
        };
        return Err(format!("Log compression failed with 7z: {detail}."));
    }

    let (deleted, kept_logs) = cleanup_archived_candidates(request, candidates, &archive_name)?;
    Ok((archive_name, deleted, kept_logs))
}

fn compress_candidates_with_zip(
    request: &ArchiveRequest,
    image_prefix: &str,
    candidates: &[String],
) -> Result<(String, usize, usize), String> {
    let archive_name = archive_file_name(&request.output_directory, image_prefix, "zip");
    let archive_path = request.output_directory.join(&archive_name);
    let archive_file = fs::File::create(&archive_path).map_err(|e| {
        format!(
            "Log compression failed: unable to create ZIP archive {}: {e}",
            archive_path.display()
        )
    })?;
    let mut zip = zip::ZipWriter::new(archive_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true)
        .unix_permissions(0o644);

    let zip_result = (|| -> Result<(), String> {
        for file_name in candidates {
            let file_path = request.output_directory.join(file_name);
            let mut input = fs::File::open(&file_path).map_err(|e| {
                format!(
                    "Log compression failed: unable to read {} for ZIP archive: {e}",
                    file_path.display()
                )
            })?;
            zip.start_file(file_name, options).map_err(|e| {
                format!("Log compression failed: unable to add {file_name} to ZIP: {e}")
            })?;
            io::copy(&mut input, &mut zip).map_err(|e| {
                format!("Log compression failed: unable to write {file_name} to ZIP: {e}")
            })?;
        }

        zip.finish()
            .map_err(|e| format!("Log compression failed: unable to finish ZIP archive: {e}"))?;
        Ok(())
    })();
    if let Err(error) = zip_result {
        let _ = fs::remove_file(&archive_path);
        return Err(error);
    }

    let (deleted, kept_logs) = cleanup_archived_candidates(request, candidates, &archive_name)?;
    Ok((archive_name, deleted, kept_logs))
}

fn cleanup_archived_candidates(
    request: &ArchiveRequest,
    candidates: &[String],
    archive_name: &str,
) -> Result<(usize, usize), String> {
    let mut deleted = 0usize;
    let mut kept_logs = 0usize;
    for file_name in candidates {
        if is_log_file(file_name) {
            kept_logs += 1;
            continue;
        }

        match fs::remove_file(request.output_directory.join(file_name)) {
            Ok(_) => deleted += 1,
            Err(e) => {
                return Err(format!(
                    "Log compression created {archive_name}, but failed deleting {file_name}: {e}"
                ));
            }
        }
    }
    Ok((deleted, kept_logs))
}

fn archive_prefix(image_name: &str) -> Option<String> {
    let trimmed = image_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = Path::new(trimmed);
    let name = path
        .file_stem()
        .or_else(|| path.file_name())?
        .to_string_lossy();
    let prefix = name.trim();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

fn find_7z_executable(custom_path: Option<&str>) -> Option<PathBuf> {
    for candidate in seven_zip_candidates(custom_path) {
        if is_7z_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn seven_zip_candidates(custom_path: Option<&str>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = custom_path.map(str::trim).filter(|path| !path.is_empty()) {
        let candidate = PathBuf::from(path);
        candidates.push(candidate.clone());
        candidates.push(candidate.join("7z.exe"));
        candidates.push(candidate.join("7z"));
        candidates.push(candidate.join("7zz"));
        candidates.push(candidate.join("7za"));
    }

    candidates.extend(
        [
            "7z",
            "7z.exe",
            "7zz",
            "7zz.exe",
            "7za",
            "7za.exe",
            "/opt/homebrew/bin/7z",
            "/opt/homebrew/bin/7zz",
            "/opt/homebrew/bin/7za",
            "/usr/local/bin/7z",
            "/usr/local/bin/7zz",
            "/usr/local/bin/7za",
        ]
        .map(PathBuf::from),
    );

    for base in [
        std::env::var_os("ProgramW6432"),
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Programs").into()),
    ]
    .into_iter()
    .flatten()
    {
        let base = PathBuf::from(base);
        for name in ["7z.exe", "7zz.exe", "7za.exe"] {
            candidates.push(base.join("7-Zip").join(name));
        }
    }

    candidates.extend(
        [
            r"C:\Program Files\7-Zip\7z.exe",
            r"C:\Program Files\7-Zip\7zz.exe",
            r"C:\Program Files\7-Zip\7za.exe",
            r"C:\Program Files (x86)\7-Zip\7z.exe",
            r"C:\Program Files (x86)\7-Zip\7zz.exe",
            r"C:\Program Files (x86)\7-Zip\7za.exe",
        ]
        .map(PathBuf::from),
    );

    candidates
}

fn is_7z_executable(candidate: &Path) -> bool {
    let mut command = Command::new(candidate);
    suppress_child_console(&mut command);
    command
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn should_exclude_from_log_archive(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let path = Path::new(&lower);
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    matches!(
        extension,
        "7z" | "zip"
            | "bin"
            | "cue"
            | "iso"
            | "img"
            | "mdf"
            | "mds"
            | "ccd"
            | "gdi"
            | "cdi"
            | "nrg"
            | "raw"
            | "scram"
            | "scrap"
            | "sdram"
            | "sbram"
    )
}

fn is_log_file(file_name: &str) -> bool {
    Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("log"))
        .unwrap_or(false)
}

fn archive_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn spawn_reader<R>(app: AppHandle, run_id: String, stream: &'static str, mut reader: R)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = [0_u8; 1024];
        let mut line = Vec::new();
        let mut throttle = OutputThrottle::default();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    if !line.is_empty() {
                        emit_output_line(&app, &run_id, stream, &line, &mut throttle);
                    }
                    break;
                }
                Ok(n) => {
                    for byte in &buf[..n] {
                        if *byte == b'\n' || *byte == b'\r' {
                            if !line.is_empty() {
                                emit_output_line(&app, &run_id, stream, &line, &mut throttle);
                                line.clear();
                            }
                        } else {
                            line.push(*byte);
                        }
                    }
                }
                Err(e) => {
                    emit_event(
                        &app,
                        RunEvent {
                            run_id: run_id.clone(),
                            kind: "error".to_string(),
                            stream: Some(stream.to_string()),
                            line: None,
                            stage: None,
                            progress: None,
                            exit_code: None,
                            message: Some(format!("Failed reading redumper {stream}: {e}")),
                            duplicate_iso_path: None,
                        },
                    );
                    break;
                }
            }
        }
    });
}

#[derive(Default)]
struct OutputThrottle {
    last_progress_event: Option<Instant>,
    last_progress_log: Option<Instant>,
    last_progress_percent: Option<u32>,
    last_transient_log: Option<Instant>,
}

fn emit_output_line(
    app: &AppHandle,
    run_id: &str,
    stream: &str,
    bytes: &[u8],
    throttle: &mut OutputThrottle,
) {
    let line = String::from_utf8_lossy(bytes).trim_end().to_string();
    if line.is_empty() {
        return;
    }

    let parsed = parse_line(&line);
    let mut kind = if stream == "stderr" {
        "stderr"
    } else {
        "stdout"
    }
    .to_string();
    if parsed.warning {
        kind = "warning".to_string();
    } else if parsed.error {
        kind = "error".to_string();
    }
    #[cfg(target_os = "macos")]
    let is_macos_unmount_error = line.contains("failed to unmount drive");
    let replaceable_progress_log = parsed.progress.is_some() || parsed.transient_progress;
    let progress_only = replaceable_progress_log
        && parsed.stage.is_none()
        && !parsed.warning
        && !parsed.error
        && stream != "stderr";

    if progress_only {
        let now = Instant::now();
        let percentage = parsed
            .progress
            .as_ref()
            .and_then(|progress| progress.percentage);
        if parsed.transient_progress && parsed.progress.is_none() {
            let transient_due = throttle
                .last_transient_log
                .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100));
            if transient_due {
                throttle.last_transient_log = Some(now);
                emit_event(
                    app,
                    RunEvent {
                        run_id: run_id.to_string(),
                        kind: "progress".to_string(),
                        stream: Some(stream.to_string()),
                        line: Some(line),
                        stage: None,
                        progress: None,
                        exit_code: None,
                        message: None,
                        duplicate_iso_path: None,
                    },
                );
            }

            return;
        }

        let progress_due = throttle
            .last_progress_event
            .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(250));
        let log_due = throttle
            .last_progress_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(3));
        let percent_changed = percentage.is_some() && percentage != throttle.last_progress_percent;

        if progress_due || percent_changed {
            throttle.last_progress_event = Some(now);
            throttle.last_progress_percent = percentage;
            emit_event(
                app,
                RunEvent {
                    run_id: run_id.to_string(),
                    kind: "progress".to_string(),
                    stream: Some(stream.to_string()),
                    line: if log_due || percent_changed {
                        throttle.last_progress_log = Some(now);
                        Some(line)
                    } else {
                        None
                    },
                    stage: None,
                    progress: parsed.progress,
                    exit_code: None,
                    message: None,
                    duplicate_iso_path: None,
                },
            );
        }

        return;
    }

    emit_event(
        app,
        RunEvent {
            run_id: run_id.to_string(),
            kind,
            stream: Some(stream.to_string()),
            line: Some(line),
            stage: parsed.stage,
            progress: parsed.progress,
            exit_code: None,
            message: None,
            duplicate_iso_path: None,
        },
    );

    #[cfg(target_os = "macos")]
    if is_macos_unmount_error {
        emit_event(
            app,
            RunEvent {
                run_id: run_id.to_string(),
                kind: "warning".to_string(),
                stream: Some(stream.to_string()),
                line: None,
                stage: None,
                progress: None,
                exit_code: None,
                message: Some(
                    "macOS reported that redumper could not take exclusive access to the optical drive. The same failure can occur when redumper is run directly, so redumper exited before dumping started. Close apps that may be touching the mounted disc, eject/reinsert the disc, then retry."
                        .to_string(),
                ),
                duplicate_iso_path: None,
            },
        );
    }
}

#[derive(Default)]
struct ParsedLine {
    stage: Option<String>,
    progress: Option<ProgressEvent>,
    transient_progress: bool,
    warning: bool,
    error: bool,
}

fn parse_line(line: &str) -> ParsedLine {
    let trimmed = line.trim();
    let mut parsed = ParsedLine::default();

    if let Some(rest) = trimmed.strip_prefix("*** ") {
        parsed.stage = Some(rest.split('(').next().unwrap_or(rest).trim().to_string());
    }

    let lower = trimmed.to_ascii_lowercase();
    parsed.warning = lower.starts_with("warning:") || lower.contains(" warning:");
    parsed.error = lower.starts_with("error:");
    parsed.transient_progress = is_replaceable_redumper_status(trimmed, &lower);

    if trimmed.contains("LBA:") && trimmed.contains('%') {
        parsed.progress = Some(ProgressEvent {
            percentage: parse_percentage(trimmed),
            lba_current: parse_lba_pair(trimmed).map(|pair| pair.0),
            lba_total: parse_lba_pair(trimmed).map(|pair| pair.1),
            scsi_errors: parse_u64_after(trimmed, "SCSIs:")
                .or_else(|| parse_u64_after(trimmed, "SCSI:")),
            c2_errors: parse_u64_after(trimmed, "C2s:"),
            q_errors: parse_u64_after(trimmed, "Q:"),
            edc_errors: parse_u64_after(trimmed, "EDC:"),
        });
    }

    parsed
}

fn is_replaceable_redumper_status(trimmed: &str, lower: &str) -> bool {
    if trimmed.starts_with('<')
        || trimmed.starts_with("*** ")
        || lower.starts_with("arguments:")
        || lower.starts_with("warning:")
        || lower.starts_with("error:")
    {
        return false;
    }

    let is_hash_status = lower.contains("hash")
        || lower.contains("crc")
        || lower.contains("md5")
        || lower.contains("sha1")
        || lower.contains("sha-1")
        || lower.contains("sha256")
        || lower.contains("sha-256");
    let is_skeleton_status = lower.contains("skeleton");
    let looks_incremental = trimmed.contains('%')
        || trimmed.contains('\u{2588}')
        || trimmed.contains("...")
        || lower.contains("progress")
        || lower.contains("calculat")
        || lower.contains("creat")
        || lower.contains("writ")
        || lower.contains("read");

    (is_hash_status || is_skeleton_status) && looks_incremental
}

fn parse_percentage(line: &str) -> Option<u32> {
    let percent = line.find('%')?;
    let before = &line[..percent];
    let start = before.rfind('[').map(|i| i + 1).unwrap_or(0);
    before[start..].trim().parse().ok()
}

fn parse_lba_pair(line: &str) -> Option<(i64, i64)> {
    let start = line.find("LBA:")? + 4;
    let rest = line[start..].trim_start();
    let slash = rest.find('/')?;
    let current = rest[..slash].trim().parse().ok()?;
    let after = &rest[slash + 1..];
    let end = after
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(after.len());
    let total = after[..end].trim().parse().ok()?;
    Some((current, total))
}

fn parse_u64_after(line: &str, marker: &str) -> Option<u64> {
    let start = line.find(marker)? + marker.len();
    let rest = line[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn emit_event(app: &AppHandle, event: RunEvent) {
    let _ = app.emit(RUN_EVENT, event);
}

fn new_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("run-{millis}")
}

fn shell_preview(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') {
                format!("{:?}", arg)
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(unix)]
fn kill_pid(pid: u32, force: bool) -> Result<(), String> {
    let signal = if force { libc::SIGKILL } else { libc::SIGINT };
    let result = unsafe { libc::kill(pid as i32, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!("Unable to signal redumper process {pid}"))
    }
}

#[cfg(windows)]
fn kill_pid(pid: u32, force: bool) -> Result<(), String> {
    let mut cmd = Command::new("taskkill");
    suppress_child_console(&mut cmd);
    cmd.arg("/PID").arg(pid.to_string()).arg("/T");
    if force {
        cmd.arg("/F");
    }
    let status = cmd
        .status()
        .map_err(|e| format!("Unable to run taskkill: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Unable to stop redumper process {pid}"))
    }
}

fn platform_drive_candidates() -> Vec<DriveCandidate> {
    #[cfg(target_os = "linux")]
    {
        linux_drive_candidates()
    }
    #[cfg(target_os = "macos")]
    {
        macos_drive_candidates()
    }
    #[cfg(target_os = "windows")]
    {
        windows_drive_candidates()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn linux_drive_candidates() -> Vec<DriveCandidate> {
    let mut drives = Vec::new();
    let mut seen = HashSet::new();
    for base in [
        "/sys/subsystem/scsi/devices",
        "/sys/bus/scsi/devices",
        "/sys/class/scsi/devices",
        "/sys/block/scsi/devices",
    ] {
        let base_path = Path::new(base);
        if !base_path.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(base_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                let device_type = fs::read_to_string(path.join("type")).unwrap_or_default();
                if device_type.trim() != "5" {
                    continue;
                }
                let sg_path = path.join("scsi_generic");
                if let Ok(generic_entries) = fs::read_dir(sg_path) {
                    for generic in generic_entries.flatten() {
                        let device = format!("/dev/{}", generic.file_name().to_string_lossy());
                        let label = linux_drive_label(&path, &device);
                        push_drive_candidate(&mut drives, &mut seen, device, label, "sysfs", None);
                    }
                }
            }
        }
        if !drives.is_empty() {
            break;
        }
    }
    drives.sort_by(|a, b| a.path.cmp(&b.path));
    drives
}

#[cfg(target_os = "linux")]
fn linux_drive_label(sysfs_path: &Path, device: &str) -> String {
    let name = [
        read_trimmed_file(&sysfs_path.join("vendor")),
        read_trimmed_file(&sysfs_path.join("model")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");

    if name.is_empty() {
        device.to_string()
    } else {
        format!("{device} ({name})")
    }
}

#[cfg(target_os = "linux")]
fn read_trimmed_file(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn macos_drive_candidates() -> Vec<DriveCandidate> {
    let fallback = scan_macos_optical_nodes();
    let output = Command::new("system_profiler")
        .args(["SPDiscBurningDataType", "-json"])
        .output();

    if let Ok(output) = output {
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
        let mut candidates = parse_macos_system_profiler_candidates(&json, &fallback);
        if !candidates.is_empty() {
            macos_add_volume_names(&mut candidates);
            return candidates;
        }
    }

    let mut candidates = macos_fallback_candidates(&fallback);
    macos_add_volume_names(&mut candidates);
    candidates
}

#[cfg(target_os = "macos")]
fn scan_macos_optical_nodes() -> HashMap<String, String> {
    let output = Command::new("diskutil").args(["list"]).output();
    let Ok(output) = output else {
        return HashMap::new();
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut candidates: HashMap<String, String> =
        parse_macos_diskutil_candidates(&text).into_iter().collect();

    for node in parse_macos_diskutil_nodes(&text) {
        if candidates.values().any(|candidate| candidate == &node) {
            continue;
        }
        if let Some(name) = macos_diskutil_info_optical_name(&format!("/dev/{node}")) {
            candidates.entry(name).or_insert(node);
        }
    }

    candidates
}

#[cfg(target_os = "windows")]
fn windows_drive_candidates() -> Vec<DriveCandidate> {
    let script = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
$drives = Get-CimInstance Win32_CDROMDrive | Select-Object Name, Drive, VolumeName, MediaLoaded;
if ($null -eq $drives) { "[]" } else { $drives | ConvertTo-Json -Compress }
"#;
    let mut command = Command::new("powershell");
    suppress_child_console(&mut command);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    parse_windows_cdrom_candidates(&String::from_utf8_lossy(&output.stdout))
}

fn push_drive_candidate(
    drives: &mut Vec<DriveCandidate>,
    seen: &mut HashSet<String>,
    path: String,
    label: String,
    source: &str,
    volume_name: Option<String>,
) {
    let path = path.trim().to_string();
    if path.is_empty() {
        return;
    }

    if seen.insert(path.to_ascii_lowercase()) {
        drives.push(DriveCandidate {
            label,
            path,
            source: source.to_string(),
            volume_name,
            redump_compliant: false,
            generic_mode_required: true,
        });
    }
}

fn parse_windows_cdrom_candidates(text: &str) -> Vec<DriveCandidate> {
    let text = text.trim();
    if text.is_empty() || text == "[]" {
        return Vec::new();
    }

    let json = serde_json::from_str::<serde_json::Value>(text).unwrap_or_default();
    let drives_json = match json {
        serde_json::Value::Array(items) => items,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => Vec::new(),
    };

    let mut drives = Vec::new();
    let mut seen = HashSet::new();
    for drive in drives_json {
        if drive
            .get("MediaLoaded")
            .and_then(|value| value.as_bool())
            .is_some_and(|loaded| !loaded)
        {
            continue;
        }

        let Some(path) = drive
            .get("Drive")
            .and_then(|value| value.as_str())
            .and_then(normalize_windows_drive_letter)
        else {
            continue;
        };

        let name = drive
            .get("Name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Optical Drive");
        let volume_name = drive
            .get("VolumeName")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let label = if let Some(volume_name) = volume_name {
            format!("{path} ({name}) - {volume_name}")
        } else if name.is_empty() {
            path.clone()
        } else {
            format!("{path} ({name})")
        };

        push_drive_candidate(
            &mut drives,
            &mut seen,
            path,
            label,
            "Win32_CDROMDrive",
            volume_name.map(str::to_string),
        );
    }

    drives.sort_by(|a, b| a.path.cmp(&b.path));
    drives
}

fn normalize_windows_drive_letter(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches(&['\\', '/'][..]);
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.ends_with(':') {
        return Some(trimmed.to_string());
    }
    if trimmed.len() == 1 && trimmed.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Some(format!("{trimmed}:"));
    }
    Some(trimmed.to_string())
}

fn parse_macos_system_profiler_candidates(
    json: &serde_json::Value,
    fallback_nodes: &HashMap<String, String>,
) -> Vec<DriveCandidate> {
    let drives_json = json
        .get("SPDiscBurningDataType")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut drives = Vec::new();
    let mut seen = HashSet::new();

    for drive in drives_json {
        let Some(name) = drive
            .get("_name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let node = [
            "spdisc_burner-devicenode",
            "spdisc_burning_device",
            "bsd_name",
        ]
        .iter()
        .find_map(|key| drive.get(key).and_then(|value| value.as_str()))
        .map(normalize_macos_node)
        .or_else(|| fallback_nodes.get(name).cloned());

        let Some(node) = node.filter(|value| !value.is_empty()) else {
            continue;
        };

        push_drive_candidate(
            &mut drives,
            &mut seen,
            node.clone(),
            macos_drive_label(name, &node),
            "system_profiler",
            None,
        );
    }

    drives.sort_by(|a, b| a.path.cmp(&b.path));
    drives
}

fn macos_fallback_candidates(fallback_nodes: &HashMap<String, String>) -> Vec<DriveCandidate> {
    let mut pairs = fallback_nodes.iter().collect::<Vec<_>>();
    pairs.sort_by(|a, b| a.1.cmp(b.1));
    pairs
        .into_iter()
        .map(|(name, node)| DriveCandidate {
            path: node.clone(),
            label: macos_drive_label(name, node),
            source: "diskutil".to_string(),
            volume_name: None,
            redump_compliant: false,
            generic_mode_required: true,
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_add_volume_names(candidates: &mut [DriveCandidate]) {
    for candidate in candidates {
        let device_path = format!("/dev/{}", normalize_macos_node(&candidate.path));
        candidate.volume_name = macos_diskutil_info_volume_name(&device_path);
    }
}

fn normalize_macos_node(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("/dev/")
        .trim_end_matches(':')
        .to_string()
}

fn macos_drive_label(name: &str, node: &str) -> String {
    let name = name.trim();
    if name.is_empty() || name == node {
        node.to_string()
    } else {
        format!("{node} ({name})")
    }
}

fn parse_macos_diskutil_candidates(text: &str) -> Vec<(String, String)> {
    let mut candidates = Vec::new();
    let mut current_node: Option<String> = None;
    let mut body: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some(node) = macos_diskutil_header_node(line) {
            push_macos_diskutil_candidate(&mut candidates, current_node.as_deref(), &body);
            current_node = Some(node);
            body.clear();
        } else if current_node.is_some() {
            body.push(line.to_string());
        }
    }
    push_macos_diskutil_candidate(&mut candidates, current_node.as_deref(), &body);

    candidates
}

fn macos_diskutil_header_node(line: &str) -> Option<String> {
    let first = line.split_whitespace().next()?;
    if first.starts_with("/dev/disk") {
        Some(normalize_macos_node(first))
    } else {
        None
    }
}

fn parse_macos_diskutil_nodes(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(macos_diskutil_header_node)
        .collect::<Vec<_>>()
}

#[cfg(target_os = "macos")]
fn macos_diskutil_info_optical_name(device_path: &str) -> Option<String> {
    let output = Command::new("diskutil")
        .args(["info", device_path])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_macos_diskutil_info_optical_name(&text)
}

#[cfg(target_os = "macos")]
fn macos_diskutil_info_volume_name(device_path: &str) -> Option<String> {
    let output = Command::new("diskutil")
        .args(["info", device_path])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_macos_diskutil_info_volume_name(&text)
}

fn parse_macos_diskutil_info_optical_name(text: &str) -> Option<String> {
    let mut is_optical = false;
    let mut media_name: Option<String> = None;
    let mut volume_name: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Optical Drive Type:") || trimmed.starts_with("Optical Media Type:")
        {
            is_optical = true;
        }
        if let Some(rest) = trimmed.strip_prefix("Device / Media Name:") {
            media_name = non_empty_macos_diskutil_value(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("Volume Name:") {
            volume_name = non_empty_macos_diskutil_value(rest);
        }
    }

    if is_optical {
        media_name.or(volume_name)
    } else {
        None
    }
}

fn parse_macos_diskutil_info_volume_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Volume Name:")
            .and_then(non_empty_macos_diskutil_value)
    })
}

fn non_empty_macos_diskutil_value(value: &str) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || lower == "(null)"
        || lower.starts_with("not applicable")
        || lower == "none"
    {
        None
    } else {
        Some(value.to_string())
    }
}

fn push_macos_diskutil_candidate(
    candidates: &mut Vec<(String, String)>,
    node: Option<&str>,
    body: &[String],
) {
    let Some(node) = node else {
        return;
    };
    if !body.iter().any(|line| macos_diskutil_line_is_optical(line)) {
        return;
    }
    let name = body
        .iter()
        .find_map(|line| macos_diskutil_line_name(line))
        .unwrap_or_else(|| "Optical disc".to_string());
    candidates.push((name, node.to_string()));
}

fn macos_diskutil_line_is_optical(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "cd_partition",
        "cd_rom",
        "cd_da",
        "iso_9660",
        "udf",
        "dvd",
        "bd_rom",
        "blu-ray",
        "blu_ray",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn macos_diskutil_line_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 6 || !parts[0].ends_with(':') || !macos_diskutil_line_is_optical(trimmed) {
        return None;
    }

    let name_end = parts.len().saturating_sub(3);
    if name_end <= 2 {
        return None;
    }

    let name = parts[2..name_end]
        .join(" ")
        .trim_start_matches('*')
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn command_uses_drive(command: &str) -> bool {
    matches!(
        command,
        "disc"
            | "dump"
            | "dump::extra"
            | "refine"
            | "dvdkey"
            | "eject"
            | "rings"
            | "drive::test"
            | "flash::mt1339"
            | "flash::mt1959"
            | "flash::sd616"
            | "flash::plextor"
    )
}

fn command_writes_files(command: &str) -> bool {
    matches!(
        command,
        "disc"
            | "dump"
            | "dump::extra"
            | "refine"
            | "dvdkey"
            | "dvdisokey"
            | "protection"
            | "split"
            | "hash"
            | "info"
            | "skeleton"
            | "rings"
            | "subchannel"
            | "fixmsf"
            | "debug::flip"
    )
}

fn command_uses_image_path(command: &str) -> bool {
    command_writes_files(command)
}

fn command_uses_output_subfolder(command: &str) -> bool {
    matches!(command, "disc" | "dump" | "dump::extra")
}

fn image_name_required(command: &str) -> bool {
    matches!(
        command,
        "dump::extra"
            | "refine"
            | "dvdisokey"
            | "protection"
            | "split"
            | "hash"
            | "info"
            | "skeleton"
            | "subchannel"
            | "fixmsf"
            | "debug::flip"
    )
}

fn allowed_commands() -> HashSet<&'static str> {
    [
        "disc",
        "dump",
        "dump::extra",
        "refine",
        "dvdkey",
        "eject",
        "dvdisokey",
        "protection",
        "split",
        "hash",
        "info",
        "skeleton",
        "flash::mt1339",
        "flash::mt1959",
        "flash::sd616",
        "flash::plextor",
        "subchannel",
        "debug",
        "debug::flip",
        "fixmsf",
        "rings",
        "drive::test",
    ]
    .into_iter()
    .collect()
}

fn allowed_options() -> HashSet<&'static str> {
    [
        "--help",
        "--version",
        "--verbose",
        "--list-recommended-drives",
        "--list-all-drives",
        "--auto-eject",
        "--skeleton",
        "--debug",
        "--image-path",
        "--image-name",
        "--overwrite",
        "--disc-type",
        "--force-split",
        "--leave-unchanged",
        "--drive",
        "--drive-type",
        "--drive-read-offset",
        "--drive-c2-shift",
        "--drive-pregap-start",
        "--drive-read-method",
        "--drive-sector-order",
        "--speed",
        "--retries",
        "--refine-subchannel",
        "--refine-sector-mode",
        "--continue",
        "--lba-start",
        "--lba-end",
        "--lba-end-by-subcode",
        "--force-qtoc",
        "--legacy-subs",
        "--skip",
        "--skip-fill",
        "--filesystem-trim",
        "--plextor-skip-leadin",
        "--plextor-leadin-retries",
        "--plextor-leadin-force-store",
        "--mediatek-skip-leadout",
        "--mediatek-leadout-retries",
        "--kreon-partial-ss",
        "--dvd-raw",
        "--bd-raw",
        "--disable-cdtext",
        "--correct-offset-shift",
        "--offset-shift-relocate",
        "--force-offset",
        "--audio-silence-threshold",
        "--dump-write-offset",
        "--dump-read-size",
        "--overread-leadout",
        "--force-unscrambled",
        "--force-refine",
        "--firmware",
        "--force-flash",
        "--drive-test-skip-plextor-leadin",
        "--drive-test-skip-cache-read",
        "--skip-subcode-desync",
        "--rings",
        "--cdr-error-threshold",
        "--scsi-timeout",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cd_progress_line() {
        let parsed = parse_line("x [ 42%] LBA:    123/456, errors: { SCSIs: 1, C2s: 2, Q: 3 }");
        assert_eq!(
            parsed.progress.unwrap(),
            ProgressEvent {
                percentage: Some(42),
                lba_current: Some(123),
                lba_total: Some(456),
                scsi_errors: Some(1),
                c2_errors: Some(2),
                q_errors: Some(3),
                edc_errors: None,
            }
        );
    }

    #[test]
    fn parses_dvd_progress_line() {
        let parsed = parse_line("x [  7%] LBA:       9/100, errors: { SCSI: 4, EDC: 5 }");
        assert_eq!(
            parsed.progress.unwrap(),
            ProgressEvent {
                percentage: Some(7),
                lba_current: Some(9),
                lba_total: Some(100),
                scsi_errors: Some(4),
                c2_errors: None,
                q_errors: None,
                edc_errors: Some(5),
            }
        );
    }

    #[test]
    fn parses_stage_line() {
        let parsed = parse_line("*** SPLIT (time check: 1s)");
        assert_eq!(parsed.stage.as_deref(), Some("SPLIT"));
    }

    #[test]
    fn marks_hash_status_as_replaceable_progress_log() {
        let parsed = parse_line("hashing image... 42%");
        assert!(parsed.transient_progress);
        assert!(parsed.progress.is_none());
    }

    #[test]
    fn marks_skeleton_status_as_replaceable_progress_log() {
        let parsed = parse_line("creating skeleton... 9%");
        assert!(parsed.transient_progress);
        assert!(parsed.progress.is_none());
    }

    #[test]
    fn keeps_final_rom_hash_line_as_regular_output() {
        let parsed =
            parse_line(r#"<rom name="dump.iso" size="1" crc="12345678" md5="abc" sha1="def" />"#);
        assert!(!parsed.transient_progress);
        assert!(parsed.progress.is_none());
    }

    #[test]
    fn parses_windows_cdrom_candidates() {
        let drives = parse_windows_cdrom_candidates(
            r#"[{"Name":"HL-DT-ST DVDRAM","Drive":"E:","VolumeName":"GAME_DISC","MediaLoaded":true}]"#,
        );

        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].path, "E:");
        assert_eq!(drives[0].label, "E: (HL-DT-ST DVDRAM) - GAME_DISC");
    }

    #[test]
    fn matches_recommended_drive_signature() {
        let signatures = parse_recommended_drive_signatures(
            "HL-DT-ST - BD-RE BU40N (revision level: 1.00, vendor specific: <empty>)",
        );

        assert!(drive_matches_recommended_list(
            "disk4 (HL-DT-ST BD-RE BU40N)",
            &signatures
        ));
        assert!(!drive_matches_recommended_list(
            "disk4 (Unknown USB Optical Drive)",
            &signatures
        ));
    }

    #[test]
    fn parses_macos_system_profiler_with_fallback_node() {
        let mut fallback = HashMap::new();
        fallback.insert("Apple SuperDrive".to_string(), "disk5".to_string());
        let json = serde_json::json!({
            "SPDiscBurningDataType": [
                {
                    "_name": "Apple SuperDrive"
                }
            ]
        });

        let drives = parse_macos_system_profiler_candidates(&json, &fallback);

        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].path, "disk5");
        assert_eq!(drives[0].label, "disk5 (Apple SuperDrive)");
    }

    #[test]
    fn parses_macos_diskutil_optical_candidates() {
        let candidates = parse_macos_diskutil_candidates(
            r#"/dev/disk4 (external, physical):
   #:                       TYPE NAME                    SIZE       IDENTIFIER
   0:     CD_partition_scheme                        *4.1 GB     disk4
   1:        CD_ROM_Mode_1 DREAMCAST GAME             4.1 GB     disk4s1
/dev/disk5 (internal, physical):
   #:                       TYPE NAME                    SIZE       IDENTIFIER
   0:      GUID_partition_scheme                        *1.0 TB     disk5
"#,
        );

        assert_eq!(
            candidates,
            vec![("DREAMCAST GAME".to_string(), "disk4".to_string())]
        );
    }

    #[test]
    fn parses_macos_whole_disc_node_and_optical_info() {
        let nodes = parse_macos_diskutil_nodes(
            r#"/dev/disk4 (external, physical):
   #:                       TYPE NAME                    SIZE       IDENTIFIER
   0:                            ASING_01_SCN           *4.4 GB     disk4
"#,
        );

        assert_eq!(nodes, vec!["disk4".to_string()]);

        let name = parse_macos_diskutil_info_optical_name(
            r#"   Device Identifier:         disk4
   Device Node:               /dev/disk4
   Device / Media Name:       HL-DT-ST BD-RE BU40N

   Volume Name:               ASING_01_SCN
   Mounted:                   Yes
   Mount Point:               /Volumes/ASING_01_SCN
   File System Personality:   UDF
   Optical Drive Type:        CD-ROM, DVD-ROM, BD-ROM
   Optical Media Type:        DVD-ROM
"#,
        );

        assert_eq!(name.as_deref(), Some("HL-DT-ST BD-RE BU40N"));
        assert_eq!(
            parse_macos_diskutil_info_volume_name(
                r#"   Volume Name:               ASING_01_SCN
   Optical Media Type:        DVD-ROM
"#
            )
            .as_deref(),
            Some("ASING_01_SCN")
        );
        assert_eq!(
            parse_macos_diskutil_info_volume_name(
                r#"   Volume Name:               Not applicable (no file system)
   Optical Media Type:        DVD-ROM
"#
            ),
            None
        );
    }

    #[test]
    fn finds_refine_candidate_for_scram_log_with_c2_errors() {
        let dir = test_temp_dir("scram-c2");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(dir.join("sample.state"), b"state").unwrap();
        fs::write(
            dir.join("sample.log"),
            r#"media errors:
  SCSI: 0 samples
  C2: 903 samples
  Q: 1110
"#,
        )
        .unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap().unwrap();

        assert_eq!(candidate.image_name, "sample");
        assert_eq!(
            candidate.files,
            vec!["sample.log".to_string(), "sample.scram".to_string()]
        );
        assert!(candidate.supports_refine);
        assert!(!candidate.supports_split);
        assert!(!candidate.supports_hash);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_refine_candidate_for_errors_detected_c2_line() {
        let dir = test_temp_dir("scram-errors-detected");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(
            dir.join("sample.log"),
            "errors detected, track: 1, sectors: {SKIP: 0, C2: 8}, samples: {SKIP: 0, C2: 472}",
        )
        .unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap().unwrap();

        assert_eq!(candidate.image_name, "sample");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_refine_candidate_for_scram_log_with_scsi_errors() {
        let dir = test_temp_dir("scram-scsi");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(
            dir.join("sample.log"),
            r#"media errors:
  SCSI: 4 samples
  C2: 0 samples
  Q: 0
"#,
        )
        .unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap().unwrap();

        assert_eq!(candidate.image_name, "sample");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_refine_candidate_for_progress_style_scsi_errors() {
        let dir = test_temp_dir("scram-scsis");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(
            dir.join("sample.log"),
            "x [ 42%] LBA:    123/456, errors: { SCSIs: 1, C2s: 0, Q: 0 }",
        )
        .unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap().unwrap();

        assert_eq!(candidate.image_name, "sample");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_refine_candidate_when_volume_name_matches_drive() {
        let dir = test_temp_dir("scram-volume-match");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("051128_1105_20260618-2036.scram"),
            b"scrambled data",
        )
        .unwrap();
        fs::write(dir.join("051128_1105_20260618-2036.log"), "C2: 903 samples").unwrap();

        let candidate = image_candidate_for_directory(
            &dir,
            ExistingImageMatchContext::from_drive(
                Some("051128_1105".to_string()),
                Some("disk10 (051128_1105)".to_string()),
            ),
        )
        .unwrap()
        .unwrap();

        assert_eq!(candidate.image_name, "051128_1105_20260618-2036");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ignores_refine_candidate_when_volume_name_does_not_match_drive() {
        let dir = test_temp_dir("scram-volume-mismatch");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("other_disc_20260618-2036.scram"),
            b"scrambled data",
        )
        .unwrap();
        fs::write(dir.join("other_disc_20260618-2036.log"), "SCSI: 4 samples").unwrap();

        let candidate = image_candidate_for_directory(
            &dir,
            ExistingImageMatchContext::from_drive(
                Some("051128_1105".to_string()),
                Some("disk10 (051128_1105)".to_string()),
            ),
        )
        .unwrap();

        assert_eq!(candidate, None);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_refine_candidate_for_no_volume_title_drive() {
        let dir = test_temp_dir("scram-no-volume");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("not_applicable_no_file_system_20260618-1455.scram"),
            b"scrambled data",
        )
        .unwrap();
        fs::write(
            dir.join("not_applicable_no_file_system_20260618-1455.log"),
            "SCSI: 4 samples",
        )
        .unwrap();

        let candidate = image_candidate_for_directory(
            &dir,
            ExistingImageMatchContext::from_drive(
                Some("Not Applicable (No File System)".to_string()),
                Some("disk10".to_string()),
            ),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            candidate.image_name,
            "not_applicable_no_file_system_20260618-1455"
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ignores_scram_log_without_read_errors() {
        let dir = test_temp_dir("scram-no-read-errors");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(
            dir.join("sample.log"),
            r#"configuration: MTK8B (read offset: +6, C2 shift: 0)
media errors:
  SCSI: 0 samples
  C2: 0 samples
  Q: 0
"#,
        )
        .unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap();

        assert_eq!(candidate, None);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ignores_scram_log_when_bin_files_exist() {
        let dir = test_temp_dir("scram-with-bin");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.scram"), b"scrambled data").unwrap();
        fs::write(dir.join("sample.bin"), b"bin data").unwrap();
        fs::write(dir.join("sample.log"), "C2: 903 samples").unwrap();

        let candidate = image_candidate_for_directory(&dir, None).unwrap();

        assert_eq!(candidate, None);

        fs::remove_dir_all(dir).unwrap();
    }

    fn test_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "redumper-ui-test-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn rejects_unknown_command() {
        let req = RunRequest {
            command: "shell".to_string(),
            options: Vec::new(),
            drive_mode: DriveMode::Auto,
            drive: None,
            image_path: None,
            image_name: None,
            working_directory: None,
            manual_command: None,
            output_subfolder: true,
            archive_tool_path: None,
            compress_log_files: true,
            archive_format: ArchiveFormat::SevenZip,
            dump_twice_compare_hashes: false,
            danger_confirmed: false,
        };
        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn accepts_manual_redumper_command_as_argv() {
        let req = RunRequest {
            command: "disc".to_string(),
            options: Vec::new(),
            drive_mode: DriveMode::Manual,
            drive: Some("disk4".to_string()),
            image_path: Some("/tmp/out".to_string()),
            image_name: Some("movie".to_string()),
            working_directory: None,
            manual_command: Some(
                r#"redumper dump "--drive=disk4" "--image-path=/tmp/out" --drive-type=GENERIC"#
                    .to_string(),
            ),
            output_subfolder: true,
            archive_tool_path: None,
            compress_log_files: true,
            archive_format: ArchiveFormat::SevenZip,
            dump_twice_compare_hashes: false,
            danger_confirmed: false,
        };

        assert!(validate_request(&req).is_ok());
        assert_eq!(
            build_args(&req, Path::new("/ignored")).unwrap(),
            vec![
                "dump".to_string(),
                "--drive=disk4".to_string(),
                "--image-path=/tmp/out".to_string(),
                "--drive-type=GENERIC".to_string(),
            ]
        );
    }

    #[test]
    fn sanitizes_output_folder_names() {
        assert_eq!(safe_output_folder_name(" ASING_01_SCN "), "ASING_01_SCN");
        assert_eq!(safe_output_folder_name("bad/name:disc"), "bad_name_disc");
        assert_eq!(safe_output_folder_name("..."), "");
        assert!(command_uses_output_subfolder("disc"));
        assert!(command_uses_output_subfolder("dump"));
        assert!(!command_uses_output_subfolder("refine"));

        let mut req = RunRequest {
            command: "disc".to_string(),
            options: Vec::new(),
            drive_mode: DriveMode::Auto,
            drive: None,
            image_path: Some("/tmp/out".to_string()),
            image_name: Some("my_disc".to_string()),
            working_directory: None,
            manual_command: None,
            output_subfolder: true,
            archive_tool_path: None,
            compress_log_files: true,
            archive_format: ArchiveFormat::SevenZip,
            dump_twice_compare_hashes: false,
            danger_confirmed: false,
        };
        assert_eq!(
            resolve_output_directory_from_base(PathBuf::from("/tmp/out"), &req, "disc"),
            PathBuf::from("/tmp/out/my_disc")
        );

        req.output_subfolder = false;
        assert_eq!(
            resolve_output_directory_from_base(PathBuf::from("/tmp/out"), &req, "disc"),
            PathBuf::from("/tmp/out")
        );
    }

    #[test]
    fn detects_existing_output_in_subfolder_mode() {
        let dir = test_temp_dir("output-conflict-subfolder");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("partial.iso"), b"partial dump").unwrap();

        let matches = existing_output_matches(&dir, Some("movie"), true).unwrap();

        assert_eq!(matches, vec!["partial.iso".to_string()]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn detects_named_existing_output_without_subfolder_mode() {
        let dir = test_temp_dir("output-conflict-root");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("movie.iso"), b"dump").unwrap();
        fs::write(dir.join("movie.log"), b"log").unwrap();
        fs::write(dir.join("other.iso"), b"other").unwrap();

        let matches = existing_output_matches(&dir, Some("movie"), false).unwrap();

        assert_eq!(
            matches,
            vec!["movie.iso".to_string(), "movie.log".to_string()]
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_manual_shell_tokens() {
        let req = RunRequest {
            command: "disc".to_string(),
            options: Vec::new(),
            drive_mode: DriveMode::Auto,
            drive: None,
            image_path: None,
            image_name: None,
            working_directory: None,
            manual_command: Some("redumper dump && rm -rf /".to_string()),
            output_subfolder: true,
            archive_tool_path: None,
            compress_log_files: true,
            archive_format: ArchiveFormat::SevenZip,
            dump_twice_compare_hashes: false,
            danger_confirmed: false,
        };

        assert!(validate_request(&req)
            .unwrap_err()
            .contains("Manual command arguments must be redumper options"));
    }

    #[test]
    fn compares_matching_dump_hashes() {
        let dir = test_temp_dir("matching-hashes");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("movie.iso"), b"same data").unwrap();
        fs::write(dir.join("movie_verify.iso"), b"same data").unwrap();

        let comparison = compare_dump_hashes(&dir, "movie", "movie_verify").unwrap();

        assert!(comparison.message.contains("Dump hashes match"));
        assert_eq!(
            comparison.duplicate_iso_path,
            Some(dir.join("movie_verify.iso"))
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hashes_crc32_with_eight_hex_digits() {
        let dir = test_temp_dir("crc32-hash");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dump.iso");
        fs::write(&file, b"123456789").unwrap();

        assert_eq!(crc32_file(&file).unwrap(), "cbf43926");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_redump_info_zero_disc_count() {
        let html = r#"<p class="disc-count-line"><small>0 discs found.</small></p>"#;

        assert_eq!(parse_redump_info_disc_count(html), Some(0));
    }

    #[test]
    fn parses_redump_info_positive_disc_count() {
        let html = r#"<p class="disc-count-line"><small>2 discs found.</small></p>"#;

        assert_eq!(parse_redump_info_disc_count(html), Some(2));
    }

    #[test]
    fn falls_back_to_disc_links_for_redump_info_count() {
        let html = r#"<tr class="clickable-row" data-href="/disc/40767">"#;

        assert_eq!(parse_redump_info_disc_count(html), Some(1));
    }

    #[test]
    fn rejects_mismatched_dump_hashes() {
        let dir = test_temp_dir("mismatched-hashes");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("movie.iso"), b"first").unwrap();
        fs::write(dir.join("movie_verify.iso"), b"second").unwrap();

        let error = compare_dump_hashes(&dir, "movie", "movie_verify").unwrap_err();

        assert!(error.contains("Dump hash mismatch"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deletes_duplicate_verify_iso() {
        let dir = test_temp_dir("delete-duplicate-iso");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let duplicate = dir.join("movie_verify.iso");
        fs::write(&duplicate, b"duplicate").unwrap();

        let message = delete_duplicate_iso(duplicate.to_string_lossy().to_string()).unwrap();

        assert!(message.contains("Deleted duplicate ISO"));
        assert!(!duplicate.exists());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zip_archive_fallback_keeps_logs_and_deletes_auxiliary_files() {
        let dir = test_temp_dir("zip-fallback");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("movie.log"), b"redumper log").unwrap();
        fs::write(dir.join("movie.txt"), b"submission info").unwrap();
        fs::write(dir.join("movie.iso"), b"disc image").unwrap();

        let request = ArchiveRequest {
            output_directory: dir.clone(),
            image_name: Some("movie".to_string()),
            archive_tool_path: None,
            archive_format: ArchiveFormat::Zip,
        };

        let (archive_name, deleted, kept_logs) = compress_candidates_with_zip(
            &request,
            "movie",
            &["movie.log".to_string(), "movie.txt".to_string()],
        )
        .unwrap();

        assert!(archive_name.ends_with(".zip"));
        assert_eq!(deleted, 1);
        assert_eq!(kept_logs, 1);
        assert!(dir.join(&archive_name).is_file());
        assert!(dir.join("movie.log").is_file());
        assert!(!dir.join("movie.txt").exists());
        assert!(dir.join("movie.iso").is_file());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn seven_zip_candidates_include_windows_official_install_paths() {
        let candidates = seven_zip_candidates(None);

        assert!(candidates
            .iter()
            .any(|path| path == Path::new(r"C:\Program Files\7-Zip\7z.exe")));
        assert!(candidates
            .iter()
            .any(|path| path == Path::new(r"C:\Program Files (x86)\7-Zip\7z.exe")));
    }

    #[test]
    fn rejects_non_duplicate_iso_delete() {
        let dir = test_temp_dir("reject-delete");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("movie.iso");
        let non_iso = dir.join("movie_verify.bin");
        fs::write(&original, b"original").unwrap();
        fs::write(&non_iso, b"duplicate").unwrap();

        assert!(delete_duplicate_iso(original.to_string_lossy().to_string()).is_err());
        assert!(delete_duplicate_iso(non_iso.to_string_lossy().to_string()).is_err());
        assert!(original.exists());
        assert!(non_iso.exists());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_double_dump_on_non_dump_commands() {
        let req = RunRequest {
            command: "hash".to_string(),
            options: Vec::new(),
            drive_mode: DriveMode::Auto,
            drive: None,
            image_path: Some("/tmp/out".to_string()),
            image_name: Some("movie".to_string()),
            working_directory: None,
            manual_command: None,
            output_subfolder: true,
            archive_tool_path: None,
            compress_log_files: true,
            archive_format: ArchiveFormat::SevenZip,
            dump_twice_compare_hashes: true,
            danger_confirmed: false,
        };

        assert!(validate_request(&req)
            .unwrap_err()
            .contains("Dump Twice if No Match"));
    }
}
