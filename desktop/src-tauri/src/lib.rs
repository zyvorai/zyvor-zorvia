mod paths;
mod secrets;
mod server;

use paths::AppSettings;
use server::ServerState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
fn app_url(state: tauri::State<ServerState>) -> Result<Option<String>, String> {
    server::app_url(&state)
}

#[tauri::command]
fn get_settings() -> AppSettings {
    paths::load_settings()
}

#[tauri::command]
fn set_settings(settings: AppSettings) -> Result<(), String> {
    paths::save_settings(&settings)
}

fn open_settings_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Zorvia Settings")
        .inner_size(460.0, 420.0)
        .resizable(false)
        .build();
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn copy_admin_password(app: &AppHandle) {
    let body = match secrets::admin_password() {
        Ok(pw) => match app.clipboard().write_text(pw) {
            Ok(()) => "Admin password copied. Sign in as “admin”.".to_string(),
            Err(e) => format!("Could not copy: {e}"),
        },
        Err(e) => format!("Could not read the password: {e}"),
    };
    let _ = app.notification().builder().title("Zorvia").body(body).show();
}

fn setup_menu(app: &AppHandle) -> tauri::Result<()> {
    let app_menu = Submenu::with_items(
        app,
        "Zorvia",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?,
            &MenuItem::with_id(app, "copy-pw", "Copy Admin Password", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    app.set_menu(Menu::with_items(app, &[&app_menu, &edit, &window])?)?;
    app.on_menu_event(|app, event| match event.id().as_ref() {
        "settings" => open_settings_window(app),
        "copy-pw" => copy_admin_password(app),
        _ => {}
    });
    Ok(())
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "tray-open", "Open Zorvia", true, None::<&str>)?,
            &MenuItem::with_id(app, "tray-settings", "Settings…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "tray-quit", "Quit Zorvia", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Zorvia")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-open" => show_main(app),
            "tray-settings" => open_settings_window(app),
            "tray-quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(ServerState::default())
        .invoke_handler(tauri::generate_handler![app_url, get_settings, set_settings])
        .setup(|app| {
            setup_menu(app.handle())?;
            setup_tray(app.handle())?;
            server::start_in_background(app.handle().clone(), paths::load_settings());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Zorvia desktop app");

    app.run(|app, event| match event {
        // Closing the main window hides it; the tray keeps the app alive.
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
        }
        // Clicking the Dock icon brings a hidden window back.
        RunEvent::Reopen { .. } => show_main(app),
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            server::shutdown(&app.state::<ServerState>());
        }
        _ => {}
    });
}
