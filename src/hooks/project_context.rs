//! Per-turn project context (`user_prompt_submit`): the repo-root AGENTS.md or
//! CLAUDE.md when the workspace has no `.octos/AGENTS.md` (Octos loads that one
//! itself), plus one line of git state. Printed to stdout with exit 0; Octos
//! prepends it to this turn only. Capped so a turn never pays more than a few
//! hundred tokens for it; git gets a 2.5 s total budget inside the hook's 5 s.

use crate::util::{run_with_timeout, which};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MAX_INSTRUCTION_CHARS: usize = 6000;
const GIT_BUDGET: Duration = Duration::from_millis(2500);

pub fn workspace(payload: &Value) -> PathBuf {
    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
        let p = PathBuf::from(cwd);
        if p.is_dir() {
            return p;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn instructions(root: &Path) -> Option<(String, String)> {
    if root.join(".octos").join("AGENTS.md").is_file() {
        return None;
    }
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let p = root.join(name);
        if !p.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let body = if text.chars().count() > MAX_INSTRUCTION_CHARS {
            let cut: String = text.chars().take(MAX_INSTRUCTION_CHARS).collect();
            format!("{cut}\n... (truncated; read {name} for the rest)")
        } else {
            text.to_string()
        };
        return Some((name.to_string(), body));
    }
    None
}

struct Git {
    root: PathBuf,
    deadline: Instant,
    dead: bool,
}

impl Git {
    fn call(&mut self, args: &[&str]) -> Option<String> {
        if self.dead {
            return None;
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(50) {
            return None;
        }
        let root = self.root.to_string_lossy().into_owned();
        let mut argv = vec!["git", "-C", &root];
        argv.extend_from_slice(args);
        match run_with_timeout(&argv, None, None, remaining) {
            Ok(r) if !r.timed_out && r.status == Some(0) => Some(r.stdout.trim().to_string()),
            Ok(r) if r.timed_out => {
                self.dead = true; // one slow call: stop asking git anything else this turn
                None
            }
            _ => None,
        }
    }
}

pub fn git_line(root: &Path) -> Option<String> {
    which("git")?;
    let mut g = Git {
        root: root.to_path_buf(),
        deadline: Instant::now() + GIT_BUDGET,
        dead: false,
    };
    if !root.join(".git").exists()
        && g.call(&["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true")
    {
        return None;
    }
    let branch = g
        .call(&["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|| "?".into());
    let status = g.call(&["status", "--porcelain", "--untracked-files=normal"]);
    let last = g
        .call(&["log", "-1", "--format=%h %s"])
        .unwrap_or_else(|| "no commits".into());
    Some(match status {
        Some(s) => {
            let dirty = s.lines().filter(|l| !l.trim().is_empty()).count();
            format!("git: branch {branch}, {dirty} uncommitted file(s), last commit: {last}")
        }
        None => format!(
            "git: branch {branch}, last commit: {last} (status skipped: repo too slow for this turn)"
        ),
    })
}

pub fn render(root: &Path) -> String {
    let mut parts = vec![];
    if let Some((name, text)) = instructions(root) {
        parts.push(format!(
            "Project instructions from {name} (repo root):\n{text}"
        ));
    }
    if let Some(g) = git_line(root) {
        parts.push(g);
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n\n") + "\n"
    }
}

pub fn run(payload: &Value) -> i32 {
    let root = workspace(payload);
    let out = render(&root);
    if !out.is_empty() {
        print!("{out}");
    }
    0
}
