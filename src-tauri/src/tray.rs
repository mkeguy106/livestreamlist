//! System-tray icon + menu.
//!
//! The tray is a secondary UI: shows a live-count tooltip, left-click toggles
//! the main window, and a small menu offers Show / Refresh / Quit.
//!
//! The tray is created once in `setup()` and its tooltip is refreshed from
//! `refresh_all` after each snapshot. Keep this module display-only; business
//! logic goes through invoke handlers so behavior is shared with the window.

use anyhow::Result;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

pub fn build(app: &AppHandle) -> Result<()> {
    let show = MenuItem::with_id(app, "tray:show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "tray:hide", "Hide", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "tray:refresh", "Refresh now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray:quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &hide, &refresh, &quit])?;

    TrayIconBuilder::with_id("main")
        .tooltip("livestreamlist")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("default window icon should be present"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "tray:show" => show_main(app),
            "tray:hide" => hide_main(app),
            "tray:refresh" => {
                let _ = app.emit("tray:refresh-requested", ());
            }
            "tray:quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                toggle_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

pub fn update_tooltip(app: &AppHandle, live: usize, total: usize) {
    if let Some(tray) = app.tray_by_id("main") {
        let text = format!("livestreamlist — {live} live / {total} channels");
        let _ = tray.set_tooltip(Some(text));
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

fn toggle_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(true) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}
