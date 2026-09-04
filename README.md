# oh-my-octos

Curated defaults for [Octos](https://github.com/octos-org/octos) as a coding agent.
One skill, one Rust binary, one install command, nothing to configure. The kernel is untouched.

```bash
# octos serve, octoscode, octoscode-web (installs into a profile)
octos skills --profile <profile-id> install octos-org/oh-my-octos

# octos chat in a single project
cd <project> && octos skills install octos-org/oh-my-octos
```

Then keep using Octos exactly as before. Remove with `octos skills remove oh-my-octos`.

Octos fetches the prebuilt `oh-my-octos` binary for your platform from the GitHub release (macOS arm64, Linux x86_64 and arm64); with no match it runs `cargo build --release` in place. There are no scripts and no runtime dependencies. `python3` and `node` are used only if present, for Python and JavaScript syntax checks.

## What you get

1. **Work discipline** (`prompts/discipline.md`). Nine rules appended to the system prompt: verify before claiming, report what happened, smallest diff, act on hook feedback, unique match text, stop when done.
2. **Project instructions every turn** (`hook project-context`). The repo-root `AGENTS.md` or `CLAUDE.md` is injected at the start of each turn when the workspace has no `.octos/AGENTS.md`, plus one line of git state. Git gets a 2.5 s budget so a huge repo can never stall a turn.
3. **Deterministic edit checks** (`hook edit-check`). After every `write_file` / `edit_file` / `diff_edit`, the file is re-read from disk and checked: Python, JSON (comments and trailing commas tolerated), TOML, shell (by shebang) and JavaScript syntax, merge-conflict markers, empty file. Problems come back to the model as `[hook] ...` in the tool result. Clean files produce nothing.
4. **Session budget guard** (`hook cost-guard`). Spend is recorded from Octos's own cost accounting; once it passes `OMO_SESSION_BUDGET_USD` (default 10) further model calls are denied with a message telling the agent to summarize and stop. One bucket per `octos chat` process; under `octos serve` one bucket per server process, because Octos does not yet pass the session id to LLM hooks (upstream [#2246](https://github.com/octos-org/octos/issues/2246)). A new session in the same process resets the bucket; spend idle for four hours is forgotten.

## Guided setup

For a machine that has nothing yet, the same binary walks the whole path in order and checks each step: octos binary, config, provider sign-in, this skill, optional packs, the [octoscode](https://github.com/octos-org/octoscode) terminal client, `octos doctor`. It stores no keys and does not replace `octos init`.

```bash
# get the binary (pick one)
cargo install --git https://github.com/octos-org/oh-my-octos          # with a Rust toolchain
curl -fsSL https://github.com/octos-org/oh-my-octos/releases/latest/download/oh-my-octos-darwin-aarch64.tar.gz | tar xz   # prebuilt

oh-my-octos setup                                   # asks which optional packs you want
oh-my-octos setup --profile <id> --with slides      # no questions
```

## Optional packs

Nothing below is bundled. Each line is one existing `octos skills install` command; `oh-my-octos setup` asks which ones you want when run in a terminal (or takes `--with slides,phonefarm`).

| Pack | Command | What it is |
|---|---|---|
| slides | `octos skills install mofa-org/mofa-skills/mofa-slides` | PPT decks with generated images (~11 MB). Needs `GEMINI_API_KEY` and the `mofa` CLI (the skill ships it as `mofa-slides/main`; `setup` prints the symlink line). |
| mofa | `octos skills install mofa-org/mofa-skills` | The whole mofa suite: slides, cards, comics, podcast, PDF, XLSX, site, and more (20 skills, ~540 MB). |
| phonefarm | `octos skills install BH3GEI/phonefarm/skills/phonefarm` | Android / OpenHarmony device automation, testing and telemetry. |

Add `--profile <id>` after `skills` for `octos serve` / octoscode; run it inside the project for `octos chat`. The agent knows this table too: ask it for slides and it will tell you the exact command if the pack is missing.

## For AI agents

Copy this to your agent (Claude Code, Codex, Cindy, octoscode, ...) to set up Octos with oh-my-octos:

```
Install Octos and oh-my-octos on this machine. Read https://github.com/octos-org/oh-my-octos and follow the steps `oh-my-octos setup` performs, using the octos commands directly: check for an existing octos binary, run `octos init` if there is no config, sign in with `octos auth login --provider <name>`, install the skill with `octos skills --profile <id> install octos-org/oh-my-octos` (and plainly `octos skills install octos-org/oh-my-octos` inside my project for `octos chat`), ask me which optional packs I want (slides, mofa, phonefarm; see the Optional packs table) and install those the same way, install octoscode, then run `octos doctor` and report what it says.
```

## Measured

`cargo run --example bench -- --octos <bin>`, 2026-09-04, octos 2.0.3-rc.10, deepseek-v4-flash, six small graded coding tasks, two repeats each, same binary and flags in both arms. Medians per run:

| | bare octos | with oh-my-octos |
|---|---|---|
| tasks passed | 12 / 12 | 12 / 12 |
| model calls per task | 5 | 5 |
| context tokens per call | 5,640 | 6,178 (+10%) |
| wall time per task | 8.5 s | 8.4 s |
| prefix cache hit rate | 97% | 96% |

Reading: on tasks this easy the pass rate cannot move, so the honest number is the cost. The fixed overhead is about 540 tokens per call (the discipline text, the skill card, the per-turn project context). The hooks themselves are free when the file is clean. One run in the oh-my-octos arm took 15 calls instead of 7 because the discipline text made the model test its shell script under two shells and fix a real bug it found; that is the intended behaviour and it is the reason the arc-bench score, not this table, is the real gate. Results land in `tests/bench/results.json`.

## Requirements

- Octos 2.0.3-rc.10 or newer (skill-declared hooks, `after_tool_call` feedback, `user_prompt_submit` context injection).
- macOS arm64 or Linux (prebuilt), or any platform with a Rust toolchain. Windows is untested.

## Layout

```
oh-my-octos/
  SKILL.md               skill card and frontmatter
  manifest.json          prompt include, four hook registrations (`./main hook <name>`), prebuilt binary URLs, no tools
  prompts/discipline.md  the rules
  src/                   the binary: hooks/{cost_guard,edit_check,project_context}.rs, setup.rs
  tests/hooks.rs         hook contract tests against the built binary (no octos needed)
  tests/e2e.rs           real octos + octoscode, skipped without OCTOS_BIN and a provider key
  examples/bench.rs      the benchmark
  docs/                  octos behaviour notes, upstream issue drafts
```

## Testing

```bash
cargo test                                      # unit + hook contract tests, offline
OCTOS_BIN=/path/to/octos DEEPSEEK_API_KEY=... cargo test --test e2e -- --test-threads=1
OCTOSCODE_BIN=/path/to/octoscode ...             # adds the real-TUI case
```

The e2e suite installs the skill into a throwaway home and asserts each hook fired and each prompt fragment reached the model, using octos's own log lines as evidence rather than the model's wording. It covers `octos chat`, `octos serve --stdio`, the real octoscode TUI driven through a pseudo-terminal, the guided setup (non-interactive, `--with`, and the interactive menu in a pty), installing from this GitHub repository, and two realistic sessions: a five-turn create/test/break/fix/verify session under `serve`, and a bug fix in an existing git repo under `chat`. It never touches `~/.octos`.

## License

Apache-2.0, same as Octos.
