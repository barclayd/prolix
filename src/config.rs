use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "$schema")]
    _schema: Option<String>,
    pub mode: Option<String>,
    /// The old name for `mode`.
    pub level: Option<String>,
    pub threshold: Option<f32>,
    /// Categories or plain-English rules for comments to keep, whatever the mode.
    #[serde(default)]
    pub keep: Vec<String>,
    /// Categories or plain-English rules for comments to remove, whatever the mode.
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Finds `prolix.jsonc` (or `prolix.json`) in the current directory or a parent; returns it with its directory.
pub fn load() -> Result<(Config, PathBuf), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    for dir in cwd.ancestors() {
        for name in ["prolix.jsonc", "prolix.json"] {
            let path = dir.join(name);
            if let Ok(text) = std::fs::read_to_string(&path) {
                let cfg = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                return Ok((cfg, dir.to_path_buf()));
            }
        }
    }
    Ok((Config::default(), cwd))
}

fn parse(text: &str) -> Result<Config, String> {
    let mut b = text.as_bytes().to_vec();
    let mut comments = Vec::new();
    crate::lex::scan(text.as_bytes(), &crate::lex::JSONC, 0, &mut comments);
    for c in comments {
        b[c.start..c.end].fill(b' ');
    }
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b',' if !in_str => {
                if matches!(
                    b[i + 1..].iter().find(|c| !c.is_ascii_whitespace()),
                    Some(b'}' | b']')
                ) {
                    b[i] = b' ';
                }
            }
            _ => {}
        }
        i += 1;
    }
    let cfg: Config = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    if cfg.threshold.is_some_and(|t| !(0.0..=1.0).contains(&t)) {
        return Err("threshold must be between 0 and 1".into());
    }
    for (key, rules) in [("keep", &cfg.keep), ("remove", &cfg.remove)] {
        for r in rules {
            // A single word is meant as a category, so a typo shouldn't become a rule.
            if !r.trim().contains(char::is_whitespace)
                && !crate::jev::CATS.iter().any(|c| c.name == r)
            {
                let names: Vec<_> = crate::jev::CATS.iter().map(|c| c.name).collect();
                return Err(format!(
                    "{key}: unknown category \"{r}\" (expected {}, or a rule in plain English)",
                    names.join(", ")
                ));
            }
        }
    }
    if let Some(r) = cfg.keep.iter().find(|r| cfg.remove.contains(r)) {
        return Err(format!("\"{r}\" is in both keep and remove"));
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    #[test]
    fn jsonc() {
        let c = super::parse(
            "{\n  // keep why\n  \"mode\": \"strict\", /* x */\n  \"keep\": [\"todo\", \"Names a tool\"],\n  \"ignore\": [\"a//b\",],\n}",
        )
        .unwrap();
        assert_eq!(c.mode.as_deref(), Some("strict"));
        assert_eq!(c.keep, ["todo", "Names a tool"]);
        assert_eq!(c.ignore, ["a//b"]);
        assert!(
            super::parse(r#"{"keep": ["todos"]}"#).is_err_and(|e| e.contains("unknown category"))
        );
        assert!(super::parse(r#"{"keep": ["todo"], "remove": ["todo"]}"#).is_err());
    }
}
