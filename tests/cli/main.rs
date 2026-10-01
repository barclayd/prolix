use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

static SEQ: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "prolix-cli-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("prolix.jsonc"), "{}").unwrap();
        Self(path)
    }
    fn put(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn get(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).unwrap()
    }
    fn run(&self, args: &[&str], server: Option<&Server>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_prolix"));
        cmd.current_dir(&self.0)
            .args(args)
            .args(["--reporter", "json"])
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("TYPESAFE_BASE_URL")
            .env("TYPESAFE_DEFAULT_MODEL", "mock");
        if let Some(server) = server {
            cmd.env("TYPESAFE_API_KEY", "test-only")
                .env("TYPESAFE_BASE_URL", &server.url);
        }
        cmd.output().unwrap()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(reply: impl Fn(Value) -> Value + Send + 'static) -> Self {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let Ok((mut connection, _)) = socket.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                connection
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut b = [0; 1024];
                    let n = connection.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let len: usize = headers
                    .lines()
                    .find_map(|s| {
                        s.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < header_end + len {
                    let mut b = [0; 8192];
                    let n = connection.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                }
                let body =
                    reply(serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap())
                        .to_string();
                write!(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn answers(request: &Value, probability: f64) -> Value {
    let answers: serde_json::Map<String, Value> = request["questions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, q)| {
            let p: serde_json::Map<String, Value> = q["criteria"]
                .as_object()
                .unwrap()
                .keys()
                .map(|c| {
                    (
                        c.clone(),
                        json!(match c.as_str() {
                            "restates-code" => probability,
                            "explains-why" => 1.0 - probability,
                            _ => 0.0,
                        }),
                    )
                })
                .collect();
            (id.clone(), json!({"probabilities":p}))
        })
        .collect();
    json!({"model":"mock-1", "answers":answers, "usage":{"input_tokens":1}})
}
fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn syntax_regressions_preserve_code_and_data() {
    for (name, before, after) in [
        (
            "a.js",
            "function f(){return/*\n*/42;}\n",
            "function f(){return\n42;}\n",
        ),
        ("a.js", "const x = 1 +/**/+2;\n", "const x = 1 + +2;\n"),
        ("a.tsx", "function f()\n{/**/}\n", "function f()\n{}\n"),
        (
            "a.tsx",
            "const x = <p>Use /* */ here.</p>;\n",
            "const x = <p>Use /* */ here.</p>;\n",
        ),
        (
            "a.js",
            "if (ok) /[//]/.test(str);\n",
            "if (ok) /[//]/.test(str);\n",
        ),
        (
            "a.yml",
            "message: |\n  # ---\n  Hello\n",
            "message: |\n  # ---\n  Hello\n",
        ),
    ] {
        let w = Workspace::new();
        w.put(name, before);
        let out = w.run(&[name, "--fix", "--mode", "strict"], None);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(w.get(name), after);
        assert_eq!(report(&out)["complete"], true);
    }
}
#[test]
fn long_comments_and_bad_inputs_cannot_pass_or_be_fixed() {
    let w = Workspace::new();
    let text = format!(
        "{}// WARNING: persist before acknowledging a payment.\ncount++;\n",
        "// Increment the counter.\n".repeat(100)
    );
    w.put("a.ts", &text);
    let out = w.run(&["a.ts", "--fix"], None);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(w.get("a.ts"), text);
    assert_eq!(report(&out)["stats"]["asked"], 0);
    assert_eq!(report(&out)["complete"], false);
    let out = w.run(&["missing.ts"], None);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(report(&out)["complete"], false);
    w.put("invalid.ts", "const = ; // nothing\n");
    assert_eq!(w.run(&["invalid.ts"], None).status.code(), Some(2));
}
#[test]
fn missing_answers_are_incomplete_and_no_files_change() {
    let server = Server::new(|_| json!({"model":"mock-1","answers":{}}));
    let w = Workspace::new();
    w.put("a.ts", "// Return the value\nfunction f() { return 42; }\n");
    let before = w.get("a.ts");
    let out = w.run(&["a.ts", "--fix"], Some(&server));
    let r = report(&out);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(r["complete"], false);
    assert_eq!(r["stats"]["unanswered"], 1);
    assert_eq!(w.get("a.ts"), before);
}
#[test]
fn deduplicates_questions_and_preserves_model_provenance() {
    let questions = Arc::new(AtomicUsize::new(0));
    let q = questions.clone();
    let server = Server::new(move |r| {
        q.fetch_add(r["questions"].as_object().unwrap().len(), Ordering::Relaxed);
        answers(&r, 1.0)
    });
    let w = Workspace::new();
    for i in 0..40 {
        w.put(
            &format!("{i}.ts"),
            "// Return the value\nfunction f() { return 42; }\n",
        );
    }
    let r = report(&w.run(&[], Some(&server)));
    assert_eq!(questions.load(Ordering::Relaxed), 1);
    assert_eq!(r["stats"]["asked"], 1);
    assert_eq!(r["stats"]["deduplicated"], 39);
    assert_eq!(r["resolvedModels"], json!(["mock-1"]));
    let cached = report(&w.run(&[], Some(&server)));
    assert_eq!(cached["stats"]["asked"], 0);
    assert_eq!(cached["stats"]["cached"], 40);
    assert_eq!(questions.load(Ordering::Relaxed), 1);
    assert_eq!(w.run(&[], None).status.code(), Some(2)); // another endpoint cannot reuse the mock cache
}
#[test]
fn separates_review_and_fix_thresholds_and_recovers_jsdoc() {
    let server = Server::new(|r| answers(&r, 0.7));
    let w = Workspace::new();
    w.put(
        "a.ts",
        "/** Gets the user.\n * @returns User\n */\nfunction getUser(): User { return user; }\n",
    );
    let before = w.get("a.ts");
    let out = w.run(&["a.ts", "--fix"], Some(&server));
    let r = report(&out);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(r["stats"]["flagged"], 1);
    assert_eq!(r["comments"][0]["fix"], Value::Null);
    assert_eq!(w.get("a.ts"), before);
    w.put("prolix.jsonc", "{\"fixThreshold\":0.6}");
    let out = w.run(&["a.ts", "--fix"], Some(&server));
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(report(&out)["stats"]["fixed"], 1);
}
#[test]
fn refuses_to_overwrite_edits_made_during_inference() {
    let w = Workspace::new();
    w.put("a.ts", "// Return the value\nfunction f() { return 42; }\n");
    let path = w.0.join("a.ts");
    let server = Server::new(move |r| {
        std::fs::write(&path, "const edited = true;\n").unwrap();
        answers(&r, 1.0)
    });
    let out = w.run(&["a.ts", "--fix"], Some(&server));
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(w.get("a.ts"), "const edited = true;\n");
}

#[test]
fn unverified_languages_remain_available_for_review() {
    let w = Workspace::new();
    w.put("a.py", "# ----\nx = 1\n");
    let out = w.run(&["a.py", "--fix", "--mode", "strict"], None);
    let r = report(&out);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(r["comments"][0]["fixStatus"], "unverified-language");
    assert_eq!(r["comments"][0]["fix"], Value::Null);
    assert_eq!(w.get("a.py"), "# ----\nx = 1\n");
}
