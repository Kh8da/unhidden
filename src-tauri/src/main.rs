#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod steam;
mod store;

use base64::Engine;
use serde::Serialize;
use std::sync::Mutex;
use steamcore::loginusers;
use steam::SteamInstall;
use store::{Meta, Store};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    store: Mutex<Store>,
    /// Held while Steam is being closed/edited/started so two actions never overlap.
    busy: Mutex<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountDto {
    /// SteamID64 is larger than JavaScript's safe integer range, so it travels as a string.
    steam_id: String,
    account_name: String,
    persona_name: String,
    timestamp: u64,
    signed_in: bool,
    remember_password: bool,
    has_avatar: bool,
    pinned: bool,
    nickname: String,
    note: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateDto {
    /// "windows", "linux" or "macos".
    platform: &'static str,
    steam_dir: Option<String>,
    steam_running: bool,
    accounts: Vec<AccountDto>,
    error: Option<String>,
}

fn find_install(state: &AppState) -> Option<SteamInstall> {
    let dir = state.store.lock().unwrap().data.steam_dir.clone();
    steam::detect(dir.as_deref())
}

fn parse_id(steam_id: &str) -> Result<u64, String> {
    steam_id.parse().map_err(|_| "invalid account id".to_string())
}

fn progress(app: &AppHandle, stage: &str) {
    let _ = app.emit("progress", stage);
}

#[tauri::command]
fn get_state(state: State<AppState>) -> StateDto {
    let Some(install) = find_install(&state) else {
        return StateDto {
            platform: std::env::consts::OS,
            steam_dir: None,
            steam_running: false,
            accounts: Vec::new(),
            error: None,
        };
    };
    let running = steam::is_running();
    let mut dto = StateDto {
        platform: std::env::consts::OS,
        steam_dir: Some(install.dir.display().to_string()),
        steam_running: running,
        accounts: Vec::new(),
        error: None,
    };
    let users = match actions::load_users(&install) {
        Ok((_, users)) => users,
        Err(e) => {
            dto.error = Some(e);
            return dto;
        }
    };
    let active = if running { steam::active_account_id(&install) } else { None };
    let store = state.store.lock().unwrap();
    dto.accounts = users
        .into_iter()
        .map(|u| {
            let id = u.steam_id.to_string();
            let meta = store.meta(&id);
            AccountDto {
                signed_in: active == Some(loginusers::account_id(u.steam_id)),
                has_avatar: install.avatar(u.steam_id).is_some(),
                remember_password: u.remember_password,
                steam_id: id,
                account_name: u.account_name,
                persona_name: u.persona_name,
                timestamp: u.timestamp,
                pinned: meta.pinned,
                nickname: meta.nickname,
                note: meta.note,
            }
        })
        .collect();
    dto
}

#[tauri::command]
fn get_avatar(state: State<AppState>, steam_id: String) -> Option<String> {
    let install = find_install(&state)?;
    let bytes = std::fs::read(install.avatar(parse_id(&steam_id).ok()?)?).ok()?;
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else {
        return None;
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{b64}"))
}

fn switch_blocking(app: &AppHandle, steam_id: &str, offline: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _busy = state.busy.try_lock().map_err(|_| "hold on, still working on the last one.")?;
    let id = parse_id(steam_id)?;
    let install = find_install(&state).ok_or("steam isn't installed or couldn't be found.")?;
    actions::switch(&install, id, offline, &|stage| progress(app, stage))
}

#[tauri::command]
async fn switch_account(app: AppHandle, steam_id: String, offline: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || switch_blocking(&app, &steam_id, offline))
        .await
        .map_err(|e| e.to_string())?
}

fn forget_blocking(app: &AppHandle, steam_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _busy = state.busy.try_lock().map_err(|_| "hold on, still working on the last one.")?;
    let id = parse_id(steam_id)?;
    let install = find_install(&state).ok_or("steam isn't installed or couldn't be found.")?;
    actions::forget(&install, id, &|stage| progress(app, stage))?;
    let mut store = state.store.lock().unwrap();
    store.set_meta(steam_id, Meta::default());
    store.save()
}

#[tauri::command]
async fn forget_account(app: AppHandle, steam_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || forget_blocking(&app, &steam_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn update_meta(state: State<AppState>, steam_id: String, meta: Meta) -> Result<(), String> {
    parse_id(&steam_id)?;
    let clean = Meta {
        pinned: meta.pinned,
        nickname: meta.nickname.trim().chars().take(40).collect(),
        note: meta.note.trim().chars().take(200).collect(),
    };
    let mut store = state.store.lock().unwrap();
    store.set_meta(&steam_id, clean);
    store.save()
}

#[tauri::command]
async fn pick_steam_folder(app: AppHandle) -> Result<bool, String> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title(if cfg!(windows) {
            "Find your Steam folder (the one with steam.exe)"
        } else {
            "Find your Steam folder (the one with a config folder inside)"
        })
        .blocking_pick_folder()
    else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|e| e.to_string())?;
    let dir = dir.display().to_string();
    if !steam::valid_dir(&dir) {
        return Err("that doesn't look like a steam folder.".into());
    }
    let state = app.state::<AppState>();
    let mut store = state.store.lock().unwrap();
    store.data.steam_dir = Some(dir);
    store.save()?;
    Ok(true)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            app.manage(AppState {
                store: Mutex::new(Store::load(path)),
                busy: Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_avatar,
            switch_account,
            forget_account,
            update_meta,
            pick_steam_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running unhidden");
}
