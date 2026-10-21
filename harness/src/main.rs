//! End-to-end test of unhidden's switching code (src-tauri/src/steam and
//! actions.rs) against `fake-steam`, on whatever OS this runs on.
//!
//! Linux runs in a throwaway home folder. Windows and macOS have to use the real
//! Steam registry key / app folder, so they refuse to run without `--ci`, which
//! is only meant for a throwaway machine such as a CI runner.

#[path = "../../src-tauri/src/steam/mod.rs"]
mod steam;
#[path = "../../src-tauri/src/actions.rs"]
mod actions;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};
use steamcore::loginusers::{self, read_users, LoginUser};

const BASE: u64 = 76561198000000000;
const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"AutoUpdateWindowEnabled\"\t\t\"0\"\n\t\t\t\t\"AlwaysShowUserChooser\"\t\t\"CHOOSER\"\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

static mut FAILS: u32 = 0;

fn check(ok: bool, msg: &str) {
    println!("{}  {}", if ok { "PASS" } else { "FAIL" }, msg);
    if !ok {
        unsafe { FAILS += 1 }
    }
}

/// Where the fake Steam lives and how to read back what unhidden wrote.
struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn vdf(&self) -> PathBuf {
        self.dir.join("config").join("loginusers.vdf")
    }
    fn config(&self) -> PathBuf {
        self.dir.join("config").join("config.vdf")
    }
    fn users(&self) -> Vec<LoginUser> {
        read_users(&fs::read_to_string(self.vdf()).unwrap()).unwrap()
    }
    fn user(&self, id: u64) -> Option<LoginUser> {
        self.users().into_iter().find(|u| u.steam_id == id)
    }
    fn raw(&self, id: u64, key: &str) -> Option<String> {
        let root = steamcore::vdf::parse(&fs::read_to_string(self.vdf()).unwrap()).unwrap();
        root.get_obj("users")?.get_obj(&id.to_string())?.get_str(key).map(str::to_string)
    }
    fn launches(&self) -> usize {
        fs::read_to_string(self.dir.join("launches.log")).unwrap_or_default().lines().count()
    }
    fn wait_launches(&self, n: usize) -> bool {
        let t = Instant::now();
        while self.launches() < n && t.elapsed() < Duration::from_secs(10) {
            sleep(Duration::from_millis(50));
        }
        self.launches() == n
    }
    fn install(&self) -> steam::SteamInstall {
        steam::detect(None).expect("steam detected")
    }
    fn switch(&self, id: u64, offline: bool) -> (Result<(), String>, Vec<String>, Duration) {
        let stages = RefCell::new(Vec::new());
        let t = Instant::now();
        let r = actions::switch(&self.install(), id, offline, &|s| stages.borrow_mut().push(s.to_string()));
        (r, stages.into_inner(), t.elapsed())
    }
    fn write_accounts(&self) {
        let mut text = String::from("\"users\"\n{\n");
        for i in 1..=12u64 {
            let on = u8::from(i == 1);
            text.push_str(&format!(
                "\t\"{}\"\n\t{{\n\t\t\"AccountName\"\t\t\"acc{i}\"\n\t\t\"PersonaName\"\t\t\"Persona {i}\"\n\t\t\"RememberPassword\"\t\t\"1\"\n\t\t\"WantsOfflineMode\"\t\t\"0\"\n\t\t\"SkipOfflineModeWarning\"\t\t\"0\"\n\t\t\"AllowAutoLogin\"\t\t\"{on}\"\n\t\t\"MostRecent\"\t\t\"{on}\"\n\t\t\"AutoLogin\"\t\t\"{on}\"\n\t\t\"Timestamp\"\t\t\"{}\"\n\t}}\n",
                BASE + i,
                1_700_000_000 + i
            ));
        }
        text.push_str("}\n");
        fs::create_dir_all(self.dir.join("config").join("avatarcache")).unwrap();
        fs::write(self.vdf(), text).unwrap();
        fs::write(self.config(), CONFIG.replace("CHOOSER", "1")).unwrap();
        fs::write(self.dir.join(format!("config/avatarcache/{}.png", BASE + 2)), [0x89, b'P', b'N', b'G']).unwrap();
        for f in ["launches.log", "stubborn.flag", "shutdown.flag", "env.log"] {
            let _ = fs::remove_file(self.dir.join(f));
        }
    }
}

fn wait_running(want: bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if steam::is_running() == want {
            return true;
        }
        sleep(Duration::from_millis(50));
    }
    false
}

/// The fake-steam binary built next to this one.
fn fake_steam_binary() -> PathBuf {
    let me = std::env::current_exe().unwrap();
    me.with_file_name(format!("fake-steam{}", std::env::consts::EXE_SUFFIX))
}

#[cfg(windows)]
mod platform {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    fn key(path: &str) -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER).create_subkey(path).unwrap().0
    }

    pub fn setup(_ci: bool) -> Fake {
        let dir = std::env::temp_dir().join("unhidden-harness").join("Steam");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::copy(fake_steam_binary(), dir.join("steam.exe")).unwrap();
        let k = key(r"Software\Valve\Steam");
        // Steam itself stores this path with forward slashes.
        k.set_value("SteamPath", &dir.to_string_lossy().replace('\\', "/")).unwrap();
        k.set_value("AutoLoginUser", &"acc1").unwrap();
        k.set_value("RememberPassword", &0u32).unwrap();
        Fake { dir }
    }

    pub fn remember_password(_: &Fake) -> Option<String> {
        key(r"Software\Valve\Steam").get_value::<u32, _>("RememberPassword").ok().map(|v| v.to_string())
    }

    pub fn set_active_user(_: &Fake, id: u32) {
        key(r"Software\Valve\Steam\ActiveProcess").set_value("ActiveUser", &id).unwrap();
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use steamcore::registry::{self, ACTIVE_PROCESS_KEY, STEAM_KEY};

    pub fn registry_vdf(fake: &Fake) -> PathBuf {
        if cfg!(target_os = "macos") {
            fake.dir.join("registry.vdf")
        } else {
            PathBuf::from(std::env::var_os("HOME").unwrap()).join(".steam/registry.vdf")
        }
    }

    fn set(fake: &Fake, path: &[&str], key: &str, value: &str) {
        let text = fs::read_to_string(registry_vdf(fake)).unwrap_or_default();
        fs::create_dir_all(registry_vdf(fake).parent().unwrap()).unwrap();
        fs::write(registry_vdf(fake), registry::set(&text, path, key, value).unwrap()).unwrap();
    }

    pub fn get(fake: &Fake, key: &str) -> Option<String> {
        registry::get(&fs::read_to_string(registry_vdf(fake)).unwrap_or_default(), STEAM_KEY, key)
    }

    pub fn remember_password(fake: &Fake) -> Option<String> {
        get(fake, "RememberPassword")
    }

    pub fn set_active_user(fake: &Fake, id: u32) {
        set(fake, ACTIVE_PROCESS_KEY, "ActiveUser", &id.to_string());
    }

    #[cfg(target_os = "linux")]
    pub fn setup(_ci: bool) -> Fake {
        // A throwaway home: nothing outside it is touched.
        let root = std::env::temp_dir().join("unhidden-harness");
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let bin = root.join("bin");
        fs::create_dir_all(home.join(".local/share/Steam")).unwrap();
        fs::create_dir_all(home.join(".steam")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink("../.local/share/Steam", home.join(".steam/steam")).unwrap();
        fs::copy(fake_steam_binary(), bin.join("steam")).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        for k in ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "XDG_SESSION_DESKTOP", "GAMESCOPE_WAYLAND_DISPLAY", "APPDIR"] {
            std::env::remove_var(k);
        }
        let fake = Fake { dir: fs::canonicalize(home.join(".local/share/Steam")).unwrap() };
        set(&fake, STEAM_KEY, "AutoLoginUser", "acc1");
        set(&fake, STEAM_KEY, "language", "english");
        fake
    }

    #[cfg(target_os = "macos")]
    pub fn setup(ci: bool) -> Fake {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let dir = home.join("Library/Application Support/Steam");
        let app = PathBuf::from("/Applications/Steam.app");
        if !ci && (dir.exists() || app.exists()) {
            eprintln!("refusing to run: this Mac has Steam. use --ci only on a throwaway machine.");
            std::process::exit(2);
        }
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&app);
        fs::create_dir_all(&dir).unwrap();
        // `open -a Steam` needs a real app bundle that macOS knows about.
        let macos = app.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::copy(fake_steam_binary(), macos.join("steam_osx")).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>CFBundleExecutable</key><string>steam_osx</string>\n<key>CFBundleIdentifier</key><string>com.valvesoftware.steam.harness</string>\n<key>CFBundleName</key><string>Steam</string>\n<key>CFBundlePackageType</key><string>APPL</string>\n<key>LSBackgroundOnly</key><true/>\n</dict></plist>\n",
        )
        .unwrap();
        let _ = std::process::Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
            .args(["-f", "/Applications/Steam.app"])
            .status();
        let fake = Fake { dir: fs::canonicalize(&dir).unwrap() };
        set(&fake, STEAM_KEY, "AutoLoginUser", "acc1");
        set(&fake, STEAM_KEY, "language", "english");
        fake
    }
}

fn main() {
    let ci = std::env::args().any(|a| a == "--ci");
    if !cfg!(target_os = "linux") && !ci {
        eprintln!("this changes the real Steam settings on {}; run it only on a throwaway machine with --ci.", std::env::consts::OS);
        std::process::exit(2);
    }
    println!("unhidden harness on {} ({})\n", std::env::consts::OS, std::env::consts::ARCH);
    let fake = platform::setup(ci);
    fake.write_accounts();

    // --- detection ---
    let install = steam::detect(None);
    check(install.as_ref().is_some_and(|i| i.dir == fake.dir || fs::canonicalize(&i.dir).ok() == Some(fake.dir.clone())), &format!("finds steam: {:?}", install.as_ref().map(|i| &i.dir)));
    let install = fake.install();
    check(install.avatar(BASE + 2).is_some() && install.avatar(BASE + 3).is_none(), "finds cached avatars");
    check(steam::valid_dir(&fake.dir.to_string_lossy()) && !steam::valid_dir(&std::env::temp_dir().join("nope").to_string_lossy()), "validates picked folders");
    check(steam::auto_login_user(&install).as_deref() == Some("acc1"), "reads the auto-login account");
    check(steam::blocker().is_none(), "not blocked on a normal desktop");

    // --- launch / running ---
    check(!steam::is_running(), "steam not running at start");
    if cfg!(target_os = "linux") {
        std::env::set_var("APPDIR", "/tmp/.mount_unhiddenXYZ");
        std::env::set_var("LD_LIBRARY_PATH", "/tmp/.mount_unhiddenXYZ/usr/lib:/opt/keep/lib");
        std::env::set_var("GDK_BACKEND", "x11");
    }
    let r = steam::launch(&install);
    check(r.is_ok() && wait_running(true) && fake.wait_launches(1), &format!("launch() starts steam: {r:?}"));
    if cfg!(target_os = "linux") {
        let leaked = fs::read_to_string(fake.dir.join("env.log")).unwrap_or_default();
        check(leaked == "LD_LIBRARY_PATH=/opt/keep/lib", &format!("AppImage's own environment isn't passed to steam (got {leaked:?})"));
        for k in ["APPDIR", "LD_LIBRARY_PATH", "GDK_BACKEND"] {
            std::env::remove_var(k);
        }
    }

    // --- switch to a 'hidden' account (#10) ---
    let (r, stages, took) = fake.switch(BASE + 10, false);
    check(r.is_ok(), &format!("switch to acc10 succeeds: {r:?}"));
    check(stages == ["closing", "preparing", "launching"], &format!("progress stages {stages:?}"));
    check(took < Duration::from_secs(8), &format!("polite shutdown path was used ({took:?})"));
    check(steam::auto_login_user(&install).as_deref() == Some("acc10"), "auto-login = acc10, written after steam saved its own");
    check(platform::remember_password(&fake).as_deref() == Some("1"), "RememberPassword = 1");
    let u10 = fake.user(BASE + 10).unwrap();
    check(u10.most_recent && u10.remember_password && !u10.wants_offline, "acc10 is MostRecent in loginusers.vdf");
    check(fake.raw(BASE + 10, "AutoLogin").as_deref() == Some("1") && fake.raw(BASE + 1, "AutoLogin").as_deref() == Some("0"), "AutoLogin moved from acc1 to acc10");
    check(u10.timestamp > 1_750_000_000, "acc10's last sign-in time set to now");
    check(fake.users().iter().filter(|u| u.most_recent).count() == 1, "exactly one MostRecent account");
    check(fake.user(BASE + 1).unwrap().persona_name == "renamed-on-exit", "edits the file steam wrote on exit, not a stale copy");
    check(fake.users().len() == 12, "all 12 accounts kept");
    check(fs::read_to_string(fake.config()).unwrap() == CONFIG.replace("CHOOSER", "0"), "\"ask which account\" turned off, rest of config.vdf unchanged");
    for f in ["loginusers.vdf", "config.vdf"] {
        check(fake.dir.join("config").join(format!("{f}.unhidden-backup")).is_file(), &format!("{f} backed up"));
    }
    #[cfg(unix)]
    {
        check(platform::get(&fake, "language").as_deref() == Some("english"), "other registry.vdf values kept");
        check(platform::registry_vdf(&fake).with_extension("vdf.unhidden-backup").is_file(), "registry.vdf backed up");
    }
    check(!fake.dir.join("config/loginusers.vdf.unhidden-tmp").exists(), "no temp file left behind");
    check(wait_running(true) && fake.wait_launches(2), &format!("steam relaunched (launches={})", fake.launches()));

    // --- signed-in detection ---
    platform::set_active_user(&fake, loginusers::account_id(BASE + 10));
    check(steam::active_account_id(&install) == Some(loginusers::account_id(BASE + 10)), "reads the signed-in account");
    platform::set_active_user(&fake, 0);
    check(steam::active_account_id(&install).is_none(), "ActiveUser 0 means nobody signed in");

    // --- unknown account: must fail without closing steam ---
    let before = fake.launches();
    let (r, stages, _) = fake.switch(BASE + 99, false);
    check(r.is_err() && stages.is_empty(), "unknown account fails before touching steam");
    check(steam::is_running() && fake.launches() == before, "steam left running after that error");

    // --- Steam Deck game mode ---
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("XDG_CURRENT_DESKTOP", "gamescope");
        let (r, stages, _) = fake.switch(BASE + 4, false);
        check(r.as_ref().is_err_and(|e| e.contains("desktop mode")) && stages.is_empty() && steam::is_running(), &format!("steam deck game mode is refused with a clear message: {r:?}"));
        std::env::remove_var("XDG_CURRENT_DESKTOP");
        std::env::set_var("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0");
        check(steam::blocker().is_some(), "GAMESCOPE_WAYLAND_DISPLAY also means game mode");
        std::env::remove_var("GAMESCOPE_WAYLAND_DISPLAY");
    }

    // --- read-only loginusers.vdf ---
    let mut perms = fs::metadata(fake.vdf()).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(fake.vdf(), perms).unwrap();
    let (r, _, _) = fake.switch(BASE + 4, false);
    check(r.is_ok() && fake.user(BASE + 4).unwrap().most_recent, &format!("works even if loginusers.vdf was made read-only: {r:?}"));
    let backup = fake.dir.join("config").join("loginusers.vdf.unhidden-backup");
    check(
        !fs::metadata(&backup).unwrap().permissions().readonly() && !fs::metadata(fake.vdf()).unwrap().permissions().readonly(),
        "neither the file nor its backup is left read-only",
    );
    let (r, _, _) = fake.switch(BASE + 7, false);
    check(r.is_ok() && fake.user(BASE + 7).unwrap().most_recent, &format!("and the next switch works too: {r:?}"));
    check(fs::read_to_string(fake.config()).unwrap() == CONFIG.replace("CHOOSER", "0"), "picker turned off again after steam saved it as on at exit");
    check(wait_running(true), "steam running again");

    // --- a steam that ignores the polite request is closed by force ---
    fs::write(fake.dir.join("stubborn.flag"), "").unwrap();
    let (r, _, took) = fake.switch(BASE + 3, true);
    let _ = fs::remove_file(fake.dir.join("stubborn.flag"));
    let _ = fs::remove_file(fake.dir.join("shutdown.flag"));
    check(r.is_ok(), &format!("hung steam is force-closed and switch succeeds ({took:?}): {r:?}"));
    let u3 = fake.user(BASE + 3).unwrap();
    check(u3.most_recent && u3.wants_offline, "offline switch sets WantsOfflineMode");
    check(steam::auto_login_user(&install).as_deref() == Some("acc3"), "auto-login points at acc3");
    check(wait_running(true), "steam running after forced switch");

    // --- forget ---
    let r = actions::forget(&install, BASE + 5, &|_| {});
    check(r.is_ok() && fake.user(BASE + 5).is_none() && fake.users().len() == 11, &format!("forget removes acc5: {r:?}"));
    check(wait_running(true), "steam restarted after forget");
    check(steam::auto_login_user(&install).as_deref() == Some("acc3"), "forgetting another account keeps the auto-login account");
    let r = actions::forget(&install, BASE + 3, &|_| {});
    check(r.is_ok() && steam::auto_login_user(&install).is_none(), &format!("forgetting the auto-login account clears it: {r:?}"));

    // --- with steam closed ---
    let r = steam::shutdown(&install);
    check(r.is_ok() && !steam::is_running(), &format!("shutdown() closes steam: {r:?}"));
    let before = fake.launches();
    let r = actions::forget(&install, BASE + 6, &|_| {});
    check(r.is_ok() && !steam::is_running() && fake.launches() == before, "forget with steam closed leaves it closed");
    let (r, _, _) = fake.switch(BASE + 12, false);
    check(r.is_ok() && wait_running(true) && fake.user(BASE + 12).unwrap().most_recent, &format!("switch with steam closed starts it: {r:?}"));

    // --- registry.vdf that steam hasn't written yet is created ---
    #[cfg(unix)]
    {
        steam::shutdown(&install).unwrap();
        fs::remove_file(platform::registry_vdf(&fake)).unwrap();
        let (r, _, _) = fake.switch(BASE + 11, false);
        check(r.is_ok() && steam::auto_login_user(&install).as_deref() == Some("acc11"), &format!("creates registry.vdf if steam hasn't yet: {r:?}"));
        check(wait_running(true), "steam running after that");
    }

    // --- unreadable files are refused before closing steam ---
    #[allow(unused_mut)] // only Unix adds registry.vdf
    let mut files = vec![(fake.config(), "config.vdf"), (fake.vdf(), "loginusers.vdf")];
    #[cfg(unix)]
    files.push((platform::registry_vdf(&fake), "registry.vdf"));
    for (path, name) in files {
        let Ok(good) = fs::read(&path) else {
            check(false, &format!("{name} exists for the invalid-file check"));
            continue;
        };
        fs::write(&path, [0xFF, 0xFE, b'x']).unwrap();
        let before = fake.launches();
        let (r, stages, _) = fake.switch(BASE + 2, false);
        check(r.is_err() && stages.is_empty() && steam::is_running() && fake.launches() == before, &format!("invalid {name} is refused without closing steam: {r:?}"));
        check(fs::read(&path).unwrap() == [0xFF, 0xFE, b'x'], &format!("invalid {name} left exactly as it was"));
        fs::write(&path, good).unwrap();
    }

    // --- Flatpak and Snap keep registry.vdf in their own home ---
    #[cfg(target_os = "linux")]
    {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        for (dir, want) in [
            (".var/app/com.valvesoftware.Steam/.local/share/Steam", ".var/app/com.valvesoftware.Steam/.steam/registry.vdf"),
            ("snap/steam/common/.local/share/Steam", "snap/steam/common/.steam/registry.vdf"),
            (".steam/debian-installation", ".steam/registry.vdf"),
        ] {
            let inst = steam::SteamInstall { dir: home.join(dir) };
            let r = steam::set_auto_login_user(&inst, "layout_test");
            let got = steamcore::registry::get(&fs::read_to_string(home.join(want)).unwrap_or_default(), steamcore::registry::STEAM_KEY, "AutoLoginUser");
            check(r.is_ok() && got.as_deref() == Some("layout_test"), &format!("{dir} keeps auto-login in {want}"));
        }
    }

    // --- nothing left behind ---
    let r = steam::shutdown(&install);
    sleep(Duration::from_millis(300));
    check(r.is_ok() && !steam::is_running(), "no leftover steam processes (no zombies)");
    check(Path::new(&fake.dir).is_dir(), "steam folder intact");

    let fails = unsafe { FAILS };
    println!("{}", if fails == 0 { "\nALL PASSED".to_string() } else { format!("\n{fails} FAILED") });
    std::process::exit(fails as i32);
}
