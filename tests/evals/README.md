# Comment evaluation

The checked-in fixtures are a development set. Keep their precision/recall gates and dataset hash intact when investigating a failure; do not lower a floor to make a new model pass.

`field-candidates.json` starts a corpus of anonymized repository patterns from the October 2026 assessment. Its proposed categories are draft labels. They do not contribute to the current quality gate, and they must not be described as independently reviewed or held out.

For a real-world evaluation:

1. Sample both flagged and retained comments from whole repositories and PR diffs, including protected, unsupported and mixed-purpose comments. Track repository and commit provenance privately.
2. Have two reviewers label each example independently. Record disagreement; use a keep/review outcome where safe deletion is not agreed. Include the entire comment and enough enclosing code to judge it.
3. Split by repository before prompt or threshold development. Reserve the held-out repositories for release evaluation and avoid using their results to tune the prompt repeatedly.
4. Place the existing dataset format (`fixtures/`, `cases.json`, `behaviour.json`, `policy.json`) in a separate directory. Set `datasetKind` to `held-out` only after the review/split process. Pin the reviewed `datasetHash` and quality floors in its policy.
5. Run it without copying private code into this repository:

   ```sh
   EVAL_DATASET_DIR=/absolute/path/to/reviewed-corpus \
     cargo test --release --test evals -- --ignored --nocapture
   ```

Results are saved under that dataset's `results/` directory. Reports include the requested model, resolved model versions and prompt hash; `EVAL_REPEAT` and `EVAL_BASELINE` work for external datasets too. Use a pinned `TYPESAFE_DEFAULT_MODEL` for release comparisons.

Measure extraction coverage and safe transformations separately from model precision. The offline CLI regression suite exercises malformed responses, code/data boundaries, long comments, duplicate inference, and source changes during a request. The live suite measures classifications; it cannot by itself establish that every automatic edit is safe.
