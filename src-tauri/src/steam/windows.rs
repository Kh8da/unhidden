//! Windows: Steam is found and pointed at an account through the registry.

use super::SteamInstall;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const STEAM_KEY: &str = r"Software\Valve\Steam";

/// The Steam client itself.
pub const MAIN: &[&str] = &["steam.exe"];
/// Everything that belongs to the Steam client (not the system service).
pub const CLIENT: &[&str] = &["steam.exe", "steamwebhelper.exe", "GameOverlayUI.exe"];

fn read_string(root: winreg::HKEY, path: &str, name: &str) -> Option<String> {
    RegKey::predef(root)
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?
        .get_value::<String, _>(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = read_string(HKEY_CURRENT_USER, STEAM_KEY, "SteamPath") {
        out.push(PathBuf::from(p.replace('/', "\\")));
    }
    if let Some(exe) = read_string(HKEY_CURRENT_USER, STEAM_KEY, "SteamExe") {
        if let Some(parent) = PathBuf::from(exe.replace('/', "\\")).parent() {
            out.push(parent.to_path_buf());
        }
    }
    for key in [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"] {
        if let Some(p) = read_string(HKEY_LOCAL_MACHINE, key, "InstallPath") {
            out.push(PathBuf::from(p));
        }
    }
    out.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    out.push(PathBuf::from(r"C:\Program Files\Steam"));
    out
}

pub fn is_steam_dir(dir: &Path) -> bool {
    dir.join("steam.exe").is_file()
}

pub fn normalize(dir: PathBuf) -> PathBuf {
    dir
}

pub fn in_game_mode() -> bool {
    false
}

/// The account id Steam is signed in as right now, if it is running.
pub fn active_account_id(_install: &SteamInstall) -> Option<u32> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(format!(r"{STEAM_KEY}\ActiveProcess"), KEY_READ)
        .ok()?;
    let id: u32 = key.get_value("ActiveUser").ok()?;
    (id != 0).then_some(id)
}

pub fn auto_login_user(_install: &SteamInstall) -> Option<String> {
    read_string(HKEY_CURRENT_USER, STEAM_KEY, "AutoLoginUser")
}

pub fn check_auto_login(_install: &SteamInstall) -> Result<(), String> {
    Ok(())
}

pub fn set_auto_login_user(_install: &SteamInstall, account_name: &str) -> Result<(), String> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey_with_flags(STEAM_KEY, KEY_READ | KEY_WRITE)
        .map_err(|e| format!("couldn't open steam's registry key: {e}"))?;
    key.set_value("AutoLoginUser", &account_name)
        .map_err(|e| format!("couldn't set the auto-login account: {e}"))?;
    if !account_name.is_empty() {
        key.set_value("RememberPassword", &1u32)
            .map_err(|e| format!("couldn't set remember password: {e}"))?;
    }
    Ok(())
}

pub mod process {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    /// Process ids whose executable name matches one of `names` (case-insensitive).
    pub fn find(names: &[&str]) -> Vec<u32> {
        let mut pids = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return pids;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    pids.push(entry.th32ProcessID);
                }
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        pids
    }

    pub fn kill(pid: u32) {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
}

fn spawn_steam(install: &SteamInstall, args: &[&str]) -> std::io::Result<()> {
    Command::new(install.dir.join("steam.exe"))
        .args(args)
        .current_dir(&install.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Asks the running client to save and exit.
pub fn request_quit(install: &SteamInstall) {
    let _ = spawn_steam(install, &["-shutdown"]);
}

pub fn launch(install: &SteamInstall) -> std::io::Result<()> {
    spawn_steam(install, &[])
}
