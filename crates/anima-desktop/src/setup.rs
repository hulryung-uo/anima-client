use crate::config::ConfigStore;
use anima_net::uo_dir::{
    self,
    check::{inspect, DataReport},
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Default, Serialize)]
pub struct SetupView {
    pub report: Option<DataReport>,
    pub saved_path: String,
    pub active_path: String,
    pub config_path: String,
    pub config_error: Option<String>,
    pub recoverable: bool,
    pub notice: String,
    pub startup_error: String,
    pub initialising: bool,
    pub starting: bool,
    pub running: bool,
}
pub struct SetupState {
    pub config: ConfigStore,
    pub view: Mutex<SetupView>,
    operation: Mutex<()>,
}
impl SetupState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            view: Mutex::new(SetupView {
                config_path: path.display().to_string(),
                initialising: true,
                ..Default::default()
            }),
            config: ConfigStore::new(path),
            operation: Mutex::new(()),
        }
    }
    pub fn snapshot(&self) -> SetupView {
        self.view.lock().unwrap().clone()
    }
}
fn state(app: &AppHandle) -> Arc<SetupState> {
    app.state::<Arc<SetupState>>().inner().clone()
}
fn trusted(window: &WebviewWindow) -> Result<(), String> {
    let url = window
        .url()
        .map_err(|_| "Cannot identify this settings window.")?;
    if window.label() != "setup" || !local_setup_url(&url) {
        return Err("Use Anima's Game files window.".into());
    }
    Ok(())
}
fn local_setup_url(url: &tauri::Url) -> bool {
    ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost")))
        && matches!(url.path(), "" | "/" | "/index.html")
}
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("setup") {
        window.show()?;
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("index.html".into()))
        .title("Anima — Game files")
        .inner_size(850.0, 780.0)
        .min_inner_size(520.0, 480.0)
        .on_navigation(local_setup_url)
        .build()?;
    Ok(())
}
pub fn initialize(app: AppHandle) {
    let state = state(&app);
    let _operation = state.operation.lock().unwrap();
    let loaded = state.config.load();
    let saved = loaded
        .as_ref()
        .ok()
        .and_then(|cfg| cfg.as_ref())
        .map(|cfg| cfg.data_dir.clone())
        .filter(|p| !p.as_os_str().is_empty());
    let dir = saved.clone().or_else(uo_dir::detect_uo_dir);
    let report = dir.as_ref().map(|p| inspect(p));
    let auto = loaded.is_ok() && saved.is_some() && report.as_ref().is_some_and(|r| r.ready);
    {
        let mut view = state.view.lock().unwrap();
        view.saved_path = saved.unwrap_or_default().display().to_string();
        view.config_error = loaded.err();
        view.recoverable = state.config.recoverable();
        view.report = report;
        view.initialising = false;
        view.notice = if auto {
            "Opening Anima…"
        } else {
            "Choose your existing Ultima Online client folder. Game files stay on your device."
        }
        .into();
        view.starting = auto;
    }
    if auto {
        crate::launch(app, dir.unwrap());
    }
}
pub fn failed(app: &AppHandle, message: String) {
    let state = state(app);
    let mut view = state.view.lock().unwrap();
    view.starting = false;
    view.startup_error = message;
    drop(view);
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = open(&app2);
    });
}
pub fn running(app: &AppHandle, path: &std::path::Path) {
    let state = state(app);
    let mut view = state.view.lock().unwrap();
    view.running = true;
    view.starting = false;
    view.active_path = path.display().to_string();
    if view.notice == "Opening Anima…" {
        view.notice.clear();
    }
}

#[tauri::command]
pub fn setup_status(window: WebviewWindow, app: AppHandle) -> Result<SetupView, String> {
    trusted(&window)?;
    Ok(state(&app).snapshot())
}
async fn operate(
    app: AppHandle,
    work: impl FnOnce(&AppHandle, &SetupState) -> Result<(), String> + Send + 'static,
) -> Result<SetupView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = state(&app);
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another file check is running. Please wait.")?;
        if state.snapshot().starting {
            return Err("Anima is opening. Please wait.".into());
        }
        if let Err(error) = work(&app, &state) {
            if let Err(config_error) = state.config.load() {
                let mut view = state.view.lock().unwrap();
                view.config_error = Some(config_error);
                view.recoverable = state.config.recoverable();
            }
            return Err(error);
        }
        Ok(state.snapshot())
    })
    .await
    .map_err(|e| format!("The file check could not finish: {e}"))?
}
fn set_candidate(state: &SetupState, path: PathBuf) {
    let report = inspect(&path);
    let mut view = state.view.lock().unwrap();
    view.report = Some(report);
    view.notice.clear();
    view.startup_error.clear();
}
#[tauri::command]
pub async fn setup_check(
    window: WebviewWindow,
    app: AppHandle,
    path: String,
) -> Result<SetupView, String> {
    trusted(&window)?;
    operate(app, move |_, state| {
        let path = path.trim();
        if path.is_empty() || path.len() > 4096 {
            return Err("Enter the folder containing your UO client files.".into());
        }
        set_candidate(state, PathBuf::from(path));
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn setup_choose(window: WebviewWindow, app: AppHandle) -> Result<SetupView, String> {
    trusted(&window)?;
    operate(app, |app, state| {
        let choice = app
            .dialog()
            .file()
            .set_title("Choose your Ultima Online client folder")
            .blocking_pick_folder();
        if let Some(choice) = choice {
            set_candidate(
                state,
                choice.into_path().map_err(|_| "Choose a local folder.")?,
            );
        } else {
            state.view.lock().unwrap().notice =
                "Folder selection cancelled. Your saved location has not changed.".into();
        }
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn setup_detect(window: WebviewWindow, app: AppHandle) -> Result<SetupView, String> {
    trusted(&window)?;
    operate(app, |_, state| {
        if let Some(dir) = uo_dir::detect_uo_dir() {
            set_candidate(state, dir);
        } else {
            state.view.lock().unwrap().notice =
                "No installation was found in the usual locations. Choose its folder below.".into();
        }
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn setup_recover(window: WebviewWindow, app: AppHandle) -> Result<SetupView, String> {
    trusted(&window)?;
    operate(app, |_, state| {
        let backup = state.config.recover()?;
        let mut view = state.view.lock().unwrap();
        view.config_error = None; view.recoverable = false; view.saved_path.clear();
        view.notice = format!("App settings recovered. The original file is backed up at {}. Choose a folder to continue.", backup.display());
        Ok(())
    }).await
}
#[tauri::command]
pub async fn setup_apply(window: WebviewWindow, app: AppHandle) -> Result<SetupView, String> {
    trusted(&window)?;
    operate(app, |app, state| {
        let snapshot = state.snapshot();
        let path = PathBuf::from(snapshot.report.ok_or("Check a game folder first.")?.path);
        set_candidate(state, path.clone());
        if !state.snapshot().report.is_some_and(|r| r.ready) { return Err("Required files are missing or unreadable. Choose a complete client folder.".into()); }
        state.config.save_data_dir(&path)?;
        let mut view = state.view.lock().unwrap();
        view.saved_path = path.display().to_string();
        view.config_error = None; view.recoverable = false;
        if view.running {
            view.notice = if view.saved_path == view.active_path {
                "Location saved. Anima is already using this folder."
            } else {
                "Location saved for the next launch. Restart Anima when you are ready; restarting disconnects the current game session."
            }.into();
        } else {
            view.starting = true; view.notice = "Opening Anima…".into();
            drop(view); crate::launch(app.clone(), path);
        }
        Ok(())
    }).await
}
#[tauri::command]
pub fn setup_close(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    trusted(&window)?;
    if !state(&app).snapshot().running {
        return Err("Finish setup before opening Anima.".into());
    }
    window.close().map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn setup_restart(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    trusted(&window)?;
    if !state(&app).snapshot().running {
        return Err("Use Open Anima to finish setup.".into());
    }
    tauri::async_runtime::spawn_blocking(move || app.restart());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_navigation_cannot_reach_remote_or_game_pages() {
        for url in [
            "tauri://localhost",
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/index.html",
        ] {
            assert!(local_setup_url(&url.parse().unwrap()));
        }
        for url in [
            "https://example.com/",
            "http://127.0.0.1:8190/",
            "http://tauri.localhost.evil/",
            "tauri://localhost/other.html",
        ] {
            assert!(!local_setup_url(&url.parse().unwrap()));
        }
    }
}
