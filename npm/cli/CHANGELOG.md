# @prolix/cli

## 0.4.0

### Minor Changes

- [#25](https://github.com/barclayd/prolix/pull/25) [`2de867b`](https://github.com/barclayd/prolix/commit/2de867bf6616779a980b7af52ffeb6bcadda16c5) Thanks [@barclayd](https://github.com/barclayd)! - Resolve obsolete Prolix review threads while preserving discussion and human resolutions. Ignore stale or incomplete checks and reuse existing suggestions.

  Use parsers for JS/TS/JSX/TSX extraction and validate executable structure before automatic removal. Keep YAML scalar data and oversized comment blocks, reject invalid model responses, and report incomplete scans as errors. Automatic fixes and PR suggestions now require a validated JS/TS/JSX/TSX edit and a separate `fixThreshold` (default 0.9); other findings remain available for review.

  Deduplicate pending inference, retain model and prompt provenance, recover short redundant JSDoc and Python inline comments, and support external evaluation corpora through `EVAL_DATASET_DIR`.

  Move GitHub review orchestration into Rust with shared typed reports, a single scan per action run, direct GitHub API access, and `prolix github review --dry-run`. Preserve existing review metadata and the npm installation path.

## 0.3.0

### Minor Changes

- [#21](https://github.com/barclayd/prolix/pull/21) [`3bfb874`](https://github.com/barclayd/prolix/commit/3bfb8744b8080df4315c8bbaeea6078ea755a5ea) Thanks [@barclayd](https://github.com/barclayd)! - The new `behaviour` setting tells Jev in plain English how your team judges its comments, and Jev weighs it when it picks each comment's category. `keep` and `remove` now take category names only:

  ```jsonc
  {
    "mode": "strict",
    "keep": ["todo"],
    "behaviour": "Keep comments that state a fact the code relies on but can't show: what a tool does, where a file is generated, what a limit or constant means."
  }
  ```

  Each comment is one question to Jev again, however many rules a config had. Plain-English rules in `keep` and `remove` still work, with a deprecation warning: each becomes a sentence of `behaviour`. The JSON report's `rule` field is gone. Changing `behaviour` asks Jev again about every comment, and configs without one keep their cached answers.

### Patch Changes

- [#23](https://github.com/barclayd/prolix/pull/23) [`8d0e2fe`](https://github.com/barclayd/prolix/commit/8d0e2fe60523b78bf5a51034d5a6f7a1993a637c) Thanks [@barclayd](https://github.com/barclayd)! - Server-side includes are kept like other directives, in Apache, nginx and IIS spellings (`<!--#include virtual="/nav" -->`, `<!--# echo var="x" -->`, `<!-- #include file="a.inc" -->`). Before, `<!--#else -->` and `<!--#endif -->` could be flagged and removed by `--fix`. A run of line comments that quotes an SSI is kept whole, so removing it never leaves half a sentence.

## 0.2.1

### Patch Changes

- [#19](https://github.com/barclayd/prolix/pull/19) [`3b6c44d`](https://github.com/barclayd/prolix/commit/3b6c44d35787fcce9471ba4346dbb0b642af791b) Thanks [@barclayd](https://github.com/barclayd)! - Ask Jev about each `keep` and `remove` rule in its own question, weighed against the categories. Rules no longer compete with each other or take probability from the categories, so adding a rule doesn't dilute the score of a comment the mode already removes, and a `remove` rule adds to the removed categories rather than splitting their score. Changing a rule asks Jev again about that rule only, and moving a rule between `keep` and `remove` reuses the cached answers.

## 0.2.0

### Minor Changes

- [#18](https://github.com/barclayd/prolix/pull/18) [`b480dd5`](https://github.com/barclayd/prolix/commit/b480dd599b4f24f43f70af89c71e30f5e28e8d75) Thanks [@barclayd](https://github.com/barclayd)! - `level` is now `mode`, with the values `off`, `standard` (the default) and `strict`. The new `keep` and `remove` settings take category names or rules in plain English, which Jev judges each comment against alongside the categories:

  ```jsonc
  {
    "mode": "strict",
    "keep": [
      "todo",
      "States a fact the code relies on but can't show: what a tool does, where a file is generated, what a limit or constant means"
    ],
    "remove": [
      "Includes a sentence reassuring the reader that another code path still works or still recovers"
    ]
  }
  ```

  `level`, `--level`, the action's `level` input and the values `all`, `value-add` and `necessary` still work, with a deprecation warning. `none` was removed: use `strict` with `"remove": ["explains-why", "warning", "api-doc", "reference"]`. `--mode off` now counts comments without asking Jev.

### Patch Changes

- [#13](https://github.com/barclayd/prolix/pull/13) [`5fab1c2`](https://github.com/barclayd/prolix/commit/5fab1c27b2f48541609695662eb21d9c1f59c361) Thanks [@barclayd](https://github.com/barclayd)! - The action comments on the pull request when `TYPESAFE_API_KEY` is missing or Jev rejects it, with a link to the repository's Actions secrets.

## 0.1.0

### Minor Changes

- [#8](https://github.com/barclayd/prolix/pull/8) [`576bb17`](https://github.com/barclayd/prolix/commit/576bb17dc8628ab00388bc21821fe792531c15ea) Thanks [@barclayd](https://github.com/barclayd)! - First release. prolix finds comments that don't earn their place, judged by Jev, and removes them, or suggests removing them on a pull request with the GitHub Action.

- [#7](https://github.com/barclayd/prolix/pull/7) [`1f6c810`](https://github.com/barclayd/prolix/commit/1f6c81025942ba8fd50fa427a8cec8bd7d942dd3) Thanks [@barclayd](https://github.com/barclayd)! - Tool directives are recognised by their shape rather than a list of tool names, so `eslint-disable-next-line`, `react-doctor-disable-line` and linters prolix has never seen are never touched. YAML block scalars are read as text, and config files (YAML, TOML, HCL, JSONC, Dockerfiles, Makefiles and CMake) keep section labels, separators and switched-off settings until `necessary`.
