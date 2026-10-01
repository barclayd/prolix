use std::path::Path;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Plain,
    Js,
    Rust,
    Go,
    Lua,
    Heredoc,
    Markup,
    Css,
    Yaml,
}

pub struct Lang {
    pub name: &'static str,
    pub line: &'static [&'static str],
    pub block: &'static [(&'static str, &'static str)],
    pub nested: bool,
    pub quotes: &'static [u8],
    /// Strings may span lines. Off where a stray apostrophe would otherwise swallow the file.
    pub multiline: bool,
    pub triple: bool,
    /// `'` is a char literal only as `'x'` / `'\n'`, otherwise a lifetime, type variable or prime.
    pub tick: bool,
    pub jsx: bool,
    pub kind: Kind,
}

impl Lang {
    /// Configuration, where comments label sections and keep switched-off settings to hand.
    pub fn config(&self) -> bool {
        matches!(
            self.name,
            "YAML" | "TOML" | "HCL" | "JSONC" | "Dockerfile" | "Makefile" | "CMake"
        )
    }
}

pub struct Raw {
    pub start: usize,
    pub end: usize,
    pub line: bool,
}

const C: Lang = Lang {
    name: "C",
    line: &["//"],
    block: &[("/*", "*/")],
    nested: false,
    quotes: b"\"'",
    multiline: false,
    triple: false,
    tick: false,
    jsx: false,
    kind: Kind::Plain,
};
const HASH: Lang = Lang {
    name: "Shell",
    line: &["#"],
    block: &[],
    ..C
};
const MARKUP: Lang = Lang {
    name: "HTML",
    line: &[],
    block: &[("<!--", "-->")],
    quotes: b"",
    kind: Kind::Markup,
    ..C
};

pub const JS: Lang = Lang {
    name: "JavaScript",
    kind: Kind::Js,
    ..C
};
pub const TS: Lang = Lang {
    name: "TypeScript",
    ..JS
};
pub const JSONC: Lang = Lang { name: "JSONC", ..C };
const CSS: Lang = Lang {
    name: "CSS",
    line: &[],
    kind: Kind::Css,
    ..C
};
const SCSS: Lang = Lang {
    name: "SCSS",
    line: &["//"],
    ..CSS
};

pub fn lang_for(path: &Path) -> Option<&'static Lang> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".min.js") || name.ends_with(".min.css") {
        return None;
    }
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    Some(match (name, ext.as_str()) {
        (n, _) if n.starts_with("Dockerfile") || n.starts_with("Containerfile") => &Lang {
            name: "Dockerfile",
            ..HASH
        },
        ("Makefile" | "GNUmakefile" | "makefile", _) | (_, "mk") => &Lang {
            name: "Makefile",
            ..HASH
        },
        ("CMakeLists.txt", _) | (_, "cmake") => &Lang {
            name: "CMake",
            ..HASH
        },
        ("Rakefile" | "Gemfile", _) | (_, "rb" | "rake" | "gemspec") => &Lang {
            name: "Ruby",
            multiline: true,
            kind: Kind::Heredoc,
            ..HASH
        },
        (_, "js" | "mjs" | "cjs") => &JS,
        (_, "jsx") => &Lang {
            name: "JavaScript (JSX)",
            jsx: true,
            ..JS
        },
        (_, "ts" | "mts" | "cts") => &TS,
        (_, "tsx") => &Lang {
            name: "TypeScript (TSX)",
            jsx: true,
            ..JS
        },
        (_, "c" | "h") => &C,
        (_, "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino") => &Lang { name: "C++", ..C },
        (_, "cs") => &Lang {
            name: "C#",
            triple: true,
            ..C
        },
        (_, "java") => &Lang {
            name: "Java",
            triple: true,
            ..C
        },
        (_, "kt" | "kts") => &Lang {
            name: "Kotlin",
            nested: true,
            triple: true,
            ..C
        },
        (_, "scala" | "sc") => &Lang {
            name: "Scala",
            nested: true,
            triple: true,
            ..C
        },
        (_, "swift") => &Lang {
            name: "Swift",
            nested: true,
            triple: true,
            ..C
        },
        (_, "dart") => &Lang {
            name: "Dart",
            nested: true,
            triple: true,
            ..C
        },
        (_, "groovy" | "gradle") => &Lang {
            name: "Groovy",
            triple: true,
            ..C
        },
        (_, "proto") => &Lang {
            name: "Protobuf",
            ..C
        },
        (_, "sol") => &Lang {
            name: "Solidity",
            ..C
        },
        (_, "jsonc" | "json5") => &JSONC,
        (_, "php") => &Lang {
            name: "PHP",
            multiline: true,
            kind: Kind::Heredoc,
            ..C
        },
        (_, "go") => &Lang {
            name: "Go",
            kind: Kind::Go,
            ..C
        },
        (_, "rs") => &Lang {
            name: "Rust",
            nested: true,
            quotes: b"\"",
            multiline: true,
            tick: true,
            kind: Kind::Rust,
            ..C
        },
        (_, "css") => &CSS,
        (_, "scss") => &SCSS,
        (_, "less") => &Lang {
            name: "Less",
            ..SCSS
        },
        (_, "sass") => &Lang {
            name: "Sass",
            ..SCSS
        },
        (_, "html" | "htm" | "xhtml") => &MARKUP,
        (_, "xml" | "svg" | "plist" | "csproj") => &Lang {
            name: "XML",
            ..MARKUP
        },
        (_, "vue") => &Lang {
            name: "Vue",
            ..MARKUP
        },
        (_, "svelte") => &Lang {
            name: "Svelte",
            ..MARKUP
        },
        (_, "astro") => &Lang {
            name: "Astro",
            ..MARKUP
        },
        (_, "py" | "pyi" | "pyw") => &Lang {
            name: "Python",
            triple: true,
            ..HASH
        },
        (_, "sh" | "bash" | "zsh" | "ksh" | "fish") => &Lang {
            multiline: true,
            kind: Kind::Heredoc,
            ..HASH
        },
        (_, "pl" | "pm") => &Lang {
            name: "Perl",
            multiline: true,
            kind: Kind::Heredoc,
            ..HASH
        },
        (_, "r") => &Lang {
            name: "R",
            multiline: true,
            ..HASH
        },
        (_, "jl") => &Lang {
            name: "Julia",
            block: &[("#=", "=#")],
            nested: true,
            triple: true,
            ..HASH
        },
        (_, "ex" | "exs") => &Lang {
            name: "Elixir",
            triple: true,
            ..HASH
        },
        (_, "nim") => &Lang {
            name: "Nim",
            block: &[("#[", "]#")],
            nested: true,
            triple: true,
            ..HASH
        },
        (_, "ps1" | "psm1") => &Lang {
            name: "PowerShell",
            block: &[("<#", "#>")],
            ..HASH
        },
        (_, "toml") => &Lang {
            name: "TOML",
            triple: true,
            ..HASH
        },
        (_, "yaml" | "yml") => &Lang {
            name: "YAML",
            kind: Kind::Yaml,
            ..HASH
        },
        (_, "graphql" | "gql") => &Lang {
            name: "GraphQL",
            quotes: b"\"",
            triple: true,
            ..HASH
        },
        (_, "tf" | "tfvars" | "hcl") => &Lang {
            name: "HCL",
            line: &["#", "//"],
            block: &[("/*", "*/")],
            ..HASH
        },
        (_, "nix") => &Lang {
            name: "Nix",
            block: &[("/*", "*/")],
            quotes: b"\"",
            multiline: true,
            ..HASH
        },
        (_, "sql") => &Lang {
            name: "SQL",
            line: &["--"],
            multiline: true,
            ..C
        },
        (_, "lua") => &Lang {
            name: "Lua",
            line: &["--"],
            block: &[],
            kind: Kind::Lua,
            ..C
        },
        (_, "hs") => &Lang {
            name: "Haskell",
            line: &["--"],
            block: &[("{-", "-}")],
            nested: true,
            quotes: b"\"",
            tick: true,
            ..C
        },
        (_, "elm") => &Lang {
            name: "Elm",
            line: &["--"],
            block: &[("{-", "-}")],
            nested: true,
            quotes: b"\"",
            tick: true,
            ..C
        },
        (_, "ml" | "mli") => &Lang {
            name: "OCaml",
            line: &[],
            block: &[("(*", "*)")],
            nested: true,
            quotes: b"\"",
            multiline: true,
            tick: true,
            ..C
        },
        (_, "fs" | "fsi" | "fsx") => &Lang {
            name: "F#",
            block: &[("(*", "*)")],
            nested: true,
            quotes: b"\"",
            triple: true,
            tick: true,
            ..C
        },
        (_, "clj" | "cljs" | "cljc" | "edn" | "lisp" | "el" | "scm" | "rkt") => &Lang {
            name: "Lisp",
            line: &[";"],
            block: &[],
            quotes: b"\"",
            multiline: true,
            ..C
        },
        _ => return None,
    })
}

/// Appends every comment in `src` (offsets shifted by `base`) to `out`.
pub fn scan(src: &[u8], lang: &Lang, base: usize, out: &mut Vec<Raw>) {
    if lang.kind == Kind::Markup {
        return markup(src, lang, base, out);
    }
    let n = src.len();
    let mut i = 0;
    let mut prev = b'\n';
    let mut kw = false;
    let mut depth = 0u32;
    let mut tmpl: Vec<u32> = Vec::new();
    let mut heredoc: Option<(usize, usize)> = None;
    let mut yblock: Option<usize> = None;
    while i < n {
        let c = src[i];
        if c == b'\n' {
            i += 1;
            if let Some((s, e)) = heredoc.take() {
                i = heredoc_end(src, i, &src[s..e]);
            }
            if let Some(parent) = yblock.take() {
                i = yaml_block_end(src, i, parent);
            }
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if let Some(&(open, close)) = lang
            .block
            .iter()
            .find(|(o, _)| rest.starts_with(o.as_bytes()))
        {
            if let Some(end) = block_end(
                src,
                i + open.len(),
                open.as_bytes(),
                close.as_bytes(),
                lang.nested,
            ) {
                out.push(Raw {
                    start: base + i,
                    end: base + end,
                    line: false,
                });
                i = end;
                continue;
            }
        }
        if let Some(m) = lang.line.iter().find(|m| rest.starts_with(m.as_bytes())) {
            if line_ok(src, i, m.as_bytes()[0], lang) {
                if let Some(end) = (lang.kind == Kind::Lua)
                    .then(|| lua_long(src, i + 2))
                    .flatten()
                {
                    out.push(Raw {
                        start: base + i,
                        end: base + end,
                        line: false,
                    });
                    i = end;
                    continue;
                }
                let nl = rest.iter().position(|&b| b == b'\n').map_or(n, |k| i + k);
                let end = if src[nl - 1] == b'\r' { nl - 1 } else { nl };
                out.push(Raw {
                    start: base + i,
                    end: base + end,
                    line: true,
                });
                i = nl;
                continue;
            }
        }
        let was_kw = std::mem::replace(&mut kw, false);
        let p = std::mem::replace(&mut prev, c);
        if lang.triple && lang.quotes.contains(&c) && rest.starts_with(&[c, c, c]) {
            i = triple_end(src, i + 3, c);
            prev = b'a';
        } else if lang.tick && c == b'\'' {
            i = tick(src, i);
            prev = b'a';
        } else if lang.quotes.contains(&c)
            && !(lang.kind == Kind::Yaml && i > 0 && is_ident(src[i - 1]))
        {
            i = string_end(src, i + 1, c, lang.multiline);
            prev = b'a';
        } else if is_ident(c) {
            let s = i;
            while i < n && is_ident(src[i]) {
                i += 1;
            }
            let w = &src[s..i];
            if lang.kind == Kind::Rust && matches!(w, b"r" | b"br" | b"cr") {
                i = rust_raw(src, i).unwrap_or(i);
            }
            kw = lang.kind == Kind::Js && JS_KW.contains(&w);
            prev = b'a';
        } else {
            i = match (lang.kind, c) {
                (Kind::Js, b'`') => template(src, i + 1, depth, &mut tmpl, &mut prev),
                (Kind::Js, b'{') => {
                    depth += 1;
                    i + 1
                }
                (Kind::Js, b'}') if tmpl.last() == Some(&depth) => {
                    tmpl.pop();
                    template(src, i + 1, depth, &mut tmpl, &mut prev)
                }
                (Kind::Js, b'}') => {
                    depth = depth.saturating_sub(1);
                    i + 1
                }
                (Kind::Js, b'/') if was_kw || b"(,=:[!&|?{};+-*%<>~^\n".contains(&p) => {
                    regex_end(src, i + 1).map_or(i + 1, |e| {
                        prev = b'a';
                        e
                    })
                }
                (Kind::Go, b'`') => {
                    prev = b'a';
                    find(src, i + 1, b"`").map_or(n, |e| e + 1)
                }
                (Kind::Lua, b'[') => lua_long(src, i).map_or(i + 1, |e| {
                    prev = b'a';
                    e
                }),
                (Kind::Heredoc, b'<') => heredoc_start(src, i, &mut heredoc),
                // `key: |`, `- >-` or `run: |2`: a block scalar, whose lines are text however they look.
                (Kind::Yaml, b'|' | b'>')
                    if b":-".contains(&p) && src[i - 1].is_ascii_whitespace() =>
                {
                    let j = i
                        + 1
                        + src[i + 1..]
                            .iter()
                            .take_while(|b| b"-+0123456789".contains(b))
                            .count();
                    if src.get(j).is_none_or(u8::is_ascii_whitespace) {
                        let ls = src[..i]
                            .iter()
                            .rposition(|&b| b == b'\n')
                            .map_or(0, |k| k + 1);
                        yblock = Some(src[ls..].iter().take_while(|&&b| b == b' ').count());
                    }
                    j
                }
                _ => i + 1,
            };
        }
    }
}

const JS_KW: &[&[u8]] = &[
    b"return",
    b"typeof",
    b"case",
    b"do",
    b"else",
    b"in",
    b"of",
    b"new",
    b"delete",
    b"void",
    b"throw",
    b"instanceof",
    b"yield",
    b"await",
];

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn line_ok(s: &[u8], i: usize, m0: u8, lang: &Lang) -> bool {
    let Some(&b) = i.checked_sub(1).map(|k| &s[k]) else {
        return true;
    };
    match m0 {
        _ if b == b'\\' => false,
        // `$#`, `a#b` and `#fff`-style tokens aren't comments.
        b'#' if matches!(lang.name, "Python" | "Ruby") => true,
        b'#' => b.is_ascii_whitespace(),
        b'/' => b != b':' && !(lang.kind == Kind::Css && b == b'('),
        _ => true,
    }
}

pub fn find(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    s.get(from..)?
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|k| from + k)
}

fn find_ci(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    s.get(from..)?
        .windows(pat.len())
        .position(|w| w.eq_ignore_ascii_case(pat))
        .map(|k| from + k)
}

fn block_end(s: &[u8], mut j: usize, open: &[u8], close: &[u8], nested: bool) -> Option<usize> {
    let mut depth = 1;
    while j < s.len() {
        if s[j..].starts_with(close) {
            depth -= 1;
            j += close.len();
            if depth == 0 {
                return Some(j);
            }
        } else if nested && s[j..].starts_with(open) {
            depth += 1;
            j += open.len();
        } else {
            j += 1;
        }
    }
    None
}

fn string_end(s: &[u8], mut j: usize, q: u8, multiline: bool) -> usize {
    while j < s.len() {
        match s[j] {
            b'\\' => j += 2,
            b'\n' if !multiline => return j,
            c if c == q => return j + 1,
            _ => j += 1,
        }
    }
    s.len()
}

fn triple_end(s: &[u8], mut j: usize, q: u8) -> usize {
    while j < s.len() {
        if s[j] == b'\\' {
            j += 2;
        } else if s[j..].starts_with(&[q, q, q]) {
            return j + 3;
        } else {
            j += 1;
        }
    }
    s.len()
}

fn tick(s: &[u8], i: usize) -> usize {
    match s.get(i + 1) {
        Some(b'\\') => string_end(s, i + 1, b'\'', false),
        Some(&b) => {
            let k = i + 1 + [1, 1, 2, 3, 4][(b.leading_ones() as usize).min(4)];
            if s.get(k) == Some(&b'\'') {
                k + 1
            } else {
                i + 1
            }
        }
        None => i + 1,
    }
}

fn template(s: &[u8], mut j: usize, depth: u32, tmpl: &mut Vec<u32>, prev: &mut u8) -> usize {
    while j < s.len() {
        match s[j] {
            b'\\' => j += 2,
            b'`' => {
                *prev = b'a';
                return j + 1;
            }
            b'$' if s.get(j + 1) == Some(&b'{') => {
                tmpl.push(depth);
                *prev = b'{';
                return j + 2;
            }
            _ => j += 1,
        }
    }
    s.len()
}

fn regex_end(s: &[u8], mut j: usize) -> Option<usize> {
    let mut class = false;
    while j < s.len() {
        match s[j] {
            b'\\' => j += 1,
            b'\n' => return None,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(j + 1),
            _ => {}
        }
        j += 1;
    }
    None
}

fn rust_raw(s: &[u8], j: usize) -> Option<usize> {
    let h = s[j..].iter().take_while(|&&b| b == b'#').count();
    if s.get(j + h) != Some(&b'"') {
        return None;
    }
    let close: Vec<u8> = std::iter::once(b'"')
        .chain(std::iter::repeat_n(b'#', h))
        .collect();
    Some(find(s, j + h + 1, &close).map_or(s.len(), |k| k + close.len()))
}

fn lua_long(s: &[u8], j: usize) -> Option<usize> {
    if s.get(j) != Some(&b'[') {
        return None;
    }
    let h = s[j + 1..].iter().take_while(|&&b| b == b'=').count();
    if s.get(j + 1 + h) != Some(&b'[') {
        return None;
    }
    let close: Vec<u8> = std::iter::once(b']')
        .chain(std::iter::repeat_n(b'=', h))
        .chain(*b"]")
        .collect();
    find(s, j + h + 2, &close).map(|k| k + close.len())
}

fn heredoc_start(s: &[u8], i: usize, tag: &mut Option<(usize, usize)>) -> usize {
    if s.get(i + 1) != Some(&b'<') {
        return i + 1;
    }
    let mut j = i + 2;
    if s.get(j) == Some(&b'<') {
        j += 1;
    }
    if matches!(s.get(j), Some(b'-' | b'~')) {
        j += 1;
    }
    let spaced = s.get(j) == Some(&b' ');
    while s.get(j) == Some(&b' ') {
        j += 1;
    }
    let quoted = matches!(s.get(j), Some(b'\'' | b'"'));
    j += quoted as usize;
    let start = j;
    while s
        .get(j)
        .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
    {
        j += 1;
    }
    let word = &s[start..j];
    // `arr << item` is a push; only an uppercase word after a space reads as a tag.
    let ok = word.first().is_some_and(|b| !b.is_ascii_digit())
        && (!spaced || word.iter().all(|b| !b.is_ascii_lowercase()));
    if !ok {
        return i + 2;
    }
    *tag = Some((start, j));
    j + quoted as usize
}

fn heredoc_end(s: &[u8], mut j: usize, tag: &[u8]) -> usize {
    while j < s.len() {
        let e = s[j..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(s.len(), |k| j + k);
        let line = &s[j..e];
        let t = &line[line.iter().take_while(|b| b.is_ascii_whitespace()).count()..];
        if t.starts_with(tag) && !t.get(tag.len()).is_some_and(|&b| is_ident(b)) {
            return e;
        }
        j = e + 1;
    }
    s.len()
}

/// Skips a block scalar opened on a line indented `parent` spaces. Its first non-blank line sets the indent the rest
/// keep, and blank lines in between belong to it. Scalar contents are data, including lines starting with `#`.
fn yaml_block_end(s: &[u8], mut j: usize, parent: usize) -> usize {
    let mut indent = None;
    while j < s.len() {
        let e = s[j..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(s.len(), |k| j + k);
        let line = &s[j..e];
        let ind = line.iter().take_while(|&&b| b == b' ').count();
        let t = line[ind..].trim_ascii_end();
        if !t.is_empty() {
            let want = *indent.get_or_insert(ind);
            if ind <= parent || ind < want {
                return j;
            }
        }
        j = e + 1;
    }
    s.len()
}

fn markup(s: &[u8], lang: &Lang, base: usize, out: &mut Vec<Raw>) {
    let mut i = 0;
    if lang.name == "Astro" && s.starts_with(b"---") {
        if let Some(k) = find(s, 3, b"\n---") {
            scan(&s[3..k], &TS, base + 3, out);
            i = k + 4;
        }
    }
    while let Some(k) = s[i..].iter().position(|&b| b == b'<') {
        i += k;
        let rest = &s[i..];
        if rest.starts_with(b"<!--") {
            let Some(e) = find(s, i + 4, b"-->") else {
                return;
            };
            out.push(Raw {
                start: base + i,
                end: base + e + 3,
                line: false,
            });
            i = e + 3;
            continue;
        }
        let tag = [&b"script"[..], b"style"].into_iter().find(|t| {
            rest.len() > t.len() + 1
                && rest[1..=t.len()].eq_ignore_ascii_case(t)
                && (rest[t.len() + 1] == b'>' || rest[t.len() + 1].is_ascii_whitespace())
        });
        let Some(tag) = tag else {
            i += 1;
            continue;
        };
        let Some(open) = find(s, i, b">") else { return };
        let close = find_ci(s, open, &[b"</", tag].concat()).unwrap_or(s.len());
        let attrs = &s[i..open];
        let inner = match tag {
            b"style" if find_ci(attrs, 0, b"lang").is_some() => &SCSS,
            b"style" => &CSS,
            _ => &TS,
        };
        scan(&s[open + 1..close], inner, base + open + 1, out);
        i = close;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(src: &str, file: &str) -> Vec<String> {
        let mut out = Vec::new();
        scan(
            src.as_bytes(),
            lang_for(Path::new(file)).unwrap(),
            0,
            &mut out,
        );
        out.iter()
            .map(|r| src[r.start..r.end].to_string())
            .collect()
    }

    #[test]
    fn js_strings_regex_templates() {
        let src = r#"const u = "http://x // no"; // yes
const r = /\/\/ no/g; a = b / c; // yes2
const t = `a ${ {b: "}"} /* in */ } // no ${`${x}`} `; /* yes3 */
const s = 'it\'s // no';
x = y // yes4
"#;
        assert_eq!(
            comments(src, "a.ts"),
            ["// yes", "// yes2", "/* in */", "/* yes3 */", "// yes4"]
        );
    }

    #[test]
    fn rust_ticks_raw_nested() {
        let src = "fn f<'a>(x: &'a str) -> char { '\"' } // one\nlet s = r#\"// no\"#; /* a /* b */ c */\nlet c = '/'; let u = \"multi\n// no\n\";";
        assert_eq!(comments(src, "a.rs"), ["// one", "/* a /* b */ c */"]);
    }

    #[test]
    fn hash_python_shell_yaml() {
        let py = "x = 1  # yes\ns = \"# no\"\nd = \"\"\"\n# no\n\"\"\"\n";
        assert_eq!(comments(py, "a.py"), ["# yes"]);
        let sh = "echo $# ${#a} # yes\ncat <<'EOF'\n# no\nEOF\narr=(1) # yes2\n";
        assert_eq!(comments(sh, "a.sh"), ["# yes", "# yes2"]);
        assert_eq!(
            comments("a: b#c # yes\nd: don't # yes2\n", "a.yml"),
            ["# yes", "# yes2"]
        );
        let y = "a: |\n  ### no\n  x # no\n\n  # yes\n# yes2\nb: >- # yes3\n  ## no\nsteps:\n  - run: |\n      echo # no\n  # yes4\nc: x | y # yes5\n";
        assert_eq!(
            comments(y, "a.yml"),
            ["# yes2", "# yes3", "# yes4", "# yes5"]
        );
    }

    #[test]
    fn markup_embeds() {
        let src = "<!-- a --><script lang=\"ts\">// b\n</script><style lang=\"scss\">a{background:url(//x)} // c\n</style>";
        assert_eq!(comments(src, "a.vue"), ["<!-- a -->", "// b", "// c"]);
    }

    #[test]
    fn lua_go_sql() {
        assert_eq!(
            comments("--[[ a ]] x = [[ -- no ]] -- b\n", "a.lua"),
            ["--[[ a ]]", "-- b"]
        );
        assert_eq!(comments("s := `// no` // yes\n", "a.go"), ["// yes"]);
        assert_eq!(comments("select '--no' -- yes\n", "a.sql"), ["-- yes"]);
    }
}
