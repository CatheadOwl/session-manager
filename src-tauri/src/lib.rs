mod commands;
mod config;
mod fork_tree;
mod fs_utils;
mod session_manager;

use session_manager::metadata::{MetadataManager, WindowState};
use tauri::{Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow, WindowEvent};

/// Validate that the saved window position falls within an available monitor.
/// If the monitor has been disconnected, fall back to the primary monitor's origin.
fn clamp_window_position<R: Runtime>(
    window: &WebviewWindow<R>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> (i32, i32) {
    let Ok(monitors) = window.available_monitors() else {
        return (x, y);
    };

    // Check if the window's center falls within any connected monitor
    let cx = x + width as i32 / 2;
    let cy = y + height as i32 / 2;

    let on_screen = monitors.iter().any(|m| {
        let m_pos = m.position();
        let m_size = m.size();
        cx >= m_pos.x
            && cx <= m_pos.x + m_size.width as i32
            && cy >= m_pos.y
            && cy <= m_pos.y + m_size.height as i32
    });

    if on_screen {
        (x, y)
    } else {
        // Fall back to primary monitor origin, or (0, 0) as last resort
        window
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let p = m.position();
                (p.x, p.y)
            })
            .unwrap_or((0, 0))
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let metadata_path =
                crate::config::get_app_metadata_path().expect("Failed to resolve metadata path");
            let manager = MetadataManager::new(metadata_path);
            app.manage(manager);

            // Build and register the provider registry
            let registry = session_manager::build_provider_registry();
            app.manage(registry);

            // Window state: restore on startup, save on close
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();

                // Priority: env override > persisted state > tauri.conf.json default
                if let Ok(size_str) = std::env::var("SM_WINDOW_SIZE") {
                    if let Some((w, h)) = size_str.split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                            let _ = window.set_size(PhysicalSize::new(w, h));
                            // Reset position for deterministic screenshots
                            let _ = window.set_position(PhysicalPosition::new(0, 0));
                        }
                    }
                } else {
                    let mgr = app.state::<MetadataManager>();
                    if let Some(state) = mgr.get_window_state() {
                        let (x, y) = clamp_window_position(
                            &window,
                            state.x,
                            state.y,
                            state.width,
                            state.height,
                        );
                        let _ = window.set_size(PhysicalSize::new(state.width, state.height));
                        let _ = window.set_position(PhysicalPosition::new(x, y));
                        if state.maximized {
                            let _ = window.maximize();
                        }
                    }
                }

                // Save window state on close (skip when size is overridden via SM_WINDOW_SIZE)
                window.clone().on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { .. } = event {
                        if std::env::var("SM_WINDOW_SIZE").is_ok() {
                            return;
                        }
                        let mgr = handle.state::<MetadataManager>();
                        if let (Ok(size), Ok(pos)) = (window.inner_size(), window.outer_position())
                        {
                            let maximized = window.is_maximized().unwrap_or(false);
                            let state = WindowState {
                                width: size.width,
                                height: size.height,
                                x: pos.x,
                                y: pos.y,
                                maximized,
                            };
                            let _ = mgr.set_window_state(state);
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::session_manager::list_sessions,
            commands::session_manager::get_session_messages,
            commands::session_manager::get_session_detail,
            commands::session_manager::delete_session,
            commands::session_manager::delete_sessions,
            commands::session_manager::archive_sessions,
            commands::session_manager::restore_sessions,
            commands::session_manager::get_app_metadata,
            commands::session_manager::set_session_starred,
            commands::session_manager::set_pinned_folders,
            commands::session_manager::archive_session,
            commands::session_manager::restore_session,
            commands::fork_tree::compute_fork_tree,
            commands::fork_tree::get_fork_tree,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
