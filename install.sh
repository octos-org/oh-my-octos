#!/usr/bin/env bash
# oh-my-octos guided install.
#
# Runs the existing octos commands in the right order and checks each step.
# It does not store keys, does not write config on its own, and does not
# replace `octos init`. Re-running is safe; every step is skipped when done.
#
# Usage:
#   bash install.sh [--profile <id>] [--project <dir>] [--source <skill source>]
#                   [--with slides,mofa,phonefarm] [--no-serve]
#
# Optional packs (each is one existing `octos skills install` command; nothing is bundled):
#   slides     PPT decks via mofa-slides           (mofa-org/mofa-skills/mofa-slides, ~11 MB; needs GEMINI_API_KEY and the `mofa` CLI)
#   mofa       the whole mofa suite, 20 skills     (mofa-org/mofa-skills, ~540 MB, a few minutes)
#   phonefarm  Android/OpenHarmony device automation (BH3GEI/phonefarm/skills/phonefarm)
# In a terminal the script asks which packs you want; with --with or without a terminal it does not ask.
#
# Environment:
#   OMO_SKIP_BINARY_INSTALL=1   do not try to install octos when it is missing

set -euo pipefail

SOURCE="octos-org/oh-my-octos"
PROFILE=""
PROJECT=""
START_SERVE=1
WITH=""
WITH_GIVEN=0

pack_source() {
  case "$1" in
    slides)    echo "mofa-org/mofa-skills/mofa-slides" ;;
    mofa)      echo "mofa-org/mofa-skills" ;;
    phonefarm) echo "BH3GEI/phonefarm/skills/phonefarm" ;;
    *) return 1 ;;
  esac
}
pack_blurb() {
  case "$1" in
    slides)    echo "PPT decks (mofa-slides, ~11 MB; needs GEMINI_API_KEY and the mofa CLI)" ;;
    mofa)      echo "the whole mofa suite: slides, cards, comics, podcast, pdf, xlsx... (20 skills, ~540 MB)" ;;
    phonefarm) echo "Android / OpenHarmony device automation and testing" ;;
  esac
}

while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --project) PROJECT="$2"; shift 2 ;;
    --source) SOURCE="$2"; shift 2 ;;
    --no-serve) START_SERVE=0; shift ;;
    --with) WITH="$2"; WITH_GIVEN=1; shift 2 ;;
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

# 4b. optional packs -----------------------------------------------------------
step "optional packs"
if [ "$WITH_GIVEN" = 0 ] && [ -t 0 ]; then
  echo "    Also install? Enter numbers separated by spaces, or press Enter to skip."
  echo "      1) slides     $(pack_blurb slides)"
  echo "      2) mofa       $(pack_blurb mofa)"
  echo "      3) phonefarm  $(pack_blurb phonefarm)"
  printf '    > '
  read -r CHOICE || CHOICE=""
  for n in $CHOICE; do
    case "$n" in
      1|slides) WITH="$WITH,slides" ;;
      2|mofa) WITH="$WITH,mofa" ;;
      3|phonefarm) WITH="$WITH,phonefarm" ;;
      *) echo "    ignoring '$n'" ;;
    esac
  done
fi
WITH="$(printf '%s' "$WITH" | tr ',' '\n' | sed '/^$/d' | sort -u | tr '\n' ' ')"
if [ -z "$WITH" ]; then
  ok "none (add later with: octos skills install <source>, sources listed in README.md)"
fi
for pack in $WITH; do
  src="$(pack_source "$pack")" || { echo "    unknown pack '$pack' (known: slides mofa phonefarm)"; continue; }
  echo "    $pack: octos skills install $src"
  if [ -n "$PROFILE" ]; then
    octos skills --profile "$PROFILE" install "$src" --force || echo "    $pack: install failed (see above)"
  fi
  if [ -n "$PROJECT" ]; then
    (cd "$PROJECT" && octos skills install "$src" --force) || echo "    $pack: install failed (see above)"
  fi
  if [ -z "$PROFILE" ] && [ -z "$PROJECT" ]; then
    octos skills install "$src" --force || echo "    $pack: install failed (see above)"
  fi
  case "$pack" in
    slides|mofa)
      [ -n "${GEMINI_API_KEY:-}" ] || echo "    note: GEMINI_API_KEY is not set; mofa skills need it (export it, or add it to the profile env)"
      if ! command -v mofa >/dev/null 2>&1; then
        # The skill's downloaded `main` is the mofa CLI itself; SKILL.md declares `requires_bins: mofa`.
        for d in "$PROJECT" "$PWD"; do
          [ -n "$d" ] && [ -x "$d/.octos/skills/mofa-slides/main" ] && { echo "    note: put the mofa CLI on PATH, e.g.: mkdir -p ~/.local/bin && ln -sf \"$d/.octos/skills/mofa-slides/main\" ~/.local/bin/mofa"; break; }
        done
        command -v mofa >/dev/null 2>&1 || echo "    note: mofa CLI not on PATH (the mofa-slides skill ships it as <skills dir>/mofa-slides/main; symlink it as 'mofa')"
      fi
      ;;
    phonefarm)
      command -v adb >/dev/null 2>&1 || command -v hdc >/dev/null 2>&1 || echo "    note: neither adb nor hdc is on PATH; phonefarm needs one of them to talk to a device"
      ;;
  esac
done

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
