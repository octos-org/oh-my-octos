#!/usr/bin/env python3
"""Deterministic post-edit check for Octos.

Registered on after_tool_call for write_file / edit_file / diff_edit.
Octos redacts file contents from hook payloads, so this script re-reads the
edited file from disk (the hook runs with cwd = workspace root) and reports:

  - syntax errors for .py .json .toml .sh/.bash .js/.mjs/.cjs (js only if node is on PATH)
  - unresolved merge conflict markers
  - a file left empty by the edit

Findings go to stdout with exit 1, which Octos appends to the tool result as
"[hook] ..." so the model sees them. A clean file exits 0 silently.
Stdlib only.
"""
import ast
import json
import os
import re
import shutil
import subprocess
import sys

MAX_BYTES = 2 * 1024 * 1024
CONFLICT = re.compile(r"^(<{7} |={7}$|>{7} )", re.M)


def target_path(payload):
    args = payload.get("arguments") or {}
    if not isinstance(args, dict):
        return None
    for key in ("path", "file_path", "filename", "file"):
        v = args.get(key)
        if isinstance(v, str) and v:
            return v
    return None


def check_python(src, path):
    try:
        ast.parse(src, filename=path)
    except SyntaxError as e:
        return ["python syntax error line %s: %s" % (e.lineno, e.msg)]
    return []


def check_json(src, path):
    try:
        json.loads(src)
    except ValueError as e:
        return ["invalid JSON: %s" % e]
    return []


def check_toml(src, path):
    try:
        import tomllib
    except ImportError:
        return []
    try:
        tomllib.loads(src)
    except Exception as e:
        return ["invalid TOML: %s" % e]
    return []


def run(cmd, timeout=10):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except Exception as e:
        return None, str(e)
    out = (p.stdout + "\n" + p.stderr).strip()
    return p.returncode, out


def check_shell(src, path):
    if not shutil.which("bash"):
        return []
    rc, out = run(["bash", "-n", path])
    if rc not in (0, None):
        return ["shell syntax error: %s" % out.splitlines()[-1] if out else "shell syntax error"]
    return []


def check_js(src, path):
    if not shutil.which("node"):
        return []
    rc, out = run(["node", "--check", path])
    if rc not in (0, None):
        lines = [l for l in out.splitlines() if l.strip()]
        return ["javascript syntax error: %s" % (lines[-1] if lines else "node --check failed")]
    return []


CHECKERS = {
    ".py": check_python,
    ".json": check_json,
    ".toml": check_toml,
    ".sh": check_shell,
    ".bash": check_shell,
    ".js": check_js,
    ".mjs": check_js,
    ".cjs": check_js,
}


def check_file(path):
    findings = []
    try:
        size = os.path.getsize(path)
    except OSError:
        return findings  # deleted or never written; nothing to check
    if size == 0:
        return ["file is empty after the edit"]
    if size > MAX_BYTES:
        return []
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            src = f.read()
    except OSError:
        return findings
    if CONFLICT.search(src):
        findings.append("unresolved merge conflict markers (<<<<<<< / ======= / >>>>>>>)")
    ext = os.path.splitext(path)[1].lower()
    checker = CHECKERS.get(ext)
    if checker:
        findings.extend(checker(src, path))
    return findings


def main():
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return 0
    if payload.get("success") is False:
        return 0  # the edit itself failed; the model already sees that error
    rel = target_path(payload)
    if not rel:
        return 0
    path = rel if os.path.isabs(rel) else os.path.join(os.getcwd(), rel)
    findings = check_file(path)
    if not findings:
        return 0
    for f in findings:
        print("edit_check %s: %s" % (rel, f))
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
