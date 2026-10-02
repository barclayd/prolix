use crate::report::{Finding, Report};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const MARKER: &str = "<!-- prolix:suggestion -->";
pub type Sources = HashMap<(String, String), Option<String>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub path: String,
    pub line: Option<usize>,
    pub start_line: Option<usize>,
    pub original_line: Option<usize>,
    pub original_start_line: Option<usize>,
    pub is_resolved: bool,
    pub comments: Connection<ReviewComment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub database_id: u64,
    pub body: String,
    pub author: Option<Author>,
    pub commit: Option<Commit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    #[serde(rename = "__typename")]
    pub kind: String,
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    pub oid: String,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct ReviewPlan {
    pub resolve: Vec<String>,
    pub update: Vec<CommentUpdate>,
    pub create: Vec<Suggestion>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CommentUpdate {
    pub id: u64,
    pub body: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub path: String,
    pub line: usize,
    pub side: &'static str,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<&'static str>,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    v: u8,
    id: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<String>,
}

pub fn bot_login(login: &str) -> &str {
    login.strip_suffix("[bot]").unwrap_or(login)
}

fn normalize(text: &str) -> String {
    text.split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn fingerprint(comment: &Finding) -> String {
    let mut hash = Sha256::new();
    hash.update(&comment.path);
    hash.update([0]);
    hash.update(normalize(&comment.text));
    format!("{:x}", hash.finalize())
}

fn metadata(body: &str) -> Option<Metadata> {
    let (_, rest) = body.split_once("<!-- prolix:finding ")?;
    let (encoded, _) = rest.split_once(" -->")?;
    let value: Metadata = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()?;
    (value.v == 1).then_some(value)
}

pub fn body(comment: &Finding, automatic: bool) -> String {
    let meta = Metadata {
        v: 1,
        id: fingerprint(comment),
        text: comment.text.clone(),
        decision: Some(comment.decision_id.clone()),
    };
    let encoded =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&meta).expect("serializable finding metadata"));
    let prefix = format!("{MARKER}\n<!-- prolix:finding {encoded} -->\n**prolix**: ");
    let Some(group) = &comment.group else {
        return format!("{prefix}The latest check did not flag this unchanged comment. The earlier finding is left open for review; no automatic edit is available.");
    };
    let prefix = format!("{prefix}`{group}`.");
    let Some(fix) = comment.fix.as_ref().filter(|_| automatic) else {
        return format!("{prefix} This finding needs review; no automatic edit is available.");
    };
    let longest = fix
        .replacement
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest + 1));
    let replacement = if fix.replacement.is_empty() {
        String::new()
    } else {
        format!("{}\n", fix.replacement)
    };
    format!(
        "{prefix} Commit the suggestion to remove it.\n\n{fence}suggestion\n{replacement}{fence}"
    )
}

pub fn suggestion(comment: &Finding) -> Option<Suggestion> {
    let fix = comment.fix.as_ref()?;
    Some(Suggestion {
        path: comment.path.clone(),
        line: fix.end_line,
        side: "RIGHT",
        body: body(comment, true),
        start_line: (fix.start_line < fix.end_line).then_some(fix.start_line),
        start_side: (fix.start_line < fix.end_line).then_some("RIGHT"),
    })
}

pub fn owned(thread: &Thread, login: &str) -> bool {
    thread.comments.nodes.first().is_some_and(|first| {
        first.body.starts_with(MARKER)
            && first
                .author
                .as_ref()
                .is_some_and(|a| a.kind == "Bot" && bot_login(&a.login) == bot_login(login))
    })
}

fn source<'a>(sources: &'a Sources, revision: &str, path: &str) -> Result<Option<&'a str>, String> {
    sources
        .get(&(revision.into(), path.into()))
        .map(|s| s.as_deref())
        .ok_or_else(|| {
            format!("source unavailable for {revision}:{path}; threads were left unchanged")
        })
}

pub fn plan(
    report: &Report,
    threads: &[Thread],
    login: &str,
    sources: &Sources,
) -> Result<ReviewPlan, String> {
    report.ensure_complete()?;
    let mut result = ReviewPlan::default();
    let mut consumed = HashSet::new();
    for thread in threads.iter().filter(|t| owned(t, login)) {
        let first = &thread.comments.nodes[0];
        let meta = metadata(&first.body);
        let legacy = if meta.is_none() {
            match (&first.commit, thread.original_line) {
                (Some(commit), Some(end)) => {
                    source(sources, &commit.oid, &thread.path)?.map(|src| {
                        let start = thread.original_start_line.unwrap_or(end);
                        src.split('\n')
                            .skip(start.saturating_sub(1))
                            .take(end.saturating_sub(start) + 1)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                }
                _ => None,
            }
        } else {
            None
        };
        let current = report
            .comments
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                !consumed.contains(i)
                    && c.path == thread.path
                    && match &meta {
                        Some(meta) => fingerprint(c) == meta.id,
                        None => {
                            Some(
                                c.fix
                                    .as_ref()
                                    .map_or(c.line + c.text.matches('\n').count(), |f| f.end_line),
                            ) == thread.line
                                && match &legacy {
                                    Some(text) => normalize(text).contains(&normalize(&c.text)),
                                    None => {
                                        body(c, c.fix.is_some())
                                            .lines()
                                            .skip(2)
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                            == first
                                                .body
                                                .lines()
                                                .skip(1)
                                                .collect::<Vec<_>>()
                                                .join("\n")
                                    }
                                }
                        }
                    }
            })
            .min_by_key(|(_, c)| {
                c.line
                    .abs_diff(thread.line.or(thread.original_line).unwrap_or(c.line))
            });
        if let Some((i, c)) = current {
            let changed = meta
                .as_ref()
                .and_then(|m| m.decision.as_deref())
                .is_some_and(|d| !d.is_empty() && !c.decision_id.is_empty() && d != c.decision_id);
            if thread.is_resolved && changed {
                continue;
            }
            consumed.insert(i);
            if thread.is_resolved {
                continue;
            }
            if c.group.is_none() && changed && c.skipped.is_none() {
                result.resolve.push(thread.id.clone());
            } else {
                let anchored = c.fix.as_ref().is_some_and(|f| {
                    thread.line == Some(f.end_line)
                        && thread.start_line.or(thread.line) == Some(f.start_line)
                });
                let updated = body(c, anchored);
                if updated != first.body {
                    result.update.push(CommentUpdate {
                        id: first.database_id,
                        body: updated,
                    });
                }
            }
            continue;
        }
        if thread.is_resolved {
            continue;
        }
        let previous = meta
            .as_ref()
            .map(|m| m.text.as_str())
            .or(legacy.as_deref())
            .unwrap_or_default();
        let previous = normalize(previous);
        if !previous.is_empty()
            && source(sources, "head", &thread.path)?
                .is_none_or(|src| !normalize(src).contains(&previous))
        {
            result.resolve.push(thread.id.clone());
        }
    }
    result.create = report
        .comments
        .iter()
        .enumerate()
        .filter(|(i, c)| c.group.is_some() && !consumed.contains(i))
        .filter_map(|(_, c)| suggestion(c))
        .take(50)
        .collect();
    Ok(result)
}

pub fn source_requests(
    threads: &[Thread],
    login: &str,
) -> std::collections::BTreeSet<(String, String)> {
    let mut requests = std::collections::BTreeSet::new();
    for thread in threads.iter().filter(|t| owned(t, login)) {
        let first = &thread.comments.nodes[0];
        if !thread.is_resolved {
            requests.insert(("head".into(), thread.path.clone()));
        }
        if metadata(&first.body).is_none() && thread.original_line.is_some() {
            if let Some(commit) = &first.commit {
                requests.insert((commit.oid.clone(), thread.path.clone()));
            }
        }
    }
    requests
}
