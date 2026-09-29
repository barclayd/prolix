---
"@prolix/cli": patch
---

Ask Jev about each `keep` and `remove` rule in its own question, weighed against the categories. Rules no longer compete with each other or take probability from the categories, so adding a rule doesn't dilute the score of a comment the mode already removes, and a `remove` rule adds to the removed categories rather than splitting their score. Changing a rule asks Jev again about that rule only, and moving a rule between `keep` and `remove` reuses the cached answers.
