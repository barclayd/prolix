use crate::lex::{Lang, Raw};
use std::collections::HashMap;
use tree_sitter::{Language, Node, Parser, Tree};

fn language(lang: &Lang) -> Option<Language> {
    match lang.name {
        "JavaScript" | "JavaScript (JSX)" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "TypeScript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "TypeScript (TSX)" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        _ => None,
    }
}

pub fn supported(lang: &Lang) -> bool {
    language(lang).is_some()
}

fn parse(src: &str, language: Language) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|e| e.to_string())?;
    let tree = parser.parse(src, None).ok_or("parser did not finish")?;
    if tree.root_node().has_error() {
        return Err("source could not be parsed; comments were not checked".into());
    }
    Ok(tree)
}

fn visit(node: Node, f: &mut impl FnMut(Node)) {
    // Iterative traversal also handles deeply nested input without a Rust stack overflow.
    let mut cursor = node.walk();
    loop {
        f(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

pub fn comments(src: &str, lang: &Lang) -> Result<Option<Vec<Raw>>, String> {
    let Some(language) = language(lang) else {
        return Ok(None);
    };
    let tree = parse(src, language)?;
    let mut comments = Vec::new();
    visit(tree.root_node(), &mut |node| {
        if node.kind() == "comment" {
            let start = node.start_byte();
            comments.push(Raw {
                start,
                end: node.end_byte(),
                line: src[start..].starts_with("//"),
            });
        }
    });
    Ok(Some(comments))
}

fn empty_jsx(node: Node) -> bool {
    if node.kind() != "jsx_expression" {
        return false;
    }
    let mut cursor = node.walk();
    let empty = node
        .named_children(&mut cursor)
        .all(|n| n.kind() == "comment");
    empty
}

pub fn jsx_wrappers(src: &str) -> HashMap<(usize, usize), (usize, usize)> {
    let Ok(tree) = parse(src, tree_sitter_typescript::LANGUAGE_TSX.into()) else {
        return HashMap::new();
    };
    let mut spans = HashMap::new();
    visit(tree.root_node(), &mut |node| {
        if node.kind() == "comment" {
            if let Some(parent) = node
                .parent()
                .filter(|p| empty_jsx(*p) && p.named_child_count() == 1)
            {
                spans.insert(
                    (node.start_byte(), node.end_byte()),
                    (parent.start_byte(), parent.end_byte()),
                );
            }
        }
    });
    spans
}

// Compare node kinds, field structure and literal/token contents, excluding comments and
// empty JSX comment containers. This catches ASI changes even when both versions parse.
fn signature(tree: &Tree, src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let text = &src[node.byte_range()];
        let skip = node.kind() == "comment"
            || empty_jsx(node)
            || (node.kind() == "jsx_text" && text.contains('\n') && text.trim().is_empty());
        if !skip {
            out.push((
                node.kind().into(),
                if node.child_count() == 0 {
                    text.into()
                } else {
                    String::new()
                },
            ));
            if cursor.goto_first_child() {
                continue;
            }
            out.push((")".into(), String::new()));
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
            out.push((")".into(), String::new()));
        }
    }
}

pub fn equivalent(before: &str, after: &str, lang: &Lang) -> Result<(), String> {
    let Some(language) = language(lang) else {
        return Ok(());
    };
    let a = parse(before, language.clone())?;
    let b = parse(after, language)?;
    if signature(&a, before) != signature(&b, after) {
        return Err("fix would change executable syntax or literal content".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn extracts_comments_without_regex_or_jsx_text() {
        let lang = crate::lex::lang_for(Path::new("a.tsx")).unwrap();
        let src = "if (ok) /[//]/.test(str);\nconst x = <p>Use /* */ here.{/* actual */}</p>;";
        let comments = comments(src, lang).unwrap().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(&src[comments[0].start..comments[0].end], "/* actual */");
    }
    #[test]
    fn rejects_semantic_changes_and_invalid_input() {
        let lang = &crate::lex::JS;
        assert!(equivalent(
            "function f(){return/*\n*/42;}",
            "function f(){return 42;}",
            lang
        )
        .is_err());
        assert!(equivalent("const x = 1 +/**/+2;", "const x = 1 ++2;", lang).is_err());
        assert!(equivalent("const x = '/* */';", "const x = '';", lang).is_err());
        assert!(comments("const = ; // x", lang).is_err());
        assert!(equivalent("// text\nconst x = 1;", "const x = 1;", lang).is_ok());
    }
    #[test]
    fn recognizes_only_jsx_comment_braces() {
        let src = "function f()\n{/* body */}\nconst x = <div>{/* child */}</div>;";
        let spans = jsx_wrappers(src);
        assert_eq!(spans.len(), 1);
        let &(a, b) = spans.values().next().unwrap();
        assert_eq!(&src[a..b], "{/* child */}");
    }
}
