#!/usr/bin/env python3
"""Session budget guard for Octos.

Registered on two events:
  after_llm_call  - records the running session spend Octos reports.
  before_llm_call - exits 1 (deny) once spend passes the budget.

Budget: OMO_SESSION_BUDGET_USD (default 10). Zero or negative disables the guard.
State: one small JSON file per session under the system temp dir.
Stdlib only. Never raises: any internal failure exits 0 so the guard can never block by accident.
"""
import json
import os
import sys
import tempfile
import time

DEFAULT_BUDGET_USD = 10.0
STATE_TTL_SECONDS = 7 * 24 * 3600


def state_dir():
    d = os.path.join(tempfile.gettempdir(), "oh-my-octos")
    os.makedirs(d, exist_ok=True)
    return d


def session_key(payload):
    sid = payload.get("session_id")
    if sid:
        safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in str(sid))
        return "s-" + safe[:120]
    # octos chat carries no session id; the parent process is the session.
    return "p-%d" % os.getppid()


def state_path(payload):
    return os.path.join(state_dir(), session_key(payload) + ".json")


def budget():
    raw = os.environ.get("OMO_SESSION_BUDGET_USD", "").strip()
    if not raw:
        return DEFAULT_BUDGET_USD
    try:
        return float(raw)
    except ValueError:
        return DEFAULT_BUDGET_USD


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return {}


def save(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f)
    os.replace(tmp, path)


def sweep_old_state():
    now = time.time()
    try:
        for name in os.listdir(state_dir()):
            p = os.path.join(state_dir(), name)
            if now - os.path.getmtime(p) > STATE_TTL_SECONDS:
                os.remove(p)
    except Exception:
        pass


def main():
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return 0
    event = payload.get("event")
    cap = budget()
    if cap <= 0:
        return 0
    path = state_path(payload)

    if event == "after_llm_call":
        cost = payload.get("session_cost")
        if isinstance(cost, (int, float)):
            data = load(path)
            data["session_cost"] = float(cost)
            data["updated"] = time.time()
            data["model"] = payload.get("model")
            save(path, data)
            sweep_old_state()
        return 0

    if event == "before_llm_call":
        data = load(path)
        spent = data.get("session_cost")
        if isinstance(spent, (int, float)) and spent >= cap:
            print(
                "session spend $%.4f reached the oh-my-octos budget of $%s "
                "(OMO_SESSION_BUDGET_USD). Summarize the current state for the "
                "user and stop." % (spent, ("%.6f" % cap).rstrip("0").rstrip("."))
            )
            return 1
        return 0
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
