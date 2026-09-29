---
"@prolix/cli": minor
---

`level` is now `mode`, with the values `off`, `standard` (the default) and `strict`. The new `keep` and `remove` settings take category names or rules in plain English, which Jev judges each comment against alongside the categories:

```jsonc
{
  "mode": "strict",
  "keep": ["todo", "States a fact the code relies on but can't show: what a tool does, where a file is generated, what a limit or constant means"],
  "remove": ["Reassures a reviewer that another code path still works, instead of explaining this one"]
}
```

`level`, `--level`, the action's `level` input and the values `all`, `value-add` and `necessary` still work, with a deprecation warning. `none` was removed: use `strict` with `"remove": ["explains-why", "warning", "api-doc", "reference"]`. `--mode off` now counts comments without asking Jev.
