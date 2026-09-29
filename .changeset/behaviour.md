---
"@prolix/cli": minor
---

The new `behaviour` setting tells Jev in plain English how your team judges its comments, and Jev weighs it when it picks each comment's category. `keep` and `remove` now take category names only:

```jsonc
{
  "mode": "strict",
  "keep": ["todo"],
  "behaviour": "Keep comments that state a fact the code relies on but can't show: what a tool does, where a file is generated, what a limit or constant means."
}
```

Each comment is one question to Jev again, however many rules a config had. Plain-English rules in `keep` and `remove` still work, with a deprecation warning: each becomes a sentence of `behaviour`. The JSON report's `rule` field is gone. Changing `behaviour` asks Jev again about every comment, and configs without one keep their cached answers.
