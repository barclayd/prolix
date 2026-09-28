# prolix

A fast, language-agnostic linter for comments that don't earn their place: ones that restate the code, commented-out code, banners, change notes and signature-only docs. LLM-written code is full of them.

prolix finds every comment in a repo with a byte-level lexer (40+ languages), asks [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev) what kind of comment each one is, and flags the ones your chosen level doesn't keep. `prolix --fix` removes them, including the lines and blank-line gaps they leave behind.

```
$ npx prolix
src/client.ts
  12:3   restates-code       // create the client
  40:1   commented-out-code  // const old = retry(req);
  57:26  change-note         // Updated to use the new API

Found 3 comments in 1 file (level: value-add):

      1  restates-code       repeats what the code already says
      1  commented-out-code  disabled code left behind
      1  change-note         narrates an edit instead of the code as it is

Run `prolix --fix` to remove them.
Checked 214 comments in 38 files in 0.61s · Jev: 214 asked, 0 cached, 41k tokens
```

## Install

```sh
npm i -D @prolix/cli
```

This works like Biome: the wrapper installs a prebuilt binary for your platform (macOS arm64/x64, Linux arm64/x64, Windows x64). If you use Rust, you can run `cargo install --path .` instead.

## Usage

```sh
export TYPESAFE_API_KEY=...        # from typesafe.ai
prolix                             # check the current directory
prolix src lib --level necessary   # check specific paths at a stricter level
prolix --fix                       # remove everything flagged
prolix --reporter json             # machine-readable output with Jev's probabilities
```

The walk respects `.gitignore`, `.ignore` and hidden files, and skips files over 1 MB (such as minified bundles).

| Exit code | Meaning |
| --- | --- |
| 0 | nothing flagged, or `--fix` succeeded |
| 1 | comments flagged |
| 2 | error (bad config, missing key, Jev unreachable) |

## Levels

| Level | Keeps | Removes |
| --- | --- | --- |
| `all` | everything | nothing |
| `value-add` (default) | anything that tells the reader something the code doesn't | restates-code, commented-out-code, decorative, change-note, redundant-doc |
| `necessary` | explains-why, warning, api-doc, reference | the above, plus todo and clarifies |
| `none` | tool directives and licences only | every other comment (no Jev call) |

Tool and compiler directives are never touched. These include `eslint-disable`, `@ts-expect-error`, `biome-ignore`, `prettier-ignore`, `# noqa`, `# type: ignore`, `//go:build`, shebangs, `/*!` licence headers and `/// <reference>`. Add `prolix-ignore` to any comment to keep it.

## Configuration

`prolix.jsonc` (or `prolix.json`) in the current directory or any parent:

```jsonc
{
  "$schema": "./node_modules/@prolix/cli/configuration_schema.json",
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

`policy.json` pins a hash of the dataset and sets the floors, so changing a label or a floor is a reviewed change. Set `EVAL_BASELINE=path/to/report.json` to also fail on a drop of more than 0.05 against an earlier run. Results go to `tests/evals/results/<run>/` and to `GITHUB_STEP_SUMMARY` in CI. A full run uses about 240k input tokens.

## Known limits

- Comment-like text inside JSX (for example `<p>a // b</p>`) is guarded for `//` but not for `/*`.
- `{/* */}` comments in Svelte and Astro templates aren't linted.
- C++ raw strings (`R"(...)"`) aren't recognised.
- The cache only grows. Delete it to reset.
