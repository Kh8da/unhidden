//! The two things that change Steam: switching account and forgetting one.

use crate::steam::{self, SteamInstall};
use std::time::{SystemTime, UNIX_EPOCH};
use steamcore::loginusers::{self, LoginUser};

pub fn load_users(install: &SteamInstall) -> Result<(String, Vec<LoginUser>), String> {
    let text = steam::read_loginusers(install)?;
    let users = loginusers::read_users(&text).map_err(|e| e.to_string())?;
    Ok((text, users))
}

fn find_user(users: &[LoginUser], id: u64) -> Result<&LoginUser, String> {
    users
        .iter()
        .find(|u| u.steam_id == id)
        .ok_or_else(|| "that account isn't saved in steam anymore. refresh and try again.".into())
}

/// Closes Steam, points it at `id` and starts it again.
pub fn switch(install: &SteamInstall, id: u64, offline: bool, progress: &dyn Fn(&str)) -> Result<(), String> {
    if let Some(reason) = steam::blocker() {
        return Err(reason.into());
    }
    // Check everything we can before closing Steam, so a bad file never costs the user their session.
    let (text, users) = load_users(install)?;
    let account_name = find_user(&users, id)?.account_name.clone();
    loginusers::select_user(&text, id, offline, 0).map_err(|e| e.to_string())?;
    steam::preflight(install)?;

    progress("closing");
    steam::shutdown(install)?;

    // Steam rewrites the file when it exits, so edit the fresh copy.
    progress("preparing");
    let (text, users) = load_users(install)?;
    find_user(&users, id)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let updated = loginusers::select_user(&text, id, offline, now).map_err(|e| e.to_string())?;
    steam::write_loginusers(install, &updated)?;
    steam::set_auto_login_user(install, &account_name)?;
    steam::disable_account_chooser(install)?;

    progress("launching");
    steam::launch(install)
}

/// Removes `id` from Steam's saved accounts, restarting Steam if it was open.
pub fn forget(install: &SteamInstall, id: u64, progress: &dyn Fn(&str)) -> Result<(), String> {
    let (text, users) = load_users(install)?;
    let account_name = find_user(&users, id)?.account_name.clone();
    loginusers::remove_user(&text, id).map_err(|e| e.to_string())?;

    let was_running = steam::is_running();
    if was_running {
        if let Some(reason) = steam::blocker() {
            return Err(reason.into());
        }
        steam::preflight(install)?;
        progress("closing");
        steam::shutdown(install)?;
    }

    progress("forgetting");
    let (text, users) = load_users(install)?;
    if users.iter().any(|u| u.steam_id == id) {
        let updated = loginusers::remove_user(&text, id).map_err(|e| e.to_string())?;
        steam::write_loginusers(install, &updated)?;
    }
    if steam::auto_login_user(install).is_some_and(|a| a.eq_ignore_ascii_case(&account_name)) {
        steam::set_auto_login_user(install, "")?;
    }

    if was_running {
        progress("launching");
        steam::launch(install)?;
    }
    Ok(())
}
