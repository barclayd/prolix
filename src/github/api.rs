use super::review::{self, CommentUpdate, Sources, Thread};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::process::Command;
use std::time::Duration;

pub trait Api {
    fn request(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value, String>;
}

pub struct Client {
    agent: ureq::Agent,
    token: String,
    base: String,
    graphql: String,
}

impl Client {
    pub fn new(token: String, base: String, graphql: String) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(30))
                .redirects(0)
                .build(),
            token,
            base,
            graphql,
        }
    }

    pub fn from_env() -> Result<Self, String> {
        let token = super::env("GH_TOKEN")
            .or_else(|| super::env("GITHUB_TOKEN"))
            .ok_or("GH_TOKEN or GITHUB_TOKEN is required to access GitHub")?;
        let base = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_string();
        let graphql = std::env::var("GITHUB_GRAPHQL_URL")
            .unwrap_or_else(|_| format!("{}/graphql", base.strip_suffix("/v3").unwrap_or(&base)));
        Ok(Self::new(token, base, graphql))
    }
}

impl Api for Client {
    fn request(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value, String> {
        let url = if endpoint == "graphql" {
            self.graphql.clone()
        } else {
            format!("{}/{}", self.base, endpoint)
        };
        let request = self
            .agent
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", concat!("prolix/", env!("CARGO_PKG_VERSION")))
            .set("X-GitHub-Api-Version", "2022-11-28");
        // Do not retry mutations: a lost response may still have created a comment.
        let response = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        }
        .map_err(|e| format!("GitHub {method} {endpoint}: {e}"))?;
        let value: Value = response
            .into_json()
            .map_err(|e| format!("invalid GitHub response: {e}"))?;
        if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
            return Err(format!(
                "GitHub GraphQL: {}",
                errors
                    .iter()
                    .filter_map(|e| e["message"].as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        Ok(value)
    }
}

pub trait Git {
    fn run(&self, args: &[&str]) -> Result<String, String>;
}

pub struct LocalGit;
impl Git for LocalGit {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "git: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    pub repository: String,
    pub pr: u64,
    pub head: String,
}
impl Context {
    pub fn validate(&self) -> Result<(), String> {
        let parts: Vec<_> = self.repository.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|s| {
                s.is_empty()
                    || *s == "."
                    || *s == ".."
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
            || self.pr == 0
            || ![40, 64].contains(&self.head.len())
            || !self.head.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid PR context (repository, PR number or head SHA)".into());
        }
        Ok(())
    }
    fn variables(&self) -> Value {
        let (owner, repo) = self
            .repository
            .split_once('/')
            .expect("validated repository");
        json!({"owner":owner,"repo":repo,"pr":self.pr})
    }
    pub fn checkout(&self, git: &impl Git) -> Result<(), String> {
        if git.run(&["rev-parse", "HEAD"])?.trim() != self.head
            || git.run(&["rev-parse", "--is-shallow-repository"])?.trim() != "false"
        {
            return Err("suggestions need the PR head checkout with fetch-depth: 0".into());
        }
        Ok(())
    }
    pub fn current(&self, api: &impl Api) -> Result<(), String> {
        let data = api.request("POST", "graphql", Some(json!({"query":"query($owner:String!,$repo:String!,$pr:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$pr){headRefOid}}}","variables":self.variables()})))?;
        if data["data"]["repository"]["pullRequest"]["headRefOid"].as_str() != Some(&self.head) {
            return Err("PR head changed or unavailable; this run is stale".into());
        }
        Ok(())
    }
}

const THREAD_QUERY: &str = "query($owner:String!,$repo:String!,$pr:Int!,$cursor:String){viewer{login} repository(owner:$owner,name:$repo){pullRequest(number:$pr){headRefOid reviewThreads(first:100,after:$cursor){nodes{id isResolved path line startLine originalLine originalStartLine comments(first:1){nodes{databaseId body author{__typename login} commit{oid}}}} pageInfo{hasNextPage endCursor}}}}}";

pub fn threads(api: &impl Api, context: &Context) -> Result<(String, Vec<Thread>), String> {
    context.validate()?;
    let mut result = Vec::new();
    let mut cursor = Value::Null;
    let mut seen = HashSet::new();
    loop {
        let mut variables = context.variables();
        variables["cursor"] = cursor;
        let value = api.request(
            "POST",
            "graphql",
            Some(json!({"query":THREAD_QUERY,"variables":variables})),
        )?;
        let pull = &value["data"]["repository"]["pullRequest"];
        if pull["headRefOid"].as_str() != Some(&context.head) {
            return Err("PR head changed or unavailable; this run is stale".into());
        }
        let login = value["data"]["viewer"]["login"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("GitHub returned no viewer login")?
            .to_string();
        result.extend(
            serde_json::from_value::<Vec<Thread>>(pull["reviewThreads"]["nodes"].clone())
                .map_err(|e| format!("invalid review threads: {e}"))?,
        );
        let page = &pull["reviewThreads"]["pageInfo"];
        match page["hasNextPage"].as_bool() {
            Some(false) => return Ok((login, result)),
            Some(true) => {
                let next = page["endCursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("missing review thread cursor")?;
                if !seen.insert(next.to_string()) {
                    return Err("repeated review thread cursor".into());
                }
                cursor = Value::String(next.into());
            }
            None => return Err("missing review thread pagination".into()),
        }
    }
}

pub fn sources(
    git: &impl Git,
    context: &Context,
    threads: &[Thread],
    login: &str,
) -> Result<Sources, String> {
    let mut sources = Sources::new();
    for (reference, path) in review::source_requests(threads, login) {
        let revision = if reference == "head" {
            &context.head
        } else {
            &reference
        };
        if ![40, 64].contains(&revision.len()) || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid source commit in review thread".into());
        }
        let entry = git.run(&[
            "--literal-pathspecs",
            "ls-tree",
            "-z",
            revision,
            "--",
            &path,
        ])?;
        let text = if entry.is_empty() {
            None
        } else {
            Some(git.run(&["show", &format!("{revision}:{path}")])?)
        };
        sources.insert((reference, path), text);
    }
    Ok(sources)
}

#[derive(Deserialize)]
struct IssueComment {
    id: u64,
    body: String,
    user: Option<IssueAuthor>,
}
#[derive(Deserialize)]
struct IssueAuthor {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

pub fn summary(
    api: &impl Api,
    context: &Context,
    login: &str,
    markdown: &str,
    create: bool,
) -> Result<Option<CommentUpdate>, String> {
    let body = format!("<!-- prolix -->\n{markdown}");
    for page in 1.. {
        let value = api.request(
            "GET",
            &format!(
                "repos/{}/issues/{}/comments?per_page=100&page={page}",
                context.repository, context.pr
            ),
            None,
        )?;
        let comments: Vec<IssueComment> =
            serde_json::from_value(value).map_err(|e| format!("invalid issue comments: {e}"))?;
        if let Some(comment) = comments.iter().find(|c| {
            c.body.starts_with("<!-- prolix -->")
                && c.user.as_ref().is_some_and(|u| {
                    u.kind == "Bot" && review::bot_login(&u.login) == review::bot_login(login)
                })
        }) {
            return Ok((comment.body != body).then_some(CommentUpdate {
                id: comment.id,
                body,
            }));
        }
        if comments.len() < 100 {
            return Ok(create.then_some(CommentUpdate { id: 0, body }));
        }
    }
    unreachable!()
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Plan {
    #[serde(flatten)]
    pub reviews: review::ReviewPlan,
    pub summary: Option<CommentUpdate>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Applied {
    pub resolved: usize,
    pub updated: usize,
    pub created: usize,
    pub failed: usize,
}

pub fn apply(
    api: &impl Api,
    context: &Context,
    plan: &Plan,
    mut warn: impl FnMut(String),
) -> Result<Applied, String> {
    let mut counts = Applied::default();
    if let Some(summary) = &plan.summary {
        context.current(api)?;
        let (method, endpoint) = if summary.id == 0 {
            (
                "POST",
                format!(
                    "repos/{}/issues/{}/comments",
                    context.repository, context.pr
                ),
            )
        } else {
            (
                "PATCH",
                format!(
                    "repos/{}/issues/comments/{}",
                    context.repository, summary.id
                ),
            )
        };
        api.request(method, &endpoint, Some(json!({"body":summary.body})))?;
    }
    for update in &plan.reviews.update {
        context.current(api)?;
        api.request(
            "PATCH",
            &format!("repos/{}/pulls/comments/{}", context.repository, update.id),
            Some(json!({"body":update.body})),
        )?;
        counts.updated += 1;
    }
    for comment in &plan.reviews.create {
        context.current(api)?;
        let mut body = serde_json::to_value(comment).map_err(|e| e.to_string())?;
        body["commit_id"] = context.head.clone().into();
        match api.request(
            "POST",
            &format!("repos/{}/pulls/{}/comments", context.repository, context.pr),
            Some(body),
        ) {
            Ok(_) => counts.created += 1,
            Err(e) => {
                counts.failed += 1;
                warn(format!(
                    "could not post suggestion at {}:{}: {e}",
                    comment.path, comment.line
                ));
            }
        }
    }
    for id in &plan.reviews.resolve {
        context.current(api)?;
        let value = api.request("POST", "graphql", Some(json!({"query":"mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}","variables":{"id":id}})))?;
        if value["data"]["resolveReviewThread"]["thread"]["isResolved"] != true {
            return Err("GitHub did not confirm thread resolution".into());
        }
        counts.resolved += 1;
    }
    Ok(counts)
}

pub fn viewer(api: &impl Api, context: &Context) -> Result<String, String> {
    context.validate()?;
    let data = api.request("POST", "graphql", Some(json!({"query":"query($owner:String!,$repo:String!,$pr:Int!){viewer{login} repository(owner:$owner,name:$repo){pullRequest(number:$pr){headRefOid}}}","variables":context.variables()})))?;
    if data["data"]["repository"]["pullRequest"]["headRefOid"].as_str() != Some(&context.head) {
        return Err("PR head changed or unavailable; this run is stale".into());
    }
    data["data"]["viewer"]["login"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or("GitHub returned no viewer login".into())
}
