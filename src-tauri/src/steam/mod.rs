//! Everything that touches the real Steam install: processes, files and the
//! auto-login setting (the registry on Windows, registry.vdf elsewhere).

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use sys::process;

#[derive(Debug, Clone)]
pub struct SteamInstall {
    pub dir: PathBuf,
}

impl SteamInstall {
    pub fn loginusers(&self) -> PathBuf {
        self.dir.join("config").join("loginusers.vdf")
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config").join("config.vdf")
    }

    pub fn avatar(&self, steam_id: u64) -> Option<PathBuf> {
        let cache = self.dir.join("config").join("avatarcache");
        ["png", "jpg"]
            .iter()
            .map(|ext| cache.join(format!("{steam_id}.{ext}")))
            .find(|p| p.is_file())
    }
}

/// Finds Steam, preferring a folder the user picked by hand.
pub fn detect(user_choice: Option<&str>) -> Option<SteamInstall> {
    user_choice
        .map(PathBuf::from)
        .into_iter()
        .chain(sys::candidates())
        .find(|d| sys::is_steam_dir(d))
        .map(|dir| SteamInstall { dir: sys::normalize(dir) })
}

pub fn valid_dir(dir: &str) -> bool {
    sys::is_steam_dir(Path::new(dir))
}

/// Why switching can't work right now, if it can't.
pub fn blocker() -> Option<&'static str> {
    sys::in_game_mode().then_some(
        "switch accounts from desktop mode. in game mode, closing steam also closes unhidden, so it can't finish the switch.",
    )
}

/// The account id Steam is signed in as right now, if it is running.
pub fn active_account_id(install: &SteamInstall) -> Option<u32> {
    sys::active_account_id(install)
}

pub fn auto_login_user(install: &SteamInstall) -> Option<String> {
    sys::auto_login_user(install)
}

pub fn set_auto_login_user(install: &SteamInstall, account_name: &str) -> Result<(), String> {
    sys::set_auto_login_user(install, account_name)
}

pub fn is_running() -> bool {
    !process::find(sys::MAIN).is_empty()
}

fn wait_for_exit(names: &[&str], timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if process::find(names).is_empty() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        sleep(Duration::from_millis(200));
    }
}

/// Closes Steam, politely first so it saves its state, then by force.
pub fn shutdown(install: &SteamInstall) -> Result<(), String> {
    if process::find(sys::CLIENT).is_empty() {
        return Ok(());
    }
    if is_running() {
        sys::request_quit(install);
        wait_for_exit(sys::MAIN, Duration::from_secs(15));
    }
    // Whatever is left (a hung client or leftover web helpers) is closed by force.
    for pid in process::find(sys::CLIENT) {
        process::kill(pid);
    }
    if !wait_for_exit(sys::CLIENT, Duration::from_secs(5)) {
        return Err("steam wouldn't close. close it yourself, then try again.".into());
    }
    // Give the OS a moment to release Steam's files.
    sleep(Duration::from_millis(400));
    Ok(())
}

pub fn launch(install: &SteamInstall) -> Result<(), String> {
    sys::launch(install).map_err(|e| format!("couldn't start steam: {e}"))
}

pub(crate) fn read_steam_file(path: &Path) -> Result<String, String> {
    let name = file_name(path);
    match fs::read(path) {
        // Never "repair" bytes we don't understand: these files hold people's names.
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| format!("{name} isn't valid text, so unhidden won't touch it.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("couldn't read {name}: {e}")),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Backs up the current file, then replaces it in one step so a crash can
/// never leave Steam with a half-written file.
pub(crate) fn write_steam_file(path: &Path, contents: &str) -> Result<(), String> {
    let name = file_name(path);
    let backup = path.with_extension("vdf.unhidden-backup");
    let tmp = path.with_extension("vdf.unhidden-tmp");
    if path.is_file() {
        fs::copy(path, &backup).map_err(|e| format!("couldn't back up {name}: {e}"))?;
        // Some guides tell people to make these files read-only, which would make Steam ignore our change.
        if let Ok(meta) = fs::metadata(path) {
            let mut perms = meta.permissions();
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = fs::set_permissions(path, perms);
            }
        }
    }
    fs::write(&tmp, contents).map_err(|e| format!("couldn't write {name}: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("couldn't save {name}: {e}")
    })
}

pub fn read_loginusers(install: &SteamInstall) -> Result<String, String> {
    read_steam_file(&install.loginusers())
}

pub fn write_loginusers(install: &SteamInstall, contents: &str) -> Result<(), String> {
    write_steam_file(&install.loginusers(), contents)
}

/// Fails if a file the switch must edit exists but can't be safely edited.
pub fn preflight(install: &SteamInstall) -> Result<(), String> {
    read_steam_file(&install.config())?;
    sys::check_auto_login(install)
}

/// Turns off "Ask which account to use each time Steam starts", which would
/// otherwise make Steam show its picker instead of signing in.
pub fn disable_account_chooser(install: &SteamInstall) -> Result<(), String> {
    let path = install.config();
    let text = read_steam_file(&path)?;
    match steamcore::config::disable_account_chooser(&text) {
        Some(updated) => write_steam_file(&path, &updated),
        None => Ok(()),
    }
}
