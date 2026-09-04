//! Hook contract tests: run the built binary with synthetic Octos payloads.
//! No octos binary or network needed.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_oh-my-octos");

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "omo-test-{tag}-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        % 1_000_000
}

struct Out {
    code: i32,
    stdout: String,
}

fn run_hook(name: &str, payload: &Value, cwd: Option<&Path>, env: &[(&str, &str)]) -> Out {
    use std::io::Write;
    let mut cmd = Command::new(BIN);
    cmd.args(["hook", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

fn write(dir: &Path, rel: &str, content: &str) -> String {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, content).unwrap();
    rel.to_string()
}

fn after_tool(rel: &str, tool: &str, success: bool) -> Value {
    // write_file is a sensitive tool: octos keeps only the path key.
    json!({"event":"after_tool_call","tool_name":tool,"tool_id":"t1",
           "arguments":{"path":rel,"redacted":true},"result":"[redacted]","success":success,"duration_ms":3})
}

// ---------------------------------------------------------------- edit-check
#[test]
fn clean_python_is_silent() {
    let d = tmpdir("ec");
    let rel = write(&d, "ok.py", "def f():\n    return 1\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn python_syntax_error_is_feedback() {
    let d = tmpdir("ec");
    let rel = write(&d, "bad.py", "def f(:\n    pass\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("python syntax error"), "{}", o.stdout);
    assert!(o.stdout.contains("bad.py"));
}

#[test]
fn invalid_json_is_feedback() {
    let d = tmpdir("ec");
    let rel = write(&d, "cfg.json", "{\"a\": [1, 2}");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "edit_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("invalid JSON"));
}

#[test]
fn json_with_comments_and_trailing_commas_is_tolerated() {
    let d = tmpdir("ec");
    let rel = write(
        &d,
        "tsconfig.json",
        "{\n  // comment\n  \"compilerOptions\": {\"strict\": true,},\n  /* block */\n}",
    );
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
    let rel = write(&d, ".vscode/settings.json", "{\"a\": 1, // trailing\n}");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn jsonc_that_is_really_broken_still_reports() {
    let d = tmpdir("ec");
    let rel = write(&d, "tsconfig.json", "{ // c\n \"a\": [1, }");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("tolerated"));
}

#[test]
fn invalid_toml_is_feedback() {
    let d = tmpdir("ec");
    let rel = write(&d, "pyproject.toml", "[project\nname = 'x'\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("invalid TOML"));
}

#[test]
fn conflict_markers_are_feedback() {
    let d = tmpdir("ec");
    let rel = write(
        &d,
        "notes.md",
        "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> branch\n",
    );
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "diff_edit", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("conflict markers"));
}

#[test]
fn empty_file_is_feedback() {
    let d = tmpdir("ec");
    let rel = write(&d, "empty.txt", "");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("empty"));
}

#[test]
fn shell_syntax_error_is_feedback() {
    let d = tmpdir("ec");
    let rel = write(&d, "run.sh", "if [ 1 ]; then\necho x\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("shell syntax error"));
}

#[test]
fn zsh_shebang_uses_zsh_or_skips() {
    let d = tmpdir("ec");
    let rel = write(
        &d,
        "z.sh",
        "#!/usr/bin/env zsh\nif [[ 1 ]]; then\n  echo x\nfi\n",
    );
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn fish_shebang_is_skipped() {
    let d = tmpdir("ec");
    let rel = write(
        &d,
        "f.sh",
        "#!/usr/bin/env fish\nif test 1\n  echo x\nend\n",
    );
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn binary_file_is_skipped() {
    let d = tmpdir("ec");
    std::fs::write(d.join("img.json"), b"\x89PNG\r\n\x1a\n\x00\x00garbage{").unwrap();
    let o = run_hook(
        "edit-check",
        &after_tool("img.json", "write_file", true),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn non_ascii_path_under_c_locale() {
    let d = tmpdir("ec");
    let rel = write(&d, "目录/坏.py", "def f(:\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", true),
        Some(&d),
        &[("LC_ALL", "C"), ("LANG", "C")],
    );
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("python syntax error"));
    assert!(o.stdout.contains("坏.py"));
}

#[test]
fn failed_edit_is_ignored() {
    let d = tmpdir("ec");
    let rel = write(&d, "bad.py", "def f(:\n");
    let o = run_hook(
        "edit-check",
        &after_tool(&rel, "write_file", false),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn missing_path_is_ignored() {
    let d = tmpdir("ec");
    let o = run_hook(
        "edit-check",
        &json!({"event":"after_tool_call","tool_name":"write_file","arguments":{"redacted":true}}),
        Some(&d),
        &[],
    );
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn garbage_stdin_never_blocks() {
    use std::io::Write;
    for name in ["edit-check", "cost-guard", "project-context"] {
        let mut child = Command::new(BIN)
            .args(["hook", name])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"not json").unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(0), "{name}");
    }
}

// ---------------------------------------------------------------- cost-guard
fn cost_env(tmp: &Path, budget: &str) -> Vec<(String, String)> {
    vec![
        ("TMPDIR".into(), tmp.to_string_lossy().into_owned()),
        ("OMO_SESSION_BUDGET_USD".into(), budget.into()),
    ]
}

fn run_cost(payload: &Value, env: &[(String, String)]) -> Out {
    let e: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    run_hook("cost-guard", payload, None, &e)
}

#[test]
fn deny_after_budget() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0.05");
    let sid = format!("unit-{}", std::process::id());
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":sid}), &env).code,
        0,
        "nothing recorded yet: allow"
    );
    run_cost(
        &json!({"event":"after_llm_call","session_id":sid,"session_cost":0.01}),
        &env,
    );
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":sid}), &env).code,
        0,
        "under budget"
    );
    run_cost(
        &json!({"event":"after_llm_call","session_id":sid,"session_cost":0.07}),
        &env,
    );
    let o = run_cost(&json!({"event":"before_llm_call","session_id":sid}), &env);
    assert_eq!(o.code, 1, "over budget: deny");
    assert!(o.stdout.contains("budget"));
}

#[test]
fn other_session_unaffected() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0.05");
    run_cost(
        &json!({"event":"after_llm_call","session_id":"a","session_cost":9}),
        &env,
    );
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":"b"}), &env).code,
        0
    );
}

#[test]
fn disabled_with_zero_budget() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0");
    run_cost(
        &json!({"event":"after_llm_call","session_id":"a","session_cost":99}),
        &env,
    );
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":"a"}), &env).code,
        0
    );
}

#[test]
fn unpriced_provider_never_denies() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0.05");
    run_cost(
        &json!({"event":"after_llm_call","session_id":"a","session_cost":null}),
        &env,
    );
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":"a"}), &env).code,
        0
    );
}

#[test]
fn new_session_in_same_process_resets_bucket() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0.05");
    // No session_id: the bucket is the parent pid (the test process). A lower cumulative = a new session.
    run_cost(&json!({"event":"after_llm_call","session_cost":0.06}), &env);
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call"}), &env).code,
        1,
        "first session over budget"
    );
    run_cost(
        &json!({"event":"after_llm_call","session_cost":0.001}),
        &env,
    );
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call"}), &env).code,
        0,
        "new session must be allowed"
    );
}

#[test]
fn stale_state_is_ignored() {
    let tmp = tmpdir("cg");
    let env = cost_env(&tmp, "0.05");
    run_cost(
        &json!({"event":"after_llm_call","session_id":"old","session_cost":9}),
        &env,
    );
    let dir = tmp.join("oh-my-octos");
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .unwrap();
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    v["updated"] = json!(v["updated"].as_f64().unwrap() - 5.0 * 3600.0);
    std::fs::write(&file, v.to_string()).unwrap();
    assert_eq!(
        run_cost(&json!({"event":"before_llm_call","session_id":"old"}), &env).code,
        0,
        "stale spend must not deny"
    );
}

// ---------------------------------------------------------------- project-context
fn ctx_payload(dir: &Path) -> Value {
    json!({"event":"user_prompt_submit","prompt":"hi","cwd":dir.to_string_lossy(),"model":"x"})
}

#[test]
fn repo_root_agents_md_is_injected() {
    let d = tmpdir("pc");
    write(&d, "AGENTS.md", "Codeword: pelican-42\n");
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("pelican-42") && o.stdout.contains("AGENTS.md"));
}

#[test]
fn claude_md_fallback() {
    let d = tmpdir("pc");
    write(&d, "CLAUDE.md", "Codeword: heron-7\n");
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert!(o.stdout.contains("heron-7"));
}

#[test]
fn skipped_when_octos_agents_md_exists() {
    let d = tmpdir("pc");
    write(&d, ".octos/AGENTS.md", "octos-managed\n");
    write(&d, "AGENTS.md", "Codeword: pelican-42\n");
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert!(!o.stdout.contains("pelican-42"));
}

#[test]
fn truncation() {
    let d = tmpdir("pc");
    write(&d, "AGENTS.md", &"x".repeat(20000));
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert!(o.stdout.contains("truncated"));
    assert!(o.stdout.len() < 7000);
}

#[test]
fn git_line() {
    let d = tmpdir("pc");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "first commit",
    ]);
    write(&d, "dirty.txt", "d");
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert!(o.stdout.contains("git: branch"), "{}", o.stdout);
    assert!(o.stdout.contains("1 uncommitted"));
    assert!(o.stdout.contains("first commit"));
}

#[test]
fn nothing_to_say_is_silent() {
    let d = tmpdir("pc");
    let o = run_hook("project-context", &ctx_payload(&d), Some(&d), &[]);
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
}

#[test]
fn slow_git_stays_inside_the_hook_timeout() {
    // A fake `git` that hangs: the hook must give up within its budget and exit 0.
    let d = tmpdir("pc");
    let fake = d.join("bin");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join("git"), "#!/bin/sh\nsleep 20\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fake.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(d.join(".git")).unwrap();
    let path = format!(
        "{}:{}",
        fake.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let t0 = std::time::Instant::now();
    let o = run_hook(
        "project-context",
        &ctx_payload(&d),
        Some(&d),
        &[("PATH", &path)],
    );
    assert_eq!(o.code, 0);
    assert!(t0.elapsed().as_secs_f64() < 4.5, "took {:?}", t0.elapsed());
}

#[test]
fn non_ascii_agents_md_under_c_locale() {
    let d = tmpdir("pc");
    write(&d, "AGENTS.md", "项目暗号：白鹭-9\n");
    let o = run_hook(
        "project-context",
        &ctx_payload(&d),
        Some(&d),
        &[("LC_ALL", "C"), ("LANG", "C")],
    );
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("白鹭-9"));
}
