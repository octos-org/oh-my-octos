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

# Paths and messages may be non-ASCII; never let stdout encoding take the hook down.
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

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


# Files that are JSON-with-comments by convention (tsconfig, VS Code settings, ...).
JSONC_NAMES = re.compile(r"(^|/)(tsconfig[^/]*\.json|jsconfig\.json|\.vscode/[^/]+\.json|devcontainer\.json|[^/]+\.jsonc)$")
_COMMENT = re.compile(r"//[^\n]*|/\*.*?\*/", re.S)
_TRAILING_COMMA = re.compile(r",(\s*[}\]])")


def _strip_jsonc(src):
    # Remove comments outside of strings, then trailing commas. Good enough for config files.
    out, i, n, in_str = [], 0, len(src), False
    while i < n:
        c = src[i]
        if in_str:
            out.append(c)
            if c == "\\" and i + 1 < n:
                out.append(src[i + 1]); i += 2; continue
            if c == '"':
                in_str = False
            i += 1
        elif c == '"':
            in_str = True; out.append(c); i += 1
        elif src.startswith("//", i):
            j = src.find("\n", i); i = n if j < 0 else j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2); i = n if j < 0 else j + 2
        else:
            out.append(c); i += 1
    return _TRAILING_COMMA.sub(r"\1", "".join(out))


def check_json(src, path):
    try:
        json.loads(src)
        return []
    except ValueError as e:
        strict_err = e
    # Second chance: JSON with comments / trailing commas is valid for many tools.
    try:
        json.loads(_strip_jsonc(src))
        return []
    except ValueError:
        pass
    if JSONC_NAMES.search(path.replace(os.sep, "/")):
        return ["invalid JSON (comments and trailing commas were tolerated): %s" % strict_err]
    return ["invalid JSON: %s" % strict_err]


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
    # Honour the shebang: a .sh file may be zsh or fish; only check with the shell it names.
    first = src.split("\n", 1)[0]
    shell = "bash"
    if first.startswith("#!"):
        m = re.search(r"(bash|zsh|sh|dash|ksh|fish)\b", first)
        if not m:
            return []
        shell = m.group(1)
        if shell == "fish":
            return []  # fish has no reliable -n; skip
    if not shutil.which(shell):
        return []
    rc, out = run([shell, "-n", path])
    if rc not in (0, None):
        last = out.splitlines()[-1] if out else "%s -n failed" % shell
        return ["shell syntax error: %s" % last]
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
        with open(path, "rb") as f:
            raw = f.read()
    except OSError:
        return findings
    if b"\x00" in raw[:8192]:
        return []  # binary; nothing to check
    src = raw.decode("utf-8", errors="replace")
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
