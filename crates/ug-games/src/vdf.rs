//! Valve KeyValues ("VDF") text format, as used by Steam's library files.
//!
//! The format is small: quoted or bare tokens, `{`/`}` for nesting, `//`
//! comments, and backslash escapes inside quotes. Steam treats keys as
//! case-insensitive — `"StateFlags"` and `"stateflags"` both occur in the wild
//! — so lookups here are too.
//!
//! ```text
//! "AppState"
//! {
//!     "appid"      "730"
//!     "name"       "Counter-Strike 2"
//! }
//! ```

/// A parsed KeyValues node.
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    /// Ordered, because duplicate keys are legal and order can matter.
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Parses a document. The top level is an implicit object, so a file with
    /// one root key comes back as `Obj([(root, ...)])`.
    pub fn parse(src: &str) -> Option<Vdf> {
        let mut p = Parser { src: src.as_bytes(), pos: 0 };
        let items = p.object_body(false)?;
        Some(Vdf::Obj(items))
    }

    /// Child by case-insensitive key.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    /// Follows a path of keys, e.g. `["AppState", "name"]`.
    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |node, k| node.get(k))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    /// Children of an object node.
    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            Vdf::Str(_) => &[],
        }
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

enum Tok {
    Str(String),
    Open,
    Close,
}

impl Parser<'_> {
    /// Parses `key value` pairs until `}` (when nested) or end of input.
    fn object_body(&mut self, nested: bool) -> Option<Vec<(String, Vdf)>> {
        let mut items = Vec::new();
        loop {
            match self.token() {
                None => return if nested { None } else { Some(items) },
                Some(Tok::Close) => return if nested { Some(items) } else { None },
                Some(Tok::Open) => return None, // a value with no key
                Some(Tok::Str(key)) => {
                    let value = match self.token()? {
                        Tok::Str(s) => Vdf::Str(s),
                        Tok::Open => Vdf::Obj(self.object_body(true)?),
                        Tok::Close => return None,
                    };
                    // Conditionals like [$WIN32] follow some values; skip them.
                    self.skip_conditional();
                    items.push((key, value));
                }
            }
        }
    }

    fn token(&mut self) -> Option<Tok> {
        self.skip_space_and_comments();
        let c = *self.src.get(self.pos)?;
        match c {
            b'{' => {
                self.pos += 1;
                Some(Tok::Open)
            }
            b'}' => {
                self.pos += 1;
                Some(Tok::Close)
            }
            b'"' => {
                self.pos += 1;
                let mut out = Vec::new();
                while let Some(&b) = self.src.get(self.pos) {
                    self.pos += 1;
                    match b {
                        b'"' => return Some(Tok::Str(String::from_utf8_lossy(&out).into_owned())),
                        b'\\' => {
                            let e = *self.src.get(self.pos)?;
                            self.pos += 1;
                            out.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other, // \\ and \" and anything else literal
                            });
                        }
                        _ => out.push(b),
                    }
                }
                None // unterminated string
            }
            _ => {
                let start = self.pos;
                while let Some(&b) = self.src.get(self.pos) {
                    if b.is_ascii_whitespace() || b == b'{' || b == b'}' || b == b'"' {
                        break;
                    }
                    self.pos += 1;
                }
                Some(Tok::Str(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()))
            }
        }
    }

    fn skip_space_and_comments(&mut self) {
        loop {
            while self.src.get(self.pos).is_some_and(|b| b.is_ascii_whitespace()) {
                self.pos += 1;
            }
            if self.src[self.pos..].starts_with(b"//") {
                while self.src.get(self.pos).is_some_and(|&b| b != b'\n') {
                    self.pos += 1;
                }
            } else {
                return;
            }
        }
    }

    fn skip_conditional(&mut self) {
        let save = self.pos;
        while self.src.get(self.pos).is_some_and(|&b| b == b' ' || b == b'\t') {
            self.pos += 1;
        }
        if self.src.get(self.pos) == Some(&b'[') {
            while self.src.get(self.pos).is_some_and(|&b| b != b']' && b != b'\n') {
                self.pos += 1;
            }
            self.pos += 1;
        } else {
            self.pos = save;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
	"appid"		"730"
	"Universe"		"1"
	"name"		"Counter-Strike 2"
	"StateFlags"		"4"
	"installdir"		"Counter-Strike Global Offensive"
	"UserConfig"
	{
		"language"		"english"
	}
}
"#;

    #[test]
    fn parses_an_app_manifest() {
        let v = Vdf::parse(MANIFEST).unwrap();
        assert_eq!(v.path(&["AppState", "appid"]).and_then(Vdf::as_str), Some("730"));
        assert_eq!(v.path(&["AppState", "name"]).and_then(Vdf::as_str), Some("Counter-Strike 2"));
        assert_eq!(
            v.path(&["AppState", "UserConfig", "language"]).and_then(Vdf::as_str),
            Some("english")
        );
    }

    #[test]
    fn keys_are_case_insensitive_like_steam() {
        let v = Vdf::parse(MANIFEST).unwrap();
        assert_eq!(v.path(&["appstate", "stateflags"]).and_then(Vdf::as_str), Some("4"));
    }

    #[test]
    fn escapes_comments_and_bare_tokens() {
        let v = Vdf::parse(
            "// a comment\n\"root\" {\n  \"path\" \"C:\\\\Games\\\\x\"  // trailing\n  bare value\n  \"q\" \"say \\\"hi\\\"\"\n}",
        )
        .unwrap();
        assert_eq!(v.path(&["root", "path"]).and_then(Vdf::as_str), Some("C:\\Games\\x"));
        assert_eq!(v.path(&["root", "bare"]).and_then(Vdf::as_str), Some("value"));
        assert_eq!(v.path(&["root", "q"]).and_then(Vdf::as_str), Some("say \"hi\""));
    }

    #[test]
    fn platform_conditionals_are_skipped() {
        let v = Vdf::parse("\"r\" { \"k\" \"v\" [$WIN32]\n \"k2\" \"v2\" }").unwrap();
        assert_eq!(v.path(&["r", "k2"]).and_then(Vdf::as_str), Some("v2"));
    }

    #[test]
    fn truncated_input_is_rejected_not_half_parsed() {
        assert!(Vdf::parse("\"AppState\" { \"appid\" \"730\"").is_none());
        assert!(Vdf::parse("\"AppState\" { \"name\" \"unterminated").is_none());
        assert!(Vdf::parse("}").is_none());
    }
}
