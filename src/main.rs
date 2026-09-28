mod config;
mod diff;
mod fix;
mod jev;
mod lex;

use ignore::{
    overrides::{Override, OverrideBuilder},
    WalkBuilder, WalkState,
};
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const USAGE: &str = "\
Find and remove comments that don't earn their place, judged by Jev.

Usage: prolix [paths...] [--fix] [--changed[=<ref>]] [--level <level>] [--reporter <reporter>]

Options:
  --fix                    Remove the flagged comments
  --changed[=<ref>]        Only check comments on lines added since <ref>   [default: HEAD]
  --level <level>          all | value-add | necessary | none   [default: value-add]
  --reporter <reporter>    text | json | markdown   [default: text]
  -h, --help               Print help
  -V, --version            Print version

Levels:
  all         keep every comment
  value-add   keep comments that tell the reader something the code doesn't
  necessary   keep only intent, warnings, API docs and references
  none        remove every comment except tool directives and licences

Settings are read from prolix.jsonc in this directory or a parent.
value-add and necessary need TYPESAFE_API_KEY.
";

const LEVELS: [&str; 4] = ["all", "value-add", "necessary", "none"];
const MAX_FILE: u64 = 1 << 20;
const MAX_COMMENT: usize = 2000;
/// Findings listed in a Markdown report, which keeps a PR comment under GitHub's 65k-character limit.
const MD_ROWS: usize = 200;

/// Tool, compiler and licence comments are kept at every level.
const DIRECTIVES: &[&str] = &[
    "eslint-",
    "@ts-",
    "prettier-ignore",
    "biome-ignore",
    "oxlint-",
    "deno-lint-",
    "deno-fmt-",
    "istanbul ",
    "c8 ignore",
    "v8 ignore",
    "jshint",
    "jslint",
    "tslint:",
    "stylelint-",
    "prolix-ignore",
    "noqa",
    "type: ignore",
    "pyright:",
    "mypy:",
    "pylint:",
    "fmt: off",
    "fmt: on",
    "fmt: skip",
    "isort:",
    "pragma: no",
    "nolint",
    "rubocop:",
    "swiftlint:",
    "nosec",
    "shellcheck ",
    "@formatter:",
    "nosonar",
    "cspell:",
    "spell-checker:",
    "markdownlint-",
    "phpcs:",
    "@phpstan-",
    "@psalm-",
    "@vite-ignore",
    "webpack",
    "#__pure__",
    "@__pure__",
    "@__no_side_effects__",
    "sourcemappingurl=",
    "@generated",
    "do not edit",
    "@jsx",
    "@flow",
    "@noflow",
    "<reference ",
    "<amd-module",
    "@jest-environment",
    "@vitest-environment",
    "@type ",
    "@type {",
    "@type{",
    "@typedef",
    "@callback",
    "@template",
    "@satisfies",
    "@overload",
    "@import",
    "@param {",
    "@returns {",
    "@return {",
    "@deprecated",
    "@internal",
    "@public",
    "@private",
    "@hidden",
    "@inheritdoc",
    "@experimental",
    "@alpha",
    "@beta",
    "@refresh reset",
    "copyright",
    "spdx-license-identifier",
    "@license",
    "@preserve",
    "frozen_string_literal",
    "-*-",
    "vim:",
];
/// Matched only at the start of the comment's text, as they're ordinary words elsewhere.
const DIRECTIVE_PREFIXES: &[&str] = &[
    "eslint",
    "global ",
    "globals ",
    "exported ",
    "go:",
    "+build",
    "region",
    "endregion",
    "pragma",
    "coding:",
    "coding=",
    "type:",
];

struct File {
    path: PathBuf,
    src: String,
    lang: &'static lex::Lang,
    units: Vec<Unit>,
}

struct Unit {
    start: usize,
    end: usize,
    line: usize,
    col: usize,
    group: Option<&'static str>,
    /// Jev's probabilities in `jev::CATS` order, once answered.
    probs: Option<Vec<f32>>,
    /// Cache key and surrounding code, while the comment awaits Jev.
    ask: Option<(String, String)>,
}

fn main() {
    let code = run().unwrap_or_else(|e| {
        eprintln!("prolix: {e}");
        2
    });
    std::process::exit(code);
}

fn run() -> Result<i32, String> {
    let t0 = Instant::now();
    let (mut paths, mut fix, mut level, mut reporter) = (Vec::new(), false, None, None);
    let mut changed = None;
    // The same run with --fix, for the hint after the findings.
    let mut again = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if !a.starts_with("--fix") && !a.starts_with("--reporter") {
            again = again + " " + &a;
        }
        match a.as_str() {
            "--fix" => fix = true,
            "--changed" => changed = Some("HEAD".to_string()),
            "--level" => {
                level = Some(args.next().ok_or("--level needs a value")?);
                again = again + " " + level.as_deref().unwrap();
            }
            "--reporter" => reporter = Some(args.next().ok_or("--reporter needs a value")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(0);
            }
            "-V" | "--version" => {
                println!("prolix {}", env!("CARGO_PKG_VERSION"));
                return Ok(0);
            }
            _ if a.starts_with("--level=") => level = Some(a["--level=".len()..].to_string()),
            _ if a.starts_with("--reporter=") => {
                reporter = Some(a["--reporter=".len()..].to_string())
            }
            _ if a.starts_with("--changed=") => changed = Some(a["--changed=".len()..].to_string()),
            _ if a.starts_with('-') => return Err(format!("unknown option {a}\n\n{USAGE}")),
            _ => paths.push(a),
        }
    }
    let reporter = reporter.unwrap_or_else(|| "text".into());
    if !["text", "json", "markdown"].contains(&reporter.as_str()) {
        return Err(format!(
            "unknown reporter \"{reporter}\" (expected text, json or markdown)"
        ));
    }
    let (cfg, root) = config::load()?;
    let level = parse_level(
        level
            .as_deref()
            .or(cfg.level.as_deref())
            .unwrap_or("value-add"),
    )?;
    let threshold = cfg.threshold.unwrap_or(0.6);
    let added = match changed.as_deref() {
        Some(base) if level > 0 => {
            if let Some(p) = paths.iter().find(|p| !Path::new(p).exists()) {
                return Err(format!(
                    "{p}: no such file or directory (to compare with a ref, use --changed=<ref>)"
                ));
            }
            Some(diff::added(base, &paths)?)
        }
        _ => None,
    };
    // Level "all" keeps every comment, so there is nothing to read.
    let targets: Vec<String> = match &added {
        Some(a) => a.keys().cloned().collect(),
        None if level == 0 => Vec::new(),
        None if paths.is_empty() => vec![".".into()],
        None => paths,
    };
    let model = jev::model();
    let scanned = AtomicUsize::new(0);
    let files = Mutex::new(Vec::new());
    if let Some((first, rest)) = targets.split_first() {
        let mut walk = WalkBuilder::new(first);
        for p in rest {
            walk.add(p);
        }
        walk.require_git(false);
        let mut ob = OverrideBuilder::new(&root);
        for g in &cfg.ignore {
            ob.add(&format!("!{g}")).map_err(|e| e.to_string())?;
        }
        // Matched on absolute paths so globs stay relative to prolix.jsonc from any cwd, and apply to explicit paths.
        let ov = Arc::new((
            ob.build().map_err(|e| e.to_string())?,
            std::env::current_dir().map_err(|e| e.to_string())?,
        ));
        let ignored = |ov: &(Override, PathBuf), p: &Path, dir: bool| {
            ov.0.matched(ov.1.join(p).components().collect::<PathBuf>(), dir)
                .is_ignore()
        };
        let ov2 = ov.clone();
        walk.filter_entry(move |e| {
            !ignored(&ov2, e.path(), e.file_type().is_some_and(|t| t.is_dir()))
        });
        walk.build_parallel().run(|| {
            Box::new(|entry| {
                match entry {
                    Ok(e) if e.depth() == 0 && ignored(&ov, e.path(), false) => {}
                    Ok(e) => {
                        if let Some(f) = read(e, level, &model, &scanned) {
                            files.lock().unwrap().push(f);
                        }
                    }
                    Err(e) => eprintln!("prolix: {e}"),
                }
                WalkState::Continue
            })
        });
    }
    let mut files = files.into_inner().unwrap();
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    if let Some(added) = &added {
        for f in &mut files {
            let lines = added
                .get(&*f.path.to_string_lossy())
                .map_or(&[][..], Vec::as_slice);
            f.units.retain(|u| {
                let end = u.line + f.src[u.start..u.end].matches('\n').count();
                lines.iter().any(|&(a, b)| u.line <= b && a <= end)
            });
        }
        files.retain(|f| !f.units.is_empty());
    }

    let cache_path = if root.join("node_modules").is_dir() {
        root.join("node_modules/.cache/prolix.json")
    } else {
        root.join(".prolixcache")
    };
    let mut cache = jev::load_cache(&cache_path);
    let mut pending = Vec::new();
    let (mut checked, mut cached) = (0, 0);
    for (fi, f) in files.iter_mut().enumerate() {
        for (ui, u) in f.units.iter_mut().enumerate() {
            checked += 1;
            if let Some((key, _)) = &u.ask {
                match cache.get(key) {
                    Some(p) => {
                        cached += 1;
                        u.group = decide(p, level, threshold);
                        u.probs = Some(p.clone());
                    }
                    None => pending.push((fi, ui)),
                }
            }
        }
    }

    let (mut tokens, mut unanswered) = (0, 0);
    if !pending.is_empty() {
        let key = std::env::var("TYPESAFE_API_KEY").ok().filter(|k| !k.is_empty()).ok_or_else(|| {
            format!("TYPESAFE_API_KEY is not set; it's needed to judge {} comments (or pass --level none)", pending.len())
        })?;
        if std::io::stderr().is_terminal() {
            eprintln!("Asking Jev about {} comments…", pending.len());
        }
        let questions: Vec<_> = pending
            .iter()
            .map(|&(fi, ui)| {
                let (f, u) = (&files[fi], &files[fi].units[ui]);
                jev::question(
                    head(&f.src[u.start..u.end], MAX_COMMENT),
                    &u.ask.as_ref().unwrap().1,
                    f.lang.name,
                )
            })
            .collect();
        let out = jev::classify(&questions, &key);
        tokens = out.tokens;
        for (&(fi, ui), p) in pending.iter().zip(out.probs) {
            let u = &mut files[fi].units[ui];
            let Some(p) = p else {
                unanswered += 1;
                continue;
            };
            let p: Vec<f32> = p.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
            u.group = decide(&p, level, threshold);
            cache.insert(u.ask.take().unwrap().0, p.clone());
            u.probs = Some(p);
        }
        jev::save_cache(&cache_path, &cache);
        if let Some(e) = out.error {
            return Err(e);
        }
    }

    if unanswered > 0 {
        eprintln!("prolix: Jev gave no answer for {unanswered} comments; they were kept");
    }
    let flagged = |f: &File| -> Vec<(usize, usize)> {
        f.units
            .iter()
            .filter(|u| u.group.is_some())
            .map(|u| (u.start, u.end))
            .collect()
    };
    if fix {
        for f in &files {
            let spans = flagged(f);
            if !spans.is_empty() {
                std::fs::write(&f.path, fix::apply(&f.src, &spans, f.lang.jsx))
                    .map_err(|e| format!("{}: {e}", shown(&f.path)))?;
            }
        }
    }
    let n = scanned.into_inner();
    if reporter == "json" {
        let found: usize = files.iter().map(|f| flagged(f).len()).sum();
        let round = |x: f32| (f64::from(x) * 1000.0).round() / 1000.0;
        let comments: Vec<_> = files
            .iter()
            .flat_map(|f| {
                f.units.iter().map(move |u| {
                    let probs = u.probs.as_ref().map(|p| {
                        jev::CATS
                            .iter()
                            .zip(p)
                            .map(|(c, &x)| (c.name.to_string(), round(x).into()))
                            .collect::<serde_json::Map<_, _>>()
                    });
                    serde_json::json!({
                        "path": shown(&f.path),
                        "line": u.line,
                        "column": u.col,
                        "text": &f.src[u.start..u.end],
                        "group": u.group,
                        "confidence": u.probs.as_ref().map(|p| round(removable(p, level))),
                        "probabilities": probs,
                        "fix": u.group.map(|_| {
                            let (a, b, r) = fix::suggestion(&f.src, (u.start, u.end), f.lang.jsx);
                            serde_json::json!({ "startLine": a, "endLine": b, "replacement": r })
                        }),
                    })
                })
            })
            .collect();
        let report = serde_json::json!({
            "level": LEVELS[level as usize],
            "threshold": round(threshold),
            "model": model,
            "comments": comments,
            "stats": {
                "files": n,
                "comments": checked,
                "flagged": found,
                "asked": pending.len(),
                "cached": cached,
                "unanswered": unanswered,
                "inputTokens": tokens,
                "elapsedMs": t0.elapsed().as_millis() as u64,
            },
        });
        println!("{report}");
        return Ok(i32::from(found > 0 && !fix));
    }

    let md = reporter == "markdown";
    let color = !md && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
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
    for f in &files {
        let spans: Vec<_> = f.units.iter().filter(|u| u.group.is_some()).collect();
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
                paint("4", &shown(&f.path))
            );
        }
        let at: Vec<_> = spans
            .iter()
            .map(|u| format!("{}:{}", u.line, u.col))
            .collect();
        let w = at.iter().map(String::len).max().unwrap_or(0);
        for (i, (u, at)) in spans.iter().zip(&at).enumerate() {
            let g = u.group.unwrap();
            match groups.iter_mut().find(|(n, _)| *n == g) {
                Some((_, c)) => *c += 1,
                None => groups.push((g, 1)),
            }
            let text = preview(&f.src[u.start..u.end]);
            if md && found - spans.len() + i < MD_ROWS {
                let at = format!("{}:{}", shown(&f.path), u.line);
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

    let lvl = LEVELS[level as usize];
    let s = |n: usize| if n == 1 { "" } else { "s" };
    groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let what = |g: &str| {
        jev::CATS
            .iter()
            .find(|c| c.name == g)
            .map_or("any comment that isn't a directive", |c| c.summary)
    };
    let mut stats = format!(
        "Checked {checked} comment{} in {n} file{} in {:.2}s",
        s(checked),
        s(n),
        t0.elapsed().as_secs_f64()
    );
    if checked > 0 && level < 3 {
        let _ = write!(
            stats,
            " · Jev: {} asked, {cached} cached, {}k tokens",
            pending.len(),
            tokens / 1000
        );
    }

    if md {
        let mut m = String::new();
        if found == 0 {
            let _ = writeln!(m, "### ✅ prolix: no comments to remove (level: {lvl})");
        } else {
            let did = if fix { "removed" } else { "found" };
            let rest = if fix { "" } else { " to remove" };
            let _ = writeln!(
                m,
                "### prolix {did} {found} comment{}{rest} (level: {lvl})\n",
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
                let _ = writeln!(m, "\nRun `npx @prolix/cli{again} --fix` to remove them.");
            }
        }
        let _ = writeln!(m, "\n<sub>{stats}</sub>");
        print!("{m}");
        return Ok(i32::from(found > 0 && !fix));
    }

    if found == 0 {
        let _ = writeln!(
            out,
            "{} No comments to remove (level: {lvl}).",
            paint("32", "✓")
        );
    } else {
        let verb = if fix { "Removed" } else { "Found" };
        let gap = if out.is_empty() { "" } else { "\n" };
        let _ = writeln!(
            out,
            "{gap}{verb} {found} comment{} in {found_files} file{} (level: {lvl}):\n",
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
            let _ = writeln!(out, "\nRun `prolix{again} --fix` to remove them.");
        }
    }
    let _ = writeln!(out, "{}", paint("2", &stats));
    print!("{out}");
    Ok(i32::from(found > 0 && !fix))
}

/// A Markdown code span that survives backticks in `s`.
fn code(s: &str) -> String {
    let fence = "`".repeat((1..).find(|&n| !s.contains(&"`".repeat(n))).unwrap());
    format!("{fence} {s} {fence}")
}

fn parse_level(s: &str) -> Result<u8, String> {
    let k: String = s
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    ["all", "valueadd", "necessary", "none"]
        .iter()
        .position(|&l| l == k)
        .map(|p| p as u8)
        .ok_or_else(|| {
            format!("unknown level \"{s}\" (expected all, value-add, necessary or none)")
        })
}

fn read(e: ignore::DirEntry, level: u8, model: &str, scanned: &AtomicUsize) -> Option<File> {
    let lang = lex::lang_for(e.path())?;
    if !e.file_type()?.is_file() || e.metadata().ok()?.len() > MAX_FILE {
        return None;
    }
    let src = std::fs::read_to_string(e.path()).ok()?;
    scanned.fetch_add(1, Relaxed);
    let units = units(&src, lang, level, model);
    (!units.is_empty()).then(|| File {
        path: e.into_path(),
        src,
        lang,
        units,
    })
}

/// Comments worth judging, with runs of whole-line comments merged into one.
fn units(src: &str, lang: &lex::Lang, level: u8, model: &str) -> Vec<Unit> {
    let b = src.as_bytes();
    let mut raws = Vec::new();
    lex::scan(b, lang, 0, &mut raws);
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for r in raws {
        let text = &src[r.start..r.end];
        // ponytail: JSX text such as `a // b</p>` lexes as a comment; skip it rather than parse JSX.
        if is_directive(text, r.start)
            || (lang.jsx && r.line && (text.contains("</") || text.contains("/>")))
        {
            continue;
        }
        let ls = fix::line_start(b, r.start);
        let alone = r.line && b[ls..r.start].iter().all(|c| c.is_ascii_whitespace());
        match spans.last_mut() {
            Some(last) if alone && last.2 && fix::line_end(b, last.1) + 1 == ls => last.1 = r.end,
            _ => spans.push((r.start, r.end, alone)),
        }
    }
    if spans.is_empty() {
        return Vec::new();
    }
    let newlines: Vec<usize> = b
        .iter()
        .enumerate()
        .filter(|(_, &c)| c == b'\n')
        .map(|(i, _)| i)
        .collect();
    spans
        .into_iter()
        .map(|(start, end, _)| {
            let text = &src[start..end];
            let line = newlines.partition_point(|&p| p < start);
            let ls = if line == 0 { 0 } else { newlines[line - 1] + 1 };
            let (group, ask) = if !text.chars().any(char::is_alphanumeric) {
                (Some("decorative"), None)
            } else if level == 3 {
                (Some("comment"), None)
            } else {
                let code = context(src, start, end);
                let key = jev::hash(&[model, jev::PROMPT_VERSION, lang.name, text, &code]);
                (None, Some((format!("{key:016x}"), code)))
            };
            Unit {
                start,
                end,
                line: line + 1,
                col: src[ls..start].chars().count() + 1,
                group,
                probs: None,
                ask,
            }
        })
        .collect()
}

fn is_directive(text: &str, start: usize) -> bool {
    if (start == 0 && text.starts_with("#!")) || text.starts_with("/*!") || text.starts_with("{-#")
    {
        return true;
    }
    let t = text.to_ascii_lowercase();
    let body = t.trim_start_matches(|c: char| c.is_whitespace() || "/*#-;!<{([=".contains(c));
    DIRECTIVES.iter().any(|d| t.contains(d))
        || DIRECTIVE_PREFIXES.iter().any(|d| body.starts_with(d))
}

/// Probability that the comment belongs to a category this level removes.
fn removable(p: &[f32], level: u8) -> f32 {
    jev::CATS
        .iter()
        .zip(p)
        .filter(|(c, _)| c.level <= level)
        .map(|(_, &x)| x)
        .sum()
}

fn decide(p: &[f32], level: u8, threshold: f32) -> Option<&'static str> {
    if removable(p, level) < threshold {
        return None;
    }
    jev::CATS
        .iter()
        .zip(p)
        .filter(|(c, _)| c.level <= level)
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(c, _)| c.name)
}

fn shown(path: &Path) -> String {
    path.strip_prefix(".").unwrap_or(path).display().to_string()
}

/// Two lines before the comment and eight after, with the comment itself replaced by a marker.
fn context(src: &str, start: usize, end: usize) -> String {
    let b = src.as_bytes();
    let mut a = fix::line_start(b, start);
    for _ in 0..2 {
        if a > 0 {
            a = fix::line_start(b, a - 1);
        }
    }
    let mut z = end;
    for _ in 0..9 {
        z = (fix::line_end(b, z) + 1).min(b.len());
    }
    format!(
        "{}<<COMMENT>>{}",
        tail(&src[a..start], 300),
        head(src[end..z].trim_end(), 700)
    )
}

fn head(s: &str, n: usize) -> &str {
    let mut k = n.min(s.len());
    while !s.is_char_boundary(k) {
        k -= 1;
    }
    &s[..k]
}

fn tail(s: &str, n: usize) -> &str {
    let mut k = s.len().saturating_sub(n);
    while !s.is_char_boundary(k) {
        k += 1;
    }
    &s[k..]
}

fn preview(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("");
    let mut p: String = first.chars().take(80).collect();
    if p.len() < first.len() || lines.next().is_some() {
        p.push_str(" …");
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directives_and_merging() {
        assert!(is_directive("// eslint-disable-next-line no-console", 5));
        assert!(is_directive("#!/usr/bin/env node", 0));
        assert!(is_directive("/// <reference types=\"vite/client\" />", 0));
        assert!(!is_directive("// set the global counter", 0));
        let src = "a\n// one\n// two\n// eslint-disable-next-line\n// three\nb // four\n// ----\n";
        let u = units(src, &lex::TS, 1, "m");
        let texts: Vec<_> = u.iter().map(|u| &src[u.start..u.end]).collect();
        assert_eq!(texts, ["// one\n// two", "// three", "// four", "// ----"]);
        assert_eq!(
            (u[0].line, u[2].col, u[3].group),
            (2, 3, Some("decorative"))
        );
    }

    #[test]
    fn decide_by_level() {
        let mut p = vec![0.0; jev::CATS.len()];
        p[0] = 0.4;
        p[6] = 0.5;
        assert_eq!(decide(&p, 1, 0.6), None);
        assert_eq!(decide(&p, 2, 0.6), Some("clarifies"));
        assert_eq!(parse_level("Value Add"), Ok(1));
    }
}
