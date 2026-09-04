//! Deterministic post-edit check (`after_tool_call` on write_file / edit_file / diff_edit).
//!
//! Octos redacts file contents from hook payloads, so the file is re-read from
//! disk (the hook runs with cwd = workspace root). Reports syntax errors for
//! .py .json .toml .sh/.bash .js/.mjs/.cjs, unresolved merge-conflict markers and
//! a file left empty. Findings: stdout + exit 1 (Octos appends them to the tool
//! result as `[hook] ...`). Clean: exit 0, silent.

use crate::util::{run_with_timeout, which};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn target_path(payload: &Value) -> Option<String> {
    let args = payload.get("arguments")?.as_object()?;
    for key in ["path", "file_path", "filename", "file"] {
        if let Some(v) = args.get(key).and_then(Value::as_str) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Files that are JSON-with-comments by convention.
pub fn is_jsonc_name(path: &str) -> bool {
    let p = path.replace('\\', "/");
    let name = p.rsplit('/').next().unwrap_or("");
    name.ends_with(".jsonc")
        || name.starts_with("tsconfig")
        || name == "jsconfig.json"
        || name == "devcontainer.json"
        || p.contains("/.vscode/")
        || p.starts_with(".vscode/")
}

/// Remove `//` and `/* */` comments outside strings, then trailing commas.
pub fn strip_jsonc(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let (mut i, n) = (0usize, b.len());
    let mut in_str = false;
    while i < n {
        let c = b[i] as char;
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < n {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
        } else if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if src[i..].starts_with("//") {
            i = src[i..].find('\n').map(|j| i + j).unwrap_or(n);
        } else if src[i..].starts_with("/*") {
            i = src[i + 2..].find("*/").map(|j| i + 2 + j + 2).unwrap_or(n);
        } else {
            out.push(c);
            i += 1;
        }
    }
    // trailing commas: `,` followed only by whitespace and `}` or `]`
    let mut res = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut k = 0;
    while k < chars.len() {
        if chars[k] == ',' {
            let mut j = k + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                k += 1;
                continue;
            }
        }
        res.push(chars[k]);
        k += 1;
    }
    res
}

fn check_json(src: &str, path: &str) -> Vec<String> {
    let strict = match serde_json::from_str::<Value>(src) {
        Ok(_) => return vec![],
        Err(e) => e,
    };
    if serde_json::from_str::<Value>(&strip_jsonc(src)).is_ok() {
        return vec![]; // JSON with comments / trailing commas is fine for most tools
    }
    if is_jsonc_name(path) {
        vec![format!(
            "invalid JSON (comments and trailing commas were tolerated): {strict}"
        )]
    } else {
        vec![format!("invalid JSON: {strict}")]
    }
}

fn check_toml(src: &str) -> Vec<String> {
    match toml::from_str::<toml::Value>(src) {
        Ok(_) => vec![],
        Err(e) => vec![format!("invalid TOML: {}", e.message())],
    }
}

fn last_line(s: &str) -> String {
    s.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn check_python(path: &Path) -> Vec<String> {
    let Some(py) = which("python3").or_else(|| which("python")) else {
        return vec![];
    };
    let code = "import ast,sys\nsrc=open(sys.argv[1],'rb').read()\ntry:\n    ast.parse(src, sys.argv[1])\nexcept SyntaxError as e:\n    print('python syntax error line %s: %s' % (e.lineno, e.msg)); sys.exit(1)\n";
    let p = path.to_string_lossy();
    match run_with_timeout(
        &[py.to_str().unwrap_or("python3"), "-c", code, &p],
        None,
        None,
        Duration::from_secs(10),
    ) {
        Ok(r) if r.status == Some(1) => vec![last_line(&r.stdout)],
        _ => vec![],
    }
}

fn check_shell(src: &str, path: &Path) -> Vec<String> {
    let first = src.lines().next().unwrap_or("");
    let mut shell = "bash";
    if first.starts_with("#!") {
        let found = ["bash", "zsh", "dash", "ksh", "fish", "sh"]
            .into_iter()
            .find(|s| {
                first
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|w| w == *s)
            });
        match found {
            Some("fish") | None => return vec![],
            Some(s) => shell = s,
        }
    }
    if which(shell).is_none() {
        return vec![];
    }
    let p = path.to_string_lossy();
    match run_with_timeout(&[shell, "-n", &p], None, None, Duration::from_secs(10)) {
        Ok(r) if matches!(r.status, Some(c) if c != 0) => {
            let out = format!("{}\n{}", r.stdout, r.stderr);
            let l = last_line(&out);
            vec![format!(
                "shell syntax error: {}",
                if l.is_empty() {
                    format!("{shell} -n failed")
                } else {
                    l
                }
            )]
        }
        _ => vec![],
    }
}

fn check_js(path: &Path) -> Vec<String> {
    let Some(node) = which("node") else {
        return vec![];
    };
    let p = path.to_string_lossy();
    match run_with_timeout(
        &[node.to_str().unwrap_or("node"), "--check", &p],
        None,
        None,
        Duration::from_secs(10),
    ) {
        Ok(r) if matches!(r.status, Some(c) if c != 0) => {
            let out = format!("{}\n{}", r.stdout, r.stderr);
            let l = last_line(&out);
            vec![format!(
                "javascript syntax error: {}",
                if l.is_empty() {
                    "node --check failed".into()
                } else {
                    l
                }
            )]
        }
        _ => vec![],
    }
}

fn has_conflict_markers(src: &str) -> bool {
    src.lines()
        .any(|l| l.starts_with("<<<<<<< ") || l == "=======" || l.starts_with(">>>>>>> "))
}

pub fn check_file(path: &Path, display: &str) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(path) else {
        return vec![];
    };
    if meta.len() == 0 {
        return vec!["file is empty after the edit".into()];
    }
    if meta.len() > MAX_BYTES {
        return vec![];
    }
    let Ok(raw) = std::fs::read(path) else {
        return vec![];
    };
    if raw.iter().take(8192).any(|&b| b == 0) {
        return vec![]; // binary
    }
    let src = String::from_utf8_lossy(&raw);
    let mut findings = vec![];
    if has_conflict_markers(&src) {
        findings.push("unresolved merge conflict markers (<<<<<<< / ======= / >>>>>>>)".into());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    findings.extend(match ext.as_str() {
        "py" => check_python(path),
        "json" => check_json(&src, display),
        "toml" => check_toml(&src),
        "sh" | "bash" => check_shell(&src, path),
        "js" | "mjs" | "cjs" => check_js(path),
        _ => vec![],
    });
    findings
}

pub fn run(payload: &Value) -> i32 {
    if payload.get("success") == Some(&Value::Bool(false)) {
        return 0; // the edit itself failed; the model already sees that error
    }
    let Some(rel) = target_path(payload) else {
        return 0;
    };
    let path = if Path::new(&rel).is_absolute() {
        std::path::PathBuf::from(&rel)
    } else {
        std::env::current_dir()
            .map(|d| d.join(&rel))
            .unwrap_or_else(|_| rel.clone().into())
    };
    let findings = check_file(&path, &rel);
    if findings.is_empty() {
        return 0;
    }
    for f in findings {
        println!("edit_check {rel}: {f}");
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_strip_handles_comments_strings_and_trailing_commas() {
        let src = "{\n // c\n \"a\": \"x//y\", /* b */ \"b\": [1,2,],\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["a"], "x//y");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn jsonc_names() {
        assert!(is_jsonc_name("tsconfig.build.json"));
        assert!(is_jsonc_name(".vscode/settings.json"));
        assert!(is_jsonc_name("a/b.jsonc"));
        assert!(!is_jsonc_name("package.json"));
    }
}
