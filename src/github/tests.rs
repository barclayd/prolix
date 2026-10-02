use super::*;
use crate::report::{Finding, Fix, FixStatus, Stats};
use api::{Applied, Git};
use review::{Sources, Thread};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::VecDeque;

const LOGIN: &str = "github-actions[bot]";
fn finding() -> Finding {
    Finding {
        path: "a.ts".into(),
        line: 2,
        column: 1,
        text: "// increment".into(),
        group: Some("restates-code".into()),
        decision_id: "evidence-1".into(),
        fix_status: FixStatus::Available,
        fix: Some(Fix {
            start_line: 2,
            end_line: 2,
            replacement: String::new(),
        }),
        ..Finding::default()
    }
}
fn report(comments: Vec<Finding>) -> Report {
    let flagged = comments.iter().filter(|c| c.group.is_some()).count();
    Report {
        complete: true,
        stats: Stats {
            comments: comments.len(),
            flagged,
            remaining: flagged,
            ..Stats::default()
        },
        comments,
        ..Report::default()
    }
}
fn thread_value() -> Value {
    json!({"id":"thread-1","path":"a.ts","line":2,"startLine":null,"originalLine":2,"originalStartLine":null,"isResolved":false,"comments":{"nodes":[{"databaseId":42,"author":{"__typename":"Bot","login":LOGIN},"body":review::body(&finding(),true),"commit":{"oid":"b".repeat(40)}}]}})
}
fn thread() -> Thread {
    serde_json::from_value(thread_value()).unwrap()
}
fn sources(text: Option<&str>) -> Sources {
    [(("head".into(), "a.ts".into()), text.map(String::from))].into()
}
fn plan(comments: Vec<Finding>, threads: Vec<Thread>, text: Option<&str>) -> review::ReviewPlan {
    review::plan(&report(comments), &threads, LOGIN, &sources(text)).unwrap()
}
fn kept() -> Finding {
    Finding {
        group: None,
        fix: None,
        ..finding()
    }
}

#[test]
fn resolves_removed_comments_and_retains_current_findings() {
    assert_eq!(
        plan(vec![], vec![thread()], Some("count++;")).resolve,
        ["thread-1"]
    );
    assert_eq!(
        plan(vec![finding()], vec![thread()], None),
        review::ReviewPlan::default()
    );
}
#[test]
fn score_flips_and_scope_exclusions_do_not_resolve_threads() {
    assert!(plan(vec![kept()], vec![thread()], None).resolve.is_empty());
    assert!(plan(vec![], vec![thread()], Some("// increment\ncount++;"))
        .resolve
        .is_empty());
}
#[test]
fn resolves_a_retained_comment_after_changed_policy_or_evidence() {
    let c = Finding {
        decision_id: "changed".into(),
        ..kept()
    };
    assert_eq!(plan(vec![c], vec![thread()], None).resolve, ["thread-1"]);
}
#[test]
fn incomplete_missing_or_skipped_results_never_resolve_threads() {
    let c = Finding {
        skipped: Some("too-long".into()),
        decision_id: "new".into(),
        ..kept()
    };
    assert!(plan(vec![c], vec![thread()], None).resolve.is_empty());
    let mut r = report(vec![]);
    r.complete = false;
    assert!(review::plan(&r, &[], LOGIN, &Sources::new()).is_err());
    r.complete = true;
    r.stats.unanswered = 1;
    assert!(review::plan(&r, &[], LOGIN, &Sources::new()).is_err());
    assert!(
        serde_json::from_value::<Report>(json!({"comments":[],"stats":{"unanswered":0}})).is_err()
    );
}
#[test]
fn respects_human_resolutions_and_line_moves() {
    let c = Finding {
        line: 10,
        fix: Some(Fix {
            start_line: 10,
            end_line: 10,
            replacement: String::new(),
        }),
        ..finding()
    };
    let mut t = thread();
    t.line = Some(10);
    t.is_resolved = true;
    assert_eq!(
        plan(vec![c.clone()], vec![t.clone()], None),
        review::ReviewPlan::default()
    );
    t.is_resolved = false;
    assert!(plan(vec![c], vec![t], None).create.is_empty());
}
#[test]
fn never_touches_human_or_other_bot_threads() {
    for (kind, login) in [("User", LOGIN), ("Bot", "other[bot]")] {
        let mut t = thread();
        let a = t.comments.nodes[0].author.as_mut().unwrap();
        a.kind = kind.into();
        a.login = login.into();
        assert_eq!(plan(vec![], vec![t], None), review::ReviewPlan::default());
    }
}
#[test]
fn preserves_distinct_occurrences_of_identical_text() {
    let c = Finding {
        line: 8,
        fix: Some(Fix {
            start_line: 8,
            end_line: 8,
            replacement: String::new(),
        }),
        ..finding()
    };
    let result = plan(vec![finding(), c], vec![thread()], None);
    assert_eq!(result.create.len(), 1);
    assert_eq!(result.create[0].line, 8);
}
#[test]
fn legacy_suggestions_migrate_without_deleting_discussion() {
    let mut t = thread();
    t.comments.nodes[0].body = review::body(&finding(), true)
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join("\n");
    let mut src = sources(Some("count++;"));
    src.insert(
        ("b".repeat(40), "a.ts".into()),
        Some("let count = 0;\n// increment\ncount++;".into()),
    );
    assert_eq!(
        review::plan(&report(vec![finding()]), &[t.clone()], LOGIN, &src)
            .unwrap()
            .update
            .len(),
        1
    );
    assert_eq!(
        review::plan(&report(vec![]), &[t.clone()], LOGIN, &src)
            .unwrap()
            .resolve,
        ["thread-1"]
    );
    src.insert(
        ("head".into(), "a.ts".into()),
        Some("let count = 0;\n// increment\ncount++;".into()),
    );
    assert!(review::plan(&report(vec![]), &[t], LOGIN, &src)
        .unwrap()
        .resolve
        .is_empty());
}
#[test]
fn missing_files_resolve_but_source_failures_abort() {
    assert_eq!(plan(vec![], vec![thread()], None).resolve, ["thread-1"]);
    assert!(
        review::plan(&report(vec![]), &[thread()], LOGIN, &Sources::new())
            .unwrap_err()
            .contains("source unavailable")
    );
    let git = TestGit { source_error: true };
    assert!(api::sources(&git, &context(), &[thread()], LOGIN).is_err());
}
#[test]
fn suggestions_are_bounded_and_support_multiline_replacements() {
    let comments = (0..51)
        .map(|i| Finding {
            path: format!("{i}.ts"),
            ..finding()
        })
        .collect();
    assert_eq!(plan(comments, vec![], None).create.len(), 50);
    let c = Finding {
        fix: Some(Fix {
            start_line: 1,
            end_line: 2,
            replacement: "code();".into(),
        }),
        ..finding()
    };
    let s = review::suggestion(&c).unwrap();
    assert_eq!(s.start_line, Some(1));
    assert!(s.body.contains("```suggestion\ncode();\n```"));
}

#[derive(Default)]
struct TestGit {
    source_error: bool,
}
impl Git for TestGit {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        match args {
            ["rev-parse", "HEAD"] => Ok("a".repeat(40)),
            ["rev-parse", "--is-shallow-repository"] => Ok("false".into()),
            _ if self.source_error => Err("git failed".into()),
            _ => Ok(String::new()),
        }
    }
}
type Request = (String, String, Option<Value>);
struct MockApi {
    replies: RefCell<VecDeque<Value>>,
    requests: RefCell<Vec<Request>>,
}
impl MockApi {
    fn new(replies: Vec<Value>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
    fn only_reads(&self) -> bool {
        self.requests
            .borrow()
            .iter()
            .all(|(method, endpoint, body)| {
                method == "GET"
                    || (endpoint == "graphql"
                        && body.as_ref().unwrap()["query"]
                            .as_str()
                            .unwrap()
                            .starts_with("query"))
            })
    }
}
impl Api for MockApi {
    fn request(&self, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value, String> {
        self.requests
            .borrow_mut()
            .push((method.into(), endpoint.into(), body));
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or("unexpected API call".into())
    }
}
fn context() -> Context {
    Context {
        repository: "owner/repo".into(),
        pr: 1,
        head: "a".repeat(40),
    }
}
fn page(threads: Vec<Value>, next: bool) -> Value {
    json!({
        "data": {
            "viewer": {"login": LOGIN},
            "repository": {"pullRequest": {
                "headRefOid": "a".repeat(40),
                "reviewThreads": {
                    "nodes": threads,
                    "pageInfo": {"hasNextPage": next, "endCursor": if next {Some("next")} else {None}}
                }
            }}
        }
    })
}
fn options() -> Options {
    Options {
        scope: "changed".into(),
        suggestions: true,
        ..Options::default()
    }
}
#[test]
fn paginates_and_aborts_stale_runs_before_mutation() {
    let api = MockApi::new(vec![
        page(vec![], true),
        page(vec![thread_value()], false),
        json!({"data":{"repository":{"pullRequest":{"headRefOid":"c".repeat(40)}}}}),
    ]);
    let err = publish(
        &report(vec![]),
        &context(),
        &api,
        &TestGit::default(),
        &options(),
    )
    .unwrap_err();
    assert!(err.contains("stale"));
    assert_eq!(api.requests.borrow().len(), 3);
    assert!(api.only_reads());
}
#[test]
fn recognizes_graphql_bot_logins_without_rest_suffix() {
    let mut t = thread();
    t.comments.nodes[0].author.as_mut().unwrap().login = "github-actions".into();
    assert_eq!(plan(vec![], vec![t], None).resolve, ["thread-1"]);
}
#[test]
fn resolves_with_graphql_and_never_deletes_comments() {
    let api = MockApi::new(vec![
        page(vec![thread_value()], false),
        page(vec![], false),
        json!({"data":{"resolveReviewThread":{"thread":{"isResolved":true}}}}),
    ]);
    publish(
        &report(vec![]),
        &context(),
        &api,
        &TestGit::default(),
        &options(),
    )
    .unwrap();
    let requests = api.requests.borrow();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|(m, _, _)| m != "DELETE"));
    assert_eq!(
        requests[2].2.as_ref().unwrap()["variables"]["id"],
        "thread-1"
    );
}
#[test]
fn materially_changed_findings_can_follow_human_resolutions() {
    let mut t = thread();
    t.is_resolved = true;
    let c = Finding {
        decision_id: "changed-context".into(),
        ..finding()
    };
    assert_eq!(plan(vec![c], vec![t], None).create.len(), 1);
}
#[test]
fn escapes_backtick_fences_in_suggestions() {
    let c = Finding {
        fix: Some(Fix {
            start_line: 1,
            end_line: 1,
            replacement: "const text = `\n```\n`;".into(),
        }),
        ..finding()
    };
    let body = review::body(&c, true);
    assert!(body.contains("````suggestion\n"));
    assert!(body.ends_with("````"));
}
#[test]
fn withdraws_automatic_edits_below_the_fix_threshold() {
    let result = plan(
        vec![Finding {
            fix: None,
            fix_status: FixStatus::BelowFixThreshold,
            ..finding()
        }],
        vec![thread()],
        None,
    );
    assert_eq!(result.update.len(), 1);
    assert!(!result.update[0].body.contains("```suggestion"));
    assert!(result.resolve.is_empty());
}
#[test]
fn withdraws_automatic_edits_after_score_flips() {
    let result = plan(vec![kept()], vec![thread()], None);
    assert_eq!(result.update.len(), 1);
    assert!(!result.update[0].body.contains("```suggestion"));
    assert!(result.update[0].body.contains("left open for review"));
    assert!(result.resolve.is_empty());
}
#[test]
fn withdraws_automatic_edits_when_the_anchor_changes() {
    let c = Finding {
        fix: Some(Fix {
            start_line: 2,
            end_line: 3,
            replacement: String::new(),
        }),
        ..finding()
    };
    let result = plan(vec![c], vec![thread()], None);
    assert_eq!(result.update.len(), 1);
    assert!(!result.update[0].body.contains("```suggestion"));
    assert!(result.create.is_empty());
}

#[test]
fn dry_run_plans_resolutions_without_mutations() {
    let api = MockApi::new(vec![page(vec![thread_value()], false)]);
    let result = publish(
        &report(vec![]),
        &context(),
        &api,
        &TestGit::default(),
        &Options {
            dry_run: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(result.reviews.resolve, ["thread-1"]);
    assert!(api.only_reads());
}
#[test]
fn incomplete_reports_never_reach_github() {
    let api = MockApi::new(vec![]);
    let mut r = report(vec![]);
    r.complete = false;
    assert!(publish(&r, &context(), &api, &TestGit::default(), &options()).is_err());
    assert!(api.requests.borrow().is_empty());
}
#[test]
fn summary_is_paginated_and_scoped_to_bot_ownership() {
    let other =
        json!({"id":9,"body":"<!-- prolix -->\nhuman text","user":{"login":LOGIN,"type":"User"}});
    let api = MockApi::new(vec![
        json!(vec![other; 100]),
        json!([{"id":42,"body":"<!-- prolix -->\nold","user":{"login":"github-actions","type":"Bot"}}]),
    ]);
    let result = api::summary(&api, &context(), LOGIN, "new", false)
        .unwrap()
        .unwrap();
    assert_eq!(result.id, 42);
    assert_eq!(api.requests.borrow().len(), 2);
    let api = MockApi::new(vec![json!([])]);
    assert!(api::summary(&api, &context(), LOGIN, "clean", false)
        .unwrap()
        .is_none());
}
#[test]
fn a_stale_run_cannot_replace_the_summary() {
    let api = MockApi::new(vec![
        json!({"data":{"repository":{"pullRequest":{"headRefOid":"c".repeat(40)}}}}),
    ]);
    let plan = Plan {
        summary: Some(review::CommentUpdate {
            id: 42,
            body: "new".into(),
        }),
        ..Plan::default()
    };
    assert!(api::apply(&api, &context(), &plan, |_| {}).is_err());
    assert!(api.only_reads());
}
#[test]
fn partial_suggestion_failures_are_counted_and_do_not_block_other_suggestions() {
    struct FailureApi;
    impl Api for FailureApi {
        fn request(&self, _: &str, endpoint: &str, body: Option<Value>) -> Result<Value, String> {
            if endpoint == "graphql" {
                return Ok(page(vec![], false));
            }
            if body.unwrap()["path"] == "a.ts" {
                Err("not in diff".into())
            } else {
                Ok(json!({}))
            }
        }
    }
    let other = Finding {
        path: "b.ts".into(),
        ..finding()
    };
    let plan = Plan {
        reviews: review::ReviewPlan {
            create: vec![
                review::suggestion(&finding()).unwrap(),
                review::suggestion(&other).unwrap(),
            ],
            ..review::ReviewPlan::default()
        },
        ..Plan::default()
    };
    let Applied {
        created, failed, ..
    } = api::apply(&FailureApi, &context(), &plan, |_| {}).unwrap();
    assert_eq!((created, failed), (1, 1));
}
#[test]
fn repeated_pagination_cursor_is_an_error() {
    let api = MockApi::new(vec![page(vec![], true), page(vec![], true)]);
    assert!(api::threads(&api, &context())
        .unwrap_err()
        .contains("repeated"));
}

#[test]
fn metadata_remains_compatible_with_javascript_suggestions() {
    let old_body = include_str!("testdata/javascript-suggestion.txt");
    assert_eq!(review::body(&finding(), true), old_body);
    let mut t = thread();
    t.comments.nodes[0].body = old_body.into();
    assert_eq!(
        plan(vec![finding()], vec![t], None),
        review::ReviewPlan::default()
    );
}
