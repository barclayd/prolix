---
"@prolix/cli": minor
---

Add a `shorten` setting. When it's on, Claude rewrites comments that are kept but could say the same in fewer words, such as one that opens with a tool's `ponytail:` label. A rewrite is only used when it's shorter and contains nothing but comments. `--fix` applies it, and the action suggests it with the new `anthropic-api-key` input.
