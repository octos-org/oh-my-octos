#!/usr/bin/env python3
"""E2E case: the real octoscode TUI, driven through a pseudo-terminal.

octoscode is a thin client; it spawns `octos serve --stdio` itself. We give it a
throwaway data dir and a pre-made profile with oh-my-octos installed, then:
  1. --prompt asks for the repo-root AGENTS.md codeword (project_context hook)
  2. type a second prompt into the composer: write a broken bad.py (edit_check hook)
  3. Ctrl+Q
Evidence is the serve log under <data_dir>/logs plus the text rendered on screen.

usage: tui_case.py OCTOSCODE_BIN OCTOS_BIN PROJECT_DIR DATA_DIR SKILL_ROOT PROVIDER MODEL KEY_ENV
"""
import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from octos_stdio import OctosStdioSession  # noqa: E402

ANSI = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[()][A-Za-z0-9]|\x1b[=>]|\x1b\][^\x07]*\x07|\r")


def plain(buf):
    return ANSI.sub(b"", buf).decode("utf-8", errors="replace")


def main():
    oc, octos, project, data_dir, skill_root, provider, model, key_env = sys.argv[1:9]
    project = Path(project).resolve(); data_dir = Path(data_dir).resolve()
    project.mkdir(parents=True, exist_ok=True)
    (project / "AGENTS.md").write_text("# demo\n\nThe project codeword is osprey-31.\n")
    env = dict(os.environ)
    env["OCTOS_HOME"] = str(data_dir); env["TERM"] = "xterm-256color"; env["OCTOSCODE_NO_AUTO_INSTALL"] = "1"
    env["OCTOS_LANG"] = "en"

    # profile + skill (same steps octoscode's own onboarding would do, minus the UI)
    s = OctosStdioSession(octos, project, env, data_dir)
    s.bootstrap_profile(provider, model, None, key_env); pid = s.profile_id; s.close()
    r = subprocess.run([octos, "skills", "--profile", pid, "install", skill_root, "--force"], env=env, capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr
    print("profile:", pid)

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
    cmd = [oc, "--no-splash", "--stdio-command", "%s serve --stdio --solo --data-dir %s" % (octos, data_dir),
           "--profile-id", pid, "--cwd", str(project), "--session", "omo-tui-%d" % os.getpid(),
           "--prompt", "Without using any tools, answer from the context you already have: what is the project codeword? Reply with just the codeword."]
    p = subprocess.Popen(cmd, stdin=slave, stdout=slave, stderr=slave, env=env, cwd=str(project), close_fds=True)
    os.close(slave)
    buf = b""

    activated = [False]

    def pump(until, timeout):
        nonlocal buf
        deadline = time.time() + timeout
        while time.time() < deadline and p.poll() is None:
            r, _, _ = select.select([master], [], [], 0.5)
            if master in r:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                buf += chunk
            text = plain(buf)
            # First launch on a folder shows "Activate this folder?" with the profile
            # preselected; a person presses Enter. So do we, once.
            if not activated[0] and "Activate this folder?" in text[-4000:]:
                time.sleep(0.3); os.write(master, b"\r"); activated[0] = True
            if until(text):
                return True
        return until(plain(buf))

    def write(data):
        try:
            os.write(master, data)
        except OSError:
            pass

    a_ok = pump(lambda t: "osprey-31" in t, 240)
    print("turn a (startup --prompt): codeword on screen =", a_ok, " activation menu answered =", activated[0])
    time.sleep(2)

    # second prompt through the composer
    prompt2 = ("Use the write_file tool to create bad.py containing exactly these two lines (deliberate syntax error "
               "for a test; do not fix it, do not run it):  def f(:  and  pass  on the next line. Then reply with any "
               "line of the tool result that contains edit_check, or NONE.")
    write(prompt2.encode()); time.sleep(0.5); write(b"\r")
    b_ok = pump(lambda t: "python syntax error" in t or ("bad.py" in t and "NONE" in t.split("bad.py")[-1]), 300)
    time.sleep(2)
    write(b"\x11")  # Ctrl+Q
    try:
        p.wait(timeout=20)
    except subprocess.TimeoutExpired:
        p.kill()
    screen = plain(buf)
    print("turn b (composer): syntax feedback on screen =", "python syntax error" in screen, " bad.py exists =", (project / "bad.py").exists())

    log = "".join(f.read_text(errors="replace") for f in sorted((data_dir / "logs").glob("serve.*.log")))
    lines = log.splitlines()
    ctx = [l for l in lines if "project_context.py" in l and "exit_code=0" in l and "stdout_len=0" not in l]
    edit = [l for l in lines if "edit_check.py" in l and "exit_code=1" in l]
    loaded = any("plugin=oh-my-octos" in l for l in lines)
    print("evidence (serve log): loaded=%s ctx_hook=%d edit_feedback=%d" % (loaded, len(ctx), len(edit)))
    ok = a_ok and loaded and len(ctx) >= 1 and len(edit) >= 1 and (project / "bad.py").exists()
    if not ok:
        print("--- screen tail ---"); print(screen[-2500:])
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
