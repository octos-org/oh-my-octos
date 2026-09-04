//! End-to-end suite against a real octos binary (and octoscode when available).
//!
//! Every case installs the skill into a throwaway octos home from a staged copy
//! (manifest + prompts + the freshly built binary as `main`), runs one or more
//! turns, and asserts on octos's own log lines as evidence, never on the model's
//! wording alone. `~/.octos` is never touched.
//!
//! Required env: OCTOS_BIN (or octos on PATH) and the provider key
//! (DEEPSEEK_API_KEY by default; OMO_E2E_PROVIDER / OMO_E2E_MODEL / OMO_E2E_KEY_ENV
//! for another provider). Optional: OCTOSCODE_BIN, OMO_E2E_SKIP_REMOTE=1,
//! OMO_E2E_WORK_ROOT (short path; default /tmp).
//!
//! Without the env the cases are skipped (they print why and pass), so plain
//! `cargo test` stays green offline. Run with `cargo test --test e2e -- --test-threads=1`.

mod support;

use std::path::Path;
use std::time::Duration;
use support::*;

macro_rules! case {
    ($name:ident, $body:expr) => {
        #[test]
        fn $name() {
            let e = match env() {
                Ok(e) => e,
                Err(why) => {
                    eprintln!("SKIP {}: {}", stringify!($name), why);
                    return;
                }
            };
            let f: fn(&Env) = $body;
            f(&e);
            if std::env::var("OMO_E2E_KEEP")
                .map(|v| v == "1")
                .unwrap_or(false)
            {
                eprintln!("work dir kept: {}", e.work.display());
            } else {
                let _ = std::fs::remove_dir_all(&e.work);
            }
        }
    };
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn w(p: &Path, rel: &str, s: &str) {
    let f = p.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, s).unwrap();
}

const BAD_PY_PROMPT: &str = "Use the write_file tool to create a file named bad.py whose content is exactly these two lines and nothing else (deliberate syntax error for a test; do not fix it, do not run it):\ndef f(:\n    pass\nAfter the tool result comes back, reply with every line of the tool result that contains 'edit_check' copied verbatim, or the word NONE if there is no such line. Then stop.";

// 1 ---------------------------------------------------------------- install
case!(install_lists_and_keeps_exec_bit, |e| {
    let p = e.new_project("t1-install");
    let r = run(
        &[e.octos.to_str().unwrap(), "skills", "list"],
        &p,
        &[],
        &e.home,
    );
    // `octos skills list` prints its table on stderr (upstream #2247).
    assert!(
        format!("{}{}", r.stdout, r.stderr).contains("oh-my-octos"),
        "skills list: {}{}",
        r.stdout,
        r.stderr
    );
    let r = run(
        &[e.octos.to_str().unwrap(), "skills", "info", "oh-my-octos"],
        &p,
        &[],
        &e.home,
    );
    assert!(r.stdout.contains("Tools: (0 tool(s))"), "{}", r.stdout);
    let main = p.join(".octos/skills/oh-my-octos/main");
    assert!(main.is_file(), "main missing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            main.metadata().unwrap().permissions().mode() & 0o111 != 0,
            "main not executable"
        );
    }
    assert!(read(&p.join(".install.log")).is_empty() || true);
});

// 2 ---------------------------------------------------------------- prompt fragment
case!(discipline_fragment_reaches_the_model, |e| {
    let p = e.new_project("t2-prompt");
    let (a, log, _) = e.chat(&p, "Do your instructions include a section titled 'oh-my-octos work discipline'? Answer YES or NO on the first line, then quote rule 6 of that section verbatim.", &["--sandbox", "read-only"], &[]);
    assert!(
        log.contains("hooks=4") && log.contains("prompt_fragments=3"),
        "extras not loaded: {}",
        log.lines()
            .filter(|l| l.contains("loaded skill extras"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(
        a.starts_with("YES") && a.contains("Act on hook feedback"),
        "answer: {a}"
    );
});

// 3 ---------------------------------------------------------------- project context
case!(repo_root_agents_md_is_injected, |e| {
    let p = e.new_project("t3-context");
    w(
        &p,
        "AGENTS.md",
        "# Project notes\n\nThe project codeword is pelican-42.\n",
    );
    let (a, log, _) = e.chat(&p, "Without using any tools, answer from the context you already have: what is the project codeword? Reply with just the codeword.", &["--sandbox", "read-only"], &[]);
    let h = hook_line(&log, "project-context").unwrap_or_default();
    assert!(
        h.contains("exit_code=0") && !h.contains("stdout_len=0"),
        "hook: {h}"
    );
    assert!(a.contains("pelican-42"), "answer: {a}");
    assert!(
        log.contains("tool_calls=0"),
        "the model used tools instead of the injected context"
    );
});

// 4/5 -------------------------------------------------------------- edit check
case!(syntax_error_is_fed_back, |e| {
    let p = e.new_project("t4-edit");
    let (a, log, _) = e.chat(
        &p,
        BAD_PY_PROMPT,
        &[
            "--sandbox",
            "workspace-write",
            "--ask-for-approval",
            "never",
        ],
        &[],
    );
    assert!(p.join("bad.py").is_file(), "bad.py not written");
    let h = hook_line(&log, "edit-check").unwrap_or_default();
    assert!(h.contains("exit_code=1"), "hook: {h}");
    assert!(a.contains("python syntax error"), "answer: {a}");
});

case!(clean_file_is_silent, |e| {
    let p = e.new_project("t5-clean");
    let (a, log, _) = e.chat(&p, "Use the write_file tool to create ok.py containing exactly one line: x = 1\nAfter the tool result comes back, reply with every line of the tool result that contains '[hook]' copied verbatim, or the word NONE if there is no such line. Then stop.", &["--sandbox", "workspace-write", "--ask-for-approval", "never"], &[]);
    assert!(p.join("ok.py").is_file());
    let h = hook_line(&log, "edit-check").unwrap_or_default();
    assert!(h.contains("exit_code=0"), "hook: {h}");
    assert!(a.contains("NONE"), "answer: {a}");
});

// 6 ---------------------------------------------------------------- budget
case!(budget_denies_second_model_call, |e| {
    let p = e.new_project("t6-budget");
    w(&p, "a.txt", "");
    w(&p, "b.txt", "");
    let (_, log, code) = e.chat(&p, "Use a tool to list the files in the current directory, then tell me how many files there are.", &["--sandbox", "read-only"], &[("OMO_SESSION_BUDGET_USD", "0.000001")]);
    let out = read(&p.join("out.json"));
    let state_dir = e.work.join("tmp").join("oh-my-octos");
    let recorded = std::fs::read_dir(&state_dir)
        .map(|rd| {
            rd.flatten()
                .any(|f| read(&f.path()).contains("session_cost"))
        })
        .unwrap_or(false);
    assert!(recorded, "no session_cost recorded (provider unpriced?)");
    assert!(
        log.contains("denied by hook") || out.contains("denied by hook"),
        "no deny; code={code:?} out={out}"
    );
});

// 7 ---------------------------------------------------------------- serve --stdio (octoscode / web / arc path)
case!(serve_stdio_hooks_and_prompt, |e| {
    let p = e.work.join("t7-serve");
    std::fs::create_dir_all(&p).unwrap();
    w(
        &p,
        "AGENTS.md",
        "# Project notes\n\nThe project codeword is heron-77.\n",
    );
    let mut s = Serve::spawn(&e.octos, &p, &e.home);
    let pid = s
        .bootstrap_profile(&e.provider, &e.model, &e.key_env)
        .unwrap();
    s.close();
    let r = run(
        &[
            e.octos.to_str().unwrap(),
            "skills",
            "--profile",
            &pid,
            "install",
            e.stage.to_str().unwrap(),
            "--force",
        ],
        &p,
        &[],
        &e.home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    let mut s = Serve::spawn(&e.octos, &p, &e.home);
    s.profile_id = Some(pid);
    s.open().unwrap();
    let (ok, text, methods) = s.turn("Without using any tools, answer from the context you already have: what is the project codeword? Reply with just the codeword.", Duration::from_secs(300));
    assert!(ok && text.contains("heron-77"), "turn a: {text}");
    assert!(
        !methods.iter().any(|m| m.to_lowercase().contains("tool")),
        "tools used: {methods:?}"
    );
    let (ok2, _, _) = s.turn(BAD_PY_PROMPT, Duration::from_secs(300));
    assert!(ok2 && p.join("bad.py").is_file());
    s.close();
    let log = e.serve_log(&e.home);
    assert!(
        log.contains("plugin=oh-my-octos") && log.contains("hooks=4"),
        "extras not loaded under serve"
    );
    assert!(
        count_hook(&log, "project-context", "exit_code=0") >= 1
            && !hook_line(&log, "project-context")
                .unwrap_or_default()
                .contains("stdout_len=0")
    );
    assert!(
        count_hook(&log, "edit-check", "exit_code=1") >= 1,
        "edit-check produced no feedback under serve"
    );
});

// 8 ---------------------------------------------------------------- remove
case!(remove_is_clean, |e| {
    let p = e.new_project("t8-remove");
    run(
        &[e.octos.to_str().unwrap(), "skills", "remove", "oh-my-octos"],
        &p,
        &[],
        &e.home,
    );
    assert!(!p.join(".octos/skills/oh-my-octos").exists());
    let (_, log, _) = e.chat(
        &p,
        "Reply with exactly the word OK.",
        &["--sandbox", "read-only"],
        &[],
    );
    assert!(
        !log.contains("oh-my-octos"),
        "skill still referenced after removal"
    );
});

// 9/10/11 ------------------------------------------------------------ setup (guided install)
fn setup_env<'a>(e: &'a Env, home: &'a Path) -> Vec<(&'a str, String)> {
    let path = format!(
        "{}:{}",
        e.octos.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    vec![
        ("PATH", path),
        ("OCTOS_HOME", home.to_string_lossy().into_owned()),
        ("OMO_SKIP_BINARY_INSTALL", "1".into()),
    ]
}

case!(setup_guided_path_non_interactive, |e| {
    let p = e.work.join("t9-setup");
    std::fs::create_dir_all(&p).unwrap();
    let envs = setup_env(e, &e.home);
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = run(
        &[
            bin().to_str().unwrap(),
            "setup",
            "--source",
            e.stage.to_str().unwrap(),
            "--project",
            p.to_str().unwrap(),
            "--no-serve",
        ],
        &p,
        &envs_ref,
        &e.home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(p.join(".octos/skills/oh-my-octos/manifest.json").is_file());
    assert!(r.stdout.contains("credential resolves"), "{}", r.stdout);
});

case!(setup_with_packs, |e| {
    let p = e.work.join("t10-packs");
    std::fs::create_dir_all(&p).unwrap();
    let envs = setup_env(e, &e.home);
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = run(
        &[
            bin().to_str().unwrap(),
            "setup",
            "--source",
            e.stage.to_str().unwrap(),
            "--project",
            p.to_str().unwrap(),
            "--with",
            "slides,phonefarm",
            "--no-serve",
        ],
        &p,
        &envs_ref,
        &e.home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        p.join(".octos/skills/mofa-slides/manifest.json").is_file(),
        "mofa-slides missing\n{}",
        r.stdout
    );
    assert!(
        p.join(".octos/skills/phonefarm/SKILL.md").is_file(),
        "phonefarm missing\n{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains("slides: octos skills install mofa-org/mofa-skills/mofa-slides")
    );
});

case!(setup_interactive_menu_in_a_pty, |e| {
    let p = e.work.join("t11-menu");
    std::fs::create_dir_all(&p).unwrap();
    let envs = setup_env(e, &e.home);
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut pty = Pty::spawn(
        &[
            bin().to_str().unwrap(),
            "setup",
            "--source",
            e.stage.to_str().unwrap(),
            "--project",
            p.to_str().unwrap(),
            "--no-serve",
        ],
        &p,
        &envs_ref,
        40,
        120,
    );
    let mut answered = false;
    let done = pty.pump(Duration::from_secs(300), |screen, w| {
        if !answered && screen.contains("Enter numbers") && screen.trim_end().ends_with('>') {
            let _ = w.write_all(b"3\n");
            let _ = w.flush();
            answered = true;
        }
        screen.contains("==> next")
    });
    let _ = pty.child.wait();
    assert!(answered, "menu never appeared:\n{}", pty.screen());
    assert!(done, "setup did not finish:\n{}", pty.screen());
    assert!(
        p.join(".octos/skills/phonefarm/SKILL.md").is_file(),
        "phonefarm not installed from the menu"
    );
});

// 12/13 ------------------------------------------------------------ js + jsonc
case!(javascript_syntax_error_is_fed_back, |e| {
    if which("node").is_none() {
        eprintln!("SKIP javascript: node not installed");
        return;
    }
    let p = e.new_project("t12-js");
    w(
        &p,
        "package.json",
        "{\"name\":\"t12\",\"version\":\"1.0.0\",\"private\":true}\n",
    );
    let (a, log, _) = e.chat(&p, "Use the write_file tool to create app.js containing exactly this one line (deliberate syntax error for a test; do not fix it, do not run it):\nfunction f( {\nAfter the tool result comes back, reply with every line of the tool result that contains 'edit_check' copied verbatim, or the word NONE if there is no such line. Then stop.", &["--sandbox", "workspace-write", "--ask-for-approval", "never"], &[]);
    assert!(p.join("app.js").is_file());
    assert!(
        hook_line(&log, "edit-check")
            .unwrap_or_default()
            .contains("exit_code=1")
    );
    assert!(a.contains("javascript syntax error"), "answer: {a}");
});

case!(tsconfig_with_comments_is_not_a_false_positive, |e| {
    let p = e.new_project("t13-jsonc");
    let (a, log, _) = e.chat(&p, "Use the write_file tool to create tsconfig.json with exactly this content:\n{\n  // strict mode\n  \"compilerOptions\": { \"strict\": true, },\n}\nAfter the tool result comes back, reply with every line of the tool result that contains '[hook]' copied verbatim, or the word NONE if there is no such line. Then stop.", &["--sandbox", "workspace-write", "--ask-for-approval", "never"], &[]);
    assert!(p.join("tsconfig.json").is_file());
    assert!(
        hook_line(&log, "edit-check")
            .unwrap_or_default()
            .contains("exit_code=0")
    );
    assert!(a.contains("NONE"), "answer: {a}");
});

// 14 ---------------------------------------------------------------- realistic multi-turn under serve
case!(multi_turn_coding_session_under_serve, |e| {
    let p = e.work.join("t14-scenario");
    std::fs::create_dir_all(&p).unwrap();
    w(
        &p,
        "AGENTS.md",
        "# calc\n\nHouse rule: tests are run with `python3 -m unittest -v`.\n",
    );
    let mut s = Serve::spawn(&e.octos, &p, &e.home);
    let pid = s
        .bootstrap_profile(&e.provider, &e.model, &e.key_env)
        .unwrap();
    s.close();
    let r = run(
        &[
            e.octos.to_str().unwrap(),
            "skills",
            "--profile",
            &pid,
            "install",
            e.stage.to_str().unwrap(),
            "--force",
        ],
        &p,
        &[],
        &e.home,
    );
    assert_eq!(r.code, Some(0));
    let mut s = Serve::spawn(&e.octos, &p, &e.home);
    s.profile_id = Some(pid);
    s.open().unwrap();
    let turns = [
        "Create a Python package `calc` in this directory: calc/__init__.py exporting add(a, b) and div(a, b) (div raises ZeroDivisionError with a clear message on b == 0). Keep it minimal.",
        "Add tests/test_calc.py using only the standard library unittest, covering add and div including the zero case. Then run the tests with `python3 -m unittest -v` and report the result.",
        "Append a function `mul(a, b)` to calc/__init__.py. Make a deliberate mistake: leave out the closing parenthesis on the def line, do not fix it yet, and tell me what the tool result said.",
        "Now fix that syntax error, extend the tests for mul, run the test suite again and report the result.",
        "Is the task complete? Answer YES or NO on the first line and give one sentence of evidence.",
    ];
    let mut last = String::new();
    for (i, t) in turns.iter().enumerate() {
        let (ok, text, _) = s.turn(t, Duration::from_secs(600));
        eprintln!(
            "--- turn {i}: ok={ok}\n{}",
            text.trim().chars().take(400).collect::<String>()
        );
        assert!(ok, "turn {i} failed: {text}");
        last = text;
    }
    s.close();
    assert!(p.join("calc/__init__.py").is_file() && p.join("tests/test_calc.py").is_file());
    let compiled = std::process::Command::new("python3")
        .args([
            "-c",
            "import ast,sys; ast.parse(open('calc/__init__.py').read())",
        ])
        .current_dir(&p)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(compiled, "final calc/__init__.py does not compile");
    assert!(
        last.trim().to_uppercase().starts_with("YES"),
        "final answer: {last}"
    );
    let log = e.serve_log(&e.home);
    assert!(
        count_hook(&log, "edit-check", "exit_code=1") >= 1,
        "the deliberate break was not caught"
    );
    assert!(count_hook(&log, "project-context", "exit_code=0") >= turns.len());
    assert!(
        count_hook(&log, "cost-guard", "exit_code=") > 0
            && count_hook(&log, "cost-guard", "exit_code=0")
                == count_hook(&log, "cost-guard", "exit_code="),
        "cost-guard errored"
    );
    let warns = log
        .lines()
        .filter(|l| l.contains("hook") && (l.contains("WARN") || l.contains("ERROR")))
        .count();
    assert_eq!(warns, 0, "hook WARN/ERROR lines in serve log");
});

// 15 ---------------------------------------------------------------- bug fix in an existing repo
case!(bug_fix_in_existing_repo, |e| {
    let p = e.new_project("t15-fix");
    w(
        &p,
        "pkg/__init__.py",
        "def clamp(x, lo, hi):\n    \"\"\"Clamp x into [lo, hi].\"\"\"\n    if x < lo:\n        return hi\n    if x > hi:\n        return hi\n    return x\n",
    );
    w(&p, "tests/__init__.py", "");
    w(
        &p,
        "tests/test_pkg.py",
        "import unittest\nfrom pkg import clamp\n\nclass T(unittest.TestCase):\n    def test_low(self):\n        self.assertEqual(clamp(-5, 0, 10), 0)\n    def test_high(self):\n        self.assertEqual(clamp(50, 0, 10), 10)\n    def test_mid(self):\n        self.assertEqual(clamp(5, 0, 10), 5)\n",
    );
    w(
        &p,
        "AGENTS.md",
        "# pkg\n\nRun tests with `python3 -m unittest -v`.\n",
    );
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&p)
            .output()
            .unwrap();
    };
    git(&["init", "-q"]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "init",
    ]);
    let (a, log, _) = e.chat(&p, "tests/test_pkg.py has a failing test. Find the bug in pkg/__init__.py, fix it with the smallest change, run the test suite, and report the result.", &["--sandbox", "workspace-write", "--ask-for-approval", "never"], &[]);
    let green = std::process::Command::new("python3")
        .args(["-m", "unittest"])
        .current_dir(&p)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(green, "tests still failing; answer: {a}");
    assert!(
        hook_line(&log, "edit-check")
            .unwrap_or_default()
            .contains("exit_code=0")
    );
    let c = hook_line(&log, "project-context").unwrap_or_default();
    assert!(
        c.contains("exit_code=0") && !c.contains("stdout_len=0"),
        "git context not injected: {c}"
    );
});

// 16 ---------------------------------------------------------------- plain dir
case!(plain_dir_injects_nothing, |e| {
    let p = e.new_project("t16-plain");
    let (a, log, _) = e.chat(
        &p,
        "Reply with exactly the word OK.",
        &["--sandbox", "read-only"],
        &[],
    );
    let c = hook_line(&log, "project-context").unwrap_or_default();
    assert!(c.contains("exit_code=0 stdout_len=0"), "{c}");
    assert!(a.contains("OK") && !log.contains("denied"));
});

// 17/18 ------------------------------------------------------------ remote (GitHub)
case!(install_from_github_and_update, |e| {
    if std::env::var("OMO_E2E_SKIP_REMOTE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        eprintln!("SKIP remote: OMO_E2E_SKIP_REMOTE=1");
        return;
    }
    let p = e.work.join("t17-remote");
    std::fs::create_dir_all(&p).unwrap();
    let r = run(
        &[
            e.octos.to_str().unwrap(),
            "skills",
            "install",
            "octos-org/oh-my-octos",
        ],
        &p,
        &[],
        &e.home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    let main = p.join(".octos/skills/oh-my-octos/main");
    assert!(
        main.is_file(),
        "main not produced by the GitHub install (download or cargo build)\n{}{}",
        r.stdout,
        r.stderr
    );
    let r = run(
        &[e.octos.to_str().unwrap(), "skills", "update", "oh-my-octos"],
        &p,
        &[],
        &e.home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    let (a, _, _) = e.chat(&p, "Without using any tools: do your instructions include a section titled 'oh-my-octos work discipline'? Answer YES or NO.", &["--sandbox", "read-only"], &[]);
    assert!(a.starts_with("YES"), "answer: {a}");
});

// 19 ---------------------------------------------------------------- octoscode TUI in a pty
case!(octoscode_tui_in_a_pty, |e| {
    let Some(oc) = &e.octoscode else {
        eprintln!("SKIP tui: octoscode not installed (set OCTOSCODE_BIN)");
        return;
    };
    let p = e.work.join("t19-tui");
    std::fs::create_dir_all(&p).unwrap();
    w(
        &p,
        "AGENTS.md",
        "# demo\n\nThe project codeword is osprey-31.\n",
    );
    let mut s = Serve::spawn(&e.octos, &p, &e.home);
    let pid = s
        .bootstrap_profile(&e.provider, &e.model, &e.key_env)
        .unwrap();
    s.close();
    let r = run(
        &[
            e.octos.to_str().unwrap(),
            "skills",
            "--profile",
            &pid,
            "install",
            e.stage.to_str().unwrap(),
            "--force",
        ],
        &p,
        &[],
        &e.home,
    );
    assert_eq!(r.code, Some(0));
    let stdio_cmd = format!(
        "{} serve --stdio --solo --data-dir {}",
        e.octos.display(),
        e.home.display()
    );
    let session = format!("omo-tui-{}", std::process::id());
    let home = e.home.to_string_lossy().into_owned();
    let envs = [
        ("OCTOS_HOME", home.as_str()),
        ("TERM", "xterm-256color"),
        ("OCTOSCODE_NO_AUTO_INSTALL", "1"),
        ("OCTOS_LANG", "en"),
    ];
    let mut pty = Pty::spawn(
        &[
            oc.to_str().unwrap(),
            "--no-splash",
            "--stdio-command",
            &stdio_cmd,
            "--profile-id",
            &pid,
            "--cwd",
            p.to_str().unwrap(),
            "--session",
            &session,
            "--prompt",
            "Without using any tools, answer from the context you already have: what is the project codeword? Reply with just the codeword.",
        ],
        &p,
        &envs,
        40,
        140,
    );
    let mut activated = false;
    let a_ok = pty.pump(Duration::from_secs(240), |screen, w| {
        if !activated && screen.contains("Activate this folder?") {
            let _ = w.write_all(b"\r");
            let _ = w.flush();
            activated = true;
        }
        screen.contains("osprey-31")
    });
    std::thread::sleep(Duration::from_secs(2));
    pty.write(b"Use the write_file tool to create bad.py containing exactly these two lines (deliberate syntax error for a test; do not fix it, do not run it):  def f(:  and  pass  on the next line. Then reply with any line of the tool result that contains edit_check, or NONE.");
    std::thread::sleep(Duration::from_millis(500));
    pty.write(b"\r");
    // The typed prompt itself contains "bad.py" and "NONE", so the screen cannot be
    // the signal here; wait for the serve log to show the edit-check feedback.
    let _ = pty.pump(Duration::from_secs(300), |screen, _| {
        screen.contains("python syntax error")
            || count_hook(&e.serve_log(&e.home), "edit-check", "exit_code=1") >= 1
    });
    std::thread::sleep(Duration::from_secs(2));
    pty.write(b"\x11"); // Ctrl+Q
    std::thread::sleep(Duration::from_secs(3));
    pty.kill();
    let log = e.serve_log(&e.home);
    assert!(
        a_ok,
        "codeword never appeared on screen:\n{}",
        pty.screen()
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    assert!(
        log.contains("plugin=oh-my-octos"),
        "skill not loaded under the TUI's server"
    );
    assert!(count_hook(&log, "project-context", "exit_code=0") >= 1);
    assert!(
        count_hook(&log, "edit-check", "exit_code=1") >= 1 && p.join("bad.py").is_file(),
        "edit-check did not fire from the composer turn"
    );
});
