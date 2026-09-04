//! The three lifecycle hooks. Each reads one JSON payload from stdin and speaks
//! to Octos through its exit code (see docs/octos-notes.md for the contract):
//!
//! - `user_prompt_submit`: exit 0, stdout = context injected for this turn
//! - `after_tool_call`:    exit 1 + stdout = feedback appended to the tool result; exit 0 silent = clean
//! - `before_llm_call`:    exit 1 = deny (stdout is the reason)
//!
//! Any internal failure exits 0 so a bug here can never block the user.

pub mod cost_guard;
pub mod edit_check;
pub mod project_context;

use serde_json::Value;

/// Read the hook payload from stdin. Garbage or empty input yields `None`.
pub fn read_payload() -> Option<Value> {
    let mut s = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).ok()?;
    serde_json::from_str(&s).ok()
}

/// Dispatch `oh-my-octos hook <name>`; returns the process exit code.
pub fn run(name: &str) -> i32 {
    let Some(payload) = read_payload() else {
        return 0;
    };
    let result = std::panic::catch_unwind(|| match name {
        "cost-guard" => cost_guard::run(&payload),
        "edit-check" => edit_check::run(&payload),
        "project-context" => project_context::run(&payload),
        _ => {
            eprintln!("oh-my-octos: unknown hook '{name}'");
            0
        }
    });
    result.unwrap_or(0)
}
