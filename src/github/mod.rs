mod api;
mod review;
#[cfg(test)]
mod tests;

use crate::{report::Report, ScanOptions};
use api::{Api, Client, Context, Git, LocalGit, Plan};
use std::io::Write;

const HELP: &str = "\
Review a GitHub pull request using one Prolix scan.

Usage: prolix github review [options]

  --repository OWNER/REPO  Defaults to GITHUB_REPOSITORY
  --pr NUMBER              Defaults to PR
  --head SHA               Defaults to HEAD_SHA, then the local HEAD
  --base REF               Compare added lines with this ref (default origin/$GITHUB_BASE_REF)
  --scope changed|full     Default changed; full scans do not post inline suggestions
  --mode MODE              Override prolix.jsonc
  --dry-run                Print the proposed GitHub changes as JSON without applying them
  --bot-login LOGIN        Match this bot's threads during a dry run only
  --no-comment             Do not update the summary comment
  --no-suggestions         Do not create, update or resolve review threads
  --fail-on-findings       Exit 1 when findings remain

Needs GH_TOKEN (or GITHUB_TOKEN) and the PR head checked out with full history for
inline reviews. Respects GITHUB_API_URL and GITHUB_GRAPHQL_URL for enterprise hosts.
The composite action uses `prolix github action` with its configured input environment.
";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}
fn input_bool(name: &str, default: bool) -> Result<bool, String> {
    match env(name).as_deref() {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(format!("{name} must be true or false")),
    }
}

#[derive(Default)]
struct Options {
    action: bool,
    repository: String,
    pr: Option<u64>,
    head: Option<String>,
    base: Option<String>,
    scope: String,
    mode: Option<String>,
    comment: bool,
    suggestions: bool,
    fail: bool,
    dry_run: bool,
    bot_login: Option<String>,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let action = match args.first().map(String::as_str) {
            Some("action") => true,
            Some("review") => false,
            _ => return Err(HELP.into()),
        };
        let mut options = Self {
            action,
            repository: env("GITHUB_REPOSITORY").unwrap_or_default(),
            pr: env("PR")
                .map(|p| p.parse().map_err(|_| "PR must be a positive number"))
                .transpose()?,
            head: env("HEAD_SHA"),
            base: env("GITHUB_BASE_REF").map(|s| format!("origin/{s}")),
            scope: if action {
                env("INPUT_SCOPE").unwrap_or_else(|| "changed".into())
            } else {
                "changed".into()
            },
            mode: if action { env("INPUT_MODE") } else { None },
            comment: !action || input_bool("INPUT_COMMENT", true)?,
            suggestions: !action || input_bool("INPUT_SUGGESTIONS", true)?,
            fail: action && input_bool("INPUT_FAIL", false)?,
            ..Self::default()
        };
        let mut args = args.iter().skip(1);
        while let Some(arg) = args.next() {
            let (name, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
            let mut value = || {
                inline
                    .map(String::from)
                    .or_else(|| args.next().cloned())
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match name {
                "--action-protocol" if action => {
                    if value()? != "1" {
                        return Err("unsupported action protocol".into());
                    }
                }
                "--repository" => options.repository = value()?,
                "--pr" => {
                    options.pr = Some(
                        value()?
                            .parse()
                            .map_err(|_| "--pr must be a positive number")?,
                    )
                }
                "--head" => options.head = Some(value()?),
                "--base" => options.base = Some(value()?),
                "--scope" => options.scope = value()?,
                "--mode" => options.mode = Some(value()?),
                "--bot-login" => options.bot_login = Some(value()?),
                "--dry-run" if inline.is_none() => options.dry_run = true,
                "--no-comment" if inline.is_none() => options.comment = false,
                "--no-suggestions" if inline.is_none() => options.suggestions = false,
                "--fail-on-findings" if inline.is_none() => options.fail = true,
                _ => return Err(format!("unknown GitHub option {arg}\n\n{HELP}")),
            }
        }
        if !["changed", "full"].contains(&options.scope.as_str()) {
            return Err("scope must be changed or full".into());
        }
        if options.bot_login.is_some() && !options.dry_run {
            return Err("--bot-login is allowed only with --dry-run".into());
        }
        if !action && options.pr.is_none() {
            return Err("--pr is required outside a pull request action".into());
        }
        Ok(options)
    }

    fn context(&self, git: &impl Git) -> Result<Context, String> {
        let context = Context {
            repository: self.repository.clone(),
            pr: self.pr.ok_or("missing PR number")?,
            head: match &self.head {
                Some(s) => s.clone(),
                None => git.run(&["rev-parse", "HEAD"])?.trim().into(),
            },
        };
        context.validate()?;
        Ok(context)
    }
}

fn warn(message: String) {
    if env("GITHUB_ACTIONS").as_deref() == Some("true") {
        eprintln!(
            "::warning::prolix: {}",
            message
                .replace('%', "%25")
                .replace('\r', "%0D")
                .replace('\n', "%0A")
        );
    } else {
        eprintln!("prolix: {message}");
    }
}

fn append_env_file(name: &str, text: &str) -> Result<(), String> {
    if let Some(path) = env(name) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("{name}: {e}"))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn markdown(report: &Report, options: &Options) -> String {
    let mut hint = String::new();
    if options.scope == "changed" && options.pr.is_some() {
        if let Some(base) = &options.base {
            hint.push_str(&format!(" --changed={base}"));
        }
    }
    if let Some(mode) = &options.mode {
        hint.push_str(&format!(" --mode={mode}"));
    }
    report.markdown(&hint)
}

fn artifacts(report: &Report, options: &Options) -> Result<(), String> {
    append_env_file(
        "GITHUB_OUTPUT",
        &format!("flagged={}\n", report.stats.flagged),
    )?;
    append_env_file("GITHUB_STEP_SUMMARY", &markdown(report, options))?;
    if let Some(tmp) = env("RUNNER_TEMP") {
        let out = std::path::Path::new(&tmp).join("prolix");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        std::fs::write(
            out.join("report.json"),
            serde_json::to_vec(report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(out.join("summary.md"), markdown(report, options))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn setup(options: &Options, message: &str, new: bool) -> Result<(), String> {
    if options.pr.is_none() || !options.comment || options.dry_run {
        return Ok(());
    }
    let context = options.context(&LocalGit)?;
    let api = Client::from_env()?;
    let login = api::viewer(&api, &context)?;
    let server = env("GITHUB_SERVER_URL").unwrap_or_else(|| "https://github.com".into());
    let repository = &options.repository;
    let run = env("GITHUB_RUN_ID").unwrap_or_default();
    let suffix = if new { "/new" } else { "" };
    let markdown = format!("### prolix couldn't check this pull request\n\n{message}\n\n1. Get a key from [typesafe.ai](https://typesafe.ai).\n2. Save it as the `TYPESAFE_API_KEY` secret in [this repository's Actions secrets]({server}/{repository}/settings/secrets/actions{suffix}), or run `gh secret set TYPESAFE_API_KEY`. An organisation secret works too, once this repository has access to it.\n3. [Re-run the prolix job]({server}/{repository}/actions/runs/{run}).\n");
    let plan = Plan {
        summary: api::summary(&api, &context, &login, &markdown, true)?,
        ..Plan::default()
    };
    api::apply(&api, &context, &plan, warn)?;
    Ok(())
}

fn prepare(
    report: &Report,
    context: &Context,
    api: &impl Api,
    git: &impl Git,
    options: &Options,
) -> Result<Plan, String> {
    report.ensure_complete()?;
    let inline = options.suggestions && options.scope == "changed";
    let (viewer, threads) = if inline {
        context.checkout(git)?;
        api::threads(api, context)?
    } else {
        (api::viewer(api, context)?, Vec::new())
    };
    let login = options.bot_login.as_deref().unwrap_or(&viewer);
    let sources = api::sources(git, context, &threads, login)?;
    Ok(Plan {
        reviews: if inline {
            review::plan(report, &threads, login, &sources)?
        } else {
            review::ReviewPlan::default()
        },
        summary: if options.comment {
            api::summary(
                api,
                context,
                login,
                &markdown(report, options),
                report.stats.flagged > 0,
            )?
        } else {
            None
        },
    })
}

fn publish(
    report: &Report,
    context: &Context,
    api: &impl Api,
    git: &impl Git,
    options: &Options,
) -> Result<Plan, String> {
    let plan = prepare(report, context, api, git, options)?;
    if !options.dry_run {
        let counts = api::apply(api, context, &plan, warn)?;
        eprintln!("prolix: {} threads resolved, {} suggestions created, {} updates, {} suggestions failed", counts.resolved, counts.created, counts.updated, counts.failed);
    }
    Ok(plan)
}

pub fn run(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|s| s == "--help" || s == "-h") {
        print!("{HELP}");
        return Ok(0);
    }
    let mut options = Options::parse(args)?;
    if options.action && env("TYPESAFE_API_KEY").is_none() {
        if env("HEAD_REPO").as_deref() == Some(&options.repository)
            && env("PR_AUTHOR").as_deref() != Some("dependabot[bot]")
        {
            if let Err(e) = setup(&options, "No Typesafe API key reached this job, so the `TYPESAFE_API_KEY` secret is missing or this repository can't see it.", true) { warn(e); }
        }
        eprintln!(
            "prolix skipped: no api-key. Forks and Dependabot do not receive repository secrets."
        );
        return Ok(0);
    }
    let context = options.pr.map(|_| options.context(&LocalGit)).transpose()?;
    let changed = if context.is_some() && options.scope == "changed" {
        let base = options
            .base
            .clone()
            .ok_or("--base or GITHUB_BASE_REF is required for changed scope")?;
        if options.action
            && LocalGit
                .run(&["rev-parse", "-q", "--verify", &format!("{base}^{{commit}}")])
                .is_err()
        {
            let branch = env("GITHUB_BASE_REF").ok_or("GITHUB_BASE_REF is missing")?;
            LocalGit.run(&[
                "fetch",
                "-q",
                "--no-tags",
                "--depth=1",
                "origin",
                &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
            ])?;
        }
        Some(base)
    } else {
        None
    };
    let scanned = crate::scan(ScanOptions {
        mode: options.mode.clone(),
        changed,
        ..ScanOptions::default()
    });
    let report = match scanned {
        Ok(report) => report,
        Err(e) => {
            if options.action && e.contains("rejected TYPESAFE_API_KEY") {
                if let Err(setup_error) = setup(&options, "Jev rejected the key in `TYPESAFE_API_KEY`, so it's wrong or has been revoked.", false) { warn(setup_error); }
            }
            return Err(e);
        }
    };
    if options.action {
        artifacts(&report, &options)?;
    }
    if report.ensure_complete().is_err() {
        if options.action
            && report
                .errors
                .iter()
                .any(|e| e.contains("rejected TYPESAFE_API_KEY"))
        {
            if let Err(e) = setup(
                &options,
                "Jev rejected the key in `TYPESAFE_API_KEY`, so it's wrong or has been revoked.",
                false,
            ) {
                warn(e);
            }
        }
        return Ok(2);
    }
    if let Some(context) = context.filter(|_| options.comment || options.suggestions) {
        if options.action && options.suggestions && options.scope == "changed" {
            if let Err(e) = context.checkout(&LocalGit) {
                warn(e);
                options.suggestions = false;
            }
        }
        let result = Client::from_env()
            .and_then(|api| publish(&report, &context, &api, &LocalGit, &options));
        match result {
            Ok(plan) if options.dry_run => println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"headSha":context.head,"plan":plan})
                )
                .map_err(|e| e.to_string())?
            ),
            Ok(_) => {}
            Err(e) if options.action => warn(e),
            Err(e) => return Err(e),
        }
    } else if options.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&Plan::default()).map_err(|e| e.to_string())?
        );
    }
    Ok(if options.fail { report.exit_code() } else { 0 })
}
