use std::collections::HashMap;
use std::process::Command;

/// Added line ranges (1-based, inclusive) per file, with paths relative to the current directory.
pub type Added = HashMap<String, Vec<(usize, usize)>>;

/// Lines added since the merge base of `base` and HEAD, counting uncommitted edits and untracked files.
pub fn added(base: &str, paths: &[String]) -> Result<Added, String> {
    // Shallow CI checkouts often have no merge base; comparing with `base` itself is then the best available.
    let from = git(&["merge-base", base, "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| base.to_string());
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "-U0",
        "--relative",
        "--no-prefix",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        &from,
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let mut added = parse(&git(&args)?);
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    args.extend(paths.iter().map(String::as_str));
    for f in git(&args)?.split('\0').filter(|f| !f.is_empty()) {
        added.insert(f.to_string(), vec![(1, usize::MAX)]);
    }
    Ok(added)
}

fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Reads added ranges from `git diff -U0 --no-prefix` output.
fn parse(diff: &str) -> Added {
    let mut added = Added::new();
    let (mut file, mut header) = (None, false);
    for l in diff.lines() {
        if l.starts_with("diff --git ") {
            header = true;
        } else if let (true, Some(p)) = (header, l.strip_prefix("+++ ")) {
            // Git adds a tab after names containing spaces.
            let p = p.strip_suffix('\t').unwrap_or(p);
            // ponytail: git quotes names holding quotes, backslashes or control characters; those files are skipped.
            file = (p != "/dev/null" && !p.starts_with('"')).then(|| p.to_string());
        } else if let Some(h) = l.strip_prefix("@@ ") {
            header = false;
            let Some(f) = &file else { continue };
            let new = h.split(' ').find_map(|t| t.strip_prefix('+')).unwrap_or("");
            let (start, count) = new.split_once(',').unwrap_or((new, "1"));
            if let (Ok(s), Ok(n)) = (start.parse::<usize>(), count.parse::<usize>()) {
                if n > 0 {
                    added.entry(f.clone()).or_default().push((s, s + n - 1));
                }
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_added_ranges() {
        let diff = "\
diff --git src/a.ts src/a.ts
--- src/a.ts
+++ src/a.ts
@@ -3,0 +4,2 @@ fn
+// one
++++ not a header
@@ -9 +11 @@
-old
+new
@@ -20,2 +21,0 @@
-gone
-gone
diff --git my file.py my file.py
--- my file.py\t
+++ my file.py\t
@@ -0,0 +1 @@
+# hi
diff --git old.rs old.rs
--- old.rs
+++ /dev/null
@@ -1 +0,0 @@
-x
";
        let a = super::parse(diff);
        assert_eq!(a["src/a.ts"], [(4, 5), (11, 11)]);
        assert_eq!(a["my file.py"], [(1, 1)]);
        assert_eq!(a.len(), 2);
    }
}

// Updated to use the new parser
fn _demo() -> usize {
    // set x to one
    let x = 1;
    // println!("{x}");
    x // return x
}
