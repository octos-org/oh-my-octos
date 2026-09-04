#!/usr/bin/env python3
"""E2E case: oh-my-octos under `octos serve --stdio --solo`.

This is the path octoscode, octoscode-web and the arc-bench adapter all use.
Steps:
  1. spawn serve, create a solo profile with the given provider/model (UI Protocol)
  2. stop serve, install the skill into that profile (`octos skills --profile <id> install <root>`)
  3. respawn serve, open a session on the profile, run two turns:
       a. codeword from repo-root AGENTS.md without tools   -> project_context hook
       b. write a broken bad.py and report [hook] lines      -> edit_check hook
Exit 0 on success; prints the evidence either way.

usage: serve_case.py OCTOS_BIN PROJECT_DIR DATA_DIR SKILL_ROOT PROVIDER MODEL KEY_ENV
"""
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from octos_stdio import OctosStdioSession  # noqa: E402


def main():
    octos_bin, project, data_dir, skill_root, provider, model, key_env = sys.argv[1:8]
    project = Path(project).resolve()
    data_dir = Path(data_dir).resolve()
    env = dict(os.environ)
    env["OCTOS_HOME"] = str(data_dir)
    env.setdefault("RUST_LOG", "info")  # serve is quiet by default; hook lines are INFO
    events = []

    def on_event(method, params):
        events.append((method, params))

    # 1. profile
    s = OctosStdioSession(octos_bin, project, env, data_dir, on_event)
    s.bootstrap_profile(provider, model, None, key_env)
    pid = s.profile_id
    s.close()
    print("profile:", pid)

    # 2. install into the profile
    r = subprocess.run([octos_bin, "skills", "--profile", pid, "install", skill_root, "--force"],
                       env=env, capture_output=True, text=True, timeout=120)
    print("install rc", r.returncode, (r.stdout + r.stderr).strip().splitlines()[-1:])
    if r.returncode != 0:
        return 1

    # 3. session + turns
    s = OctosStdioSession(octos_bin, project, env, data_dir, on_event)
    s.profile_id = pid
    s.open()
    ok, text = s.run_turn(
        "Without using any tools, answer from the context you already have: "
        "what is the project codeword? Reply with just the codeword.", timeout=300)
    print("turn a:", ok, repr(text[:200]))
    a_methods = sorted({m for m, _ in events})
    a_tool_events = [m for m, _ in events if "tool" in m.lower()]
    print("turn a events:", a_methods)
    a_pass = ok and "heron-77" in text and not a_tool_events
    events.clear()

    ok2, text2 = s.run_turn(
        "Use the write_file tool to create a file named bad.py whose content is exactly these two "
        "lines and nothing else (deliberate syntax error for a test; do not fix it, do not run it):\n"
        "def f(:\n    pass\n"
        "After the tool result comes back, reply with every line of the tool result that contains "
        "'edit_check' copied verbatim, or the word NONE if there is no such line. Then stop.", timeout=300)
    print("turn b:", ok2, repr(text2[:300]))
    # Whether the model echoes the [hook] line is up to the model; the proof that
    # the hook ran and produced feedback is octos's own log under <data_dir>/logs/.
    b_pass = ok2 and (project / "bad.py").exists()
    if "python syntax error" not in text2:
        print("note: model did not echo the feedback line; checking the serve log instead")

    s.close()
    log = ""
    for f in sorted((data_dir / "logs").glob("serve.*.log")):
        log += f.read_text(errors="replace")
    lines = log.splitlines()
    hooks_loaded = any("plugin=oh-my-octos" in l for l in lines) and any("hooks=4" in l and "prompt_fragments=" in l for l in lines)
    ctx_hook = any("project_context.py" in l and "exit_code=0" in l and "stdout_len=0" not in l for l in lines)
    edit_hook = any("edit_check.py" in l and "exit_code=1" in l for l in lines)
    print("evidence (serve log): hooks_loaded=%s ctx_hook=%s edit_hook=%s a=%s b=%s" % (hooks_loaded, ctx_hook, edit_hook, a_pass, b_pass))
    if not (a_pass and b_pass and hooks_loaded and ctx_hook and edit_hook):
        print("--- serve log tail ---")
        print("\n".join(lines[-30:]))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
