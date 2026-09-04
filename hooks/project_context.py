#!/usr/bin/env python3
"""Per-turn project context for Octos.

Registered on user_prompt_submit. Exit 0 with stdout = context injected for
this turn only (Octos prepends it as a system-context note; nothing is persisted).

Injects:
  1. The repo-root AGENTS.md or CLAUDE.md, when the workspace has no
     .octos/AGENTS.md (Octos already loads that one itself).
  2. One line of git state: branch, dirty file count, last commit subject.

Capped so a turn never pays more than a few hundred tokens for it. Stdlib only.
"""
import json
import os
import shutil
import subprocess
import sys
import time

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

# The hook has a 5 s budget in the manifest; keep git well inside it.
GIT_BUDGET_SECONDS = 2.5
_deadline = [0.0]

MAX_INSTRUCTION_CHARS = 6000
INSTRUCTION_FILES = ("AGENTS.md", "CLAUDE.md")


def workspace(payload):
    cwd = payload.get("cwd")
    if isinstance(cwd, str) and os.path.isdir(cwd):
        return cwd
    return os.getcwd()


def instructions(root):
    if os.path.isfile(os.path.join(root, ".octos", "AGENTS.md")):
        return None  # octos injects this one already
    for name in INSTRUCTION_FILES:
        p = os.path.join(root, name)
        if os.path.isfile(p):
            try:
                with open(p, "r", encoding="utf-8", errors="replace") as f:
                    text = f.read().strip()
            except OSError:
                continue
            if not text:
                continue
            if len(text) > MAX_INSTRUCTION_CHARS:
                text = text[:MAX_INSTRUCTION_CHARS] + "\n... (truncated; read %s for the rest)" % name
            return name, text
    return None


def git(root, *args):
    if not shutil.which("git"):
        return None
    remaining = _deadline[0] - time.monotonic()
    if remaining <= 0.05:
        return None
    try:
        p = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True, timeout=remaining)
    except Exception:
        _deadline[0] = 0.0  # one slow call: stop asking git anything else this turn
        return None
    if p.returncode != 0:
        return None
    return p.stdout.strip()


def git_line(root):
    _deadline[0] = time.monotonic() + GIT_BUDGET_SECONDS
    if not os.path.isdir(os.path.join(root, ".git")) and git(root, "rev-parse", "--is-inside-work-tree") != "true":
        return None
    branch = git(root, "rev-parse", "--abbrev-ref", "HEAD") or "?"
    status = git(root, "status", "--porcelain", "--untracked-files=normal")
    last = git(root, "log", "-1", "--format=%h %s") or "no commits"
    if status is None:
        return "git: branch %s, last commit: %s (status skipped: repo too slow for this turn)" % (branch, last)
    dirty = len([l for l in status.splitlines() if l.strip()])
    return "git: branch %s, %d uncommitted file(s), last commit: %s" % (branch, dirty, last)


def main():
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return 0
    root = workspace(payload)
    parts = []
    ins = instructions(root)
    if ins:
        name, text = ins
        parts.append("Project instructions from %s (repo root):\n%s" % (name, text))
    g = git_line(root)
    if g:
        parts.append(g)
    if parts:
        sys.stdout.write("\n\n".join(parts) + "\n")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
