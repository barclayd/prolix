/// Removes each `(start, end)` comment span from `src`, taking whole lines with it when the comment stood alone.
pub fn apply(src: &str, spans: &[(usize, usize)], jsx: bool) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut cur = 0;
    for &(s, e) in spans {
        let (s, e) = if jsx { braces(b, s, e) } else { (s, e) };
        let (ls, le) = (line_start(b, s), line_end(b, e));
        let eol = if le > e && b[le - 1] == b'\r' {
            le - 1
        } else {
            le
        };
        let before = ws_len_back(b, s);
        let after = b[e..eol]
            .iter()
            .take_while(|&&c| c == b' ' || c == b'\t')
            .count();
        let own = s - before == ls && e + after == eol;
        let (mut rs, mut re, mut fill) = (s, e, "");
        if own {
            (rs, re) = (ls, (le + 1).min(b.len()));
        } else if e + after == eol {
            (rs, re) = (s - before, eol);
        } else if before > 0 && after > 0 {
            re = e + after;
        } else if before == 0
            && after == 0
            && s > 0
            && is_word(b[s - 1])
            && b.get(e).is_some_and(|&c| is_word(c))
        {
            fill = " ";
        }
        let rs = rs.max(cur);
        out.push_str(&src[cur..rs]);
        out.push_str(fill);
        if own {
            let prev_blank = match out.strip_suffix('\n') {
                Some(o) => o.rsplit('\n').next().is_some_and(|l| l.trim().is_empty()),
                None => out.is_empty(),
            };
            let next = line_end(b, re);
            if prev_blank && next < b.len() && src[re..next].trim().is_empty() {
                re = next + 1;
            } else if prev_blank && re == b.len() {
                let n = if out.ends_with("\r\n\r\n") {
                    2
                } else {
                    usize::from(out.ends_with("\n\n"))
                };
                out.truncate(out.len() - n);
            }
        }
        cur = re;
    }
    out.push_str(&src[cur..]);
    out
}

/// The lines that removing one comment changes, as 1-based `(first, last, replacement)`, for a review suggestion.
// ponytail: re-applies the fix to the whole file per comment, O(file × flagged); fine for PR-sized output.
pub fn suggestion(src: &str, span: (usize, usize), jsx: bool) -> (usize, usize, String) {
    let fixed = apply(src, &[span], jsx);
    let (a, b): (Vec<_>, Vec<_>) = (src.lines().collect(), fixed.lines().collect());
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(a.len().min(b.len()) - pre);
    let suf = suf.take_while(|(x, y)| x == y).count();
    (pre + 1, a.len() - suf, b[pre..b.len() - suf].join("\n"))
}

/// Widens `{/* ... */}` to include its braces when that is a whole line or a JSX child.
fn braces(b: &[u8], s: usize, e: usize) -> (usize, usize) {
    let a = s - ws_len_back(b, s);
    let z = e + b[e..].iter().take_while(|&&c| c == b' ').count();
    if a == 0 || b[a - 1] != b'{' || b.get(z) != Some(&b'}') {
        return (s, e);
    }
    let (bs, be) = (a - 1, z + 1);
    let alone = line_start(b, bs) == bs - ws_len_back(b, bs)
        && b[be..line_end(b, be)].iter().all(u8::is_ascii_whitespace);
    let prev = b[..bs].iter().rposition(|c| !c.is_ascii_whitespace());
    let next = b[be..]
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .map(|k| b[be + k]);
    let child =
        prev.is_some_and(|p| b[p] == b'>' && (p == 0 || b[p - 1] != b'=')) && next == Some(b'<');
    if alone || child {
        (bs, be)
    } else {
        (s, e)
    }
}

pub fn line_start(b: &[u8], i: usize) -> usize {
    b[..i]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |p| p + 1)
}

pub fn line_end(b: &[u8], i: usize) -> usize {
    b[i..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |k| i + k)
}

fn ws_len_back(b: &[u8], i: usize) -> usize {
    b[..i]
        .iter()
        .rev()
        .take_while(|&&c| c == b' ' || c == b'\t')
        .count()
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

#[cfg(test)]
mod tests {
    use super::apply;

    fn fix(src: &str, pats: &[&str], jsx: bool) -> String {
        let spans: Vec<_> = pats
            .iter()
            .map(|p| {
                let s = src.find(p).unwrap();
                (s, s + p.len())
            })
            .collect();
        apply(src, &spans, jsx)
    }

    #[test]
    fn removes_cleanly() {
        assert_eq!(fix("a\n  // x\nb\n", &["// x"], false), "a\nb\n");
        assert_eq!(fix("a(); // x\nb\n", &["// x"], false), "a();\nb\n");
        assert_eq!(fix("a\n\n// x\n\nb\n", &["// x"], false), "a\n\nb\n");
        assert_eq!(fix("// x\n\nimport a\n", &["// x"], false), "import a\n");
        assert_eq!(fix("a\n\n// x\n", &["// x"], false), "a\n");
        assert_eq!(fix("f(a /* x */ b)", &["/* x */"], false), "f(a b)");
        assert_eq!(fix("f(/* x */1)", &["/* x */"], false), "f(1)");
        assert_eq!(fix("return/* x */v", &["/* x */"], false), "return v");
        assert_eq!(fix("a\r\n// x\r\nb\r\n", &["// x"], false), "a\r\nb\r\n");
        assert_eq!(fix("a\n// x\n// y\nb\n", &["// x\n// y"], false), "a\nb\n");
    }

    #[test]
    fn suggestions() {
        let at = |src: &str, p: &str| {
            let s = src.find(p).unwrap();
            super::suggestion(src, (s, s + p.len()), false)
        };
        assert_eq!(at("a\n  // x\nb\n", "// x"), (2, 2, "".into()));
        assert_eq!(at("a(); // x\nb\n", "// x"), (1, 1, "a();".into()));
        assert_eq!(at("a\n\n// x\n\nb\n", "// x"), (3, 4, "".into()));
        assert_eq!(at("a\n// x\n// y\nb", "// x\n// y"), (2, 3, "".into()));
    }

    #[test]
    fn jsx_braces() {
        assert_eq!(
            fix("<div>\n  {/* x */}\n  <p/>\n</div>", &["/* x */"], true),
            "<div>\n  <p/>\n</div>"
        );
        assert_eq!(
            fix("<a>{/* x */}<b/></a>", &["/* x */"], true),
            "<a><b/></a>"
        );
        assert_eq!(
            fix("const f = () => {/* x */};", &["/* x */"], true),
            "const f = () => {};"
        );
    }
}
