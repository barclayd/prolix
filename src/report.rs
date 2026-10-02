use crate::jev;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub complete: bool,
    pub errors: Vec<String>,
    pub mode: String,
    pub threshold: f64,
    pub fix_threshold: f64,
    pub prompt_version: String,
    pub prompt_hash: String,
    pub resolved_models: BTreeSet<String>,
    pub model: String,
    pub comments: Vec<Finding>,
    pub stats: Stats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
    pub group: Option<String>,
    pub skipped: Option<String>,
    pub model: Option<String>,
    pub fix_status: FixStatus,
    pub decision_id: String,
    pub confidence: Option<f64>,
    pub probabilities: Option<BTreeMap<String, f64>>,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FixStatus {
    Available,
    #[default]
    NotFlagged,
    IncompleteCheck,
    UnverifiedLanguage,
    BelowFixThreshold,
    VerificationFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub start_line: usize,
    pub end_line: usize,
    pub replacement: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub files: usize,
    pub comments: usize,
    pub flagged: usize,
    pub asked: usize,
    pub cached: usize,
    pub deduplicated: usize,
    pub skipped: usize,
    pub fixed: usize,
    pub remaining: usize,
    pub unanswered: usize,
    pub input_tokens: u64,
    pub elapsed_ms: u64,
}

impl Report {
    pub fn ensure_complete(&self) -> Result<(), String> {
        if !self.complete
            || !self.errors.is_empty()
            || self.stats.unanswered != 0
            || self.stats.skipped != 0
        {
            return Err("incomplete report; review threads were left unchanged".into());
        }
        Ok(())
    }

    pub fn exit_code(&self) -> i32 {
        if self.ensure_complete().is_err() {
            2
        } else {
            i32::from(self.stats.remaining > 0)
        }
    }

    pub fn markdown(&self, again: &str) -> String {
        render(self, true, false, false, again)
    }
}

const MD_ROWS: usize = 200;

pub fn render(report: &Report, md: bool, color: bool, fix: bool, again: &str) -> String {
    let complete = report.complete;
    let errors = &report.errors;
    let Stats {
        files: n,
        comments: checked,
        flagged: total,
        asked,
        cached,
        deduplicated,
        fixed,
        input_tokens: tokens,
        ..
    } = report.stats;
    let judged = asked + cached + deduplicated;

    let paint = |code: &str, s: &str| {
        if color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let mut out = String::new();
    let mut groups: Vec<(&str, usize)> = Vec::new();
    let (mut found, mut found_files) = (0, 0);
    let mut files = std::collections::BTreeMap::<&str, Vec<&Finding>>::new();
    for comment in &report.comments {
        if comment.group.is_some() {
            files.entry(&comment.path).or_default().push(comment);
        }
    }
    for (path, spans) in files {
        if spans.is_empty() {
            continue;
        }
        found += spans.len();
        found_files += 1;
        if !fix && !md {
            let _ = writeln!(
                out,
                "{}{}",
                if out.is_empty() { "" } else { "\n" },
                paint("4", path)
            );
        }
        let at: Vec<_> = spans
            .iter()
            .map(|u| format!("{}:{}", u.line, u.column))
            .collect();
        let w = at.iter().map(String::len).max().unwrap_or(0);
        for (i, (u, at)) in spans.iter().zip(&at).enumerate() {
            let g = u.group.as_deref().unwrap();
            match groups.iter_mut().find(|(n, _)| *n == g) {
                Some((_, c)) => *c += 1,
                None => groups.push((g, 1)),
            }
            let text = crate::preview(&u.text);
            if md && found - spans.len() + i < MD_ROWS {
                let at = format!("{}:{}", path, u.line);
                let _ = writeln!(out, "- {} {g}: {}", code(&at), code(&text));
            } else if !fix && !md {
                let _ = writeln!(
                    out,
                    "  {}  {}  {text}",
                    paint("2", &format!("{at:<w$}")),
                    paint("33", &format!("{g:<18}")),
                );
            }
        }
    }

    let mode = &report.mode;
    let s = |n: usize| if n == 1 { "" } else { "s" };
    groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let what = |g: &str| {
        jev::CATS
            .iter()
            .find(|c| c.name == g)
            .map(|c| c.summary)
            .unwrap_or_default()
    };
    let mut stats = format!(
        "Checked {checked} comment{} in {n} file{} in {:.2}s",
        s(checked),
        s(n),
        report.stats.elapsed_ms as f64 / 1000.0
    );
    if judged > 0 {
        let _ = write!(
            stats,
            " · Jev: {asked} asked, {cached} cached, {deduplicated} deduplicated, {}k tokens",
            tokens / 1000
        );
    }

    if md {
        let mut m = String::new();
        if !complete {
            let _ = writeln!(m, "### prolix: check incomplete\n\n{}", errors.join("\n\n"));
        } else if found == 0 {
            let _ = writeln!(m, "### ✅ prolix: no comments to remove (mode: {mode})");
        } else {
            let did = "found";
            let rest = if fix { "" } else { " to remove" };
            let _ = writeln!(
                m,
                "### prolix {did} {found} comment{}{rest} (mode: {mode})\n",
                s(found)
            );
            m.push_str("| Count | Category | Description |\n| --: | --- | --- |\n");
            for (g, c) in &groups {
                let _ = writeln!(m, "| {c} | `{g}` | {} |", what(g));
            }
            let _ = write!(
                m,
                "\n<details><summary>{found} comment{} in {found_files} file{}</summary>\n\n{out}",
                s(found),
                s(found_files)
            );
            if found > MD_ROWS {
                let _ = writeln!(m, "- …and {} more", found - MD_ROWS);
            }
            m.push_str("\n</details>\n");
            if !fix {
                let _ = writeln!(
                    m,
                    "\nRun `npx @prolix/cli{again} --fix` to apply eligible fixes."
                );
            }
        }
        if fix && complete {
            let _ = writeln!(
                m,
                "\nApplied {fixed} fixes; {} findings remain for review.",
                total - fixed
            );
        }
        let _ = writeln!(m, "\n<sub>{stats}</sub>");
        return m;
    }

    if !complete {
        let _ = writeln!(out, "Check incomplete; no fixes applied.");
    } else if found == 0 {
        let _ = writeln!(
            out,
            "{} No comments to remove (mode: {mode}).",
            paint("32", "✓")
        );
    } else {
        let verb = "Found";
        let gap = if out.is_empty() { "" } else { "\n" };
        let _ = writeln!(
            out,
            "{gap}{verb} {found} comment{} in {found_files} file{} (mode: {mode}):\n",
            s(found),
            s(found_files)
        );
        for (g, c) in &groups {
            let _ = writeln!(
                out,
                "  {c:>5}  {}  {}",
                paint("33", &format!("{g:<18}")),
                what(g)
            );
        }
        if !fix {
            let _ = writeln!(out, "\nRun `prolix{again} --fix` to apply eligible fixes.");
        }
    }
    if fix && complete {
        let _ = writeln!(
            out,
            "Applied {fixed} fixes; {} findings remain for review.",
            total - fixed
        );
    }
    let _ = writeln!(out, "{}", paint("2", &stats));
    out
}
/// A Markdown code span that survives backticks in `s`.
fn code(s: &str) -> String {
    let fence = "`".repeat((1..).find(|&n| !s.contains(&"`".repeat(n))).unwrap());
    format!("{fence} {s} {fence}")
}
