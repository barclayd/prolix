---
"@prolix/cli": minor
---

Resolve obsolete Prolix review threads while preserving discussion and human resolutions. Ignore stale or incomplete checks and reuse existing suggestions.

Use parsers for JS/TS/JSX/TSX extraction and validate executable structure before automatic removal. Keep YAML scalar data and oversized comment blocks, reject invalid model responses, and report incomplete scans as errors. Automatic fixes and PR suggestions now require a validated JS/TS/JSX/TSX edit and a separate `fixThreshold` (default 0.9); other findings remain available for review.

Deduplicate pending inference, retain model and prompt provenance, recover short redundant JSDoc and Python inline comments, and support external evaluation corpora through `EVAL_DATASET_DIR`.
