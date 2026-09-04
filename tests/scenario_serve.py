#!/usr/bin/env python3
"""Realistic multi-turn scenario under `octos serve --stdio --solo`.

A small Python project is built over several turns the way a person would drive
octoscode: create module, add tests, run them, make a deliberate mistake and fix it.
After the run we check octos's own log for what the hooks did, and the budget
state file for monotonic session spend.

usage: scenario_serve.py OCTOS_BIN PROJECT_DIR DATA_DIR SKILL_ROOT PROVIDER MODEL KEY_ENV
Prints a report; exit 0 when every check holds.
"""
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from octos_stdio import OctosStdioSession  # noqa: E402

TURNS = [
    ("create",
     "Create a Python package `calc` in this directory: calc/__init__.py exporting add(a, b) and "
     "div(a, b) (div raises ZeroDivisionError with a clear message on b == 0). Keep it minimal."),
    ("tests",
     "Add tests/test_calc.py using only the standard library unittest, covering add and div "
     "including the zero case. Then run the tests with `python3 -m unittest -v` and report the result."),
    ("break",
     "Append a function `mul(a, b)` to calc/__init__.py. Make a deliberate mistake: leave out the "
     "closing parenthesis on the def line, do not fix it yet, and tell me what the tool result said."),
    ("fix",
     "Now fix that syntax error, extend the tests for mul, run the test suite again and report the result."),
    ("done",
     "Is the task complete? Answer YES or NO on the first line and give one sentence of evidence."),
]


def main():
    octos_bin, project, data_dir, skill_root, provider, model, key_env = sys.argv[1:8]
    project = Path(project).resolve(); data_dir = Path(data_dir).resolve()
    project.mkdir(parents=True, exist_ok=True)
    (project / "AGENTS.md").write_text("# calc\n\nHouse rule: tests are run with `python3 -m unittest -v`.\n")
    env = dict(os.environ); env["OCTOS_HOME"] = str(data_dir)
    env["OMO_SESSION_BUDGET_USD"] = env.get("OMO_SESSION_BUDGET_USD", "2")
    events = []

    def on_event(method, params):
        events.append((method, params))

    s = OctosStdioSession(octos_bin, project, env, data_dir, on_event)
    s.bootstrap_profile(provider, model, None, key_env)
    pid = s.profile_id; s.close()
    r = subprocess.run([octos_bin, "skills", "--profile", pid, "install", skill_root, "--force"],
                       env=env, capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        print(r.stdout, r.stderr); return 1

    s = OctosStdioSession(octos_bin, project, env, data_dir, on_event)
    s.profile_id = pid; s.open()
    transcript = []
    for name, prompt in TURNS:
        t0 = time.time()
        ok, text = s.run_turn(prompt, timeout=600)
        transcript.append((name, ok, text, time.time() - t0))
        print("--- turn %-6s ok=%s %.0fs\n%s\n" % (name, ok, time.time() - t0, text.strip()[:700]))
        if not ok:
            break
    s.close()

    # ---------------------------------------------------------------- evidence
    log = ""
    for f in sorted((data_dir / "logs").glob("serve.*.log")):
        log += f.read_text(errors="replace")
    lines = log.splitlines()
    edit_hits = [l for l in lines if "edit_check.py" in l and "hook executed" in l]
    edit_feedback = [l for l in edit_hits if "exit_code=1" in l]
    ctx_hits = [l for l in lines if "project_context.py" in l and "exit_code=0" in l and "stdout_len=0" not in l]
    cost_hits = [l for l in lines if "cost_guard.py" in l and "hook executed" in l]
    cost_bad = [l for l in cost_hits if "exit_code=0" not in l]
    hook_errors = [l for l in lines if ("hook" in l and ("WARN" in l or "ERROR" in l))]

    state = {}
    tmp = Path(env.get("TMPDIR") or "/tmp") / "oh-my-octos"
    for f in tmp.glob("*.json"):  # `s-<session>` under chat-with-ids, `p-<pid>` under serve
        try:
            state[f.name] = json.loads(f.read_text())
        except Exception:
            pass

    tests_ran = any(m == "tool/started" or "tool" in m for m, p in events
                    if "unittest" in json.dumps(p))
    if not tests_ran:  # tool event names vary; fall back to the answer text
        tests_ran = any(re.search(r"\bOK\b|Ran \d+ tests", t) for _, _, t, _ in transcript)
    files_ok = (project / "calc" / "__init__.py").exists() and (project / "tests" / "test_calc.py").exists()
    final_ok = bool(transcript) and transcript[-1][0] == "done" and transcript[-1][2].strip().upper().startswith("YES")
    syntax_clean = True
    try:
        compile((project / "calc" / "__init__.py").read_text(), "calc/__init__.py", "exec")
    except Exception:
        syntax_clean = False

    checks = {
        "all turns completed": all(ok for _, ok, _, _ in transcript) and len(transcript) == len(TURNS),
        "project files exist": files_ok,
        "final calc/__init__.py compiles": syntax_clean,
        "tests were run": tests_ran,
        "final answer says YES": final_ok,
        "edit_check fed back at least once (the deliberate break)": len(edit_feedback) >= 1,
        "project_context injected every turn": len(ctx_hits) >= len(TURNS),
        "cost_guard never errored": cost_hits and not cost_bad,
        "session spend recorded and > 0": any(v.get("session_cost", 0) > 0 for v in state.values()),
        "no hook WARN/ERROR lines": not hook_errors,
    }
    print("=== evidence")
    print("edit_check runs=%d feedback=%d  ctx=%d  cost_guard runs=%d  state=%s" % (
        len(edit_hits), len(edit_feedback), len(ctx_hits), len(cost_hits),
        {k: round(v.get("session_cost", 0), 4) for k, v in state.items()}))
    for l in hook_errors[:5]:
        print("hook warn:", l[:200])
    bad = [k for k, v in checks.items() if not v]
    for k, v in checks.items():
        print("%s %s" % ("PASS" if v else "FAIL", k))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
