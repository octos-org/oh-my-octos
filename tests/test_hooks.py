#!/usr/bin/env python3
"""Unit tests: run each hook script with synthetic Octos payloads.

No octos binary needed. Exercises the exit-code contract each event relies on:
  user_prompt_submit  exit 0 + stdout = injected context
  after_tool_call     exit 1 + stdout = feedback to the model, exit 0 silent = clean
  before_llm_call     exit 1 = deny
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HOOKS = os.path.join(ROOT, "hooks")


def run_hook(name, payload, cwd=None, env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    p = subprocess.run(
        [sys.executable, os.path.join(HOOKS, name)],
        input=json.dumps(payload), capture_output=True, text=True, cwd=cwd, env=e, timeout=30,
    )
    return p.returncode, p.stdout, p.stderr


class EditCheck(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="omo-edit-")

    def write(self, rel, content):
        p = os.path.join(self.dir, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(content)
        return rel

    def payload(self, rel, tool="write_file", success=True):
        # write_file is a sensitive tool: octos keeps only the path key.
        return {"event": "after_tool_call", "tool_name": tool, "tool_id": "t1",
                "arguments": {"path": rel, "redacted": True}, "result": "[redacted]",
                "success": success, "duration_ms": 3}

    def test_clean_python_is_silent(self):
        rel = self.write("ok.py", "def f():\n    return 1\n")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel), cwd=self.dir)
        self.assertEqual((rc, out), (0, ""))

    def test_python_syntax_error_is_feedback(self):
        rel = self.write("bad.py", "def f(:\n    pass\n")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel), cwd=self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("python syntax error", out)
        self.assertIn("bad.py", out)

    def test_invalid_json(self):
        rel = self.write("cfg.json", '{"a": 1,}')
        rc, out, _ = run_hook("edit_check.py", self.payload(rel, tool="edit_file"), cwd=self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("invalid JSON", out)

    def test_conflict_markers(self):
        rel = self.write("notes.md", "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> branch\n")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel, tool="diff_edit"), cwd=self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("conflict markers", out)

    def test_empty_file(self):
        rel = self.write("empty.txt", "")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel), cwd=self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("empty", out)

    def test_shell_syntax(self):
        rel = self.write("run.sh", "if [ 1 ]; then\necho x\n")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel), cwd=self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("shell syntax error", out)

    def test_failed_edit_is_ignored(self):
        rel = self.write("bad.py", "def f(:\n")
        rc, out, _ = run_hook("edit_check.py", self.payload(rel, success=False), cwd=self.dir)
        self.assertEqual((rc, out), (0, ""))

    def test_missing_path_is_ignored(self):
        rc, out, _ = run_hook("edit_check.py", {"event": "after_tool_call", "tool_name": "write_file",
                                                "arguments": {"redacted": True}}, cwd=self.dir)
        self.assertEqual((rc, out), (0, ""))

    def test_garbage_stdin_never_blocks(self):
        p = subprocess.run([sys.executable, os.path.join(HOOKS, "edit_check.py")], input="not json",
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 0)


class CostGuard(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="omo-cost-")
        self.env = {"TMPDIR": self.tmp, "OMO_SESSION_BUDGET_USD": "0.05"}
        self.sid = "unit-test-%d" % os.getpid()

    def test_deny_after_budget(self):
        rc, out, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": self.sid}, env=self.env)
        self.assertEqual(rc, 0, "nothing recorded yet: allow")
        rc, _, _ = run_hook("cost_guard.py", {"event": "after_llm_call", "session_id": self.sid,
                                              "session_cost": 0.01}, env=self.env)
        self.assertEqual(rc, 0)
        rc, out, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": self.sid}, env=self.env)
        self.assertEqual(rc, 0, "under budget: allow")
        run_hook("cost_guard.py", {"event": "after_llm_call", "session_id": self.sid, "session_cost": 0.07}, env=self.env)
        rc, out, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": self.sid}, env=self.env)
        self.assertEqual(rc, 1, "over budget: deny")
        self.assertIn("budget", out)

    def test_other_session_unaffected(self):
        run_hook("cost_guard.py", {"event": "after_llm_call", "session_id": self.sid, "session_cost": 9}, env=self.env)
        rc, _, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": "someone-else"}, env=self.env)
        self.assertEqual(rc, 0)

    def test_disabled_with_zero_budget(self):
        env = dict(self.env, OMO_SESSION_BUDGET_USD="0")
        run_hook("cost_guard.py", {"event": "after_llm_call", "session_id": self.sid, "session_cost": 99}, env=env)
        rc, _, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": self.sid}, env=env)
        self.assertEqual(rc, 0)

    def test_unpriced_provider_never_denies(self):
        run_hook("cost_guard.py", {"event": "after_llm_call", "session_id": self.sid, "session_cost": None}, env=self.env)
        rc, _, _ = run_hook("cost_guard.py", {"event": "before_llm_call", "session_id": self.sid}, env=self.env)
        self.assertEqual(rc, 0)


class ProjectContext(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="omo-ctx-")

    def payload(self):
        return {"event": "user_prompt_submit", "prompt": "hi", "cwd": self.dir, "model": "x"}

    def test_repo_root_agents_md_is_injected(self):
        with open(os.path.join(self.dir, "AGENTS.md"), "w") as f:
            f.write("Codeword: pelican-42\n")
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertEqual(rc, 0)
        self.assertIn("pelican-42", out)
        self.assertIn("AGENTS.md", out)

    def test_claude_md_fallback(self):
        with open(os.path.join(self.dir, "CLAUDE.md"), "w") as f:
            f.write("Codeword: heron-7\n")
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertIn("heron-7", out)

    def test_skipped_when_octos_agents_md_exists(self):
        os.makedirs(os.path.join(self.dir, ".octos"))
        with open(os.path.join(self.dir, ".octos", "AGENTS.md"), "w") as f:
            f.write("octos-managed\n")
        with open(os.path.join(self.dir, "AGENTS.md"), "w") as f:
            f.write("Codeword: pelican-42\n")
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertNotIn("pelican-42", out)

    def test_truncation(self):
        with open(os.path.join(self.dir, "AGENTS.md"), "w") as f:
            f.write("x" * 20000)
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertIn("truncated", out)
        self.assertLess(len(out), 7000)

    def test_git_line(self):
        subprocess.run(["git", "init", "-q"], cwd=self.dir, check=True)
        subprocess.run(["git", "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty",
                        "-m", "first commit"], cwd=self.dir, check=True)
        with open(os.path.join(self.dir, "dirty.txt"), "w") as f:
            f.write("d")
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertIn("git: branch", out)
        self.assertIn("1 uncommitted", out)
        self.assertIn("first commit", out)

    def test_nothing_to_say_is_silent(self):
        rc, out, _ = run_hook("project_context.py", self.payload(), cwd=self.dir)
        self.assertEqual((rc, out), (0, ""))


if __name__ == "__main__":
    unittest.main(verbosity=1)
