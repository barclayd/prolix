---
"@prolix/cli": patch
---

Server-side includes are kept like other directives, in Apache, nginx and IIS spellings (`<!--#include virtual="/nav" -->`, `<!--# echo var="x" -->`, `<!-- #include file="a.inc" -->`). Before, `<!--#else -->` and `<!--#endif -->` could be flagged and removed by `--fix`. A run of line comments that quotes an SSI is kept whole, so removing it never leaves half a sentence.
