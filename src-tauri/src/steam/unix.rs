//! Linux (including Steam Deck) and macOS: Steam keeps its "registry" in a
//! `registry.vdf` file, and is closed with SIGTERM like any Unix program.

use super::{read_steam_file, write_steam_file, SteamInstall};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use steamcore::registry::{self, ACTIVE_PROCESS_KEY, STEAM_KEY};

#[cfg(target_os = "linux")]
pub const MAIN: &[&str] = &["steam"];
#[cfg(target_os = "linux")]
pub const CLIENT: &[&str] = &["steam", "steamwebhelper"];

#[cfg(target_os = "macos")]
pub const MAIN: &[&str] = &["steam_osx"];
/// A trailing `*` matches by prefix (Steam Helper, Steam Helper (GPU), ...).
#[cfg(target_os = "macos")]
pub const CLIENT: &[&str] = &["steam_osx", "steamwebhelper", "Steam Helper*"];

#[cfg(target_os = "linux")]
const FLATPAK_ID: &str = "com.valvesoftware.Steam";

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

#[cfg(target_os = "linux")]
pub fn candidates() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".steam/steam"),
        h.join(".steam/root"),
        h.join(".local/share/Steam"),
        h.join(".steam/debian-installation"),
        h.join(format!(".var/app/{FLATPAK_ID}/.local/share/Steam")),
        h.join("snap/steam/common/.local/share/Steam"),
    ]
}

#[cfg(target_os = "macos")]
pub fn candidates() -> Vec<PathBuf> {
    vec![home().join("Library/Application Support/Steam")]
}

/// Steam's data folder always has its account list or its config in `config/`.
pub fn is_steam_dir(dir: &Path) -> bool {
    let config = dir.join("config");
    config.join("loginusers.vdf").is_file() || config.join("config.vdf").is_file()
}

/// Resolves `~/.steam/steam`-style symlinks so one install has one path.
pub fn normalize(dir: PathBuf) -> PathBuf {
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

/// Steam Deck's Game Mode: Steam *is* the session there, so closing it also
/// closes this app before it can finish a switch.
pub fn in_game_mode() -> bool {
    let var = |k: &str| std::env::var(k).unwrap_or_default().to_lowercase();
    !var("GAMESCOPE_WAYLAND_DISPLAY").trim().is_empty()
        || ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "XDG_SESSION_DESKTOP"]
            .iter()
            .any(|k| var(k).contains("gamescope"))
}

/// Where this install keeps registry.vdf.
pub fn registry_vdf(install: &SteamInstall) -> PathBuf {
    if cfg!(target_os = "macos") {
        return install.dir.join("registry.vdf");
    }
    // Flatpak and Snap keep a whole home of their own: <prefix>/.local/share/Steam
    // pairs with <prefix>/.steam/registry.vdf. A native install uses ~/.steam.
    let dir = &install.dir;
    if dir.ends_with(".local/share/Steam") {
        if let Some(prefix) = dir.parent().and_then(Path::parent).and_then(Path::parent) {
            return prefix.join(".steam").join("registry.vdf");
        }
    }
    home().join(".steam").join("registry.vdf")
}

fn registry_text(install: &SteamInstall) -> String {
    std::fs::read(registry_vdf(install))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

pub fn active_account_id(install: &SteamInstall) -> Option<u32> {
    let id: u32 = registry::get(&registry_text(install), ACTIVE_PROCESS_KEY, "ActiveUser")?
        .trim()
        .parse()
        .ok()?;
    (id != 0).then_some(id)
}

pub fn auto_login_user(install: &SteamInstall) -> Option<String> {
    registry::get(&registry_text(install), STEAM_KEY, "AutoLoginUser").filter(|s| !s.trim().is_empty())
}

/// Fails if registry.vdf exists but can't be safely edited.
pub fn check_auto_login(install: &SteamInstall) -> Result<(), String> {
    let text = read_steam_file(&registry_vdf(install))?;
    if text.trim().is_empty() {
        return Ok(());
    }
    steamcore::vdf::parse(&text)
        .map(|_| ())
        .map_err(|e| format!("registry.vdf could not be read: {e}"))
}

pub fn set_auto_login_user(install: &SteamInstall, account_name: &str) -> Result<(), String> {
    let path = registry_vdf(install);
    let text = read_steam_file(&path)?;
    let bad = |e| format!("registry.vdf could not be read: {e}");
    let mut text = registry::set(&text, STEAM_KEY, "AutoLoginUser", account_name).map_err(bad)?;
    if !account_name.is_empty() {
        text = registry::set(&text, STEAM_KEY, "RememberPassword", "1").map_err(bad)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    write_steam_file(&path, &text)
}

pub mod process {
    use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

    fn matches(name: &str, pattern: &str) -> bool {
        match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == pattern,
        }
    }

    /// Live processes of this user whose name matches one of `names`.
    pub fn find(names: &[&str]) -> Vec<u32> {
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
        );
        let me = unsafe { libc::getuid() };
        sys.processes()
            .iter()
            .filter(|(_, p)| {
                let name = p.name().to_string_lossy();
                // A zombie has already exited; it just hasn't been reaped yet.
                !matches!(p.status(), ProcessStatus::Zombie | ProcessStatus::Dead)
                    && p.user_id().is_some_and(|u| **u == me)
                    && names.iter().any(|n| matches(&name, n))
            })
            .map(|(pid, _)| pid.as_u32())
            .collect()
    }

    pub fn signal(pid: u32, sig: i32) {
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
    }

    pub fn kill(pid: u32) {
        signal(pid, libc::SIGKILL);
    }
}

/// Asks the running client to save and exit. Steam treats SIGTERM as "quit".
pub fn request_quit(_install: &SteamInstall) {
    for pid in process::find(MAIN) {
        process::signal(pid, libc::SIGTERM);
    }
}

#[cfg(target_os = "linux")]
fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|p| {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

#[cfg(target_os = "linux")]
fn launch_command(install: &SteamInstall) -> Command {
    let dir = install.dir.to_string_lossy();
    if dir.contains(&format!("/.var/app/{FLATPAK_ID}/")) {
        let mut c = Command::new("flatpak");
        c.args(["run", FLATPAK_ID]);
        c
    } else if dir.contains("/snap/steam/") {
        let mut c = Command::new("snap");
        c.args(["run", "steam"]);
        c
    } else if let Some(steam) = on_path("steam") {
        Command::new(steam)
    } else {
        Command::new(install.dir.join("steam.sh"))
    }
}

#[cfg(target_os = "macos")]
fn launch_command(_install: &SteamInstall) -> Command {
    let mut c = Command::new("open");
    c.args(["-a", "Steam"]);
    c
}

/// Removes what an AppImage adds to the environment (its own libraries, GTK
/// modules, data dirs) so Steam starts exactly as it would from the desktop.
fn clean_env(cmd: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR").filter(|d| !d.is_empty()) else {
        return;
    };
    let appdir = appdir.to_string_lossy().into_owned();
    for (key, value) in std::env::vars_os() {
        let value = value.to_string_lossy();
        if !value.contains(&appdir) {
            continue;
        }
        let kept: Vec<&str> = value.split(':').filter(|p| !p.is_empty() && !p.starts_with(&appdir)).collect();
        if kept.is_empty() {
            cmd.env_remove(&key);
        } else {
            cmd.env(&key, kept.join(":"));
        }
    }
    for key in ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "GDK_BACKEND"] {
        cmd.env_remove(key);
    }
}

pub fn launch(install: &SteamInstall) -> std::io::Result<()> {
    let mut cmd = launch_command(install);
    cmd.current_dir(home())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    clean_env(&mut cmd);
    // Its own session, so Steam keeps running if this app (or its terminal) closes.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    if cfg!(target_os = "macos") {
        // `open` hands Steam to macOS and exits right away; a failure means Steam.app wasn't found.
        let status = child.wait()?;
        if !status.success() {
            return Err(std::io::Error::other("Steam isn't in your Applications folder"));
        }
        return Ok(());
    }
    // Reap it when it exits so it never lingers as a zombie named "steam".
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
