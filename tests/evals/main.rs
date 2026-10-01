//! Live evals of Jev's judgements. Each file in `fixtures/` has its comments labelled in
//! `cases.json` with every category a careful reviewer would accept, and prolix is run
//! against it end to end. `behaviour.json` adds a config with a `behaviour` and fixtures
//! labelled with what that config should do, so a behaviour in plain English is measured too.
//!
//!     cargo test --release --test evals -- --ignored --nocapture
//!
//! Needs TYPESAFE_API_KEY. EVAL_REPEAT (1-5, default 3) sets how many times each fixture
//! is judged, and EVAL_BASELINE names an earlier report.json to compare against. Reports
//! are written to `results/<run>/`.

#[allow(dead_code)]
#[path = "../../src/jev.rs"]
mod jev;
#[allow(dead_code)]
#[path = "../../src/lex.rs"]
mod lex;

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Fixture file → comment substring → acceptable categories, or `["directive"]`.
type Labels = BTreeMap<String, BTreeMap<String, Vec<String>>>;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/evals");
const MODES: [(&str, u8); 2] = [("standard", 1), ("strict", 2)];
const SWEEP: [f64; 5] = [0.4, 0.5, 0.6, 0.7, 0.8];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    Remove,
    Keep,
    /// The acceptable categories fall on both sides of the mode, so either is right.
    Either,
}

struct Decision {
    file: String,
    find: String,
    cats: Vec<String>,
    mode: usize,
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

fn expect(cats: &[String], mode: u8, config: bool) -> Expect {
    // Behaviour cases are labelled with the outcome itself.
    match cats {
        [c] if c == "remove" => return Expect::Remove,
        [c] if c == "keep" => return Expect::Keep,
        _ => {}
    }
    let removed = cats
        .iter()
        .filter(|c| {
            let cat = jev::CATS.iter().find(|k| k.name == c.as_str());
            jev::mode(
                cat.unwrap_or_else(|| panic!("unknown category {c:?} in cases.json")),
                config,
            ) <= mode
        })
        .count();
    match removed {
        0 => Expect::Keep,
        n if n == cats.len() => Expect::Remove,
        _ => Expect::Either,
    }
}

/// Turns one run's reports (one per mode) into decisions and returns any contract breaks:
/// a label that doesn't match exactly one comment, a comment nobody labelled, a directive
/// that reached Jev, or a comment Jev never answered.
fn grade(
    file: &str,
    labels: &BTreeMap<String, Vec<String>>,
    reports: &[Value],
    out: &mut Vec<Decision>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let config = lex::lang_for(Path::new(file)).is_some_and(|l| l.config());
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
                mode: li,
                expect: expect(cats, MODES[li].1, config),
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

/// Share of comments whose decision in `mode` differed between repeats.
fn flip_rate(ds: &[Decision], mode: usize) -> f64 {
    let mut seen: BTreeMap<(&str, &str), (bool, bool)> = BTreeMap::new();
    for d in ds.iter().filter(|d| d.mode == mode) {
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
/// cache. Later modes reuse the first mode's answers from that run's cache.
fn run(
    file: &str,
    src: &str,
    repeat: usize,
    config: &str,
    modes: &[&str],
) -> Result<Vec<Value>, String> {
    let path = Path::new(file);
    if path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("fixture paths must be relative and cannot contain '..'".into());
    }
    let tmp = std::env::temp_dir().join(format!(
        "prolix-eval-{}-{repeat}-{:016x}",
        std::process::id(),
        jev::hash(&[file, config, &modes.join(",")])
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(path).parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(tmp.join(file), src).map_err(|e| e.to_string())?;
    // Stops the config search from finding a prolix.jsonc above the temp directory.
    std::fs::write(tmp.join("prolix.jsonc"), config).map_err(|e| e.to_string())?;
    let reports = modes
        .iter()
        .map(|mode| {
            let out = Command::new(env!("CARGO_BIN_EXE_prolix"))
                .args(["--reporter", "json", "--mode", mode])
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
    let dataset_dir = std::env::var_os("EVAL_DATASET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(DIR).to_path_buf());
    let dir = dataset_dir.as_path();
    let cases = read(dir.join("cases.json"));
    let labels: Labels = serde_json::from_str(&cases).expect("cases.json");
    let behaviour_json = read(dir.join("behaviour.json"));
    let behaviour: Value = serde_json::from_str(&behaviour_json).expect("behaviour.json");
    let behaviour_labels: Labels =
        serde_json::from_value(behaviour["cases"].clone()).expect("behaviour.json cases");
    let behaviour_config = behaviour["config"].to_string();
    // The categories are judged in each mode with an empty config; the behaviour in its config's own mode.
    let sets: [(&Labels, &str, Vec<&str>); 2] = [
        (&labels, "{}", MODES.iter().map(|(m, _)| *m).collect()),
        (
            &behaviour_labels,
            &behaviour_config,
            vec![behaviour["config"]["mode"].as_str().unwrap_or("standard")],
        ),
    ];
    let policy: Value = serde_json::from_str(&read(dir.join("policy.json"))).expect("policy.json");
    let dataset_kind = policy["datasetKind"].as_str().unwrap_or("development");
    let repeat: usize = std::env::var("EVAL_REPEAT").map_or(3, |v| v.parse().expect("EVAL_REPEAT"));
    assert!((1..=5).contains(&repeat), "EVAL_REPEAT must be 1-5");
    assert!(
        std::env::var_os("TYPESAFE_API_KEY").is_some(),
        "TYPESAFE_API_KEY is not set"
    );

    let fixtures: Vec<(usize, String, String)> = sets
        .iter()
        .enumerate()
        .flat_map(|(set, (l, ..))| {
            l.keys()
                .map(move |f| (set, f.clone(), read(dir.join("fixtures").join(f))))
        })
        .collect();
    let mut gates = Vec::new();

    let mut parts = vec![cases.as_str(), behaviour_json.as_str()];
    parts.extend(
        fixtures
            .iter()
            .flat_map(|(_, f, s)| [f.as_str(), s.as_str()]),
    );
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
        for (_, f, src) in &fixtures {
            if body.chars().any(char::is_alphanumeric) && src.to_lowercase().contains(&body) {
                gates.push(format!("{f} reuses the prompt example {e:?}"));
            }
        }
    }

    // Decisions for the categories, then for the behaviour.
    let (mut decisions, mut problems, mut ms) = ([Vec::new(), Vec::new()], Vec::new(), Vec::new());
    let (mut tokens, mut model, mut threshold) = (0, String::new(), 0.0);
    let mut resolved_models = std::collections::BTreeSet::new();
    let mut prompt_hash = String::new();
    for r in 0..repeat {
        let runs: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = fixtures
                .iter()
                .map(|(set, f, src)| {
                    let (_, config, modes) = &sets[*set];
                    s.spawn(move || run(f, src, r, config, modes))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for ((set, f, _), res) in fixtures.iter().zip(runs) {
            match res {
                Err(e) => problems.push(format!("{f}: {e}")),
                Ok(reports) => {
                    tokens += reports
                        .iter()
                        .map(|r| r["stats"]["inputTokens"].as_u64().unwrap_or(0))
                        .sum::<u64>();
                    ms.push(reports[0]["stats"]["elapsedMs"].as_u64().unwrap_or(0));
                    model = reports[0]["model"].as_str().unwrap_or_default().to_string();
                    prompt_hash = reports[0]["promptHash"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    if let Some(models) = reports[0]["resolvedModels"].as_array() {
                        resolved_models
                            .extend(models.iter().filter_map(Value::as_str).map(String::from));
                    }
                    threshold = reports[0]["threshold"].as_f64().unwrap_or_default();
                    problems.extend(grade(f, &sets[*set].0[f], &reports, &mut decisions[*set]));
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
    let [decisions, behaviour_decisions] = &decisions;
    // Each row of the report: its name, its decisions and the mode they were made in.
    let rows: Vec<(&str, &[Decision], usize)> = MODES
        .iter()
        .enumerate()
        .map(|(li, (name, _))| (*name, &decisions[..], li))
        .chain([("behaviour", &behaviour_decisions[..], 0)])
        .collect();
    let mut modes = Map::new();
    for &(name, ds, li) in &rows {
        let at = || ds.iter().filter(move |d| d.mode == li);
        let s = score(at(), |d| d.flagged);
        let flips = flip_rate(ds, li);
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
                .and_then(|b| b["modes"][name][metric].as_f64())
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
        modes.insert(
            name.to_string(),
            json!({
                "precision": r3(s.precision()), "recall": r3(s.recall()),
                "wrongRemovals": s.fp, "missed": s.fn_, "flipRate": r3(flips), "sweep": sweep,
            }),
        );
    }

    // [standard, strict, top-1] as (right, total), keyed by each label's first category.
    let mut cats: BTreeMap<&str, [(usize, usize); 3]> = BTreeMap::new();
    let mut confusions: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut errors: BTreeMap<(bool, &str, &str, &str), (usize, f64)> = BTreeMap::new();
    for &(name, ds, li) in &rows {
        for d in ds
            .iter()
            .filter(|d| d.mode == li && d.expect != Expect::Either)
        {
            if d.flagged != (d.expect == Expect::Remove) {
                let err = errors
                    .entry((!d.flagged, name, &d.file, &d.find))
                    .or_default();
                *err = (err.0 + 1, err.1.max(d.confidence));
            }
        }
    }
    for d in decisions {
        let e = cats.entry(&d.cats[0]).or_default();
        if d.expect != Expect::Either {
            e[d.mode].1 += 1;
            if d.flagged == (d.expect == Expect::Remove) {
                e[d.mode].0 += 1;
            }
        }
        if d.mode == 0 {
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
    md += "| Mode | Precision | Recall | Wrong removals | Missed | Flip rate |\n| --- | --- | --- | --- | --- | --- |\n";
    for (name, ..) in &rows {
        let l = &modes[*name];
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
    md += "\nThreshold sweep (precision / recall):\n\n| Threshold | standard | strict |\n| --- | --- | --- |\n";
    for (i, t) in SWEEP.iter().enumerate() {
        let cell = |name: &str| fmt_pr(&modes[name]["sweep"][i]);
        let _ = writeln!(md, "| {t} | {} | {} |", cell("standard"), cell("strict"));
    }
    md += "\n| Category | Judgements | standard | strict | Top-1 |\n| --- | --- | --- | --- | --- |\n";
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
        for ((missed, name, file, find), (n, conf)) in &errors {
            let what = if !missed {
                "removed a keeper"
            } else {
                "missed"
            };
            let _ = writeln!(
                md,
                "- {name} {what}: `{find}` in {file}, {n}/{repeat} runs, confidence ≤ {conf:.2}"
            );
        }
    }
    if !passed {
        md += "\nGate failures:\n\n";
        for g in &gates {
            let _ = writeln!(md, "- {g}");
        }
    }
    let _ = writeln!(md, "\nDataset kind: `{dataset_kind}`. Results describe this corpus, not a production accuracy guarantee. Each run starts with an empty cache.");

    let out = dir.join("results").join(run_id.to_string());
    std::fs::create_dir_all(&out).unwrap();
    let report = json!({
        "metadata": {
            "run": run_id, "commit": commit, "dirty": dirty, "model": model, "threshold": threshold,
            "repeat": repeat, "promptVersion": jev::PROMPT_VERSION, "promptHash": prompt_hash, "resolvedModels": resolved_models, "datasetHash": dataset,
            "datasetKind": dataset_kind,
            "jevHash": format!("{:016x}", jev::hash(&[&read(concat!(env!("CARGO_MANIFEST_DIR"), "/src/jev.rs"))])),
        },
        "gates": { "passed": passed, "failures": gates },
        "usage": { "inputTokens": tokens, "p50Ms": pct(0.5), "p95Ms": pct(0.95) },
        "modes": modes,
        "decisions": rows.iter().flat_map(|&(name, ds, li)| ds.iter().filter(move |d| d.mode == li).map(move |d| json!({
            "file": d.file, "find": d.find, "cats": d.cats, "mode": name,
            "expect": format!("{:?}", d.expect), "flagged": d.flagged, "confidence": d.confidence, "top": d.top,
        }))).collect::<Vec<_>>(),
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
fn nested_fixtures_and_policies_run_in_isolation() {
    let src = "// ---\nconst x = 1;\n";
    std::thread::scope(|s| {
        let standard = s.spawn(|| run("nested/a.ts", src, 0, "{}", &["standard"]));
        let off = s.spawn(|| run("nested/a.ts", src, 0, r#"{"mode":"off"}"#, &["off"]));
        assert_eq!(standard.join().unwrap().unwrap()[0]["stats"]["flagged"], 1);
        assert_eq!(off.join().unwrap().unwrap()[0]["stats"]["flagged"], 0);
    });
    assert!(run("../escape.ts", src, 0, "{}", &["standard"]).is_err());
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
    let va = score(ds.iter().filter(|d| d.mode == 0), |d| d.flagged);
    assert_eq!((va.tp, va.fp, va.fn_), (1, 1, 0));
    let ne = score(ds.iter().filter(|d| d.mode == 1), |d| d.flagged);
    assert_eq!((ne.tp, ne.fp, ne.fn_), (0, 0, 2));
    assert_eq!(
        expect(&["todo".into(), "reference".into()], 2, false),
        Expect::Either
    );
    assert_eq!(expect(&["restates-code".into()], 1, true), Expect::Keep);
    assert_eq!(expect(&["remove".into()], 0, false), Expect::Remove);
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
