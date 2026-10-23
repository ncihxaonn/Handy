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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, Rect, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    objc2_app_kit::NSScreen, tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask,
    WebviewWindowExt,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(MenubarQuickPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

pub const PANEL_LABEL: &str = "menubar_panel";
const PANEL_WIDTH: f64 = 320.0;
const PANEL_HEIGHT: f64 = 404.0;
const MEETING_BINDING_ID: &str = "menubar_meeting";
const STATUS_EVENT: &str = "panel-status-changed";
const PANEL_BLUR_GUARD: Duration = Duration::from_millis(400);

struct PanelWindowState {
    shown_at: Mutex<Option<Instant>>,
    pinned: AtomicBool,
}

impl Default for PanelWindowState {
    fn default() -> Self {
        Self {
            shown_at: Mutex::new(None),
            pinned: AtomicBool::new(true),
        }
    }
}

impl PanelWindowState {
    fn shown_at(&self) -> Option<Instant> {
        *self
            .shown_at
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn set_shown_at(&self, shown_at: Option<Instant>) {
        *self
            .shown_at
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = shown_at;
    }
}

#[derive(Clone, Copy, Debug)]
struct PanelRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
fn macos_tray_icon_rect(tray_rect: Rect, scale: f64, screen_top: f64) -> PanelRect {
    let position = tray_rect.position.to_logical::<f64>(scale);
    let size = tray_rect.size.to_logical::<f64>(scale);
    PanelRect {
        x: position.x,
        y: screen_top,
        width: size.width,
        height: size.height,
    }
}

fn calculate_panel_position(icon: PanelRect, work_area: PanelRect) -> (f64, f64) {
    let gap = 6.0;
    let left = work_area.x + gap;
    let right = work_area.x + work_area.width - PANEL_WIDTH - gap;
    let x = (icon.x + icon.width / 2.0 - PANEL_WIDTH / 2.0).clamp(left, right.max(left));
    let top = work_area.y + gap;
    let bottom = work_area.y + work_area.height - PANEL_HEIGHT - gap;
    let y = (icon.y + icon.height + gap).clamp(top, bottom.max(top));
    (x, y)
}

fn should_dismiss_on_blur(
    visible: bool,
    focused: bool,
    pinned: bool,
    _cursor_over: bool,
    _dragging: bool,
    shown_elapsed: Option<Duration>,
) -> bool {
    visible
        && !focused
        && !pinned
        && shown_elapsed.is_some_and(|elapsed| elapsed >= PANEL_BLUR_GUARD)
}

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
    app.manage(PanelWindowState::default());
    let result = WebviewWindowBuilder::new(
        app,
        PANEL_LABEL,
        WebviewUrl::App("src/panel/index.html".into()),
    )
    .title("Handy")
    .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .accept_first_mouse(true)
    .focused(false)
    .visible(false)
    .on_page_load(|_, payload| {
        info!(
            "Menubar panel page load: event={:?} url={}",
            payload.event(),
            payload.url()
        );
    })
    .build();

    match result {
        Ok(window) => {
            #[cfg(target_os = "macos")]
            {
                let Some(mtm) = MainThreadMarker::new() else {
                    error!("Menubar panel creation requires the main thread");
                    return;
                };
                let panel = match window.to_panel::<MenubarQuickPanel>() {
                    Ok(panel) => panel,
                    Err(e) => {
                        error!("Failed to convert menubar panel to NSPanel: {}", e);
                        return;
                    }
                };
                panel.set_level(PanelLevel::Status.value());
                panel.set_style_mask(
                    StyleMask::empty()
                        .borderless()
                        .nonactivating_panel()
                        .value(),
                );
                panel.set_hides_on_deactivate(false);
                panel.set_becomes_key_only_if_needed(false);
                panel.set_collection_behavior(
                    CollectionBehavior::new()
                        .can_join_all_spaces()
                        .full_screen_auxiliary()
                        .value(),
                );
                panel.hide();
                info!(
                    "Menubar panel native configuration: can_become_key={} can_become_main={} activation_policy={:?}",
                    panel.can_become_key_window(),
                    panel.can_become_main_window(),
                    tauri_nspanel::objc2_app_kit::NSApplication::sharedApplication(mtm).activationPolicy()
                );
            }

            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(focused) = event {
                    info!("Menubar panel focus changed: focused={}", focused);
                    if !focused {
                        handle_panel_blur(&handle);
                    }
                } else if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                    event
                {
                    info!(
                        "Menubar panel file drop: files={} visible={:?} focused={:?} pinned={}",
                        paths.len(),
                        handle.is_visible(),
                        handle.is_focused(),
                        handle
                            .app_handle()
                            .state::<PanelWindowState>()
                            .pinned
                            .load(Ordering::Relaxed)
                    );
                }
            });
            info!(
                "Menubar panel created: label={} native={} url=src/panel/index.html visible=false blur_guard_ms={}",
                PANEL_LABEL,
                if cfg!(target_os = "macos") { "NSPanel" } else { "window" },
                PANEL_BLUR_GUARD.as_millis()
            );
        }
        Err(e) => error!("Failed to create menubar panel: {}", e),
    }
}

fn handle_panel_blur(window: &WebviewWindow) {
    let state = window.app_handle().state::<PanelWindowState>();
    let shown_at = state.shown_at();
    let pinned = state.pinned.load(Ordering::Relaxed);
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    let elapsed = shown_at.map(|time| time.elapsed());
    info!(
        "Menubar panel blur: visible={} focused={} pinned={} shown_ms={:?}",
        visible,
        focused,
        pinned,
        elapsed.map(|time| time.as_millis())
    );
    if pinned {
        info!("Menubar panel blur: ignored while pinned");
    } else if should_dismiss_on_blur(visible, focused, pinned, false, false, elapsed) {
        info!("Menubar panel blur: hiding");
        hide_panel(window.app_handle());
    } else if visible && !focused && elapsed.is_some_and(|time| time < PANEL_BLUR_GUARD) {
        info!("Menubar panel blur: deferred during show guard");
        let window = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(PANEL_BLUR_GUARD.saturating_sub(elapsed.unwrap_or_default()));
            let handle = window.clone();
            if let Err(e) = window.run_on_main_thread(move || {
                let current = handle.app_handle().state::<PanelWindowState>().shown_at();
                if current == shown_at {
                    handle_panel_blur(&handle);
                }
            }) {
                warn!("Failed to recheck menubar panel blur: {}", e);
            }
        });
    } else {
        info!("Menubar panel blur: ignored");
    }
}

#[cfg(target_os = "macos")]
fn panel_position(_window: &WebviewWindow, tray_rect: Rect) -> Result<(f64, f64), String> {
    let mtm = MainThreadMarker::new().ok_or("Panel positioning requires the main thread")?;
    let screens = NSScreen::screens(mtm);
    let primary = screens.iter().next().ok_or("No macOS screens available")?;
    let primary_frame = primary.frame();
    let primary_top = primary_frame.origin.y + primary_frame.size.height;
    let cursor = NSEvent::mouseLocation();
    let screen = screens
        .iter()
        .find(|screen| {
            let frame = screen.frame();
            cursor.x >= frame.origin.x
                && cursor.x < frame.origin.x + frame.size.width
                && cursor.y >= frame.origin.y
                && cursor.y < frame.origin.y + frame.size.height
        })
        .unwrap_or(primary);
    let frame = screen.frame();
    let visible_frame = screen.visibleFrame();
    let scale = screen.backingScaleFactor();
    let icon = macos_tray_icon_rect(
        tray_rect,
        scale,
        primary_top - frame.origin.y - frame.size.height,
    );
    let work_area = PanelRect {
        x: visible_frame.origin.x,
        y: primary_top - visible_frame.origin.y - visible_frame.size.height,
        width: visible_frame.size.width,
        height: visible_frame.size.height,
    };
    let position = calculate_panel_position(icon, work_area);
    info!(
        "Menubar panel geometry: tray_rect={:?} screen={:?} work_area={:?} scale={} normalized_icon={:?} logical_position={:?}",
        tray_rect, frame, work_area, scale, icon, position
    );
    Ok(position)
}

#[cfg(not(target_os = "macos"))]
fn panel_position(window: &WebviewWindow, tray_rect: Rect) -> Result<(f64, f64), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let icon_position = tray_rect.position.to_physical::<f64>(scale);
    let monitor = window
        .monitor_from_point(icon_position.x, icon_position.y)
        .map_err(|e| e.to_string())?
        .or(window.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("No monitor available for menubar panel")?;
    let scale = monitor.scale_factor();
    let icon_position = tray_rect.position.to_logical::<f64>(scale);
    let icon_size = tray_rect.size.to_logical::<f64>(scale);
    let bounds = monitor.work_area();
    let origin = bounds.position.to_logical::<f64>(scale);
    let size = bounds.size.to_logical::<f64>(scale);
    let icon = PanelRect {
        x: icon_position.x,
        y: icon_position.y,
        width: icon_size.width,
        height: icon_size.height,
    };
    let work_area = PanelRect {
        x: origin.x,
        y: origin.y,
        width: size.width,
        height: size.height,
    };
    let position = calculate_panel_position(icon, work_area);
    info!(
        "Menubar panel geometry: tray_rect={:?} work_area={:?} scale={} logical_position={:?}",
        tray_rect, work_area, scale, position
    );
    Ok(position)
}

pub fn toggle_panel(app: &AppHandle, tray_rect: Rect) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || toggle_panel_on_main_thread(&handle, tray_rect))
    {
        error!("Failed to dispatch menubar panel toggle: {}", e);
    }
}

fn toggle_panel_on_main_thread(app: &AppHandle, tray_rect: Rect) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        error!(
            "Menubar panel toggle: window missing; tray_rect={:?}",
            tray_rect
        );
        return;
    };
    let visible = match window.is_visible() {
        Ok(visible) => visible,
        Err(e) => {
            error!("Menubar panel toggle: failed to read visibility: {}", e);
            return;
        }
    };
    info!(
        "Menubar panel toggle: visible={} tray_rect={:?}",
        visible, tray_rect
    );
    if visible {
        hide_panel(app);
        return;
    }

    let (x, y) = match panel_position(&window, tray_rect) {
        Ok(position) => position,
        Err(e) => {
            error!("Menubar panel toggle: failed to calculate position: {}", e);
            return;
        }
    };
    if let Err(e) = window.set_position(LogicalPosition::new(x, y)) {
        error!("Menubar panel toggle: failed to set position: {}", e);
        return;
    }
    app.state::<PanelWindowState>()
        .set_shown_at(Some(Instant::now()));

    #[cfg(target_os = "macos")]
    match app.get_webview_panel(PANEL_LABEL) {
        Ok(panel) => panel.show_and_make_key(),
        Err(e) => {
            error!("Menubar panel toggle: native panel missing: {:?}", e);
            return;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Err(e) = window.show().and_then(|()| window.set_focus()) {
            error!("Menubar panel toggle: failed to show/focus: {}", e);
            return;
        }
    }

    info!(
        "Menubar panel shown: logical_position=({}, {}) visible={:?} focused={:?} actual_position={:?}",
        x, y, window.is_visible(), window.is_focused(), window.outer_position()
    );
    if let Err(e) = window.emit("panel-shown", ()) {
        warn!("Failed to notify menubar panel frontend: {}", e);
    }
}

fn hide_panel(app: &AppHandle) {
    app.state::<PanelWindowState>().set_shown_at(None);
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        match window.hide() {
            Ok(()) => info!("Menubar panel hidden"),
            Err(e) => error!("Failed to hide menubar panel: {}", e),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_panel_pinned(app: AppHandle) -> bool {
    app.state::<PanelWindowState>()
        .pinned
        .load(Ordering::Relaxed)
}

#[tauri::command]
#[specta::specta]
pub fn set_panel_pinned(app: AppHandle, pinned: bool) -> bool {
    app.state::<PanelWindowState>()
        .pinned
        .store(pinned, Ordering::Relaxed);
    info!("Menubar panel pin changed: pinned={}", pinned);
    pinned
}

#[tauri::command]
#[specta::specta]
pub fn hide_menubar_panel(app: AppHandle) {
    info!("Menubar panel dismiss requested");
    hide_panel(&app);
}

#[tauri::command]
#[specta::specta]
pub fn panel_frontend_ready(window: WebviewWindow) {
    if window.label() == PANEL_LABEL {
        info!("Menubar panel frontend ready: label={}", PANEL_LABEL);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn menubar_blur_during_show_is_not_dismissed() {
        assert!(!should_dismiss_on_blur(
            true,
            false,
            false,
            Some(Duration::from_millis(10))
        ));
    }

    #[test]
    fn menubar_pinned_panel_stays_visible_when_finder_takes_focus() {
        assert!(!should_dismiss_on_blur(
            true,
            false,
            true,
            Some(PANEL_BLUR_GUARD * 2),
        ));
    }

    #[test]
    fn menubar_panel_starts_pinned_and_does_not_dismiss_on_blur() {
        let state = PanelWindowState::default();
        assert!(state.pinned.load(Ordering::Relaxed));
        assert!(!should_dismiss_on_blur(
            true,
            false,
            state.pinned.load(Ordering::Relaxed),
            Some(PANEL_BLUR_GUARD * 2),
        ));
    }

    #[test]
    fn menubar_pin_survives_show_and_hide_during_the_session() {
        let state = PanelWindowState::default();
        assert!(state.pinned.load(Ordering::Relaxed));
        state.set_shown_at(Some(Instant::now()));
        state.set_shown_at(None);
        state.set_shown_at(Some(Instant::now()));
        assert!(state.pinned.load(Ordering::Relaxed));
        assert!(!should_dismiss_on_blur(
            true,
            false,
            state.pinned.load(Ordering::Relaxed),
            Some(PANEL_BLUR_GUARD),
        ));
        state.pinned.store(false, Ordering::Relaxed);
        assert!(should_dismiss_on_blur(
            true,
            false,
            state.pinned.load(Ordering::Relaxed),
            Some(PANEL_BLUR_GUARD),
        ));
    }

    #[test]
    fn menubar_position_stays_inside_a_short_monitor() {
        let icon = PanelRect {
            x: 500.0,
            y: 780.0,
            width: 24.0,
            height: 24.0,
        };
        let work_area = PanelRect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 600.0,
        };
        let (_, y) = calculate_panel_position(icon, work_area);
        assert!(y + PANEL_HEIGHT <= work_area.height - 6.0);
    }

    #[test]
    fn menubar_blur_after_guard_dismisses_only_an_unfocused_visible_panel() {
        let elapsed = Some(PANEL_BLUR_GUARD);
        assert!(should_dismiss_on_blur(true, false, false, elapsed));
        assert!(!should_dismiss_on_blur(true, true, false, elapsed));
        assert!(!should_dismiss_on_blur(false, false, false, elapsed));
        assert!(!should_dismiss_on_blur(true, false, false, None));
    }

    #[test]
    fn menubar_position_clamps_both_horizontal_edges_on_a_negative_origin_monitor() {
        let work_area = PanelRect {
            x: -1920.0,
            y: -1080.0,
            width: 1920.0,
            height: 1049.0,
        };
        for icon_x in [-1920.0, -24.0] {
            let icon = PanelRect {
                x: icon_x,
                y: -1080.0,
                width: 24.0,
                height: 31.0,
            };
            let (x, y) = calculate_panel_position(icon, work_area);
            assert!(x >= work_area.x + 6.0);
            assert!(x + PANEL_WIDTH <= work_area.x + work_area.width - 6.0);
            assert!(y >= work_area.y + 6.0);
            assert!(y + PANEL_HEIGHT <= work_area.y + work_area.height - 6.0);
        }
    }

    #[test]
    fn menubar_position_handles_a_work_area_smaller_than_the_panel() {
        let work_area = PanelRect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
        let icon = PanelRect {
            x: 180.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        };
        assert_eq!(calculate_panel_position(icon, work_area), (6.0, 6.0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn menubar_retina_tray_y_uses_native_screen_top_instead_of_upstream_rect_y() {
        let rect = Rect {
            position: tauri::PhysicalPosition::new(1800.0, 2160.0).into(),
            size: tauri::PhysicalSize::new(48.0, 62.0).into(),
        };
        let icon = macos_tray_icon_rect(rect, 2.0, 0.0);
        assert_eq!(icon.x, 900.0);
        assert_eq!(icon.y, 0.0);
        assert_eq!(icon.width, 24.0);
        assert_eq!(icon.height, 31.0);
        let work_area = PanelRect {
            x: 0.0,
            y: 31.0,
            width: 1920.0,
            height: 1049.0,
        };
        let (_, y) = calculate_panel_position(icon, work_area);
        assert!(y + PANEL_HEIGHT < work_area.height);
    }

    #[test]
    fn menubar_build_has_no_updater_capability() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(!permissions.iter().any(|permission| {
            permission
                .as_str()
                .is_some_and(|name| name.starts_with("updater:"))
        }));
    }

    #[test]
    fn menubar_build_has_no_official_update_endpoint() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let endpoints = config["plugins"]["updater"]["endpoints"]
            .as_array()
            .unwrap();
        assert!(endpoints.is_empty());
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
    Some(
        entry
            .post_processed_text
            .unwrap_or(entry.transcription_text),
    )
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

    let ffmpeg = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "ffmpeg",
    ]
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
