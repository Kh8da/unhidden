//! Edits to Steam's `config/config.vdf`. That file holds a lot of unrelated
//! settings, so only the one value is replaced and every other byte is kept.

/// Steam's "Ask which account to use each time Steam starts" setting. While it
/// is on, Steam shows its account picker instead of signing in to `AutoLoginUser`.
pub const ACCOUNT_CHOOSER_KEY: &str = "AlwaysShowUserChooser";

/// Replaces the value of every `"key" "value"` line for `key`. Returns `None`
/// when the key isn't in the file or already has that value.
pub fn set_line_value(text: &str, key: &str, value: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 8);
    let mut changed = false;
    for line in text.split_inclusive('\n') {
        match replace_value(line, key, value) {
            Some(new_line) => {
                changed = true;
                out.push_str(&new_line);
            }
            None => out.push_str(line),
        }
    }
    changed.then_some(out)
}

fn replace_value(line: &str, key: &str, value: &str) -> Option<String> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let quoted_key = rest.get(..key.len() + 2)?;
    if !quoted_key.starts_with('"')
        || !quoted_key.ends_with('"')
        || !quoted_key[1..key.len() + 1].eq_ignore_ascii_case(key)
    {
        return None;
    }
    let after_key = &rest[key.len() + 2..];
    let gap = after_key.len() - after_key.trim_start_matches([' ', '\t']).len();
    let value_part = &after_key[gap..];
    let inner = value_part.strip_prefix('"')?;
    let end = inner.find('"')?;
    if &inner[..end] == value {
        return None;
    }
    let value_start = indent + key.len() + 2 + gap + 1;
    Some(format!("{}{}{}", &line[..value_start], value, &line[value_start + end..]))
}

/// Turns Steam's account picker off. `None` means nothing needed changing.
pub fn disable_account_chooser(text: &str) -> Option<String> {
    set_line_value(text, ACCOUNT_CHOOSER_KEY, "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"AutoUpdateWindowEnabled\"\t\t\"0\"\n\t\t\t\t\"AlwaysShowUserChooser\"\t\t\"1\"\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn turns_the_picker_off_and_changes_nothing_else() {
        let out = disable_account_chooser(CONFIG).unwrap();
        assert_eq!(out, CONFIG.replace("\"AlwaysShowUserChooser\"\t\t\"1\"", "\"AlwaysShowUserChooser\"\t\t\"0\""));
        assert!(crate::vdf::parse(&out).is_ok());
    }

    #[test]
    fn handles_minus_one_crlf_and_spaces() {
        let text = "\"Steam\"\r\n{\r\n    \"alwaysshowuserchooser\"   \"-1\"\r\n}\r\n";
        assert_eq!(
            disable_account_chooser(text).unwrap(),
            "\"Steam\"\r\n{\r\n    \"alwaysshowuserchooser\"   \"0\"\r\n}\r\n"
        );
    }

    #[test]
    fn leaves_the_file_alone_when_not_needed() {
        assert_eq!(disable_account_chooser(&CONFIG.replace("\"1\"\n\t\t\t\t\"Compat", "\"0\"\n\t\t\t\t\"Compat")), None);
        assert_eq!(disable_account_chooser("\"Steam\"\n{\n}\n"), None);
        assert_eq!(disable_account_chooser(""), None);
    }

    #[test]
    fn does_not_touch_similar_keys_or_values() {
        let text = "\t\"AlwaysShowUserChooserX\"\t\t\"1\"\n\t\"Other\"\t\t\"AlwaysShowUserChooser\"\n\t\"AlwaysShowUserChooser\"\n\t{\n\t}\n";
        assert_eq!(disable_account_chooser(text), None);
    }

    #[test]
    fn last_line_without_newline() {
        assert_eq!(
            disable_account_chooser("\"AlwaysShowUserChooser\" \"1\"").as_deref(),
            Some("\"AlwaysShowUserChooser\" \"0\"")
        );
    }
}
