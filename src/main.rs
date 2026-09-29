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

Usage: prolix [paths...] [--fix] [--changed[=<ref>]] [--mode <mode>] [--reporter <reporter>]

Options:
  --fix                    Remove the flagged comments
  --changed[=<ref>]        Only check comments on lines added since <ref>   [default: HEAD]
  --mode <mode>            off | standard | strict   [default: standard]
  --reporter <reporter>    text | json | markdown   [default: text]
  -h, --help               Print help
  -V, --version            Print version

Modes:
  off        keep every comment that no rule removes
  standard   remove comments that restate the code, disabled code, banners, change notes and signature-only docs
  strict     also remove TODOs and comments that only summarise what code does

Settings, including keep and remove rules in plain English, are read from prolix.jsonc in this directory or a parent.
Asking Jev needs TYPESAFE_API_KEY.
";

const MODES: [&str; 3] = ["off", "standard", "strict"];
const MAX_FILE: u64 = 1 << 20;
const MAX_COMMENT: usize = 2000;
/// Findings listed in a Markdown report, which keeps a PR comment under GitHub's 65k-character limit.
const MD_ROWS: usize = 200;

/// Words tools use to switch a check off or back on, as in `eslint-disable`, `ignore=DL3008` or `c8 ignore`.
const VERBS: &[&str] = &[
    "disable", "enable", "ignore", "expect", "suppress", "skip", "allow", "nocheck", "off", "on",
    "restore",
];
/// Words that narrow a verb, as in `c8 ignore next` or `ReSharper disable once`.
const SCOPES: &[&str] = &[
    "next", "line", "file", "start", "end", "else", "if", "once", "all",
];
/// Licences, generated-file markers and bundler annotations, kept wherever they appear.
const MARKERS: &[&str] = &[
    "prolix-ignore",
    "copyright",
    "spdx-license-identifier",
    "@license",
    "@preserve",
    "@generated",
    "do not edit",
    "__pure__",
    "__no_side_effects__",
    "-*-",
    "<reference ",
    "<amd-module",
];
/// Language keywords, kept at the start of a line.
const KEYWORDS: &[&str] = &[
    "pragma",
    "region",
    "endregion",
    "+build",
    "global ",
    "globals ",
    "exported ",
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
    let (mut paths, mut fix, mut mode, mut reporter) = (Vec::new(), false, None, None);
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
            "--mode" | "--level" => {
                if a == "--level" {
                    eprintln!("prolix: --level is deprecated; use --mode");
                }
                mode = Some(args.next().ok_or(format!("{a} needs a value"))?);
                again = again + " " + mode.as_deref().unwrap();
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
            _ if a.starts_with("--mode=") => mode = Some(a["--mode=".len()..].to_string()),
            _ if a.starts_with("--level=") => {
                eprintln!("prolix: --level is deprecated; use --mode");
                mode = Some(a["--level=".len()..].to_string())
            }
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
    if cfg.level.is_some() {
        eprintln!("prolix: the \"level\" setting is deprecated; use \"mode\"");
    }
    let mode = parse_mode(
        mode.as_deref()
            .or(cfg.mode.as_deref())
            .or(cfg.level.as_deref())
            .unwrap_or("standard"),
    )?;
    let policy = Policy::new(mode, &cfg.keep, &cfg.remove);
    let threshold = cfg.threshold.unwrap_or(0.6);
    let added = match changed.as_deref() {
        Some(base) => {
            if let Some(p) = paths.iter().find(|p| !Path::new(p).exists()) {
                return Err(format!(
                    "{p}: no such file or directory (to compare with a ref, use --changed=<ref>)"
                ));
            }
            Some(diff::added(base, &paths)?)
        }
        _ => None,
    };
    let targets: Vec<String> = match &added {
        Some(a) => a.keys().cloned().collect(),
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
                        if let Some(f) = read(e, &policy, &model, &scanned) {
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
                        u.group = policy.decide(p, threshold, f.lang.config());
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
            format!("TYPESAFE_API_KEY is not set; it's needed to judge {} comments (or pass --mode off)", pending.len())
        })?;
        if std::io::stderr().is_terminal() {
            eprintln!("Asking Jev about {} comments…", pending.len());
        }
        let criteria = jev::criteria(&policy.rules);
        let questions: Vec<_> = pending
            .iter()
            .map(|&(fi, ui)| {
                let (f, u) = (&files[fi], &files[fi].units[ui]);
                jev::question(
                    head(&f.src[u.start..u.end], MAX_COMMENT),
                    &u.ask.as_ref().unwrap().1,
                    f.lang.name,
                    &criteria,
                )
            })
            .collect();
        let out = jev::classify(&questions, &policy.names, &key);
        tokens = out.tokens;
        for (&(fi, ui), p) in pending.iter().zip(out.probs) {
            let config = files[fi].lang.config();
            let u = &mut files[fi].units[ui];
            let Some(p) = p else {
                unanswered += 1;
                continue;
            };
            let p: Vec<f32> = p.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
            u.group = policy.decide(&p, threshold, config);
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
        let policy = &policy;
        let comments: Vec<_> = files
            .iter()
            .flat_map(|f| {
                f.units.iter().map(move |u| {
                    let probs = u.probs.as_ref().map(|p| {
                        policy
                            .names
                            .iter()
                            .zip(p)
                            .map(|(n, &x)| (n.to_string(), round(x).into()))
                            .collect::<serde_json::Map<_, _>>()
                    });
                    serde_json::json!({
                        "path": shown(&f.path),
                        "line": u.line,
                        "column": u.col,
                        "text": &f.src[u.start..u.end],
                        "group": u.group,
                        "rule": u.group.and_then(|g| policy.rule(g)),
                        "confidence": u.probs.as_ref().map(|p| round(policy.removable(p, f.lang.config()))),
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
            "mode": MODES[mode as usize],
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

    let mode = MODES[mode as usize];
    let s = |n: usize| if n == 1 { "" } else { "s" };
    groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let what = |g: &str| {
        jev::CATS
            .iter()
            .find(|c| c.name == g)
            .map(|c| c.summary)
            .or_else(|| policy.rule(g))
            .unwrap_or_default()
    };
    let mut stats = format!(
        "Checked {checked} comment{} in {n} file{} in {:.2}s",
        s(checked),
        s(n),
        t0.elapsed().as_secs_f64()
    );
    if !pending.is_empty() || cached > 0 {
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
            let _ = writeln!(m, "### ✅ prolix: no comments to remove (mode: {mode})");
        } else {
            let did = if fix { "removed" } else { "found" };
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
            "{} No comments to remove (mode: {mode}).",
            paint("32", "✓")
        );
    } else {
        let verb = if fix { "Removed" } else { "Found" };
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

/// A mode's index in `MODES`, also accepting the names the modes had as levels.
fn parse_mode(s: &str) -> Result<u8, String> {
    let k: String = s
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let old = match k.as_str() {
        "all" => Some(0),
        "valueadd" => Some(1),
        "necessary" => Some(2),
        "none" => return Err("mode \"none\" was removed; to remove every comment, set \"mode\": \"strict\" and \"remove\": [\"explains-why\", \"warning\", \"api-doc\", \"reference\"] in prolix.jsonc".into()),
        _ => None,
    };
    if let Some(m) = old {
        eprintln!("prolix: \"{s}\" is deprecated; use \"{}\"", MODES[m]);
        return Ok(m as u8);
    }
    MODES
        .iter()
        .position(|&m| m == k)
        .map(|p| p as u8)
        .ok_or_else(|| format!("unknown mode \"{s}\" (expected off, standard or strict)"))
}

fn read(e: ignore::DirEntry, policy: &Policy, model: &str, scanned: &AtomicUsize) -> Option<File> {
    let lang = lex::lang_for(e.path())?;
    if !e.file_type()?.is_file() || e.metadata().ok()?.len() > MAX_FILE {
        return None;
    }
    let src = std::fs::read_to_string(e.path()).ok()?;
    scanned.fetch_add(1, Relaxed);
    let units = units(&src, lang, policy, model);
    (!units.is_empty()).then(|| File {
        path: e.into_path(),
        src,
        lang,
        units,
    })
}

/// Comments worth judging, with runs of whole-line comments merged into one.
fn units(src: &str, lang: &lex::Lang, policy: &Policy, model: &str) -> Vec<Unit> {
    let b = src.as_bytes();
    let mut raws = Vec::new();
    lex::scan(b, lang, 0, &mut raws);
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    let (mut head, mut prev) = (true, 0);
    for r in raws {
        let text = &src[r.start..r.end];
        head &= b
            .get(prev..r.start)
            .is_some_and(|s| s.iter().all(u8::is_ascii_whitespace));
        prev = r.end;
        // ponytail: JSX text such as `a // b</p>` lexes as a comment; skip it rather than parse JSX.
        if is_directive(text, head)
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
    // With nothing to remove, as in mode "off" without remove rules, comments are only counted.
    let judge = policy.removes.iter().flatten().any(|&r| r);
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
                let decorative = policy.removes[usize::from(lang.config())][2];
                (decorative.then_some(jev::CATS[2].name), None)
            } else if !judge {
                (None, None)
            } else {
                let code = context(src, start, end);
                let mut parts = vec![model, jev::PROMPT_VERSION, lang.name, text, &code];
                if !policy.key.is_empty() {
                    parts.push(&policy.key);
                }
                (None, Some((format!("{:016x}", jev::hash(&parts)), code)))
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

/// Tool, compiler and licence comments, which are kept at every level. Tools are recognised by the shape of what
/// they read, so a new linter's `x-disable-next-line rule` is kept without prolix knowing its name. `head` is set
/// for comments before any code, where shebangs and file-wide settings live.
fn is_directive(text: &str, head: bool) -> bool {
    if (head && text.starts_with("#!")) || text.starts_with("/*!") || text.starts_with("{-#") {
        return true;
    }
    let t = text.to_ascii_lowercase();
    let inline = !text.contains('\n') && (head || text.starts_with("/*"));
    MARKERS.iter().any(|m| t.contains(m))
        || text.lines().any(|l| {
            let l = l.trim_matches(|c: char| c.is_whitespace() || "/*#-;!<>{}()[]=".contains(c));
            KEYWORDS
                .iter()
                .any(|k| l.to_ascii_lowercase().starts_with(k))
                || directive_line(l, inline)
        })
}

/// One line of a comment, with its markers trimmed. Directives open with a tool's keyword and go on in rule names,
/// codes or settings; prose goes on in plain words.
fn directive_line(l: &str, inline: bool) -> bool {
    let w: Vec<&str> = l.split_whitespace().collect();
    let Some(&t0) = w.first() else { return false };
    // A plain word, as opposed to a rule name, code, path or setting.
    let prose = |s: &str| {
        let s = s.trim_matches(|c: char| ",.;:!?\"'".contains(c));
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphabetic() || c == '\'')
            && !s[1..].chars().any(|c| c.is_ascii_uppercase())
    };
    // Nothing more, or a rule name or code after any scope words.
    let args = |rest: &[&str]| {
        let rest: Vec<_> = rest
            .iter()
            .skip_while(|r| SCOPES.contains(&&*r.to_ascii_lowercase()))
            .collect();
        rest.first().is_none_or(|r| !prose(r))
    };
    // `@ts-ignore`, `@flow`, `@jsx h`, `@type {Foo}`, `$FlowFixMe`. A plain tag followed by prose is documentation.
    if let Some(tag) = t0.strip_prefix(['@', '$']) {
        let name = tag.split(['=', '(']).next().unwrap_or("");
        if name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c))
        {
            return !prose(tag) || w.len() <= 2 || w[1].starts_with('{');
        }
    }
    // `noqa: E501`, `nosec`, `NOLINTNEXTLINE(rule)`, `nolint:errcheck`.
    let head = t0.split(|c: char| ":([=".contains(c)).next().unwrap_or("");
    let rest = head.get(2..).unwrap_or("");
    if head.len() > 3
        && ((head.starts_with("no") && rest.bytes().all(|b| b.is_ascii_lowercase()))
            || (head.starts_with("NO") && rest.bytes().all(|b| b.is_ascii_uppercase())))
        && args(&w[1..])
    {
        return true;
    }
    // `rubocop:disable`, `go:generate`, `cspell:words`, `CHECKSTYLE:OFF`.
    if let Some((a, b)) = t0.split_once(':') {
        if a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && b.starts_with(|c: char| c.is_ascii_alphabetic())
            && (a.starts_with(|c: char| c.is_ascii_lowercase()) || t0 == t0.to_ascii_uppercase())
        {
            return true;
        }
    }
    // `eslint-disable-next-line rule`, `c8 ignore next`, `shellcheck disable=SC2086`, `fmt: off`. A spaced verb
    // needs a tool before it, which prose's capitalised first word isn't.
    let tool =
        !t0.contains('\'') && !(t0.starts_with(|c: char| c.is_ascii_uppercase()) && prose(t0));
    for (k, tok) in w.iter().take(2).enumerate() {
        let segs: Vec<String> = tok
            .to_ascii_lowercase()
            .split(|c: char| "-:=_([".contains(c))
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if segs.iter().any(|s| VERBS.contains(&s.as_str()))
            && (if k == 0 { segs.len() > 1 } else { tool })
            && args(&w[k + 1..])
        {
            return true;
        }
    }
    // Settings: `syntax=docker/dockerfile:1`, `shellcheck shell=bash`, `renovate: datasource=docker`, and in a file's
    // header or a one-line block comment `frozen_string_literal: true`, `webpackChunkName: "x"` or `jshint esversion: 6`.
    // Elsewhere a lone `key: value` is more likely commented-out YAML or an object literal. Each key may take one word.
    let key = |s: &str| {
        s.starts_with(|c: char| c.is_ascii_lowercase() || c == '$')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.$".contains(c))
    };
    let pair = |s: &str| {
        s.split_once('=')
            .is_some_and(|(a, b)| key(a) && !b.is_empty())
    };
    let label = |s: &str| s.strip_suffix(':').is_some_and(key);
    let k =
        usize::from(w.len() > 1 && prose(t0) && !t0.ends_with(':') && (label(w[1]) || pair(w[1])));
    if pair(w[k]) || (label(w[k]) && (inline || w[k + 1..].iter().any(|s| pair(s)))) {
        let keys = w[k..].iter().filter(|s| label(s)).count();
        if w[k + 1..].iter().filter(|s| prose(s)).count() <= keys {
            return true;
        }
    }
    // A version kept beside a pinned hash, as in `uses: actions/checkout@<sha> # v4.1.1`, which update bots rewrite.
    let v = t0.trim_start_matches(['v', 'V']);
    w.len() == 1
        && v.starts_with(|c: char| c.is_ascii_digit())
        && (v.len() < t0.len() || v.contains('.'))
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
}

/// The criteria Jev chooses between, and which of them the mode and the `keep` and `remove` lists remove.
struct Policy {
    /// `jev::CATS`, then each plain-English rule, in the order of Jev's probabilities.
    names: Vec<&'static str>,
    /// Plain-English rules as (name, text): `keep-1`, `keep-2`, …, then `remove-1`, ….
    rules: Vec<(&'static str, &'static str)>,
    /// Which of `names` are removed, in code and then in configuration files.
    removes: [Vec<bool>; 2],
    /// Joins every cache key when there are rules, since they change the question.
    key: String,
}

impl Policy {
    fn new(mode: u8, keep: &[String], remove: &[String]) -> Policy {
        let listed = |list: &[String], name: &str| list.iter().any(|r| r == name);
        let mut rules = Vec::new();
        for (verb, list) in [("keep", keep), ("remove", remove)] {
            let english = list
                .iter()
                .filter(|r| !jev::CATS.iter().any(|c| c.name == *r));
            for (i, r) in english.enumerate() {
                // Leaked once per run, so rules can name groups like the built-in categories.
                let name: &'static str = format!("{verb}-{}", i + 1).leak();
                rules.push((name, &*r.clone().leak()));
            }
        }
        let removes = [false, true].map(|config| {
            jev::CATS
                .iter()
                .map(|c| {
                    !listed(keep, c.name)
                        && (jev::mode(c, config) <= mode || listed(remove, c.name))
                })
                .chain(rules.iter().map(|r| r.0.starts_with("remove")))
                .collect()
        });
        let parts: Vec<&str> = rules.iter().flat_map(|&(n, t)| [n, t]).collect();
        Policy {
            names: jev::CATS
                .iter()
                .map(|c| c.name)
                .chain(rules.iter().map(|r| r.0))
                .collect(),
            key: if rules.is_empty() {
                String::new()
            } else {
                format!("{:016x}", jev::hash(&parts))
            },
            rules,
            removes,
        }
    }

    /// Probability that the comment is one this run removes.
    fn removable(&self, p: &[f32], config: bool) -> f32 {
        self.removes[usize::from(config)]
            .iter()
            .zip(p)
            .filter(|(&r, _)| r)
            .map(|(_, &x)| x)
            .sum()
    }

    fn decide(&self, p: &[f32], threshold: f32, config: bool) -> Option<&'static str> {
        if self.removable(p, config) < threshold {
            return None;
        }
        self.names
            .iter()
            .zip(&self.removes[usize::from(config)])
            .zip(p)
            .filter(|((_, &r), _)| r)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|((&n, _), _)| n)
    }

    /// The text of the rule a group is named after.
    fn rule(&self, group: &str) -> Option<&'static str> {
        self.rules.iter().find(|r| r.0 == group).map(|r| r.1)
    }
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
        assert!(is_directive("#!/usr/bin/env node", true));
        assert!(is_directive(
            "/// <reference types=\"vite/client\" />",
            false
        ));
        assert!(is_directive("# frozen_string_literal: true", true));
        assert!(!is_directive("# timeout: 30", false));
        assert!(!is_directive("// set the global counter", false));
        // Kept by shape alone: none of these tools is named in prolix.
        for d in [
            "// eslint-disable-next-line no-console",
            "// react-doctor-disable-next-line react-doctor/no-array-index-key -- stable order",
            "{/* react-doctor-disable-line */}",
            "// biome-ignore lint/suspicious/noExplicitAny: third-party callback shape",
            "// @ts-expect-error window.acme is injected by the host page",
            "/* c8 ignore next */",
            "/* istanbul ignore else */",
            "# noqa: E501",
            "# type: ignore[attr-defined]",
            "# pylint: disable=invalid-name",
            "# fmt: off",
            "# pragma: no cover",
            "// NOLINTNEXTLINE(bugprone-use-after-move)",
            "//nolint:errcheck",
            "# nosec B101",
            "// NOSONAR",
            "# shellcheck disable=SC2086",
            "# shellcheck source=./lib.sh",
            "# hadolint ignore=DL3008",
            "# yamllint disable-line rule:line-length",
            "# rubocop:disable Style/GuardClause",
            "# tfsec:ignore:aws-s3-enable-bucket-logging",
            "# checkov:skip=CKV_AWS_20: public site",
            "// gitleaks:allow",
            "// clang-format off",
            "// ReSharper disable once InconsistentNaming",
            "// swiftlint:disable:next force_cast",
            "// @formatter:off",
            "//go:generate stringer -type=Pill",
            "/* @vite-ignore */",
            "/* webpackChunkName: \"admin\", webpackPrefetch: true */",
            "/* jshint esversion: 6 */",
            "# syntax=docker/dockerfile:1",
            "# yaml-language-server: $schema=https://json.schemastore.org/github-workflow.json",
            "# renovate: datasource=docker depName=nginx",
            "# v4.1.1",
            "// @flow",
            "/** @jsx h */",
            "/**\n * Adds.\n * @param {number} a\n */",
            "/* #__PURE__ */",
            "// #region Helpers",
        ] {
            assert!(is_directive(d, false), "{d}");
        }
        for p in [
            "// Please ignore this",
            "// We disable caching here",
            "// Re-enable the button once loaded",
            "// just ignore it",
            "// nothing to do here",
            "// TODO: fix this",
            "// Note: this is slow",
            "// @param x the count",
            "// Don't re-enable",
            "# x=5 is the limit we chose",
            "// see https://example.com/docs",
            "// localhost:3000 is the dev server",
            "// one-off script for the migration",
            "# - uses: actions/cache@v4",
            "// @pqina/flip builds the panels itself",
            "/**\n * ![icon](data:image/svg+xml;utf-8,%3Csvg)\n */",
            "/*\n * (anyvan monolith: templates/amp.tpl)\n */",
        ] {
            assert!(!is_directive(p, false), "{p}");
        }
        let src = "a\n// one\n// two\n// eslint-disable-next-line\n// three\nb // four\n// ----\n";
        let u = units(src, &lex::TS, &Policy::new(1, &[], &[]), "m");
        let texts: Vec<_> = u.iter().map(|u| &src[u.start..u.end]).collect();
        assert_eq!(texts, ["// one\n// two", "// three", "// four", "// ----"]);
        assert_eq!(
            (u[0].line, u[2].col, u[3].group),
            (2, 3, Some("decorative"))
        );
    }

    #[test]
    fn decide_by_mode_and_rules() {
        let (standard, strict) = (Policy::new(1, &[], &[]), Policy::new(2, &[], &[]));
        let mut p = vec![0.0; jev::CATS.len()];
        p[0] = 0.4;
        p[6] = 0.5;
        assert_eq!(standard.decide(&p, 0.6, false), None);
        assert_eq!(strict.decide(&p, 0.6, false), Some("clarifies"));
        p[0] = 0.9;
        assert_eq!(standard.decide(&p, 0.6, false), Some("restates-code"));
        assert_eq!(standard.decide(&p, 0.6, true), None);
        assert_eq!(parse_mode("Value Add"), Ok(1));
        assert_eq!(parse_mode("strict"), Ok(2));
        assert!(parse_mode("none").is_err());

        // Categories named in a list switch on or off; anything else becomes a criterion of its own.
        let keep = [
            "clarifies".to_string(),
            "States a fact the code relies on".to_string(),
        ];
        let rules = Policy::new(2, &keep, &["todo".into(), "Reassures a reviewer".into()]);
        assert_eq!(&rules.names[11..], ["keep-1", "remove-1"]);
        assert_eq!(rules.rule("remove-1"), Some("Reassures a reviewer"));
        assert!(!rules.key.is_empty() && strict.key.is_empty());
        let mut p = vec![0.0; 13];
        p[6] = 0.5;
        p[12] = 0.5;
        assert_eq!(rules.decide(&p, 0.6, false), None);
        p[6] = 0.3;
        p[12] = 0.7;
        assert_eq!(rules.decide(&p, 0.6, false), Some("remove-1"));
        p[12] = 0.0;
        p[11] = 0.7;
        assert_eq!(rules.decide(&p, 0.6, false), None);
        assert!(Policy::new(0, &[], &["todo".into()]).removes[0][5]);
    }
}
