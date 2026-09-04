#!/usr/bin/env python3
"""E2E case: install.sh's interactive pack menu, driven through a pseudo-terminal.

usage: menu_case.py OCTOS_BIN_DIR SKILL_ROOT PROJECT_DIR
Types "3" (phonefarm) at the menu and expects the phonefarm skill to be installed.
The provider key env var named in the config must be set, otherwise install.sh
(correctly) stops at `octos auth login` and waits for a key to be pasted.
"""
import os
import pty
import select
import subprocess
import sys
import time


def main():
    bin_dir, root, project = sys.argv[1:4]
    env = dict(os.environ)
    env["PATH"] = bin_dir + os.pathsep + env.get("PATH", "")
    env["OMO_SKIP_BINARY_INSTALL"] = "1"
    master, slave = pty.openpty()
    p = subprocess.Popen(["bash", os.path.join(root, "install.sh"), "--source", root, "--project", project, "--no-serve"],
                         stdin=slave, stdout=slave, stderr=slave, env=env, cwd=project, close_fds=True)
    os.close(slave)
    out = b""
    answered = False
    deadline = time.time() + 300
    while time.time() < deadline and p.poll() is None:
        r, _, _ = select.select([master], [], [], 1.0)
        if master in r:
            try:
                chunk = os.read(master, 4096)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if not answered and b"Enter numbers" in out and b"> " in out[-400:]:
                os.write(master, b"3\n"); answered = True
            if b"Paste your" in out[-300:] and b"API key" in out[-300:]:
                print("install.sh is asking for an API key: export the provider key env var before running this case")
                break
    if p.poll() is None:
        p.kill()
    try:
        p.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass
    text = out.decode(errors="replace")
    installed = os.path.exists(os.path.join(project, ".octos", "skills", "phonefarm", "SKILL.md"))
    print(text[-1500:])
    print("answered=%s installed=%s rc=%s" % (answered, installed, p.returncode))
    return 0 if (answered and installed and p.returncode == 0) else 1


if __name__ == "__main__":
    sys.exit(main())
