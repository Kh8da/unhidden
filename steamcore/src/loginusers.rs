//! Reading and editing Steam's `config/loginusers.vdf`, the list of every
//! account that has signed in on this PC (including the ones Steam's account
//! picker doesn't show).

use crate::vdf::{self, Obj, ParseError, Value};
use std::fmt;

/// SteamID64 of account id 0 in the public universe.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginUser {
    pub steam_id: u64,
    pub account_name: String,
    pub persona_name: String,
    pub timestamp: u64,
    pub most_recent: bool,
    pub remember_password: bool,
    pub wants_offline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Parse(ParseError),
    MissingUsersBlock,
    UserNotFound(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(e) => write!(f, "loginusers.vdf could not be read: {e}"),
            Error::MissingUsersBlock => write!(f, "loginusers.vdf has no \"users\" section"),
            Error::UserNotFound(id) => write!(f, "account {id} is not in loginusers.vdf"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Error::Parse(e)
    }
}

/// The 32-bit account id Steam uses in the registry (`ActiveProcess\ActiveUser`).
pub fn account_id(steam_id: u64) -> u32 {
    steam_id.wrapping_sub(STEAMID64_BASE) as u32
}

fn flag(o: &Obj, key: &str) -> bool {
    o.get_str(key).map(|v| v.trim() == "1").unwrap_or(false)
}

fn users_block(root: &mut Obj) -> Result<&mut Obj, Error> {
    root.get_obj_mut("users").ok_or(Error::MissingUsersBlock)
}

pub fn read_users(text: &str) -> Result<Vec<LoginUser>, Error> {
    let root = vdf::parse(text)?;
    let Some(users) = root.get_obj("users") else {
        // A fresh Steam install may have an empty or missing file.
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (key, value) in &users.entries {
        let (Ok(steam_id), Value::Obj(o)) = (key.trim().parse::<u64>(), value) else {
            continue;
        };
        let account_name = vdf::unescape(o.get_str("AccountName").unwrap_or_default());
        if account_name.is_empty() {
            continue;
        }
        out.push(LoginUser {
            steam_id,
            persona_name: vdf::unescape(o.get_str("PersonaName").unwrap_or_default()),
            account_name,
            timestamp: o
                .get_str("Timestamp")
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0),
            most_recent: flag(o, "MostRecent"),
            remember_password: flag(o, "RememberPassword"),
            wants_offline: flag(o, "WantsOfflineMode"),
        });
    }
    Ok(out)
}

/// Marks `steam_id` as the account Steam should sign in to on next launch.
/// `now` (unix seconds) becomes its last sign-in time, which also puts it among
/// the accounts Steam's own picker shows. Returns the new file contents.
pub fn select_user(text: &str, steam_id: u64, offline: bool, now: u64) -> Result<String, Error> {
    let mut root = vdf::parse(text)?;
    let users = users_block(&mut root)?;
    let target = steam_id.to_string();
    let mut found = false;
    for (key, value) in users.entries.iter_mut() {
        let Value::Obj(o) = value else { continue };
        if key.trim() == target {
            found = true;
            let off = if offline { "1" } else { "0" };
            o.set_str("MostRecent", "1");
            // Current Steam reads `AutoLogin`; older versions read `AllowAutoLogin`.
            o.set_str("AutoLogin", "1");
            o.set_str("AllowAutoLogin", "1");
            o.set_str("RememberPassword", "1");
            o.set_str("WantsOfflineMode", off);
            o.set_str("SkipOfflineModeWarning", off);
            o.set_str("Timestamp", &now.to_string());
        } else {
            for marker in ["MostRecent", "AutoLogin"] {
                if o.get(marker).is_some() {
                    o.set_str(marker, "0");
                }
            }
        }
    }
    if !found {
        return Err(Error::UserNotFound(steam_id));
    }
    Ok(vdf::write(&root))
}

/// Removes `steam_id` from the file. Returns the new file contents.
pub fn remove_user(text: &str, steam_id: u64) -> Result<String, Error> {
    let mut root = vdf::parse(text)?;
    let users = users_block(&mut root)?;
    if !users.remove(&steam_id.to_string()) {
        return Err(Error::UserNotFound(steam_id));
    }
    Ok(vdf::write(&root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    const SAMPLE: &str = r#""users"
{
	"76561198000000001"
	{
		"AccountName"		"alpha"
		"PersonaName"		"Alpha \"A\""
		"RememberPassword"		"1"
		"WantsOfflineMode"		"0"
		"SkipOfflineModeWarning"		"0"
		"AllowAutoLogin"		"1"
		"MostRecent"		"1"
		"AutoLogin"		"1"
		"Timestamp"		"1700000000"
	}
	"76561198000000002"
	{
		"AccountName"		"bravo"
		"PersonaName"		"Bravo"
		"RememberPassword"		"1"
		"WantsOfflineMode"		"0"
		"SkipOfflineModeWarning"		"0"
		"AllowAutoLogin"		"0"
		"MostRecent"		"0"
		"AutoLogin"		"0"
		"Timestamp"		"1600000000"
	}
	"76561198000000003"
	{
		"AccountName"		"charlie"
		"PersonaName"		"Charlie"
		"Timestamp"		"1500000000"
	}
}
"#;

    #[test]
    fn reads_all_users() {
        let users = read_users(SAMPLE).unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].account_name, "alpha");
        assert_eq!(users[0].persona_name, "Alpha \"A\"");
        assert!(users[0].most_recent);
        assert_eq!(users[1].timestamp, 1_600_000_000);
        assert!(!users[2].most_recent && !users[2].remember_password);
    }

    #[test]
    fn reads_more_than_eight_accounts() {
        let mut text = String::from("\"users\"\n{\n");
        for i in 0..25u64 {
            text.push_str(&format!(
                "\t\"{}\"\n\t{{\n\t\t\"AccountName\"\t\t\"acc{i}\"\n\t}}\n",
                76561198000000000u64 + i
            ));
        }
        text.push_str("}\n");
        assert_eq!(read_users(&text).unwrap().len(), 25);
    }

    #[test]
    fn select_moves_most_recent_and_keeps_everything_else() {
        let out = select_user(SAMPLE, 76561198000000002, false, NOW).unwrap();
        let users = read_users(&out).unwrap();
        assert!(!users[0].most_recent);
        assert!(users[1].most_recent && users[1].remember_password && !users[1].wants_offline);
        // Charlie never had a MostRecent key, so none is added.
        assert!(!out.contains("\"charlie\"\n\t\t\"MostRecent\""));
        // Escaped persona name survives untouched.
        assert!(out.contains(r#""Alpha \"A\"""#));
        let root = vdf::parse(&out).unwrap();
        let users_obj = root.get_obj("users").unwrap();
        let bravo = users_obj.get_obj("76561198000000002").unwrap();
        assert_eq!(bravo.get_str("AllowAutoLogin"), Some("1"));
        assert_eq!(bravo.get_str("AutoLogin"), Some("1"));
        assert_eq!(bravo.get_str("Timestamp"), Some("1800000000"));
        // The previous account loses both "sign in to me" markers but keeps its own settings.
        let alpha = users_obj.get_obj("76561198000000001").unwrap();
        assert_eq!(alpha.get_str("AutoLogin"), Some("0"));
        assert_eq!(alpha.get_str("MostRecent"), Some("0"));
        assert_eq!(alpha.get_str("AllowAutoLogin"), Some("1"));
        assert_eq!(alpha.get_str("RememberPassword"), Some("1"));
        assert_eq!(alpha.get_str("Timestamp"), Some("1700000000"));
        // Charlie had no AutoLogin key, so none is added.
        assert!(users_obj.get_obj("76561198000000003").unwrap().get("AutoLogin").is_none());
    }

    #[test]
    fn select_adds_missing_keys_and_offline_flags() {
        let out = select_user(SAMPLE, 76561198000000003, true, NOW).unwrap();
        let root = vdf::parse(&out).unwrap();
        let c = root.get_obj("users").unwrap().get_obj("76561198000000003").unwrap();
        for key in ["MostRecent", "AutoLogin", "AllowAutoLogin", "RememberPassword", "WantsOfflineMode", "SkipOfflineModeWarning"] {
            assert_eq!(c.get_str(key), Some("1"), "{key}");
        }
        assert!(!read_users(&out).unwrap()[0].most_recent);
    }

    #[test]
    fn select_then_back_online_clears_offline() {
        let off = select_user(SAMPLE, 76561198000000001, true, NOW).unwrap();
        let on = select_user(&off, 76561198000000001, false, NOW).unwrap();
        assert!(!read_users(&on).unwrap()[0].wants_offline);
    }

    #[test]
    fn select_unknown_user_fails_without_changes() {
        assert_eq!(
            select_user(SAMPLE, 1, false, NOW),
            Err(Error::UserNotFound(1))
        );
    }

    #[test]
    fn remove_user_drops_only_that_account() {
        let out = remove_user(SAMPLE, 76561198000000002).unwrap();
        let names: Vec<_> = read_users(&out).unwrap().into_iter().map(|u| u.account_name).collect();
        assert_eq!(names, ["alpha", "charlie"]);
        assert!(remove_user(&out, 76561198000000002).is_err());
    }

    #[test]
    fn empty_or_missing_file_means_no_accounts() {
        assert!(read_users("").unwrap().is_empty());
        assert!(read_users("\"users\"\n{\n}\n").unwrap().is_empty());
        assert_eq!(select_user("", 1, false, NOW), Err(Error::MissingUsersBlock));
    }

    #[test]
    fn account_id_matches_steam3() {
        assert_eq!(account_id(76561197960287930), 22202);
    }
}
