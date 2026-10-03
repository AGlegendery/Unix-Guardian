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

/// Quotes a value the way Steam writes it.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Sets `key` to `value` in the object at `path`, editing the text in place:
/// everything else stays byte for byte, so a file another program owns (Steam's
/// `localconfig.vdf`) changes in exactly one value. Objects missing from the
/// end of the path are created inside the deepest one that exists; the first
/// element of the path must exist. Keys match case-insensitively.
pub fn set_string(src: &str, path: &[&str], key: &str, value: &str) -> Result<String, String> {
    let mut p = Parser { src: src.as_bytes(), pos: 0 };
    // Open objects: (key, whether it and every parent match the path so far).
    let mut stack: Vec<(String, bool)> = Vec::new();
    // Per depth: a block of the path's name there was already closed. Only
    // the first block of a name counts, as for `Vdf::get`.
    let mut done = vec![false; path.len() + 1];
    let mut value_span: Option<(usize, usize)> = None;
    // The deepest object on the path seen so far: (depth, position of its `}`).
    let mut deepest: Option<(usize, usize)> = None;
    let on_path = |stack: &[(String, bool)]| stack.last().is_none_or(|(_, m)| *m);
    loop {
        p.skip_space_and_comments();
        let start = p.pos;
        let Some(tok) = p.token() else {
            if !stack.is_empty() {
                return Err("the file ends inside a block".into());
            }
            break;
        };
        match tok {
            Tok::Close => {
                let Some((_, matched)) = stack.pop() else { return Err("a `}` closes nothing".into()) };
                let depth = stack.len() + 1;
                if matched {
                    done[depth] = true;
                    if deepest.is_none_or(|(d, _)| depth > d) {
                        deepest = Some((depth, start));
                    }
                }
            }
            Tok::Open => return Err("a block without a name".into()),
            Tok::Str(k) => {
                p.skip_space_and_comments();
                let vstart = p.pos;
                match p.token().ok_or("a name without a value")? {
                    Tok::Str(_) => {
                        p.skip_conditional();
                        if stack.len() == path.len() && on_path(&stack) && k.eq_ignore_ascii_case(key) && value_span.is_none() {
                            value_span = Some((vstart, p.pos));
                        }
                    }
                    Tok::Open => {
                        let depth = stack.len();
                        let matched = path.get(depth).is_some_and(|want| want.eq_ignore_ascii_case(&k))
                            && on_path(&stack)
                            && !done[depth + 1];
                        stack.push((k, matched));
                    }
                    Tok::Close => return Err("a name without a value".into()),
                }
            }
        }
    }
    if let Some((s, e)) = value_span {
        // Replace the quoted (or bare) value; a trailing conditional stays out.
        let tok_end = value_end(src, s, e);
        return Ok(format!("{}{}{}", &src[..s], quote(value), &src[tok_end..]));
    }
    let Some((depth, close)) = deepest else {
        return Err(format!("the file has no {:?} block", path.first().copied().unwrap_or("")));
    };
    // Build the missing blocks and the value, indented with tabs like Steam.
    let tabs = |n: usize| "\t".repeat(n);
    let mut add = String::new();
    for (i, name) in path[depth..].iter().enumerate() {
        let d = depth + i;
        add.push_str(&format!("{}{}\n{}{{\n", tabs(d), quote(name), tabs(d)));
    }
    add.push_str(&format!("{}{}\t\t{}\n", tabs(path.len()), quote(key), quote(value)));
    for d in (depth..path.len()).rev() {
        add.push_str(&format!("{}}}\n", tabs(d)));
    }
    // Before the `}` line of the deepest block (or the `}` itself on a shared line).
    let line_start = src[..close].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if src[line_start..close].trim().is_empty() {
        Ok(format!("{}{}{}", &src[..line_start], add, &src[line_start..]))
    } else {
        Ok(format!("{}\n{}{}", &src[..close], add, &src[close..]))
    }
}

/// End of a value token starting at `s` (the parser's position `e` may have
/// moved past a trailing `[$CONDITION]`).
fn value_end(src: &str, s: usize, e: usize) -> usize {
    let b = src.as_bytes();
    if b.get(s) == Some(&b'"') {
        let mut i = s + 1;
        while i < e {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
        e
    } else {
        (s..e).find(|&i| b[i].is_ascii_whitespace() || b[i] == b'{' || b[i] == b'}' || b[i] == b'"').unwrap_or(e)
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

    const LOCALCONFIG: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"730\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t\t\"cloud\"\n\t\t\t\t\t\t{\n\t\t\t\t\t\t\t\"last_sync_state\"\t\t\"synchronized\"\n\t\t\t\t\t\t}\n\t\t\t\t\t}\n\t\t\t\t\t\"760\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"cloud\"\t\t{ \"quota_files\" \"50000\" }\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t\t\"PlayerLevel\"\t\t\"0\"\n\t\t\t}\n\t\t}\n\t}\n}\n";
    const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

    fn launch(src: &str, appid: &str) -> Option<String> {
        let mut path = APPS.to_vec();
        path.push(appid);
        path.push("LaunchOptions");
        Vdf::parse(src)?.path(&path).and_then(Vdf::as_str).map(str::to_string)
    }

    fn with_app(appid: &str) -> Vec<&str> {
        let mut p = APPS.to_vec();
        p.push(appid);
        p
    }

    #[test]
    fn an_existing_value_is_replaced_and_nothing_else_moves() {
        let out = set_string(LOCALCONFIG, &with_app("730"), "LaunchOptions", "mangohud %command% -novid").unwrap();
        assert_eq!(launch(&out, "730").as_deref(), Some("mangohud %command% -novid"));
        assert_eq!(out.replace("mangohud %command% -novid", "-novid"), LOCALCONFIG, "one value changed, every other byte kept");
    }

    #[test]
    fn a_missing_value_and_a_missing_app_are_added_where_steam_would_have_them() {
        let out = set_string(LOCALCONFIG, &with_app("760"), "LaunchOptions", "x %command%").unwrap();
        assert_eq!(launch(&out, "760").as_deref(), Some("x %command%"));
        assert_eq!(launch(&out, "730").as_deref(), Some("-novid"));
        let out = set_string(&out, &with_app("413150"), "launchoptions", "y %command%").unwrap();
        assert_eq!(launch(&out, "413150").as_deref(), Some("y %command%"));
        let doc = Vdf::parse(&out).unwrap();
        assert_eq!(doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "PlayerLevel"]).and_then(Vdf::as_str), Some("0"));
        assert!(out.contains("\n\t\t\t\t\t\"413150\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"launchoptions\"\t\t\"y %command%\"\n\t\t\t\t\t}\n\t\t\t\t}"), "{out}");
    }

    #[test]
    fn quotes_and_backslashes_survive_a_round_trip() {
        let v = r#"FOO="a b" C:\x %command%"#;
        let out = set_string(LOCALCONFIG, &with_app("730"), "LaunchOptions", v).unwrap();
        assert_eq!(launch(&out, "730").as_deref(), Some(v));
    }

    #[test]
    fn a_file_without_the_root_or_broken_is_refused() {
        assert!(set_string("\"Other\" { }", &with_app("730"), "LaunchOptions", "x").is_err());
        assert!(set_string("\"UserLocalConfigStore\" { \"Software\" {", &with_app("730"), "LaunchOptions", "x").is_err());
    }

    #[test]
    fn truncated_input_is_rejected_not_half_parsed() {
        assert!(Vdf::parse("\"AppState\" { \"appid\" \"730\"").is_none());
        assert!(Vdf::parse("\"AppState\" { \"name\" \"unterminated").is_none());
        assert!(Vdf::parse("}").is_none());
    }
}
