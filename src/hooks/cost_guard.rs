//! Session budget guard.
//!
//! `after_llm_call` records the cumulative session spend Octos reports;
//! `before_llm_call` denies once it passes `OMO_SESSION_BUDGET_USD` (default 10,
//! zero or negative disables). State: one JSON file per bucket under the temp dir.
//! The bucket is the session id when the payload carries one; Octos rc.10 does not
//! (see upstream issue #2246), so it falls back to the parent process: one bucket
//! per `octos chat` / `octos serve` process. A cumulative value lower than the last
//! one means a new session started reporting in the same process: the bucket resets.
//! State idle for more than four hours is ignored (pid reuse, day-old sessions).

use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_BUDGET_USD: f64 = 10.0;
pub const STALE_SECONDS: f64 = 4.0 * 3600.0;
const TTL_SECONDS: f64 = 7.0 * 24.0 * 3600.0;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn state_dir() -> PathBuf {
    let d = std::env::temp_dir().join("oh-my-octos");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn bucket(payload: &Value) -> String {
    if let Some(sid) = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let safe: String = sid
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .take(120)
            .collect();
        return format!("s-{safe}");
    }
    format!("p-{}", crate::util::parent_pid())
}

pub fn budget() -> f64 {
    match std::env::var("OMO_SESSION_BUDGET_USD") {
        Ok(v) if !v.trim().is_empty() => v.trim().parse().unwrap_or(DEFAULT_BUDGET_USD),
        _ => DEFAULT_BUDGET_USD,
    }
}

fn load(path: &PathBuf) -> Value {
    let Ok(text) = std::fs::read_to_string(path) else {
        return json!({});
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return json!({});
    };
    match v.get("updated").and_then(Value::as_f64) {
        Some(t) if now() - t <= STALE_SECONDS => v,
        _ => json!({}),
    }
}

fn save(path: &PathBuf, v: &Value) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn sweep() {
    let Ok(rd) = std::fs::read_dir(state_dir()) else {
        return;
    };
    for e in rd.flatten() {
        if let Ok(m) = e.metadata() {
            if let Ok(t) = m.modified() {
                if let Ok(age) = SystemTime::now().duration_since(t) {
                    if age.as_secs_f64() > TTL_SECONDS {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
    }
}

fn fmt_budget(cap: f64) -> String {
    let s = format!("{cap:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn run(payload: &Value) -> i32 {
    let cap = budget();
    if cap <= 0.0 {
        return 0;
    }
    let path = state_dir().join(format!("{}.json", bucket(payload)));
    match payload.get("event").and_then(Value::as_str) {
        Some("after_llm_call") => {
            if let Some(cost) = payload.get("session_cost").and_then(Value::as_f64) {
                let mut data = load(&path);
                let prev = data.get("session_cost").and_then(Value::as_f64);
                if matches!(prev, Some(p) if cost < p - 1e-9) {
                    data = json!({}); // a new session in the same process
                }
                data["session_cost"] = json!(cost);
                data["updated"] = json!(now());
                data["model"] = payload.get("model").cloned().unwrap_or(Value::Null);
                save(&path, &data);
                sweep();
            }
            0
        }
        Some("before_llm_call") => {
            let data = load(&path);
            match data.get("session_cost").and_then(Value::as_f64) {
                Some(spent) if spent >= cap => {
                    println!(
                        "session spend ${spent:.4} reached the oh-my-octos budget of ${} (OMO_SESSION_BUDGET_USD). \
                         Summarize the current state for the user and stop.",
                        fmt_budget(cap)
                    );
                    1
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}
