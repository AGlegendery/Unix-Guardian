//! Key combinations the user picks for the global shortcuts.
//!
//! The window and the TUI record a press and send it in any common spelling
//! ("Meta+Shift+G", "LOGO+SHIFT+g", "ctrl+alt+F5"); the agent keeps it in the
//! XDG shortcuts format (modifiers CTRL, ALT, SHIFT, LOGO joined with `+`,
//! then an xkbcommon key name), shows it the way KDE does ("Meta+Shift+G"),
//! and hands KDE's kglobalaccel the Qt key code.

/// A key with its modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    /// The xkbcommon key name: `g`, `5`, `F5`, `space`, `Page_Up`, `comma`…
    pub key: String,
}

/// Named keys: (xkb name, label, Qt key code, other spellings in lower case).
const NAMED: &[(&str, &str, i32, &[&str])] = &[
    ("space", "Space", 0x20, &[" "]),
    ("Return", "Return", 0x0100_0004, &["enter"]),
    ("Tab", "Tab", 0x0100_0001, &[]),
    ("Escape", "Esc", 0x0100_0000, &["esc"]),
    ("BackSpace", "Backspace", 0x0100_0003, &[]),
    ("Insert", "Ins", 0x0100_0006, &["ins"]),
    ("Delete", "Del", 0x0100_0007, &["del"]),
    ("Pause", "Pause", 0x0100_0008, &[]),
    ("Print", "Print", 0x0100_0009, &["printscreen"]),
    ("Home", "Home", 0x0100_0010, &[]),
    ("End", "End", 0x0100_0011, &[]),
    ("Left", "Left", 0x0100_0012, &["arrowleft"]),
    ("Up", "Up", 0x0100_0013, &["arrowup"]),
    ("Right", "Right", 0x0100_0014, &["arrowright"]),
    ("Down", "Down", 0x0100_0015, &["arrowdown"]),
    ("Page_Up", "PgUp", 0x0100_0016, &["prior", "pageup", "pgup"]),
    (
        "Page_Down",
        "PgDown",
        0x0100_0017,
        &["next", "pagedown", "pgdown"],
    ),
    ("comma", ",", 0x2c, &[","]),
    ("period", ".", 0x2e, &["."]),
    ("minus", "-", 0x2d, &["-"]),
    ("equal", "=", 0x3d, &["="]),
    ("slash", "/", 0x2f, &["/"]),
    ("backslash", "\\", 0x5c, &["\\"]),
    ("semicolon", ";", 0x3b, &[";"]),
    ("apostrophe", "'", 0x27, &["'", "quote"]),
    ("grave", "`", 0x60, &["`", "backquote"]),
    ("bracketleft", "[", 0x5b, &["["]),
    ("bracketright", "]", 0x5d, &["]"]),
];

const SHIFT: i32 = 0x0200_0000;
const CTRL: i32 = 0x0400_0000;
const ALT: i32 = 0x0800_0000;
const META: i32 = 0x1000_0000;

/// F1…F24 as its number.
fn function_key(k: &str) -> Option<i32> {
    let n: i32 = k.strip_prefix(['F', 'f'])?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

impl Combo {
    /// Reads a combination in any of the usual spellings. A shortcut needs
    /// Ctrl, Alt or Meta (Shift alone would steal typing), except F1–F24.
    pub fn parse(text: &str) -> Result<Combo, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("No keys given.".into());
        }
        let mut c = Combo {
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
            key: String::new(),
        };
        // "+" itself can't be the key (it is the separator); "Ctrl++" is not
        // offered.
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (last, mods) = parts.split_last().expect("split gives one part at least");
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "primary" => c.ctrl = true,
                "alt" | "mod1" => c.alt = true,
                "shift" => c.shift = true,
                "logo" | "meta" | "super" | "win" | "mod4" => c.logo = true,
                "" => return Err(format!("`{text}`: an empty part.")),
                other => {
                    return Err(format!(
                        "`{other}` is not a modifier (Ctrl, Alt, Shift, Meta)."
                    ))
                }
            }
        }
        c.key = key_name(last).ok_or_else(|| {
            if is_modifier(last) {
                "Press a key together with the modifiers, not only Ctrl, Alt, Shift or Meta."
                    .to_string()
            } else {
                format!("`{last}` is not a key a shortcut can use.")
            }
        })?;
        if !(c.ctrl || c.alt || c.logo) && function_key(&c.key).is_none() {
            return Err(format!(
                "{} needs Ctrl, Alt or Meta with it, or it would take the key from everything you type.",
                c.label()
            ));
        }
        Ok(c)
    }

    /// The XDG shortcuts format, e.g. `LOGO+SHIFT+g`.
    pub fn xdg(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.ctrl, "CTRL"),
            (self.alt, "ALT"),
            (self.shift, "SHIFT"),
            (self.logo, "LOGO"),
        ] {
            if on {
                s.push_str(name);
                s.push('+');
            }
        }
        s + &self.key
    }

    /// As KDE shows it, e.g. `Meta+Shift+G`.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.logo, "Meta"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if on {
                s.push_str(name);
                s.push('+');
            }
        }
        let key = NAMED
            .iter()
            .find(|n| n.0 == self.key)
            .map(|n| n.1.to_string())
            .unwrap_or_else(|| self.key.to_uppercase());
        s + &key
    }

    /// The Qt key code with its modifiers (what kglobalaccel stores).
    pub fn qt(&self) -> i32 {
        let key = if let Some(n) = function_key(&self.key) {
            0x0100_0030 + n - 1
        } else if let Some(n) = NAMED.iter().find(|n| n.0 == self.key) {
            n.2
        } else {
            // A letter or a digit: Qt uses the upper-case character.
            self.key
                .to_ascii_uppercase()
                .chars()
                .next()
                .map_or(0, |c| c as i32)
        };
        let mut code = key;
        for (on, bit) in [
            (self.shift, SHIFT),
            (self.ctrl, CTRL),
            (self.alt, ALT),
            (self.logo, META),
        ] {
            if on {
                code |= bit;
            }
        }
        code
    }
}

fn is_modifier(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    let k = k.trim_end_matches("_l").trim_end_matches("_r");
    matches!(
        k,
        "ctrl" | "control" | "alt" | "shift" | "logo" | "meta" | "super" | "win" | "os"
    )
}

/// The xkb name for a key in any common spelling (an xkb name, a browser
/// `KeyboardEvent.key`, a character).
fn key_name(k: &str) -> Option<String> {
    if k.chars().count() == 1 {
        let c = k.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_lowercase().to_string());
        }
    }
    if let Some(n) = function_key(k) {
        return Some(format!("F{n}"));
    }
    let lower = k.to_ascii_lowercase();
    NAMED
        .iter()
        .find(|n| n.0.eq_ignore_ascii_case(k) || n.3.contains(&k) || n.3.contains(&lower.as_str()))
        .map(|n| n.0.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_usual_spellings_into_one_form() {
        for s in [
            "Meta+Shift+G",
            "LOGO+SHIFT+g",
            "shift+super+g",
            " Win + Shift + g ",
        ] {
            let c = Combo::parse(s).unwrap();
            assert_eq!(c.xdg(), "SHIFT+LOGO+g", "{s}");
            assert_eq!(c.label(), "Meta+Shift+G", "{s}");
        }
        assert_eq!(Combo::parse("ctrl+alt+F5").unwrap().xdg(), "CTRL+ALT+F5");
        assert_eq!(
            Combo::parse("Ctrl+Alt+PageUp").unwrap().xdg(),
            "CTRL+ALT+Page_Up"
        );
        assert_eq!(
            Combo::parse("Ctrl+Alt+Page_Up").unwrap().label(),
            "Ctrl+Alt+PgUp"
        );
        assert_eq!(Combo::parse("Meta+,").unwrap().xdg(), "LOGO+comma");
        assert_eq!(Combo::parse("Meta+Space").unwrap().xdg(), "LOGO+space");
    }

    #[test]
    fn a_shortcut_needs_ctrl_alt_or_meta_unless_it_is_a_function_key() {
        assert!(Combo::parse("g")
            .unwrap_err()
            .contains("needs Ctrl, Alt or Meta"));
        assert!(Combo::parse("Shift+g")
            .unwrap_err()
            .contains("Shift+G needs"));
        assert_eq!(Combo::parse("F9").unwrap().xdg(), "F9");
        assert!(Combo::parse("Meta+Shift")
            .unwrap_err()
            .contains("not only Ctrl"));
        assert!(Combo::parse("Meta+Shift_L")
            .unwrap_err()
            .contains("not only Ctrl"));
        assert!(Combo::parse("Hyper+g")
            .unwrap_err()
            .contains("not a modifier"));
        assert!(Combo::parse("Meta+F25").is_err());
        assert!(Combo::parse("").is_err());
    }

    #[test]
    fn gives_kde_the_qt_key_code() {
        // Values from Qt: Key_G 0x47, Key_F5 0x01000034, MetaModifier
        // 0x10000000, ShiftModifier 0x02000000.
        assert_eq!(Combo::parse("Meta+Shift+G").unwrap().qt(), 0x1200_0047);
        assert_eq!(
            Combo::parse("Ctrl+Alt+F5").unwrap().qt(),
            0x0C00_0000 | 0x0100_0034
        );
        assert_eq!(Combo::parse("Meta+5").unwrap().qt(), 0x1000_0035);
        assert_eq!(Combo::parse("Meta+Space").unwrap().qt(), 0x1000_0020);
        assert_eq!(
            Combo::parse("Alt+Down").unwrap().qt(),
            0x0800_0000 | 0x0100_0015
        );
    }
}
