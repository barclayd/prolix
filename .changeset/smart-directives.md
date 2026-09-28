---
"@prolix/cli": minor
---

Tool directives are recognised by their shape rather than a list of tool names, so `eslint-disable-next-line`, `react-doctor-disable-line` and linters prolix has never seen are never touched. YAML block scalars are read as text, and config files (YAML, TOML, HCL, JSONC, Dockerfiles, Makefiles and CMake) keep section labels, separators and switched-off settings until `necessary`.
