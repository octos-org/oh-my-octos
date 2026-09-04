#!/usr/bin/env bash
# End-to-end suite for oh-my-octos against a real octos binary.
#
# Everything runs in a throwaway OCTOS_HOME; ~/.octos is never read or written.
# Each case installs the skill from this checkout with `octos skills install <path>`,
# runs one octos turn, and asserts on (a) the model's answer and (b) octos's own
# log lines proving the hook fired (`hook executed ... exit_code=N stdout_len=N`).
#
# Required env:
#   OCTOS_BIN            path to an octos binary (2.0.3-rc.10 or newer)
#   DEEPSEEK_API_KEY     or set OMO_E2E_PROVIDER / OMO_E2E_MODEL / OMO_E2E_KEY_ENV for another provider
# Optional:
#   OMO_E2E_KEEP=1       keep the work dir for inspection
#   OMO_E2E_WORK_ROOT    parent of the throwaway work dir (default /tmp; keep it short)
#   OMO_E2E_SKIP_SERVE=1 skip the `octos serve --stdio` case
set -u  # no pipefail: `cmd | grep -q` closes the pipe early and octos would report SIGPIPE

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OCTOS_BIN="${OCTOS_BIN:-$(command -v octos || true)}"
[ -x "$OCTOS_BIN" ] || { echo "OCTOS_BIN not set and octos not on PATH" >&2; exit 2; }
PROVIDER="${OMO_E2E_PROVIDER:-deepseek}"
MODEL="${OMO_E2E_MODEL:-deepseek-v4-flash}"
KEY_ENV="${OMO_E2E_KEY_ENV:-DEEPSEEK_API_KEY}"
[ -n "${!KEY_ENV:-}" ] || { echo "$KEY_ENV is not set" >&2; exit 2; }

# Short path on purpose: octos serve creates a unix socket under the data dir and
# the OS caps socket paths at ~104 bytes. Never inside the repo: the skill
# install copies the checkout, and a work dir inside it would be copied too.
WORK="$(mktemp -d "${OMO_E2E_WORK_ROOT:-/tmp}/omo-e2e-XXXX")"
export OCTOS_HOME="$WORK/home"
mkdir -p "$OCTOS_HOME"
printf '{"provider":"%s","model":"%s","api_key_env":"%s"}\n' "$PROVIDER" "$MODEL" "$KEY_ENV" > "$OCTOS_HOME/config.json"
export TMPDIR="$WORK/tmp"; mkdir -p "$TMPDIR"

PASS=0; FAIL=0; SKIP=0
result() { # name status detail
  case "$2" in
    PASS) PASS=$((PASS+1)); printf 'PASS  %s\n' "$1" ;;
    SKIP) SKIP=$((SKIP+1)); printf 'SKIP  %s  (%s)\n' "$1" "$3" ;;
    *)    FAIL=$((FAIL+1)); printf 'FAIL  %s\n      %s\n' "$1" "$3" ;;
  esac
}

new_project() { # name -> path; skill installed
  local p="$WORK/$1"; mkdir -p "$p"
  if ! (cd "$p" && "$OCTOS_BIN" skills install "$ROOT" --force >"$p/.install.log" 2>&1); then
    echo "skill install failed in $p:" >&2; cat "$p/.install.log" >&2; exit 1
  fi
  echo "$p"
}

chat() { # project prompt [extra flags...] -> stdout JSON in $OUT, stderr in $ERR
  local p="$1"; shift; local prompt="$1"; shift
  OUT="$p/out.json"; ERR="$p/err.log"
  (cd "$p" && "$OCTOS_BIN" chat -v --no-session-persistence --json "$@" -m "$prompt" >"$OUT" 2>"$ERR")
}

answer() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("text",""))' "$OUT" 2>/dev/null; }
hook_line() { grep -E "hook executed hook=\[\"[^\"]*$1\"\]" "$ERR" | tail -1; }

# ----------------------------------------------------------------------------- 1 install
P="$(new_project t1-install)" || exit 1
# `octos skills list` prints its table on stderr.
if (cd "$P" && "$OCTOS_BIN" skills list 2>&1 | grep -q "oh-my-octos") \
   && (cd "$P" && "$OCTOS_BIN" skills info oh-my-octos 2>/dev/null | grep -q "Tools: (0 tool(s))"); then
  result "install: skills install <path> + skills list" PASS
else
  result "install: skills install <path> + skills list" FAIL "$(tail -3 "$P/.install.log")"
fi
if [ -x "$P/.octos/skills/oh-my-octos/hooks/edit_check.py" ]; then
  result "install: hook scripts keep the executable bit" PASS
else
  result "install: hook scripts keep the executable bit" FAIL "$(ls -l "$P/.octos/skills/oh-my-octos/hooks/")"
fi

# ----------------------------------------------------------------------------- 2 prompt fragment
P="$(new_project t2-prompt)"
chat "$P" "Do your instructions include a section titled 'oh-my-octos work discipline'? Answer YES or NO on the first line, then quote rule 6 of that section verbatim." --sandbox read-only
A="$(answer)"
if grep -q "prompt_fragments=3" "$ERR" && grep -q "hooks=4" "$ERR" && printf '%s' "$A" | grep -q "^YES" && printf '%s' "$A" | grep -q "Act on hook feedback"; then
  result "prompt: discipline fragment reaches the model (rule 6 quoted)" PASS
else
  result "prompt: discipline fragment reaches the model" FAIL "answer=$(printf '%s' "$A" | head -c 200) / $(grep -E 'loaded skill extras' "$ERR" | tail -1)"
fi

# ----------------------------------------------------------------------------- 3 project context
P="$(new_project t3-context)"
printf '# Project notes\n\nThe project codeword is pelican-42.\n' > "$P/AGENTS.md"
chat "$P" "Without using any tools, answer from the context you already have: what is the project codeword? Reply with just the codeword." --sandbox read-only
A="$(answer)"; H="$(hook_line project_context.py)"
if printf '%s' "$H" | grep -q "exit_code=0" && printf '%s' "$H" | grep -Eq "stdout_len=[1-9][0-9]*" \
   && printf '%s' "$A" | grep -q "pelican-42" && grep -q "tool_calls=0" "$ERR"; then
  result "context: repo-root AGENTS.md injected via user_prompt_submit (no tools used)" PASS
else
  result "context: repo-root AGENTS.md injected" FAIL "hook=[$H] answer=$(printf '%s' "$A" | head -c 120)"
fi

# ----------------------------------------------------------------------------- 4 edit check feedback
P="$(new_project t4-edit)"
chat "$P" "Use the write_file tool to create a file named bad.py whose content is exactly these two lines and nothing else (deliberate syntax error for a test; do not fix it, do not run it):
def f(:
    pass
After the tool result comes back, reply with every line of the tool result that contains 'edit_check' copied verbatim, or the word NONE if there is no such line. Then stop." --sandbox workspace-write --ask-for-approval never
A="$(answer)"; H="$(hook_line edit_check.py)"
if [ -f "$P/bad.py" ] && printf '%s' "$H" | grep -q "exit_code=1" && printf '%s' "$A" | grep -q "python syntax error"; then
  result "edit: syntax error fed back to the model as [hook] feedback" PASS
else
  result "edit: syntax error fed back to the model" FAIL "hook=[$H] answer=$(printf '%s' "$A" | head -c 200) bad.py=$([ -f "$P/bad.py" ] && echo yes || echo missing)"
fi

# ----------------------------------------------------------------------------- 5 clean edit is silent
P="$(new_project t5-clean)"
chat "$P" "Use the write_file tool to create ok.py containing exactly one line: x = 1
After the tool result comes back, reply with every line of the tool result that contains '[hook]' copied verbatim, or the word NONE if there is no such line. Then stop." --sandbox workspace-write --ask-for-approval never
A="$(answer)"; H="$(hook_line edit_check.py)"
if [ -f "$P/ok.py" ] && printf '%s' "$H" | grep -q "exit_code=0" && printf '%s' "$A" | grep -q "NONE"; then
  result "edit: clean file produces no feedback" PASS
else
  result "edit: clean file produces no feedback" FAIL "hook=[$H] answer=$(printf '%s' "$A" | head -c 200)"
fi

# ----------------------------------------------------------------------------- 6 budget guard
P="$(new_project t6-budget)"
touch "$P/a.txt" "$P/b.txt"
OMO_SESSION_BUDGET_USD=0.000001 chat "$P" "Use a tool to list the files in the current directory, then tell me how many files there are." --sandbox read-only
A="$(answer)"; H="$(hook_line cost_guard.py)"
if grep -q "session_cost" "$TMPDIR"/oh-my-octos/*.json 2>/dev/null && grep -qi "denied by hook" "$ERR" "$OUT" 2>/dev/null; then
  result "budget: second model call denied once spend passes the budget" PASS
elif ! grep -q "session_cost" "$TMPDIR"/oh-my-octos/*.json 2>/dev/null; then
  result "budget: second model call denied" FAIL "no session_cost recorded (provider unpriced?) hook=[$H]"
else
  result "budget: second model call denied" FAIL "hook=[$H] answer=$(printf '%s' "$A" | head -c 160)"
fi

# ----------------------------------------------------------------------------- 7 serve --stdio path (octoscode / web / arc share it)
if [ "${OMO_E2E_SKIP_SERVE:-0}" = "1" ]; then
  result "serve: hooks and prompt active under octos serve --stdio" SKIP "OMO_E2E_SKIP_SERVE=1"
else
  P="$WORK/t7-serve"; mkdir -p "$P"
  printf '# Project notes\n\nThe project codeword is heron-77.\n' > "$P/AGENTS.md"
  if python3 "$ROOT/tests/serve_case.py" "$OCTOS_BIN" "$P" "$OCTOS_HOME" "$ROOT" "$PROVIDER" "$MODEL" "$KEY_ENV" >"$P/serve.log" 2>&1; then
    result "serve: hooks and prompt active under octos serve --stdio" PASS
  else
    result "serve: hooks and prompt active under octos serve --stdio" FAIL "$(tail -5 "$P/serve.log" | tr '\n' ' ' | head -c 400)"
  fi
fi

# ----------------------------------------------------------------------------- 8 remove
P="$(new_project t8-remove)"
(cd "$P" && "$OCTOS_BIN" skills remove oh-my-octos >/dev/null 2>&1)
if [ ! -e "$P/.octos/skills/oh-my-octos" ]; then
  chat "$P" "Reply with exactly the word OK." --sandbox read-only
  if ! grep -q "oh-my-octos" "$ERR"; then
    result "remove: directory gone and nothing loads afterwards" PASS
  else
    result "remove: directory gone and nothing loads afterwards" FAIL "skill still referenced in logs"
  fi
else
  result "remove: directory gone" FAIL "directory still present"
fi

# ----------------------------------------------------------------------------- 9 install.sh guided path (non-interactive parts)
P="$WORK/t9-install-sh"; mkdir -p "$P"
if (cd "$P" && PATH="$(dirname "$OCTOS_BIN"):$PATH" OMO_SKIP_BINARY_INSTALL=1 bash "$ROOT/install.sh" --source "$ROOT" --project "$P" --no-serve </dev/null >"$P/install-sh.log" 2>&1) \
   && [ -f "$P/.octos/skills/oh-my-octos/manifest.json" ]; then
  result "install.sh: guided path installs into --project without prompting" PASS
else
  result "install.sh: guided path" FAIL "$(tail -4 "$P/install-sh.log" | tr '\n' ' ' | head -c 300)"
fi

echo
echo "passed=$PASS failed=$FAIL skipped=$SKIP  work=$WORK"
[ "${OMO_E2E_KEEP:-0}" = "1" ] || [ "$FAIL" != 0 ] || rm -rf "$WORK"
[ "$FAIL" = 0 ]
