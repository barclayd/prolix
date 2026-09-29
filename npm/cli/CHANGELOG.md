# @prolix/cli

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
