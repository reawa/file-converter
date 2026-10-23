use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, WindowEvent};

const PREF_FILE: &str = "close-to-tray";

pub struct TrayPrefs {
    pub close_to_tray: AtomicBool,
}

impl TrayPrefs {
    pub fn load(app: &AppHandle) -> Self {
        let enabled = read_pref(&pref_path(app)).unwrap_or(true);
        Self {
            close_to_tray: AtomicBool::new(enabled),
        }
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let prefs = app.state::<TrayPrefs>();
    let initial = prefs.close_to_tray.load(Ordering::Relaxed);

    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let close_to_tray = CheckMenuItem::with_id(
        app,
        "close-to-tray",
        "Close to tray",
        true,
        initial,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &close_to_tray, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("missing window icon")?;

    let toggle = close_to_tray.clone();
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("File Converter")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            "close-to-tray" => {
                let enabled = toggle.is_checked().unwrap_or(true);
                if let Some(prefs) = app.try_state::<TrayPrefs>() {
                    prefs.close_to_tray.store(enabled, Ordering::Relaxed);
                }
                let _ = write_pref(&pref_path(app), enabled);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        let handle = app.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let stay = handle
                    .try_state::<TrayPrefs>()
                    .map(|p| p.close_to_tray.load(Ordering::Relaxed))
                    .unwrap_or(true);
                if stay {
                    api.prevent_close();
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
        });
    }

    Ok(())
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn pref_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(PREF_FILE))
}

fn read_pref(path: &Option<PathBuf>) -> Option<bool> {
    let path = path.as_ref()?;
    let raw = fs::read_to_string(path).ok()?;
    Some(parse_close_to_tray(&raw))
}

fn write_pref(path: &Option<PathBuf>, enabled: bool) -> Result<(), String> {
    let path = path
        .as_ref()
        .ok_or_else(|| "config directory unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, if enabled { "1" } else { "0" }).map_err(|e| e.to_string())
}

/// Missing or unrecognized values stay on. Only `0` / `false` / `off` turn it off.
pub fn parse_close_to_tray(raw: &str) -> bool {
    let value = raw.trim().to_ascii_lowercase();
    !matches!(value.as_str(), "0" | "false" | "off" | "no")
}

#[cfg(test)]
mod tests {
    use super::parse_close_to_tray;

    #[test]
    fn close_to_tray_defaults_on() {
        assert!(parse_close_to_tray(""));
        assert!(parse_close_to_tray("1"));
        assert!(parse_close_to_tray("true"));
        assert!(!parse_close_to_tray("0"));
        assert!(!parse_close_to_tray("false"));
        assert!(!parse_close_to_tray(" off "));
    }
}
