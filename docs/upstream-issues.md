# Findings worth reporting to octos-org/octos (verified on v2.0.3-rc.10)

Drafts, one per issue. Each has a reproduction the maintainer can run without this repo.

## 1. `on_turn_end` hook event is documented but never fired

`book/src/advanced.md` lists `on_turn_end` among the ten lifecycle events and `HookPayload::on_turn_end` exists, but no runtime path calls `hooks.run(HookEvent::OnTurnEnd, ..)` (grep over `crates/` at rc.10 and `main`). A hook registered on it never runs and there is no warning.

Repro: register `{"event":"on_turn_end","command":["sh","-c","echo hi >> /tmp/turn_end"]}` in a profile, run any turn, `/tmp/turn_end` stays absent.

Suggested: fire it at the end of `run_conversation` with `turn_summary`, or drop it from the docs until it exists.

## 2. Skill-manifest hooks: only `command[0]` is resolved against the skill directory

`plugins/extras.rs::resolve_hook` joins `skill_dir` onto `command[0]` when it starts with `./`, and leaves every later argv element untouched. The common shape `["python3", "./hooks/x.py"]` therefore resolves the script against the workspace cwd and fails on every call ("can't open file"), which under `after_tool_call` is even fed back to the model as `[hook]` output. The docs say "Relative paths resolved against skill directory" without the argv[0] caveat.

Suggested: resolve every argv element that starts with `./` or `../`, or document that the script must be argv[0].

## 3. `session_id` / `profile_id` missing from hook payloads on `octos chat` and `octos serve`

`HookContext` is only populated on some paths. On rc.10 both `octos chat` and `octos serve --stdio --solo` deliver `before_llm_call` / `after_llm_call` / `after_tool_call` payloads without `session_id` or `profile_id` (verified by a dump hook), while the serve log line for the same call does print `session=…`. Any per-session policy (budget, rate limit, audit) has to fall back to the parent pid, which under `serve` means one bucket for the whole server.

Suggested: set the hook context when the session actor builds the agent, on every path.

## 4. `octos skills list` prints its table to stderr

`octos skills list 2>/dev/null` prints nothing; `skills info` uses stdout. Scripts that pipe `skills list` get an empty result.

## 5. Unix socket under the data dir limits `--data-dir` path length

`octos serve` creates `<data_dir>/.octos-goal-control.sock`; when the data dir path is long (~100+ bytes) startup fails with `path must be shorter than SUN_LEN` from `commands/goal.rs`. The error does not say which path or how to fix it.

Suggested: fall back to a socket in `$TMPDIR` (or `XDG_RUNTIME_DIR`) when the data-dir path is too long, and name the path in the error.

## 6. A `before_llm_call` deny is logged as an internal harness error

`harness error classified variant="internal" recovery=bug error=LLM call denied by hook: …`. A policy deny is expected behaviour, not a bug; the classification pollutes operator dashboards.

## 7. `octos skills install user/repo/skill` only looks at the repo root

`user/repo/name` resolves `<clone>/name`; a skill kept under `skills/name` (the layout `octos skills` itself creates in projects) has to be addressed as `user/repo/skills/name`, which the docs do not mention.
