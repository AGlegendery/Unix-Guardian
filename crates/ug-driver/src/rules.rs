//! The rule engine: problems are data (`data/driver-rules/*.toml`), not code.
//!
//! A rule names a problem, its severity, its text in English and Persian,
//! the fix that solves it, and `when`: conditions that must all hold. Each
//! condition is one comparison on the facts from `inventory`:
//!
//! ```text
//! module.nvidia                       a fact that is true / non-empty
//! !nouveau.blacklisted                one that is false / empty
//! nvidia.lib_major >= 545             numbers compare as numbers
//! nvidia.family == "kepler"           strings in quotes
//! nvidia.kernel_version != $nvidia.lib_version    `$` names another fact
//! kernel.cmdline contains "nomodeset"
//! ```
//!
//! Adding a rule is an edit to a TOML file; a test checks that every fact a
//! shipped rule names really exists, so a typo cannot silently never match.

use crate::inventory::{Facts, Value};
use serde::Deserialize;

/// The rule files shipped with Unix Guardian.
pub const SHIPPED: &[(&str, &str)] = &[
    ("nvidia.toml", include_str!("../../../data/driver-rules/nvidia.toml")),
    ("common.toml", include_str!("../../../data/driver-rules/common.toml")),
];

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    pub severity: String,
    pub when: Vec<String>,
    pub title_en: String,
    pub title_fa: String,
    pub what_en: String,
    pub what_fa: String,
    pub why_en: String,
    pub why_fa: String,
    pub how_en: String,
    pub how_fa: String,
    #[serde(default)]
    pub fix: String,
    #[serde(default)]
    pub doc_url: String,
}

#[derive(Deserialize)]
struct File {
    rule: Vec<Rule>,
}

/// Parses rule files; an error names the file and the problem.
pub fn load(files: &[(&str, &str)]) -> Result<Vec<Rule>, String> {
    let mut all = Vec::new();
    for (name, text) in files {
        let f: File = toml::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        for r in &f.rule {
            if !matches!(r.severity.as_str(), "critical" | "important" | "advice") {
                return Err(format!("{name}: rule {}: severity must be critical, important or advice", r.id));
            }
            for c in &r.when {
                parse(c).map_err(|e| format!("{name}: rule {}: {e}", r.id))?;
            }
        }
        all.extend(f.rule);
    }
    Ok(all)
}

/// The shipped rules. They are checked by the tests, so this cannot fail in
/// a release; if it ever does, diagnosis says so instead of panicking.
pub fn shipped() -> Result<Vec<Rule>, String> {
    load(SHIPPED)
}

#[derive(Debug, PartialEq)]
enum Operand {
    Fact(String),
    Lit(Value),
}

#[derive(Debug, PartialEq)]
enum Cond {
    Truthy(String),
    Falsy(String),
    Cmp(String, String, Operand),
}

fn parse(c: &str) -> Result<Cond, String> {
    let c = c.trim();
    let words: Vec<&str> = c.splitn(3, ' ').collect();
    if words.len() == 1 {
        return Ok(match c.strip_prefix('!') {
            Some(f) => Cond::Falsy(f.to_string()),
            None => Cond::Truthy(c.to_string()),
        });
    }
    if words.len() != 3 {
        return Err(format!("cannot read condition `{c}`"));
    }
    let op = words[1];
    if !matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains") {
        return Err(format!("unknown comparison `{op}` in `{c}`"));
    }
    let rhs = words[2].trim();
    let operand = if let Some(f) = rhs.strip_prefix('$') {
        Operand::Fact(f.to_string())
    } else if rhs.len() >= 2 && (rhs.starts_with('"') && rhs.ends_with('"') || rhs.starts_with('\'') && rhs.ends_with('\'')) {
        Operand::Lit(Value::Str(rhs[1..rhs.len() - 1].to_string()))
    } else if rhs == "true" || rhs == "false" {
        Operand::Lit(Value::Bool(rhs == "true"))
    } else if let Ok(n) = rhs.parse::<f64>() {
        Operand::Lit(Value::Num(n))
    } else {
        return Err(format!("`{rhs}` is not a number, true/false, a quoted string or a $fact"));
    };
    Ok(Cond::Cmp(words[0].to_string(), op.to_string(), operand))
}

/// The facts a condition names, for the test that they all exist.
pub fn named_facts(c: &str) -> Vec<String> {
    match parse(c) {
        Ok(Cond::Truthy(f)) | Ok(Cond::Falsy(f)) => vec![f],
        Ok(Cond::Cmp(f, _, Operand::Fact(g))) => vec![f, g],
        Ok(Cond::Cmp(f, _, Operand::Lit(_))) => vec![f],
        Err(_) => vec![],
    }
}

fn get(facts: &Facts, k: &str) -> Value {
    facts.get(k).cloned().unwrap_or(Value::Str(String::new()))
}

fn holds(c: &Cond, facts: &Facts) -> bool {
    match c {
        Cond::Truthy(f) => get(facts, f).truthy(),
        Cond::Falsy(f) => !get(facts, f).truthy(),
        Cond::Cmp(f, op, rhs) => {
            let a = get(facts, f);
            let b = match rhs {
                Operand::Fact(g) => get(facts, g),
                Operand::Lit(v) => v.clone(),
            };
            if op == "contains" {
                return a.text().contains(&b.text());
            }
            let ord = match (&a, &b) {
                (Value::Num(x), Value::Num(y)) => x.partial_cmp(y),
                _ => Some(a.text().cmp(&b.text())),
            };
            let Some(o) = ord else { return false };
            match op.as_str() {
                "==" => o.is_eq(),
                "!=" => o.is_ne(),
                "<" => o.is_lt(),
                "<=" => o.is_le(),
                ">" => o.is_gt(),
                _ => o.is_ge(),
            }
        }
    }
}

/// Whether every condition of a rule holds.
pub fn matches(rule: &Rule, facts: &Facts) -> bool {
    rule.when.iter().all(|c| parse(c).is_ok_and(|c| holds(&c, facts)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(pairs: &[(&str, Value)]) -> Facts {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn conditions_compare_facts_strings_and_numbers() {
        let f = facts(&[
            ("a.on", Value::Bool(true)),
            ("a.off", Value::Bool(false)),
            ("k", Value::Str("550.163.01".into())),
            ("l", Value::Str("550.120".into())),
            ("n", Value::Num(550.0)),
            ("cmd", Value::Str("quiet splash nvidia-drm.modeset=1".into())),
        ]);
        let ok = |c: &str| holds(&parse(c).unwrap(), &f);
        assert!(ok("a.on") && ok("!a.off") && ok("!missing"));
        assert!(!ok("missing") && !ok("!a.on"));
        assert!(ok("k != $l") && !ok("k == $l") && ok("k == \"550.163.01\""));
        assert!(ok("n >= 545") && ok("n < 555") && !ok("n > 550"));
        assert!(ok("n > 99"), "numbers compare as numbers, not text");
        assert!(ok("cmd contains \"modeset=1\"") && !ok("cmd contains 'nomodeset'"));
    }

    #[test]
    fn a_bad_rule_file_is_refused_with_its_name() {
        let bad = r#"
[[rule]]
id = "x"
severity = "urgent"
when = ["a"]
title_en = ""
title_fa = ""
what_en = ""
what_fa = ""
why_en = ""
why_fa = ""
how_en = ""
how_fa = ""
"#;
        assert!(load(&[("bad.toml", bad)]).unwrap_err().contains("bad.toml"));
        let worse = bad.replace("urgent", "advice").replace("[\"a\"]", "[\"a ~ 3\"]");
        assert!(load(&[("bad.toml", &worse)]).unwrap_err().contains("unknown comparison"));
    }
}
