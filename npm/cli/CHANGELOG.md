# @prolix/cli

## 0.1.0

### Minor Changes

- [#8](https://github.com/barclayd/prolix/pull/8) [`576bb17`](https://github.com/barclayd/prolix/commit/576bb17dc8628ab00388bc21821fe792531c15ea) Thanks [@barclayd](https://github.com/barclayd)! - First release. prolix finds comments that don't earn their place, judged by Jev, and removes them, or suggests removing them on a pull request with the GitHub Action.

- [#7](https://github.com/barclayd/prolix/pull/7) [`1f6c810`](https://github.com/barclayd/prolix/commit/1f6c81025942ba8fd50fa427a8cec8bd7d942dd3) Thanks [@barclayd](https://github.com/barclayd)! - Tool directives are recognised by their shape rather than a list of tool names, so `eslint-disable-next-line`, `react-doctor-disable-line` and linters prolix has never seen are never touched. YAML block scalars are read as text, and config files (YAML, TOML, HCL, JSONC, Dockerfiles, Makefiles and CMake) keep section labels, separators and switched-off settings until `necessary`.
