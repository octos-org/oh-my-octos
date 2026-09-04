# oh-my-octos

Curated defaults for [Octos](https://github.com/octos-org/octos) as a coding agent.
One skill, one install command, nothing to configure. The kernel is untouched.

```bash
# octos serve, octoscode, octoscode-web (installs into a profile)
octos skills --profile <profile-id> install octos-org/oh-my-octos

# octos chat in a single project
cd <project> && octos skills install octos-org/oh-my-octos
```

Then keep using Octos exactly as before. Remove with `octos skills remove oh-my-octos`.

## What you get

1. **Work discipline** (`prompts/discipline.md`). Nine rules appended to the system prompt: verify before claiming, report what happened, smallest diff, act on hook feedback, unique match text, stop when done.
2. **Project instructions every turn** (`hooks/project_context.py`). The repo-root `AGENTS.md` or `CLAUDE.md` is injected at the start of each turn when the workspace has no `.octos/AGENTS.md`, plus one line of git state.
3. **Deterministic edit checks** (`hooks/edit_check.py`). After every `write_file` / `edit_file` / `diff_edit`, the file is re-read from disk and checked: Python, JSON, TOML, shell and JavaScript syntax, merge-conflict markers, empty file. Problems come back to the model as `[hook] ...` in the tool result. Clean files produce nothing.
4. **Session budget guard** (`hooks/cost_guard.py`). Spend per session is recorded from Octos's own cost accounting; once it passes `OMO_SESSION_BUDGET_USD` (default 10) further model calls are denied with a message telling the agent to summarize and stop.

Everything is a prompt fragment or a stdlib Python hook. No new vocabulary, no config file, no daemon.

## Why these four

Each one either removes a whole class of failure or replaces "the model should remember to" with a check that always runs. Nothing gets added to this package without a measurement showing it moves a number (task pass rate, tokens per task, or failures avoided). See `tests/` for the end-to-end suite that gates every change.

## Optional packs

Nothing below is bundled. Each line is one existing `octos skills install` command; `install.sh` asks which ones you want when run in a terminal (or takes `--with slides,phonefarm`).

| Pack | Command | What it is |
|---|---|---|
| slides | `octos skills install mofa-org/mofa-skills/mofa-slides` | PPT decks with generated images (~11 MB). Needs `GEMINI_API_KEY` and the `mofa` CLI. |
| mofa | `octos skills install mofa-org/mofa-skills` | The whole mofa suite: slides, cards, comics, podcast, PDF, XLSX, site, and more (20 skills, ~540 MB). |
| phonefarm | `octos skills install BH3GEI/phonefarm/skills/phonefarm` | Android / OpenHarmony device automation, testing and telemetry. |

Add `--profile <id>` after `skills` for `octos serve` / octoscode; run it inside the project for `octos chat`. The agent knows this table too: ask it for slides and it will tell you the exact command if the pack is missing.

## For AI agents

Copy this to your agent (Claude Code, Codex, Cindy, octoscode, ...) to set up Octos with oh-my-octos:

```
Install Octos and oh-my-octos on this machine. Follow https://raw.githubusercontent.com/octos-org/oh-my-octos/main/install.sh step by step: check for an existing octos binary, run `octos init` if there is no config, sign in with `octos auth login --provider <name>`, install the skill with `octos skills --profile <id> install octos-org/oh-my-octos` (and plainly `octos skills install octos-org/oh-my-octos` inside my project for `octos chat`), ask me which optional packs I want (slides, mofa, phonefarm; see the Optional packs table in the README) and install those the same way, then run `octos doctor` and report what it says.
```

## Requirements

- Octos 2.0.3-rc.10 or newer (skill-declared hooks, `after_tool_call` feedback, `user_prompt_submit` context injection).
- `python3` on PATH. `node` is optional and only used for JavaScript syntax checks.
- macOS or Linux. The hooks are executable scripts with a shebang, which is how Octos resolves skill-relative hook commands; Windows is untested.

## Layout

```
oh-my-octos/
  SKILL.md               skill card and frontmatter
  manifest.json          prompt include + four hook registrations, no tools
  prompts/discipline.md  the rules
  hooks/*.py             the three hook scripts (stdlib only)
  install.sh             guided install path (existing octos commands in order, each verified)
  tests/                 unit tests for the hooks and the end-to-end suite
```

## Testing

```bash
python3 tests/test_hooks.py                       # hook scripts against synthetic payloads
OCTOS_BIN=/path/to/octos DEEPSEEK_API_KEY=... tests/e2e.sh   # real octos, isolated OCTOS_HOME
```

The e2e suite installs the skill into a throwaway home, runs `octos chat` and `octos serve --stdio`, and asserts each hook fired and each prompt fragment reached the model. It never touches `~/.octos`.

## License

Apache-2.0, same as Octos.
