//! Steam's `registry.vdf`: what the Windows registry holds, as a file, on Linux
//! and macOS. Values live under `Registry/HKCU/Software/Valve/Steam`.

use crate::vdf::{self, Obj, ParseError, Value};

pub const STEAM_KEY: &[&str] = &["Registry", "HKCU", "Software", "Valve", "Steam"];
pub const ACTIVE_PROCESS_KEY: &[&str] = &["Registry", "HKCU", "Software", "Valve", "Steam", "ActiveProcess"];

fn find<'a>(root: &'a Obj, path: &[&str]) -> Option<&'a Obj> {
    path.iter().try_fold(root, |node, name| node.get_obj(name))
}

/// Reads a value, or `None` if the file, the path or the key is missing.
pub fn get(text: &str, path: &[&str], key: &str) -> Option<String> {
    let root = vdf::parse(text).ok()?;
    find(&root, path)?.get_str(key).map(vdf::unescape)
}

/// Sets a value, creating the path if needed. `text` may be empty (Steam has
/// not written the file yet). Returns the new file contents.
pub fn set(text: &str, path: &[&str], key: &str, value: &str) -> Result<String, ParseError> {
    let mut root = vdf::parse(text)?;
    let mut node = &mut root;
    for name in path {
        if node.get_obj(name).is_none() {
            // A string where a section belongs would hide the section from Steam, so replace it.
            node.remove(name);
            node.entries.push((name.to_string(), Value::Obj(Obj::default())));
        }
        node = node.get_obj_mut(name).expect("section was just ensured");
    }
    node.set_str(key, &escape(value));
    Ok(vdf::write(&root))
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "\"Registry\"\n{\n\t\"HKLM\"\n\t{\n\t\t\"Software\"\n\t\t{\n\t\t\t\"Valve\"\n\t\t\t{\n\t\t\t\t\"Steam\"\n\t\t\t\t{\n\t\t\t\t\t\"SteamPID\"\t\t\"0\"\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"HKCU\"\n\t{\n\t\t\"Software\"\n\t\t{\n\t\t\t\"Valve\"\n\t\t\t{\n\t\t\t\t\"Steam\"\n\t\t\t\t{\n\t\t\t\t\t\"language\"\t\t\"english\"\n\t\t\t\t\t\"AutoLoginUser\"\t\t\"old_user\"\n\t\t\t\t\t\"RememberPassword\"\t\t\"1\"\n\t\t\t\t\t\"ActiveProcess\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"pid\"\t\t\"4242\"\n\t\t\t\t\t\t\"ActiveUser\"\t\t\"22202\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn reads_auto_login_and_active_user() {
        assert_eq!(get(LINUX, STEAM_KEY, "AutoLoginUser").as_deref(), Some("old_user"));
        assert_eq!(get(LINUX, ACTIVE_PROCESS_KEY, "ActiveUser").as_deref(), Some("22202"));
        assert_eq!(get(LINUX, STEAM_KEY, "Missing"), None);
        assert_eq!(get("", STEAM_KEY, "AutoLoginUser"), None);
        assert_eq!(get("not { valid", STEAM_KEY, "AutoLoginUser"), None);
    }

    #[test]
    fn set_changes_only_that_value() {
        let out = set(LINUX, STEAM_KEY, "AutoLoginUser", "new_user").unwrap();
        assert_eq!(out, LINUX.replace("\"old_user\"", "\"new_user\""));
    }

    #[test]
    fn set_is_case_insensitive_on_the_path() {
        let text = LINUX.replace("\"Valve\"", "\"valve\"");
        let out = set(&text, STEAM_KEY, "autologinuser", "x").unwrap();
        assert_eq!(get(&out, STEAM_KEY, "AutoLoginUser").as_deref(), Some("x"));
        assert_eq!(out.matches("alve\"").count(), 2, "no duplicate Valve section");
    }

    #[test]
    fn set_creates_missing_file_and_sections() {
        let out = set("", STEAM_KEY, "AutoLoginUser", "first").unwrap();
        assert_eq!(get(&out, STEAM_KEY, "AutoLoginUser").as_deref(), Some("first"));
        let out = set(&out, STEAM_KEY, "RememberPassword", "1").unwrap();
        assert_eq!(get(&out, STEAM_KEY, "AutoLoginUser").as_deref(), Some("first"));
        assert!(out.starts_with("\"Registry\"\n{\n\t\"HKCU\""));
    }

    #[test]
    fn set_escapes_values_and_rejects_broken_files() {
        let out = set(LINUX, STEAM_KEY, "AutoLoginUser", "we\"ird\\").unwrap();
        assert_eq!(get(&out, STEAM_KEY, "AutoLoginUser").as_deref(), Some("we\"ird\\"));
        assert!(set("\"Registry\" {", STEAM_KEY, "AutoLoginUser", "x").is_err());
    }
}
