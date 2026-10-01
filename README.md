# prolix

A fast, language-agnostic linter for comments that don't earn their place: ones that restate the code, commented-out code, banners, change notes and signature-only docs. LLM-written code is full of them.

prolix extracts JS/TS/JSX/TSX comments with a parser and uses a byte-level lexer for 40+ other languages, asks [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev) what kind of comment each one is, and flags the ones your mode doesn't keep. `prolix --fix` applies eligible JS/TS/JSX/TSX removals after checking that executable syntax and literal content are preserved. Other languages remain available for review.

**[Docs](https://prolix.barclaysd.workers.dev)**, with prompts to hand to your coding agent for [setting it up](https://prolix.barclaysd.workers.dev/setup/) and [cleaning up an existing repo](https://prolix.barclaysd.workers.dev/adopt/).

```
$ npx @prolix/cli
src/client.ts
  12:3   restates-code       // create the client
  40:1   commented-out-code  // const old = retry(req);
  57:26  change-note         // Updated to use the new API

Found 3 comments in 1 file (mode: standard):

      1  restates-code       repeats what the code already says
      1  commented-out-code  disabled code left behind
      1  change-note         narrates an edit instead of the code as it is

Run `prolix --fix` to apply eligible fixes.
Checked 214 comments in 38 files in 0.61s · Jev: 214 asked, 0 cached, 228k tokens
```

## Install

```sh
npm i -D @prolix/cli
```

This works like Biome: the wrapper installs a prebuilt binary for your platform (macOS arm64/x64, Linux arm64/x64, Windows x64). If you use Rust, you can run `cargo install --path .` instead.

Every pull request that changes the CLI publishes a canary under the `canary` tag (`npx @prolix/cli@canary`), and the pull request gets a comment with its exact version.

Releases use [changesets](https://github.com/changesets/changesets). Run `npx changeset` in a pull request to describe a change. When it's merged, a Version PR collects the pending changesets. Merging that PR publishes the new version to npm and creates a GitHub release.

## Usage

```sh
export TYPESAFE_API_KEY=...        # from typesafe.ai
prolix                             # check the current directory
prolix src lib --mode strict       # check specific paths in strict mode
prolix --fix                       # apply validated fixes; review remaining findings
prolix --changed=main              # only comments on lines added since the branch left main
prolix --changed --fix             # remove flagged comments in uncommitted changes, e.g. in a pre-commit hook
prolix --reporter json             # machine-readable output with Jev's probabilities and each fix
prolix --reporter markdown         # a summary for a pull request comment or job summary
```

`--changed` reads `git diff` against the merge base with the ref (default `HEAD`). It counts uncommitted edits and untracked files, and only sends Jev the comments that touch added lines. In the JSON report, each eligible finding's `fix` gives the lines to replace (`startLine` to `endLine`) and their `replacement`, which is what the GitHub Action posts as a suggestion.

The walk respects `.gitignore`, `.ignore` and hidden files, and excludes files over 1 MB. Excluding a supported oversized file marks the check incomplete; add intentional exclusions to `ignore`.

| Exit code | Meaning |
| --- | --- |
| 0 | nothing flagged, or all findings fixed |
| 1 | findings remain, including those without an eligible fix |
| 2 | error or incomplete check (including invalid source, unreadable files, oversized comments or missing Jev answers) |

## GitHub Action

The action runs prolix on the lines a pull request adds. When it flags something, it posts a summary comment and one-click suggestions for findings with validated fixes. Later runs update the same comment (to ✅ once the pull request is clean), reuse existing suggestions and resolve their review threads once the underlying finding no longer applies.

```yaml
# .github/workflows/prolix.yml
name: prolix
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  prolix:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ github.event.pull_request.head.sha }}
          fetch-depth: 0
      - uses: barclayd/prolix@v0
        with:
          api-key: ${{ secrets.TYPESAFE_API_KEY }}
```

| Input | Default | |
| --- | --- | --- |
| `api-key` | | Typesafe API key. When it's empty or Jev rejects it, the pull request gets a comment that links to the repository's Actions secrets. Pull requests from forks and Dependabot don't get secrets, so for those the check is skipped with a notice. |
| `mode` | `prolix.jsonc`, then `standard` | `off`, `standard` or `strict` |
| `scope` | `changed` | `full` checks the whole repository |
| `comment` | `true` | post and update the summary comment |
| `suggestions` | `true` | post a suggestion per flagged comment |
| `fail-on-findings` | `false` | fail the job when anything is flagged, instead of only advising |
| `version` | `latest` | the `@prolix/cli` version to run |

The `flagged` output is the number of comments flagged. The summary also goes to the run page.

`ref` and `fetch-depth: 0` check out the pull request's own commits rather than GitHub's merge commit, so suggestions land on the right lines. Without them the action still comments, and warns that it skipped the suggestions.

The action runs the Rust `prolix github action` command through the npm launcher. It scans once and uses shared report types for the summary and review plan. `prolix github review --dry-run` previews proposed GitHub changes locally; run it with `--help` for options. This action revision requires the new command in CLI 0.4.0 or a corresponding canary/local build.

## Adopting prolix in an existing repo

The action only checks the lines a pull request adds, so turning it on never flags the comments already in a repo. To clear those:

1. Add the workflow first, so new comments are checked from the next pull request.
2. Remove the backlog in one pull request in `standard` mode, which only removes comments that restate the code, disabled code, banners, change notes and signature-only docs. Review the diff like any other, and restore anything worth keeping with `prolix-ignore` added to it. In a large repo, fix a directory at a time (`npx @prolix/cli src/components --fix`) so each pull request stays reviewable.

   ```sh
   npx @prolix/cli --fix
   ```

3. For a stricter bar, set `"mode": "strict"` in `prolix.jsonc` and run `--fix` again. Strict mode also flags TODOs and clarifying comments; only eligible fixes are applied.

## Modes and behaviour

| Mode | Keeps | Removes |
| --- | --- | --- |
| `off` | everything | nothing |
| `standard` (default) | anything that tells the reader something the code doesn't | restates-code, commented-out-code, decorative, change-note, redundant-doc |
| `strict` | explains-why, warning, api-doc, reference | the above, plus todo and clarifies |

`keep` and `remove` in `prolix.jsonc` switch categories on or off whatever the mode. `behaviour` tells Jev in plain English how your team judges its comments, and Jev weighs it when it picks a category:

```jsonc
{
  "mode": "strict",
  "keep": ["todo"],
  "behaviour": "Keep comments that state a fact the code relies on but can't show: what a tool does, where a file is generated, what a limit or constant means."
}
```

A comment is flagged when the chance that it fits a category the mode removes reaches `threshold`, and it's labelled with the likeliest of them. `behaviour` changes which category Jev picks, not what the mode does with it, so name a whole category in `keep` or `remove` instead of describing it. Changing `behaviour` asks Jev again about every comment.

Plain-English rules in `keep` and `remove`, from 0.2, still work with a warning: each becomes a sentence of `behaviour`.

`level` and its values (`all`, `value-add`, `necessary`) still work, with a warning.

Tool and compiler directives are never touched. prolix recognises them by shape rather than by a list of tool names, so a linter it has never heard of is still respected:

- a switch word joined to a name or followed by a rule: `eslint-disable-next-line`, `react-doctor-disable-line`, `biome-ignore`, `# hadolint ignore=DL3008`, `/* c8 ignore next */`
- recognised `noqa`, `nolint`, `nosec` and `NOSONAR` markers and `tool:setting` tokens: `# noqa: E501`, `// NOLINTNEXTLINE`, `# rubocop:disable`, `//go:build`, `// gitleaks:allow`
- tags and settings: `@ts-expect-error`, `$FlowFixMe`, `# shellcheck source=lib.sh`, `# syntax=docker/dockerfile:1`, `/* webpackChunkName: "x" */`
- version pins beside a hash, as in `uses: actions/checkout@<sha> # v4.1.1`
- shebangs, pragmas, regions, `/*!` licence headers, `@generated` markers and `/// <reference>`
- server-side includes in any server's spelling, such as `<!--#include virtual="/nav" -->`, `<!--# echo var="x" -->` or `<!-- #include file="a.inc" -->`, along with any run of line comments that quotes one

Add `prolix-ignore` to any comment to keep it.

## Configuration

`prolix.jsonc` (or `prolix.json`) in the current directory or any parent:

```jsonc
{
  "mode": "standard",
  // How sure Jev must be (0-1) that a comment is removable before flagging it.
  "threshold": 0.6,
  // Higher bar for automatic edits and review suggestions.
  "fixThreshold": 0.9,
  // Globs relative to this file, on top of .gitignore.
  "ignore": ["vendor/**", "**/*.generated.ts"]
}
```

`--mode` on the command line overrides the file.

## Jev and caching

| Variable | Default |
| --- | --- |
| `TYPESAFE_API_KEY` | required unless the mode is `off` with nothing in `remove` |
| `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` |
| `TYPESAFE_DEFAULT_MODEL` | `jev-latest` |

Each comment is sent with a few lines of surrounding code. Blocks over 2,000 bytes are kept without judgment and mark the check incomplete. Identical pending questions are deduplicated. prolix batches up to 32 comments per request and sends up to 16 requests in parallel. It retries rate limits and 5xx responses with backoff.

Validated answers and their resolved model versions are cached by comment, context, language, requested model, API endpoint, prompt hash and `behaviour` in `node_modules/.cache/prolix.json`, or in `.prolixcache` when there is no `node_modules`. Re-runs only ask about new or changed comments, and switching modes needs no new calls.

## Evals

`tests/evals` measures Jev's judgements end to end. It runs the release binary against 15 fixtures (TypeScript, TSX, Rust, Python, Go, Java, C, Ruby, shell, SQL, CSS and YAML). Every comment in them is labelled in `cases.json` with the categories a careful reviewer would accept. The fixtures cover every category, plus these harder cases:

- warnings dressed as banners
- TODOs that also explain why
- prose that quotes code
- prompt-injection comments
- directives, which must never reach Jev

`behaviour.json` adds two fixtures judged with a `keep` list and a `behaviour`, each comment labelled with what that config should do.

```sh
export TYPESAFE_API_KEY=...
cargo test --release --test evals -- --ignored --nocapture
```

Each fixture is judged `EVAL_REPEAT` times (default 3) from an empty cache.

The report gives, for `standard`, `strict` and the behaviour:

- removal precision, the gate, because deleting a useful comment is the costly mistake
- recall
- flip rate between repeats
- a threshold sweep
- per-category accuracy
- top-1 confusions
- every wrong decision, for error analysis

`policy.json` pins a hash of the dataset and sets the floors, so changing a label or a floor is a reviewed change. Set `EVAL_BASELINE=path/to/report.json` to also fail on a drop of more than 0.05 against an earlier run. Results go to `tests/evals/results/<run>/`. The `evals` workflow runs on pull requests that touch `src` or `tests/evals`, or on demand, and posts the summary to the run page. It needs a `TYPESAFE_API_KEY` repository secret. A full run uses about 270k input tokens.

## Known limits

- Automatic fixes and PR suggestions currently require parser-validated JS/TS/JSX/TSX. Other languages produce review findings without fixes.
- YAML scalar contents are treated as data; embedded scripts inside them are not linted.
- `fixThreshold` defaults to 0.9, separately from the 0.6 reporting threshold. This is a conservative starting policy, not a calibrated accuracy guarantee.
- `{/* */}` comments in Svelte and Astro templates aren't linted.
- C++ raw strings (`R"(...)"`) aren't recognised.
- The cache only grows. Delete it to reset.

Review-thread reconciliation is tested with mocked GitHub responses (`node --test action/reviews.test.mjs`). Offline CLI tests cover code/data boundaries, incomplete responses, long comments, cache deduplication and edits during inference. For independently reviewed real-world corpora, run the eval harness with `EVAL_DATASET_DIR=/absolute/path/to/corpus`; see [the evaluation workflow](tests/evals/README.md).
