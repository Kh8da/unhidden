//! The app's own settings: pins, nicknames, notes and a manually picked Steam folder.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Meta {
    pub pinned: bool,
    pub nickname: String,
    pub note: String,
}

impl Meta {
    fn is_empty(&self) -> bool {
        *self == Meta::default()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Data {
    pub steam_dir: Option<String>,
    pub accounts: HashMap<String, Meta>,
}

pub struct Store {
    path: PathBuf,
    pub data: Data,
}

impl Store {
    pub fn load(path: PathBuf) -> Store {
        let data = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
                // Keep the unreadable file around instead of silently losing it.
                let _ = fs::rename(&path, path.with_extension("json.broken"));
                Data::default()
            }),
            Err(_) => Data::default(),
        };
        Store { path, data }
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("couldn't save settings: {e}"))?;
        }
        let json = serde_json::to_string_pretty(&self.data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| format!("couldn't save settings: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("couldn't save settings: {e}"))
    }

    pub fn meta(&self, steam_id: &str) -> Meta {
        self.data.accounts.get(steam_id).cloned().unwrap_or_default()
    }

    pub fn set_meta(&mut self, steam_id: &str, meta: Meta) {
        if meta.is_empty() {
            self.data.accounts.remove(steam_id);
        } else {
            self.data.accounts.insert(steam_id.to_string(), meta);
        }
    }
}
