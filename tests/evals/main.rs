//! Live evals of Jev's judgements. Each file in `fixtures/` has its comments labelled in
//! `cases.json` with every category a careful reviewer would accept, and prolix is run
//! against it end to end.
//!
//!     cargo test --release --test evals -- --ignored --nocapture
//!
//! Needs TYPESAFE_API_KEY. EVAL_REPEAT (1-5, default 3) sets how many times each fixture
//! is judged, and EVAL_BASELINE names an earlier report.json to compare against. Reports
//! are written to `results/<run>/`.

#[allow(dead_code)]
#[path = "../../src/jev.rs"]
mod jev;

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Fixture file → comment substring → acceptable categories, or `["directive"]`.
type Labels = BTreeMap<String, BTreeMap<String, Vec<String>>>;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/evals");
const LEVELS: [(&str, u8); 2] = [("value-add", 1), ("necessary", 2)];
const SWEEP: [f64; 5] = [0.4, 0.5, 0.6, 0.7, 0.8];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    Remove,
    Keep,
    /// The acceptable categories fall on both sides of the level, so either is right.
    Either,
}

struct Decision {
    file: String,
    find: String,
    cats: Vec<String>,
    level: usize,
    expect: Expect,
    flagged: bool,
    confidence: f64,
    top: String,
}

#[derive(Default)]
struct Score {
    tp: usize,
    fp: usize,
    fn_: usize,
}

impl Score {
    fn precision(&self) -> f64 {
        ratio(self.tp, self.tp + self.fp)
    }
    fn recall(&self) -> f64 {
        ratio(self.tp, self.tp + self.fn_)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        1.0
    } else {
        a as f64 / b as f64
    }
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn expect(cats: &[String], level: u8) -> Expect {
    let removed = cats
        .iter()
        .filter(|c| {
            let cat = jev::CATS.iter().find(|k| k.name == c.as_str());
            cat.unwrap_or_else(|| panic!("unknown category {c:?} in cases.json"))
                .level
                <= level
        })
        .count();
    match removed {
        0 => Expect::Keep,
        n if n == cats.len() => Expect::Remove,
        _ => Expect::Either,
    }
}

/// Turns one run's reports (one per level) into decisions and returns any contract breaks:
/// a label that doesn't match exactly one comment, a comment nobody labelled, a directive
/// that reached Jev, or a comment Jev never answered.
fn grade(
    file: &str,
    labels: &BTreeMap<String, Vec<String>>,
    reports: &[Value],
    out: &mut Vec<Decision>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (li, report) in reports.iter().enumerate() {
        let comments = report["comments"].as_array().map_or(&[][..], Vec::as_slice);
        let text = |c: &Value| c["text"].as_str().unwrap_or_default().to_string();
        if report["stats"]["unanswered"].as_u64() != Some(0) {
            problems.push(format!("{file}: Jev left comments unanswered"));
        }
        for c in comments {
            if !labels.keys().any(|f| text(c).contains(f.as_str())) {
                problems.push(format!("{file}: unlabelled comment {:?}", text(c)));
            }
        }
        for (find, cats) in labels {
            let hits: Vec<_> = comments
                .iter()
                .filter(|c| text(c).contains(find.as_str()))
                .collect();
            if cats.iter().any(|c| c == "directive") {
                if !hits.is_empty() {
                    problems.push(format!("{file}: directive {find:?} was judged"));
                }
                continue;
            }
            let [c] = hits[..] else {
                problems.push(format!("{file}: {find:?} matched {} comments", hits.len()));
                continue;
            };
            let flagged = !c["group"].is_null();
            let top = c["probabilities"]
                .as_object()
                .and_then(|p| {
                    let best = p
                        .iter()
                        .max_by(|a, b| a.1.as_f64().partial_cmp(&b.1.as_f64()).unwrap());
                    best.map(|(k, _)| k.clone())
                })
                .unwrap_or_else(|| c["group"].as_str().unwrap_or("none").to_string());
            out.push(Decision {
                file: file.into(),
                find: find.clone(),
                cats: cats.clone(),
                level: li,
                expect: expect(cats, LEVELS[li].1),
                flagged,
                // Comments without letters are flagged locally and have no confidence.
                confidence: c["confidence"]
                    .as_f64()
                    .unwrap_or(if flagged { 1.0 } else { 0.0 }),
                top,
            });
        }
    }
    problems
}

fn score<'a>(ds: impl Iterator<Item = &'a Decision>, flagged: impl Fn(&Decision) -> bool) -> Score {
    let mut s = Score::default();
    for d in ds {
        match (flagged(d), d.expect) {
            (true, Expect::Remove) => s.tp += 1,
            (true, Expect::Keep) => s.fp += 1,
            (false, Expect::Remove) => s.fn_ += 1,
            _ => {}
        }
    }
    s
}

/// Share of comments whose decision at `level` differed between repeats.
fn flip_rate(ds: &[Decision], level: usize) -> f64 {
    let mut seen: BTreeMap<(&str, &str), (bool, bool)> = BTreeMap::new();
    for d in ds.iter().filter(|d| d.level == level) {
        let e = seen.entry((&d.file, &d.find)).or_default();
        if d.flagged {
            e.0 = true;
        } else {
            e.1 = true;
        }
    }
    ratio(seen.values().filter(|(a, b)| *a && *b).count(), seen.len())
}

/// Runs prolix on one fixture in a fresh directory, so nothing comes from an earlier
/// cache. The second level reuses the first level's answers from that run's cache.
fn run(file: &str, src: &str, repeat: usize) -> Result<Vec<Value>, String> {
    let tmp = std::env::temp_dir().join(format!(
        "prolix-eval-{}-{repeat}-{file}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    std::fs::write(tmp.join(file), src).map_err(|e| e.to_string())?;
    // Stops the config search from finding a prolix.jsonc above the temp directory.
    std::fs::write(tmp.join("prolix.jsonc"), "{}").map_err(|e| e.to_string())?;
    let reports = LEVELS
        .iter()
        .map(|(level, _)| {
            let out = Command::new(env!("CARGO_BIN_EXE_prolix"))
                .args(["--reporter", "json", "--level", level])
                .current_dir(&tmp)
                .output()
                .map_err(|e| e.to_string())?;
            if !matches!(out.status.code(), Some(0 | 1)) {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&tmp);
    reports
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(DIR)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[test]
#[ignore = "calls Jev; needs TYPESAFE_API_KEY"]
fn evals() {
    let dir = Path::new(DIR);
    let cases = read(dir.join("cases.json"));
    let labels: Labels = serde_json::from_str(&cases).expect("cases.json");
    let policy: Value = serde_json::from_str(&read(dir.join("policy.json"))).expect("policy.json");
    let repeat: usize = std::env::var("EVAL_REPEAT").map_or(3, |v| v.parse().expect("EVAL_REPEAT"));
    assert!((1..=5).contains(&repeat), "EVAL_REPEAT must be 1-5");
    assert!(
        std::env::var_os("TYPESAFE_API_KEY").is_some(),
        "TYPESAFE_API_KEY is not set"
    );

    let fixtures: Vec<(String, String)> = labels
        .keys()
        .map(|f| (f.clone(), read(dir.join("fixtures").join(f))))
        .collect();
    let mut gates = Vec::new();

    let mut parts = vec![cases.as_str()];
    parts.extend(fixtures.iter().flat_map(|(f, s)| [f.as_str(), s.as_str()]));
    let dataset = format!("{:016x}", jev::hash(&parts));
    if policy["datasetHash"] != dataset.as_str() {
        gates.push(format!(
            "the dataset changed (now {dataset}): review the labels, then update policy.json"
        ));
    }
    // The prompt's own examples would make the eval an exam with the answers attached.
    for e in jev::CATS.iter().flat_map(|c| c.examples) {
        let body = e
            .trim_start_matches(['/', '*', '#', ' '])
            .trim_end_matches(['/', '*', ' '])
            .to_lowercase();
        for (f, src) in &fixtures {
            if body.chars().any(char::is_alphanumeric) && src.to_lowercase().contains(&body) {
                gates.push(format!("{f} reuses the prompt example {e:?}"));
            }
        }
    }

    let (mut decisions, mut problems, mut ms) = (Vec::new(), Vec::new(), Vec::new());
    let (mut tokens, mut model, mut threshold) = (0, String::new(), 0.0);
    for r in 0..repeat {
        let runs: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = fixtures
                .iter()
                .map(|(f, src)| s.spawn(move || run(f, src, r)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for ((f, _), res) in fixtures.iter().zip(runs) {
            match res {
                Err(e) => problems.push(format!("{f}: {e}")),
                Ok(reports) => {
                    tokens += reports
                        .iter()
                        .map(|r| r["stats"]["inputTokens"].as_u64().unwrap_or(0))
                        .sum::<u64>();
                    ms.push(reports[0]["stats"]["elapsedMs"].as_u64().unwrap_or(0));
                    model = reports[0]["model"].as_str().unwrap_or_default().to_string();
                    threshold = reports[0]["threshold"].as_f64().unwrap_or_default();
                    problems.extend(grade(f, &labels[f], &reports, &mut decisions));
                }
            }
        }
    }
    problems.sort();
    problems.dedup();
    gates.extend(problems);
    ms.sort_unstable();
    let pct = |p: f64| {
        ms.get(((ms.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0)
    };

    let baseline: Option<Value> = std::env::var("EVAL_BASELINE")
        .ok()
        .map(|p| serde_json::from_str(&read(p)).expect("EVAL_BASELINE"));
    let max_drop = policy["maximumBaselineDrop"].as_f64().unwrap_or(0.0);
    let max_flips = policy["maximumFlipRate"].as_f64().unwrap_or(0.0);
    let mut levels = Map::new();
    for (li, (name, _)) in LEVELS.iter().enumerate() {
        let at = || decisions.iter().filter(move |d| d.level == li);
        let s = score(at(), |d| d.flagged);
        let flips = flip_rate(&decisions, li);
        let floor = &policy["floors"][name];
        for (metric, value) in [("precision", s.precision()), ("recall", s.recall())] {
            let min = floor[metric].as_f64().unwrap_or(1.0);
            if value < min {
                gates.push(format!(
                    "{name} {metric} {value:.3} is below the floor of {min}"
                ));
            }
            if let Some(was) = baseline
                .as_ref()
                .and_then(|b| b["levels"][name][metric].as_f64())
            {
                if was - value > max_drop {
                    gates.push(format!(
                        "{name} {metric} fell from {was:.3} to {value:.3} against the baseline"
                    ));
                }
            }
        }
        if flips > max_flips {
            gates.push(format!("{name} flip rate {flips:.3} is above {max_flips}"));
        }
        let sweep: Vec<_> = SWEEP
            .iter()
            .map(|&t| {
                let s = score(at(), |d| d.confidence >= t);
                json!({ "threshold": t, "precision": r3(s.precision()), "recall": r3(s.recall()) })
            })
            .collect();
        levels.insert(
            name.to_string(),
            json!({
                "precision": r3(s.precision()), "recall": r3(s.recall()),
                "wrongRemovals": s.fp, "missed": s.fn_, "flipRate": r3(flips), "sweep": sweep,
            }),
        );
    }

    // [value-add, necessary, top-1] as (right, total), keyed by each label's first category.
    let mut cats: BTreeMap<&str, [(usize, usize); 3]> = BTreeMap::new();
    let mut confusions: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut errors: BTreeMap<(usize, bool, &str, &str), (usize, f64)> = BTreeMap::new();
    for d in &decisions {
        let e = cats.entry(&d.cats[0]).or_default();
        if d.expect != Expect::Either {
            e[d.level].1 += 1;
            if d.flagged == (d.expect == Expect::Remove) {
                e[d.level].0 += 1;
            } else {
                let err = errors
                    .entry((d.level, d.flagged, &d.file, &d.find))
                    .or_default();
                *err = (err.0 + 1, err.1.max(d.confidence));
            }
        }
        if d.level == 0 {
            e[2].1 += 1;
            if d.cats.contains(&d.top) {
                e[2].0 += 1;
            } else {
                *confusions.entry((&d.cats[0], &d.top)).or_default() += 1;
            }
        }
    }

    let commit = git(&["rev-parse", "--short", "HEAD"]);
    let dirty = !git(&["status", "--porcelain"]).is_empty();
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let passed = gates.is_empty();
    let fmt_pr = |x: &Value| {
        format!(
            "{:.2} / {:.2}",
            x["precision"].as_f64().unwrap(),
            x["recall"].as_f64().unwrap()
        )
    };

    let mut md = format!(
        "# prolix evals: {}\n\n{} fixtures × {repeat} repeats · {model} · threshold {threshold} · {tokens} input tokens · \
         p50/p95 {} / {} ms per fixture · {commit}{}\n\n",
        if passed { "PASS" } else { "FAIL" },
        fixtures.len(),
        pct(0.5),
        pct(0.95),
        if dirty { " (dirty)" } else { "" },
    );
    md += "| Level | Precision | Recall | Wrong removals | Missed | Flip rate |\n| --- | --- | --- | --- | --- | --- |\n";
    for (name, l) in &levels {
        let _ = writeln!(
            md,
            "| {name} | {:.3} | {:.3} | {} | {} | {:.3} |",
            l["precision"].as_f64().unwrap(),
            l["recall"].as_f64().unwrap(),
            l["wrongRemovals"],
            l["missed"],
            l["flipRate"].as_f64().unwrap(),
        );
    }
    md += "\nThreshold sweep (precision / recall):\n\n| Threshold | value-add | necessary |\n| --- | --- | --- |\n";
    for (i, t) in SWEEP.iter().enumerate() {
        let cell = |name: &str| fmt_pr(&levels[name]["sweep"][i]);
        let _ = writeln!(
            md,
            "| {t} | {} | {} |",
            cell("value-add"),
            cell("necessary")
        );
    }
    md += "\n| Category | Judgements | value-add | necessary | Top-1 |\n| --- | --- | --- | --- | --- |\n";
    for (c, e) in &cats {
        let cell = |(a, b): (usize, usize)| {
            if b == 0 {
                "–".to_string()
            } else {
                format!("{:.2}", ratio(a, b))
            }
        };
        let _ = writeln!(
            md,
            "| {c} | {} | {} | {} | {} |",
            e[2].1,
            cell(e[0]),
            cell(e[1]),
            cell(e[2])
        );
    }
    if !confusions.is_empty() {
        let mut list: Vec<_> = confusions.iter().collect();
        list.sort_by(|a, b| b.1.cmp(a.1));
        let items: Vec<_> = list
            .iter()
            .map(|((want, got), n)| format!("{want} → {got} ×{n}"))
            .collect();
        let _ = writeln!(
            md,
            "\nTop-1 confusions (label → Jev's answer): {}",
            items.join(", ")
        );
    }
    if !errors.is_empty() {
        md += "\nErrors (a wrong removal costs more than a miss):\n\n";
        let mut list: Vec<_> = errors.iter().collect();
        list.sort_by_key(|((level, removed, ..), _)| (!removed, *level));
        for ((level, removed, file, find), (n, conf)) in list {
            let what = if *removed {
                "removed a keeper"
            } else {
                "missed"
            };
            let _ = writeln!(
                md,
                "- {} {what}: `{find}` in {file}, {n}/{repeat} runs, confidence ≤ {conf:.2}",
                LEVELS[*level].0
            );
        }
    }
    if !passed {
        md += "\nGate failures:\n\n";
        for g in &gates {
            let _ = writeln!(md, "- {g}");
        }
    }
    md += "\nSynthetic development set, not a production accuracy guarantee. Each run starts with an empty cache.\n";

    let out = dir.join("results").join(run_id.to_string());
    std::fs::create_dir_all(&out).unwrap();
    let report = json!({
        "metadata": {
            "run": run_id, "commit": commit, "dirty": dirty, "model": model, "threshold": threshold,
            "repeat": repeat, "promptVersion": jev::PROMPT_VERSION, "datasetHash": dataset,
            "jevHash": format!("{:016x}", jev::hash(&[&read(concat!(env!("CARGO_MANIFEST_DIR"), "/src/jev.rs"))])),
        },
        "gates": { "passed": passed, "failures": gates },
        "usage": { "inputTokens": tokens, "p50Ms": pct(0.5), "p95Ms": pct(0.95) },
        "levels": levels,
        "decisions": decisions.iter().map(|d| json!({
            "file": d.file, "find": d.find, "cats": d.cats, "level": LEVELS[d.level].0,
            "expect": format!("{:?}", d.expect), "flagged": d.flagged, "confidence": d.confidence, "top": d.top,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .unwrap();
    std::fs::write(out.join("summary.md"), &md).unwrap();
    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        let _ = f.write_all(md.as_bytes());
    }
    println!("{md}\nReport: {}", out.join("report.json").display());
    assert!(passed, "{}", gates.join("\n"));
}

#[test]
fn grading() {
    let labels: BTreeMap<String, Vec<String>> = [
        ("same", "restates-code"),
        ("why", "explains-why"),
        ("later", "todo"),
        ("noqa", "directive"),
    ]
    .into_iter()
    .map(|(f, c)| (f.to_string(), vec![c.to_string()]))
    .collect();
    let c = |text: &str, group: Option<&str>| json!({ "text": text, "group": group, "confidence": 0.9 });
    let report = |cs: Vec<Value>| json!({ "comments": cs, "stats": { "unanswered": 0 } });
    let mut ds = Vec::new();
    let problems = grade(
        "a.ts",
        &labels,
        &[
            report(vec![
                c("// same", Some("restates-code")),
                c("// why", Some("explains-why")),
                c("// later", None),
            ]),
            report(vec![
                c("// same", None),
                c("// why", None),
                c("// later", None),
                c("// stray", None),
            ]),
        ],
        &mut ds,
    );
    assert_eq!(problems, ["a.ts: unlabelled comment \"// stray\""]);
    let va = score(ds.iter().filter(|d| d.level == 0), |d| d.flagged);
    assert_eq!((va.tp, va.fp, va.fn_), (1, 1, 0));
    let ne = score(ds.iter().filter(|d| d.level == 1), |d| d.flagged);
    assert_eq!((ne.tp, ne.fp, ne.fn_), (0, 0, 2));
    assert_eq!(
        expect(&["todo".into(), "reference".into()], 2),
        Expect::Either
    );
    assert_eq!(
        ds.iter().find(|d| d.find == "same").unwrap().top,
        "restates-code"
    );

    let mut again = Vec::new();
    grade(
        "a.ts",
        &labels,
        &[report(vec![
            c("// same", None),
            c("// why", Some("x")),
            c("// later", None),
        ])],
        &mut again,
    );
    ds.extend(again);
    assert!((flip_rate(&ds, 0) - 1.0 / 3.0).abs() < 1e-9);
}
