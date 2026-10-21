//! A stand-in for the Steam client, used only by the harness. It behaves like
//! the real one where unhidden depends on it: it quits when asked (`-shutdown`
//! on Windows, SIGTERM elsewhere) unless `stubborn.flag` exists, and on the way
//! out it saves its files, including the account it was signed in to.

use std::path::PathBuf;
use std::{env, fs, thread, time::Duration};

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn data_dir() -> PathBuf {
    if cfg!(windows) {
        env::current_exe().unwrap().parent().unwrap().to_path_buf()
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Steam")
    } else {
        home().join(".local/share/Steam")
    }
}

#[cfg(windows)]
mod auto_login {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    fn key() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Software\Valve\Steam").unwrap().0
    }
    pub fn get() -> String {
        key().get_value("AutoLoginUser").unwrap_or_default()
    }
    pub fn set(name: &str) {
        key().set_value("AutoLoginUser", &name).unwrap();
    }
}

#[cfg(unix)]
mod auto_login {
    use steamcore::registry::{self, STEAM_KEY};

    fn path() -> std::path::PathBuf {
        if cfg!(target_os = "macos") {
            super::data_dir().join("registry.vdf")
        } else {
            super::home().join(".steam/registry.vdf")
        }
    }
    pub fn get() -> String {
        registry::get(&std::fs::read_to_string(path()).unwrap_or_default(), STEAM_KEY, "AutoLoginUser").unwrap_or_default()
    }
    pub fn set(name: &str) {
        if let Ok(text) = std::fs::read_to_string(path()) {
            if let Ok(out) = registry::set(&text, STEAM_KEY, "AutoLoginUser", name) {
                std::fs::write(path(), out).unwrap();
            }
        }
    }
}

#[cfg(unix)]
mod quit {
    use std::sync::atomic::{AtomicBool, Ordering};
    static TERM: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_term(_: libc::c_int) {
        TERM.store(true, Ordering::SeqCst);
    }
    pub fn listen() {
        unsafe {
            libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        }
    }
    pub fn requested() -> bool {
        TERM.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
mod quit {
    pub fn listen() {}
    pub fn requested() -> bool {
        super::data_dir().join("shutdown.flag").exists()
    }
}

fn main() {
    let dir = data_dir();
    if cfg!(windows) && env::args().any(|a| a == "-shutdown") {
        // A second steam.exe forwards the request to the running one.
        fs::write(dir.join("shutdown.flag"), "").unwrap();
        return;
    }
    let log = dir.join("launches.log");
    let prev = fs::read_to_string(&log).unwrap_or_default();
    fs::write(&log, prev + "launch\n").unwrap();
    let leaked: Vec<String> = ["APPDIR", "APPIMAGE", "LD_LIBRARY_PATH", "GDK_BACKEND"]
        .iter()
        .filter_map(|k| env::var(k).ok().map(|v| format!("{k}={v}")))
        .collect();
    fs::write(dir.join("env.log"), leaked.join("\n")).unwrap();

    let signed_in = auto_login::get();
    quit::listen();
    loop {
        thread::sleep(Duration::from_millis(100));
        if !quit::requested() || dir.join("stubborn.flag").exists() {
            continue;
        }
        let _ = fs::remove_file(dir.join("shutdown.flag"));
        let p = dir.join("config/loginusers.vdf");
        if let Ok(s) = fs::read_to_string(&p) {
            fs::write(&p, s.replace("\"Persona 1\"", "\"renamed-on-exit\"")).unwrap();
        }
        let c = dir.join("config/config.vdf");
        if let Ok(s) = fs::read_to_string(&c) {
            fs::write(&c, s.replace("\"AlwaysShowUserChooser\"\t\t\"0\"", "\"AlwaysShowUserChooser\"\t\t\"1\"")).unwrap();
        }
        auto_login::set(&signed_in);
        return;
    }
}
