//! Tauri shell: window, tray and autostart wiring around `cutokyo_desktop`.

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Cutokyo")
                .build(),
        )
        .setup(|app| {
            setup_system_tray(app)?;
            if cutokyo_desktop::settings::effective_settings().auto_connect
                && cutokyo_desktop::activation::has_persisted_activation()
            {
                if let Err(error) = cutokyo_desktop::enable_owned_autostart() {
                    eprintln!("Cutokyo autostart could not be enabled: {error}");
                }
            }
            #[cfg(all(unix, not(debug_assertions)))]
            if let Err(error) = cutokyo_desktop::sync_cli_path(true) {
                eprintln!("Cutokyo CLI installation did not complete: {error}");
            }
            tauri::async_runtime::spawn(async {
                if let Err(error) = cutokyo_desktop::serve_desktop_services().await {
                    eprintln!("Cutokyo desktop services did not start: {error}");
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Cutokyo desktop app");
}

fn setup_system_tray(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open Cutokyo", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cutokyo", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("Cutokyo is observing AI traffic")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
