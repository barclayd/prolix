use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Cat {
    pub name: &'static str,
    pub summary: &'static str,
    /// Loosest mode that removes it: 1 standard, 2 strict, 3 none (only a `remove` rule does).
    pub mode: u8,
    what: &'static str,
    not_for: Option<&'static str>,
    pub examples: &'static [&'static str],
}

/// The mode that removes `cat`. Configuration keeps section labels and switched-off settings until `strict`.
pub fn mode(cat: &Cat, config: bool) -> u8 {
    match cat.name {
        "restates-code" | "decorative" | "commented-out-code" if config => 2,
        _ => cat.mode,
    }
}

pub const CATS: [Cat; 11] = [
    Cat {
        name: "restates-code",
        summary: "repeats what the code already says",
        mode: 1,
        what: "Repeats what the code already says plainly (names, operations, obvious steps), or is conversational filler addressed to a reader, reviewer or user",
        not_for: Some("Summaries that save a reader from working through non-obvious code"),
        examples: &["// increment the counter", "// loop through the users", "# return the result", "// Here we create the client"],
    },
    Cat {
        name: "commented-out-code",
        summary: "disabled code left behind",
        mode: 1,
        what: "Source code that has been disabled by commenting it out",
        not_for: Some("Prose that quotes a short identifier or usage example"),
        examples: &["// const total = compute(items);", "# print(debug_info)"],
    },
    Cat {
        name: "decorative",
        summary: "banners, dividers and section labels",
        mode: 1,
        what: "A banner, divider or section heading that only labels a region whose purpose is obvious from the code",
        not_for: None,
        examples: &["// ===== Helpers =====", "/* ---------- */", "// Imports"],
    },
    Cat {
        name: "change-note",
        summary: "narrates an edit instead of the code as it is",
        mode: 1,
        what: "Describes an edit, fix or earlier version (what was added, changed, removed or updated) rather than the code as it is now",
        not_for: None,
        examples: &["// Updated to use the new API", "// Fixed: was off by one", "// Removed the old cache logic", "// NEW: added retry"],
    },
    Cat {
        name: "redundant-doc",
        summary: "docs that only repeat the signature",
        mode: 1,
        what: "A documentation comment that only repeats the name, signature or types, adding no behaviour, constraint or edge case",
        not_for: Some("Docs that state behaviour, errors, units or constraints the signature doesn't show"),
        examples: &["/** Gets the user. @param id The id. @returns The user. */", "/// Creates a new Config."],
    },
    Cat {
        name: "todo",
        summary: "TODO / FIXME notes",
        mode: 2,
        what: "A TODO, FIXME, HACK or note about unfinished or planned work",
        not_for: None,
        examples: &["// TODO: handle pagination", "# FIXME: breaks on empty input"],
    },
    Cat {
        name: "clarifies",
        summary: "summarises what non-obvious code does",
        mode: 2,
        what: "Summarises or explains what a non-obvious piece of code does, so a reader can follow it faster",
        not_for: Some("Restating a line that is already obvious"),
        examples: &["// Binary search over the sorted offsets", "// Normalise to UTC before bucketing by day"],
    },
    Cat {
        name: "explains-why",
        summary: "explains intent or trade-offs",
        mode: 3,
        what: "Explains why the code is written this way: intent, trade-off, business rule or rejected alternative that the code cannot express",
        not_for: None,
        examples: &["// Retry once: the upstream drops the first request after idle", "# Sorted descending so the newest wins ties"],
    },
    Cat {
        name: "warning",
        summary: "warns about pitfalls or invariants",
        mode: 3,
        what: "Warns about a pitfall, invariant, ordering constraint, safety or security requirement, or behaviour that would surprise a maintainer",
        not_for: None,
        examples: &["// Must run before init(); it mutates globals", "// SAFETY: ptr is non-null and aligned", "// Not thread-safe"],
    },
    Cat {
        name: "api-doc",
        summary: "documents public behaviour",
        mode: 3,
        what: "Documents a public interface's behaviour, parameters, return value, errors or usage beyond what its signature shows",
        not_for: Some("Docs that only restate the name or types"),
        examples: &["/** Returns null when the key has expired; never throws. */", "/// Panics if `n` is zero."],
    },
    Cat {
        name: "reference",
        summary: "links to issues, specs or sources",
        mode: 3,
        what: "Points to an external source: an issue, spec, RFC, ticket, documentation page or the origin of copied code",
        not_for: None,
        examples: &["// See RFC 7231 section 6.5.1", "// Workaround for https://github.com/org/repo/issues/123"],
    },
];

/// Bump when the prompt changes so cached answers are re-asked.
pub const PROMPT_VERSION: &str = "1";

const STATE: &str = "Each question reviews one comment from a codebase. `comment` is the comment's full text, `code` is the \
source around it with <<COMMENT>> marking where the comment sits, and `language` is the file's language.";

const BATCH_QUESTIONS: usize = 32;
const BATCH_TOKENS: usize = 40_000;
const WORKERS: usize = 16;
const ATTEMPTS: u32 = 6;

/// The categories, then each plain-English rule from `keep` and `remove` as (name, text).
pub fn criteria(rules: &[(&str, &str)]) -> Map<String, Value> {
    let mut criteria: Map<String, Value> = CATS
        .iter()
        .map(|c| {
            let mut o = json!({ "what": c.what, "examples": c.examples });
            if let Some(n) = c.not_for {
                o["not_for"] = n.into();
            }
            (c.name.to_string(), o)
        })
        .collect();
    for (name, what) in rules {
        criteria.insert(name.to_string(), json!({ "what": what }));
    }
    criteria
}

pub fn question(comment: &str, code: &str, language: &str, criteria: &Map<String, Value>) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "comment": comment,
            "code": code,
            "language": language,
            "question": "Which best describes `comment`, judged against `code`?",
        },
        "criteria": criteria,
    })
}

pub fn model() -> String {
    std::env::var("TYPESAFE_DEFAULT_MODEL").unwrap_or_else(|_| "jev-latest".into())
}

pub struct Outcome {
    pub probs: Vec<Option<Vec<f32>>>,
    pub tokens: u64,
    pub error: Option<String>,
}

/// Asks Jev every question, batched and in parallel. Answers are probabilities in the order of `names`.
pub fn classify(qs: &[Value], names: &[&str], key: &str) -> Outcome {
    let base =
        std::env::var("TYPESAFE_BASE_URL").unwrap_or_else(|_| "https://api.typesafe.ai".into());
    let model = model();
    let url = format!("{}/v1/systemone", base.trim_end_matches('/'));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();

    // ponytail: bytes/3 over-estimates tokens for code; fine until batches need to be fuller.
    let mut batches = Vec::new();
    let (mut start, mut tokens) = (0, 0);
    for (i, q) in qs.iter().enumerate() {
        let t = q.to_string().len() / 3;
        if i > start && (i - start == BATCH_QUESTIONS || tokens + t > BATCH_TOKENS) {
            batches.push(start..i);
            (start, tokens) = (i, 0);
        }
        tokens += t;
    }
    if start < qs.len() {
        batches.push(start..qs.len());
    }

    let next = AtomicUsize::new(0);
    let used = AtomicU64::new(0);
    let error = Mutex::new(None);
    let probs = Mutex::new(vec![None; qs.len()]);
    std::thread::scope(|s| {
        for _ in 0..WORKERS.min(batches.len()) {
            s.spawn(|| loop {
                let b = next.fetch_add(1, Relaxed);
                if b >= batches.len() || error.lock().unwrap().is_some() {
                    break;
                }
                let questions: Map<String, Value> = batches[b]
                    .clone()
                    .map(|i| (i.to_string(), qs[i].clone()))
                    .collect();
                let body = json!({ "model": model, "state": STATE, "questions": questions });
                match post(&agent, &url, key, &body) {
                    Ok(v) => {
                        used.fetch_add(v["usage"]["input_tokens"].as_u64().unwrap_or(0), Relaxed);
                        let mut probs = probs.lock().unwrap();
                        for i in batches[b].clone() {
                            probs[i] = parse(&v["answers"][i.to_string()], names);
                        }
                    }
                    Err(e) => {
                        error.lock().unwrap().get_or_insert(e);
                        break;
                    }
                }
            });
        }
    });
    Outcome {
        probs: probs.into_inner().unwrap(),
        tokens: used.into_inner(),
        error: error.into_inner().unwrap(),
    }
}

fn parse(a: &Value, names: &[&str]) -> Option<Vec<f32>> {
    let probs = a["probabilities"].as_object();
    let choice = a["choice"].as_str();
    if probs.is_none() && choice.is_none() {
        return None;
    }
    Some(
        names
            .iter()
            .map(|&n| match probs {
                Some(p) => p.get(n).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                None => f32::from(choice == Some(n)),
            })
            .collect(),
    )
}

fn post(agent: &ureq::Agent, url: &str, key: &str, body: &Value) -> Result<Value, String> {
    let mut wait = Duration::from_millis(500);
    for attempt in 1..=ATTEMPTS {
        let retry_after = match agent
            .post(url)
            .set("Authorization", &format!("Bearer {key}"))
            .send_json(body)
        {
            Ok(r) => {
                return r
                    .into_json()
                    .map_err(|e| format!("unreadable response from Jev: {e}"))
            }
            Err(ureq::Error::Status(401, _)) => {
                return Err("Jev rejected TYPESAFE_API_KEY (401)".into())
            }
            Err(ureq::Error::Status(code, r)) if code == 429 || code >= 500 => {
                r.header("retry-after").and_then(|s| s.parse::<f64>().ok())
            }
            Err(ureq::Error::Status(code, r)) => {
                return Err(format!(
                    "Jev returned {code}: {}",
                    r.into_string().unwrap_or_default()
                ))
            }
            Err(e) if attempt == ATTEMPTS => return Err(format!("could not reach Jev: {e}")),
            Err(_) => None,
        };
        let jitter = Duration::from_millis(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_millis() as u64
                % 250,
        );
        std::thread::sleep(
            retry_after.map_or(wait, |s| Duration::from_secs_f64(s.clamp(0.0, 60.0))) + jitter,
        );
        wait *= 2;
    }
    Err(format!(
        "Jev is overloaded; gave up after {ATTEMPTS} attempts"
    ))
}

/// FNV-1a over NUL-separated parts.
pub fn hash(parts: &[&str]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        for &b in p.as_bytes().iter().chain(&[0]) {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

pub type Cache = HashMap<String, Vec<f32>>;

pub fn load_cache(path: &Path) -> Cache {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

// ponytail: the cache only grows; prune by last-seen run if it ever gets large.
pub fn save_cache(path: &Path, cache: &Cache) {
    let tmp = path.with_extension("tmp");
    let _ = path.parent().map(std::fs::create_dir_all);
    if serde_json::to_vec(cache).is_ok_and(|b| std::fs::write(&tmp, b).is_ok()) {
        let _ = std::fs::rename(&tmp, path);
    }
}
