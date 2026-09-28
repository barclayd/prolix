# prolix

A fast, language-agnostic linter for comments that don't earn their place: ones that restate the code, commented-out code, banners, change notes and signature-only docs. LLM-written code is full of them.

prolix finds every comment in a repo with a byte-level lexer (40+ languages), asks [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev) what kind of comment each one is, and flags the ones your chosen level doesn't keep. `prolix --fix` removes them, including the lines and blank-line gaps they leave behind.

**[Docs](https://prolix.barclaysd.workers.dev)**, with prompts to hand to your coding agent for [setting it up](https://prolix.barclaysd.workers.dev/setup/) and [cleaning up an existing repo](https://prolix.barclaysd.workers.dev/adopt/).

```
$ npx @prolix/cli
src/client.ts
  12:3   restates-code       // create the client
  40:1   commented-out-code  // const old = retry(req);
  57:26  change-note         // Updated to use the new API

Found 3 comments in 1 file (level: value-add):

      1  restates-code       repeats what the code already says
      1  commented-out-code  disabled code left behind
      1  change-note         narrates an edit instead of the code as it is

Run `prolix --fix` to remove them.
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
prolix src lib --level necessary   # check specific paths at a stricter level
prolix --fix                       # remove everything flagged
prolix --changed=main              # only comments on lines added since the branch left main
prolix --changed --fix             # remove flagged comments in uncommitted changes, e.g. in a pre-commit hook
prolix --reporter json             # machine-readable output with Jev's probabilities and each fix
prolix --reporter markdown         # a summary for a pull request comment or job summary
```

`--changed` reads `git diff` against the merge base with the ref (default `HEAD`). It counts uncommitted edits and untracked files, and only sends Jev the comments that touch added lines. In the JSON report, each flagged comment's `fix` gives the lines to replace (`startLine` to `endLine`) and their `replacement`, which is what the GitHub Action posts as a suggestion.

The walk respects `.gitignore`, `.ignore` and hidden files, and skips files over 1 MB (such as minified bundles).

| Exit code | Meaning |
| --- | --- |
| 0 | nothing flagged, or `--fix` succeeded |
| 1 | comments flagged |
| 2 | error (bad config, missing key, Jev unreachable) |

## GitHub Action

The action runs prolix on the lines a pull request adds. When it flags something, it posts a summary comment and a one-click suggestion to remove each comment. Later runs update the same comment (to ✅ once the pull request is clean), don't repeat suggestions and delete ones that no longer apply.

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
| `api-key` | | Typesafe API key. When it's empty the check is skipped with a notice, because GitHub doesn't pass secrets to pull requests from forks or Dependabot. |
| `level` | `prolix.jsonc`, then `value-add` | `all`, `value-add`, `necessary` or `none` |
| `scope` | `changed` | `full` checks the whole repository |
| `comment` | `true` | post and update the summary comment |
| `suggestions` | `true` | post a suggestion per flagged comment |
| `fail-on-findings` | `false` | fail the job when anything is flagged, instead of only advising |
| `version` | `latest` | the `@prolix/cli` version to run |

The `flagged` output is the number of comments flagged. The summary also goes to the run page.

`ref` and `fetch-depth: 0` check out the pull request's own commits rather than GitHub's merge commit, so suggestions land on the right lines. Without them the action still comments, and warns that it skipped the suggestions.

## Adopting prolix in an existing repo

The action only checks the lines a pull request adds, so turning it on never flags the comments already in a repo. To clear those:

1. Add the workflow first, so new comments are checked from the next pull request.
2. Remove the backlog in one pull request at `value-add`, which only removes comments that restate the code, disabled code, banners, change notes and signature-only docs. Review the diff like any other, and restore anything worth keeping with `prolix-ignore` added to it. In a large repo, fix a directory at a time (`npx @prolix/cli src/components --level value-add --fix`) so each pull request stays reviewable.

   ```sh
   npx @prolix/cli --level value-add --fix
   ```

3. For a stricter bar, set `"level": "necessary"` in `prolix.jsonc` and run `--fix` again. It removes TODOs and clarifying comments too.

## Levels

| Level | Keeps | Removes |
| --- | --- | --- |
| `all` | everything | nothing |
| `value-add` (default) | anything that tells the reader something the code doesn't | restates-code, commented-out-code, decorative, change-note, redundant-doc |
| `necessary` | explains-why, warning, api-doc, reference | the above, plus todo and clarifies |
| `none` | tool directives and licences only | every other comment (no Jev call) |

Tool and compiler directives are never touched. prolix recognises them by shape rather than by a list of tool names, so a linter it has never heard of is still respected:

- a switch word joined to a name or followed by a rule: `eslint-disable-next-line`, `react-doctor-disable-line`, `biome-ignore`, `# hadolint ignore=DL3008`, `/* c8 ignore next */`
- `no…` markers and `tool:setting` tokens: `# noqa: E501`, `// NOLINTNEXTLINE`, `# rubocop:disable`, `//go:build`, `// gitleaks:allow`
- tags and settings: `@ts-expect-error`, `$FlowFixMe`, `# shellcheck source=lib.sh`, `# syntax=docker/dockerfile:1`, `/* webpackChunkName: "x" */`
- version pins beside a hash, as in `uses: actions/checkout@<sha> # v4.1.1`
- shebangs, pragmas, regions, `/*!` licence headers, `@generated` markers and `/// <reference>`

Add `prolix-ignore` to any comment to keep it.

## Configuration

`prolix.jsonc` (or `prolix.json`) in the current directory or any parent:

```jsonc
{
  "level": "value-add",
  // How sure Jev must be (0-1) that a comment is removable before flagging it.
  "threshold": 0.6,
  // Globs relative to this file, on top of .gitignore.
  "ignore": ["vendor/**", "**/*.generated.ts"]
}
```

`--level` on the command line overrides the file.

## Jev and caching

| Variable | Default |
| --- | --- |
| `TYPESAFE_API_KEY` | required for `value-add` and `necessary` |
| `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` |
| `TYPESAFE_DEFAULT_MODEL` | `jev-latest` |

Each comment is sent with a few lines of surrounding code. prolix batches up to 32 comments per request and sends up to 16 requests in parallel. It retries rate limits and 5xx responses with backoff.

Answers are cached by comment, context, language and model in `node_modules/.cache/prolix.json`, or in `.prolixcache` when there is no `node_modules`. Re-runs only ask about new or changed comments, and switching levels needs no new calls.

## Evals

`tests/evals` measures Jev's judgements end to end. It runs the release binary against 15 fixtures (TypeScript, TSX, Rust, Python, Go, Java, C, Ruby, shell, SQL, CSS and YAML). Every comment in them is labelled in `cases.json` with the categories a careful reviewer would accept. The fixtures cover every category, plus these harder cases:

- warnings dressed as banners
- TODOs that also explain why
- prose that quotes code
- prompt-injection comments
- directives, which must never reach Jev

```sh
export TYPESAFE_API_KEY=...
cargo test --release --test evals -- --ignored --nocapture
```

Each fixture is judged `EVAL_REPEAT` times (default 3) from an empty cache.

The report gives, for `value-add` and `necessary`:

- removal precision, the gate, because deleting a useful comment is the costly mistake
- recall
- flip rate between repeats
- a threshold sweep
- per-category accuracy
- top-1 confusions
- every wrong decision, for error analysis

`policy.json` pins a hash of the dataset and sets the floors, so changing a label or a floor is a reviewed change. Set `EVAL_BASELINE=path/to/report.json` to also fail on a drop of more than 0.05 against an earlier run. Results go to `tests/evals/results/<run>/`. The `evals` workflow runs on pull requests that touch `src` or `tests/evals`, or on demand, and posts the summary to the run page. It needs a `TYPESAFE_API_KEY` repository secret. A full run uses about 240k input tokens.

## Known limits

- Comment-like text inside JSX (for example `<p>a // b</p>`) is guarded for `//` but not for `/*`.
- `{/* */}` comments in Svelte and Astro templates aren't linted.
- C++ raw strings (`R"(...)"`) aren't recognised.
- The cache only grows. Delete it to reset.
