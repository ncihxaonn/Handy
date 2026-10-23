//! Menubar quick panel: a small popover shown when the tray icon is
//! left-clicked, plus the long-form recording and file-import commands it
//! drives. Results are saved to history and copied to the clipboard instead of
//! being pasted, since the panel itself holds focus while it is open.

use crate::actions::process_transcription_output;
use crate::audio_toolkit::VadPolicy;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::get_settings;
use crate::tray::{change_tray_icon, TrayIconState};
use log::{error, info, warn};
use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;

pub const PANEL_LABEL: &str = "menubar_panel";
const PANEL_WIDTH: f64 = 320.0;
const PANEL_HEIGHT: f64 = 404.0;
const MEETING_BINDING_ID: &str = "menubar_meeting";
const STATUS_EVENT: &str = "panel-status-changed";

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PanelPhase {
    #[default]
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
pub struct PanelStatus {
    pub phase: PanelPhase,
    /// Unix ms when the current recording started.
    pub started_at_ms: Option<i64>,
    /// File name being transcribed, when the job is a file import.
    pub file_name: Option<String>,
    /// Error from the most recent job, cleared when a new one starts.
    pub error: Option<String>,
}

/// Tauri managed state for the panel's single in-flight job.
pub struct PanelState(Mutex<PanelStatus>);

impl PanelState {
    pub fn new() -> Self {
        Self(Mutex::new(PanelStatus::default()))
    }
}

fn set_status(app: &AppHandle, status: PanelStatus) {
    *app.state::<PanelState>().0.lock().unwrap() = status.clone();
    let _ = app.emit(STATUS_EVENT, status);
}

fn fail(app: &AppHandle, message: String) {
    error!("Menubar panel job failed: {}", message);
    change_tray_icon(app, TrayIconState::Idle);
    set_status(
        app,
        PanelStatus {
            error: Some(message),
            ..Default::default()
        },
    );
}

/* ---------- window --------------------------------------------------------- */

pub fn create_panel(app: &AppHandle) {
    let result = WebviewWindowBuilder::new(
        app,
        PANEL_LABEL,
        WebviewUrl::App("src/panel/index.html".into()),
    )
    .title("Handy")
    .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build();

    match result {
        Ok(window) => {
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    let _ = handle.hide();
                }
            });
        }
        Err(e) => error!("Failed to create menubar panel: {}", e),
    }
}

/// Toggle the panel, anchoring it under the tray icon that was clicked.
pub fn toggle_panel(app: &AppHandle, tray_rect: Rect) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    let scale = window.scale_factor().unwrap_or(1.0);
    let icon_pos = tray_rect.position.to_physical::<f64>(scale);
    let icon_size = tray_rect.size.to_physical::<f64>(scale);
    let width = PANEL_WIDTH * scale;
    let gap = 6.0 * scale;

    let mut x = icon_pos.x + icon_size.width / 2.0 - width / 2.0;
    // Keep the panel on the monitor that holds the tray icon.
    if let Ok(Some(monitor)) = window.monitor_from_point(icon_pos.x, icon_pos.y) {
        let left = monitor.position().x as f64 + gap;
        let right = left + monitor.size().width as f64 - width - 2.0 * gap;
        x = x.clamp(left, right.max(left));
    }
    let y = icon_pos.y + icon_size.height + gap;

    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("panel-shown", ());
}

fn hide_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.hide();
    }
}

/* ---------- shared output -------------------------------------------------- */

/// Saves the samples + transcription to history and copies the text.
async fn finish_transcription(app: &AppHandle, samples: Vec<f32>) {
    let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
    let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

    let file_name = format!("handy-{}.wav", chrono::Utc::now().timestamp());
    let wav_path = hm.recordings_dir().join(&file_name);
    let samples_for_wav = samples.clone();
    let wav_saved = tauri::async_runtime::spawn_blocking(move || {
        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
    });

    let result = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples)).await;
    let wav_saved = matches!(wav_saved.await, Ok(Ok(())));

    let transcription = match result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return fail(app, e.to_string()),
        Err(e) => return fail(app, e.to_string()),
    };

    let processed = process_transcription_output(app, &transcription, false).await;

    if wav_saved {
        if let Err(e) = hm.save_entry(file_name, transcription, false, None, None) {
            error!("Failed to save panel history entry: {}", e);
        }
    }

    if processed.final_text.trim().is_empty() {
        return fail(app, "No speech was detected.".to_string());
    }
    if let Err(e) = app.clipboard().write_text(&processed.final_text) {
        warn!("Failed to copy panel transcript: {}", e);
    }

    change_tray_icon(app, TrayIconState::Idle);
    set_status(app, PanelStatus::default());
    let _ = app.emit("panel-transcript-ready", processed.final_text);
}

/* ---------- commands ------------------------------------------------------- */

#[tauri::command]
#[specta::specta]
pub fn get_panel_status(app: AppHandle) -> PanelStatus {
    let mut status = app.state::<PanelState>().0.lock().unwrap().clone();
    // A global cancel (Esc / tray) can stop the recorder behind our back.
    if matches!(status.phase, PanelPhase::Recording)
        && !app.state::<Arc<AudioRecordingManager>>().is_recording()
    {
        status = PanelStatus::default();
        set_status(&app, status.clone());
    }
    status
}

#[tauri::command]
#[specta::specta]
pub fn start_panel_recording(app: AppHandle) -> Result<(), String> {
    if !matches!(get_panel_status(app.clone()).phase, PanelPhase::Idle) {
        return Err("Handy is already busy.".to_string());
    }
    let rm = app.state::<Arc<AudioRecordingManager>>();
    if rm.is_recording() {
        return Err("A dictation is already recording.".to_string());
    }

    app.state::<Arc<TranscriptionManager>>()
        .initiate_model_load();
    rm.try_start_recording(MEETING_BINDING_ID, VadPolicy::Disabled)?;

    change_tray_icon(&app, TrayIconState::Recording);
    set_status(
        &app,
        PanelStatus {
            phase: PanelPhase::Recording,
            started_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            ..Default::default()
        },
    );
    info!("Menubar panel recording started");
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn stop_panel_recording(app: AppHandle) -> Result<(), String> {
    if !matches!(get_panel_status(app.clone()).phase, PanelPhase::Recording) {
        return Err("No recording is in progress.".to_string());
    }

    change_tray_icon(&app, TrayIconState::Transcribing);
    set_status(
        &app,
        PanelStatus {
            phase: PanelPhase::Transcribing,
            ..Default::default()
        },
    );

    tauri::async_runtime::spawn(async move {
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let generation = rm.cancel_generation();
        let samples = tauri::async_runtime::spawn_blocking(move || {
            rm.stop_recording(MEETING_BINDING_ID, generation)
        })
        .await
        .ok()
        .flatten();

        match samples {
            Some(samples) if !samples.is_empty() => finish_transcription(&app, samples).await,
            _ => fail(&app, "The recording contains no audio.".to_string()),
        }
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn cancel_panel_recording(app: AppHandle) {
    crate::utils::cancel_current_operation(&app);
    set_status(&app, PanelStatus::default());
}

#[tauri::command]
#[specta::specta]
pub fn transcribe_panel_file(app: AppHandle, path: String) -> Result<(), String> {
    if !matches!(get_panel_status(app.clone()).phase, PanelPhase::Idle) {
        return Err("Handy is already busy.".to_string());
    }
    let source = PathBuf::from(&path);
    if !source.is_file() {
        return Err(format!("File not found: {}", path));
    }

    app.state::<Arc<TranscriptionManager>>()
        .initiate_model_load();
    change_tray_icon(&app, TrayIconState::Transcribing);
    set_status(
        &app,
        PanelStatus {
            phase: PanelPhase::Transcribing,
            file_name: source.file_name().map(|n| n.to_string_lossy().into_owned()),
            ..Default::default()
        },
    );

    tauri::async_runtime::spawn(async move {
        let decoded = tauri::async_runtime::spawn_blocking(move || decode_to_samples(&source))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match decoded {
            Ok(samples) if !samples.is_empty() => finish_transcription(&app, samples).await,
            Ok(_) => fail(&app, "The file contains no audio.".to_string()),
            Err(e) => fail(&app, e),
        }
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_last_transcript(app: AppHandle) -> Option<String> {
    let entry = app
        .state::<Arc<HistoryManager>>()
        .get_latest_completed_entry()
        .ok()
        .flatten()?;
    Some(entry.post_processed_text.unwrap_or(entry.transcription_text))
}

#[tauri::command]
#[specta::specta]
pub fn copy_last_transcript_command(app: AppHandle) -> bool {
    crate::tray::copy_last_transcript(&app)
}

#[tauri::command]
#[specta::specta]
pub fn open_main_window_from_panel(app: AppHandle) {
    hide_panel(&app);
    crate::show_main_window(&app);
}

#[tauri::command]
#[specta::specta]
pub fn get_selected_model_name(app: AppHandle) -> Option<String> {
    let selected = get_settings(&app).selected_model;
    app.state::<Arc<crate::managers::model::ModelManager>>()
        .get_available_models()
        .into_iter()
        .find(|m| m.id == selected)
        .map(|m| m.name)
}

/* ---------- decoding ------------------------------------------------------- */

/// Converts any audio/video file to 16 kHz mono samples. Prefers ffmpeg (handles
/// video containers), falling back to macOS `afconvert` for common audio types.
fn decode_to_samples(source: &Path) -> Result<Vec<f32>, String> {
    let tmp = std::env::temp_dir().join(format!(
        "handy-import-{}.wav",
        chrono::Utc::now().timestamp_millis()
    ));

    let ffmpeg = ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "ffmpeg"]
        .into_iter()
        .find(|p| !p.starts_with('/') || Path::new(p).exists());

    let mut converted = false;
    if let Some(ffmpeg) = ffmpeg {
        converted = Command::new(ffmpeg)
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(source)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(&tmp)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    if !converted && cfg!(target_os = "macos") {
        converted = Command::new("/usr/bin/afconvert")
            .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(source)
            .arg(&tmp)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    if !converted {
        let _ = std::fs::remove_file(&tmp);
        return Err("Could not read this file. Install ffmpeg for video and other formats.".into());
    }

    let samples = crate::audio_toolkit::read_wav_samples(&tmp).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&tmp);
    samples
}
