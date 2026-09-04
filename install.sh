#!/usr/bin/env bash
# oh-my-octos guided install.
#
# Runs the existing octos commands in the right order and checks each step.
# It does not store keys, does not write config on its own, and does not
# replace `octos init`. Re-running is safe; every step is skipped when done.
#
# Usage:
#   bash install.sh [--profile <id>] [--project <dir>] [--source <skill source>] [--no-serve]
#
# Environment:
#   OMO_SKIP_BINARY_INSTALL=1   do not try to install octos when it is missing

set -euo pipefail

SOURCE="octos-org/oh-my-octos"
PROFILE=""
PROJECT=""
START_SERVE=1

while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --project) PROJECT="$2"; shift 2 ;;
    --source) SOURCE="$2"; shift 2 ;;
    --no-serve) START_SERVE=0; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

step() { printf '\n==> %s\n' "$*"; }
ok()   { printf '    ok: %s\n' "$*"; }
fail() { printf '    FAILED: %s\n' "$*" >&2; exit 1; }

# 1. octos binary --------------------------------------------------------------
step "octos binary"
if ! command -v octos >/dev/null 2>&1; then
  if [ "${OMO_SKIP_BINARY_INSTALL:-0}" = "1" ]; then
    fail "octos is not on PATH and OMO_SKIP_BINARY_INSTALL=1"
  fi
  case "$(uname -s)" in
    Darwin)
      command -v brew >/dev/null 2>&1 || fail "Homebrew is required on macOS: https://brew.sh"
      brew tap octos-org/octos https://github.com/octos-org/octos
      brew install octos-org/octos/octos
      ;;
    Linux)
      curl -fsSL https://github.com/octos-org/octos/releases/latest/download/install.sh | bash
      ;;
    *) fail "unsupported OS $(uname -s); install octos manually: https://github.com/octos-org/octos#start-here" ;;
  esac
fi
command -v octos >/dev/null 2>&1 || fail "octos still not on PATH after install; open a new shell and re-run"
ok "$(octos --version)"

# 2. config --------------------------------------------------------------------
step "config"
HOME_DIR="${OCTOS_HOME:-$HOME/.octos}"
if [ -f "$HOME_DIR/config.json" ] || [ -f "$HOME_DIR/.octos/config.json" ]; then
  ok "config exists under $HOME_DIR"
elif [ -t 0 ]; then
  echo "    no config yet; running 'octos init' (interactive: pick a provider and a real model name)"
  octos init --cwd "$HOME_DIR"
else
  fail "no config and no terminal to run 'octos init'; run it yourself, then re-run this script"
fi

# 3. provider credential -------------------------------------------------------
step "provider credential"
PROVIDER="$(python3 - "$HOME_DIR" <<'PY' 2>/dev/null || true
import json, os, sys
for p in (os.path.join(sys.argv[1], "config.json"), os.path.join(sys.argv[1], ".octos", "config.json")):
    try:
        print(json.load(open(p)).get("provider", "")); break
    except Exception:
        pass
PY
)"
if [ -n "$PROVIDER" ]; then
  if octos doctor 2>/dev/null | grep -qiE 'provider.*(configured|ok|\[✓\])' ; then
    ok "provider $PROVIDER"
  elif [ -t 0 ]; then
    echo "    signing in to $PROVIDER"
    octos auth login --provider "$PROVIDER" || fail "octos auth login failed"
  else
    echo "    provider $PROVIDER: sign in later with: octos auth login --provider $PROVIDER"
  fi
else
  echo "    no provider in config; 'octos init' sets one"
fi

# 4. the skill -----------------------------------------------------------------
step "oh-my-octos skill"
INSTALLED=0
if [ -n "$PROFILE" ]; then
  octos skills --profile "$PROFILE" install "$SOURCE" --force && INSTALLED=1
fi
if [ -n "$PROJECT" ]; then
  (cd "$PROJECT" && octos skills install "$SOURCE" --force) && INSTALLED=1
fi
if [ "$INSTALLED" = 0 ]; then
  echo "    no --profile or --project given; installing into the current directory for 'octos chat'"
  octos skills install "$SOURCE" --force && INSTALLED=1
fi
[ "$INSTALLED" = 1 ] || fail "skill install did not succeed"
ok "installed from $SOURCE"

# 5. doctor --------------------------------------------------------------------
step "octos doctor"
octos doctor || true

# 6. next ----------------------------------------------------------------------
step "next"
if [ "$START_SERVE" = 1 ]; then
  cat <<TXT
    octos serve --solo            # then open http://localhost:50080
    octos chat                    # terminal, in a project with the skill installed
    brew install octos-org/octoscode/octoscode   # optional terminal client
TXT
fi
