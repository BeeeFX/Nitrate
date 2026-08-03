//! Tray icon and the popup-window behaviour that hangs off it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Rect, WebviewWindow, WindowEvent,
};

pub fn setup(app: &AppHandle, suppress_hide: Arc<AtomicBool>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Nitrate", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Nitrate — drop a video to compress")
        .menu(&menu)
        // Left click drives the popup; the menu is for right click only.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_popup(app, None),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_popup(tray.app_handle(), Some(rect));
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        attach_popup_behaviour(&window, suppress_hide);
    }

    Ok(())
}

/// Closing the popup should put it back in the tray, not kill the app.
fn attach_popup_behaviour(window: &WebviewWindow, suppress_hide: Arc<AtomicBool>) {
    let w = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = w.hide();
        }
        // Clicking elsewhere dismisses the popup — unless a native dialog is up
        // or the user has pinned the window, either of which would make this
        // feel like the app vanishing at random.
        WindowEvent::Focused(false) if !suppress_hide.load(Ordering::Relaxed) => {
            let _ = w.hide();
        }
        _ => {}
    });
}

fn toggle_popup(app: &AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    show_popup(app, tray_rect);
}

/// Shows the popup and tells the frontend, which plays its entrance animation.
/// Positioning happens before `show` so the window never appears at its old
/// spot and then jumps.
pub fn show_popup(app: &AppHandle, tray_rect: Option<Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if let Some(rect) = tray_rect {
        position_near_tray(&window, rect);
    }

    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit("popup://shown", ());
}

/// Compact size, and the roomier one used while the editor is open.
const COMPACT: (f64, f64) = (440.0, 660.0);
const EXPANDED: (f64, f64) = (760.0, 800.0);

/// Grows the popup for the editor and shrinks it back afterwards.
///
/// Resizing keeps the top-left corner where it is and grows right and down,
/// which is wrong for a window that lives against the bottom-right of the
/// screen: the wider window then had to be pulled back on screen, and shrinking
/// it again left it where it had been pulled to. Opening and closing the editor
/// therefore walked the popup a few hundred pixels away from the tray.
///
/// So the edges nearest the screen edges are the ones held still. That's the
/// bottom-right for a taskbar in its usual place, the top-left on a monitor
/// where the window sits up there, and it returns to exactly the original
/// position when the size goes back.
pub fn resize_popup(app: &AppHandle, expanded: bool) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let before = window
        .outer_position()
        .ok()
        .zip(window.outer_size().ok())
        .zip(anchor_corner(&window));

    let (w, h) = if expanded { EXPANDED } else { COMPACT };
    if window.set_size(LogicalSize::new(w, h)).is_err() {
        return;
    }

    if let Some(((position, size), (keep_right, keep_bottom))) = before {
        if let Ok(grown) = window.outer_size() {
            let dx = grown.width as i32 - size.width as i32;
            let dy = grown.height as i32 - size.height as i32;
            let x = if keep_right { position.x - dx } else { position.x };
            let y = if keep_bottom { position.y - dy } else { position.y };
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
    }

    clamp_into_monitor(&window);
}

/// Which corner of the window to hold still through a resize: the one closest
/// to the corner of the screen it's sitting in.
fn anchor_corner(window: &WebviewWindow) -> Option<(bool, bool)> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())?;

    let area = monitor.size();
    let origin = monitor.position();
    let centre_x = position.x + size.width as i32 / 2;
    let centre_y = position.y + size.height as i32 / 2;

    Some((
        centre_x > origin.x + area.width as i32 / 2,
        centre_y > origin.y + area.height as i32 / 2,
    ))
}

/// Nudges the window back on screen after a resize.
fn clamp_into_monitor(window: &WebviewWindow) {
    const MARGIN: i32 = 12;

    let (Ok(size), Ok(position)) = (window.outer_size(), window.outer_position()) else {
        return;
    };

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };

    let area = monitor.size();
    let origin = monitor.position();
    let max_x = origin.x + area.width as i32 - size.width as i32 - MARGIN;
    let max_y = origin.y + area.height as i32 - size.height as i32 - MARGIN;

    // A window taller than the screen would make the maximum smaller than the
    // minimum, which clamp would panic on.
    let x = position.x.min(max_x).max(origin.x + MARGIN);
    let y = position.y.min(max_y).max(origin.y + MARGIN);

    if x != position.x || y != position.y {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// Anchors the popup to the tray icon, flipping to whichever side of the
/// taskbar has room and clamping so it never hangs off the monitor.
fn position_near_tray(window: &WebviewWindow, rect: Rect) {
    const GAP: i32 = 12;

    let scale = window.scale_factor().unwrap_or(1.0);
    let tray_pos = rect.position.to_physical::<i32>(scale);
    let tray_size = rect.size.to_physical::<i32>(scale);

    let Ok(win_size) = window.outer_size() else {
        return;
    };
    let win_w = win_size.width as i32;
    let win_h = win_size.height as i32;

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());

    let (mon_x, mon_y, mon_w, mon_h) = match &monitor {
        Some(m) => {
            let p = m.position();
            let s = m.size();
            (p.x, p.y, s.width as i32, s.height as i32)
        }
        None => (0, 0, 1920, 1080),
    };

    // Centre horizontally on the icon.
    let mut x = tray_pos.x + tray_size.width / 2 - win_w / 2;

    // A tray in the upper half of the screen means the bar is at the top,
    // so the popup has to drop downwards instead.
    let tray_centre_y = tray_pos.y + tray_size.height / 2;
    let bar_at_top = tray_centre_y < mon_y + mon_h / 2;
    let mut y = if bar_at_top {
        tray_pos.y + tray_size.height + GAP
    } else {
        tray_pos.y - win_h - GAP
    };

    x = x.clamp(mon_x + GAP, mon_x + mon_w - win_w - GAP);
    y = y.clamp(mon_y + GAP, mon_y + mon_h - win_h - GAP);

    let _ = window.set_position(PhysicalPosition::new(x, y));
}
