// Island window on Linux: there isn't one.
//
// On KDE Plasma the island lives inside a Plasma widget (see `plasma/` at the
// repository root), which owns its own placement, sizing, focus and mouse
// handling. These are the same entry points as island.rs so lib.rs reads the
// same on both platforms; every window operation is a no-op here.

use std::sync::atomic::AtomicBool;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};

pub const WINDOW_LABEL: &str = "island";

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Kept for the shared command signatures; nothing polls on Linux.
pub struct PollGate {
    pub collapsed: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self { collapsed: AtomicBool::new(true) }
    }
    pub fn set_rect(&self, _rect: IslandRect) {}
    pub fn forget_ignore_state(&self) {}
    pub fn set_active(&self, _on: bool) {}
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

pub fn screen_info(app: &AppHandle, _pref: &str) -> ScreenInfo {
    match app.primary_monitor().ok().flatten() {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

pub fn apply_geometry(_app: &AppHandle, _pref: &str, _collapsed: bool) {}
pub fn make_non_activating(_win: &WebviewWindow) {}
pub fn set_activating(_win: &WebviewWindow, _activating: bool) {}
pub fn set_ignore_cursor(_app: &AppHandle, _ignore: bool) {}
pub fn spawn_cursor_poll(_app: AppHandle, _gate: std::sync::Arc<PollGate>) {}
