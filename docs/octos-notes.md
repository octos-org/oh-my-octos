# Notes on Octos behaviour this package depends on (verified against 2.0.3-rc.10)

Kept here so the next person does not have to re-read `octos-agent` to change a hook.

## Hook mechanics that matter

- Skill-declared hooks (`manifest.json` `hooks[]`) support `event`, `command`, `timeout_ms`, `tool_filter`. `path_filter` and `requires_bin` exist only for hooks written in profile/config JSON. Filter by path inside the script.
- Only `command[0]` is resolved against the skill directory (when it starts with `./`). Any later argv element is left as-is and resolves against the workspace. So the script itself must be `command[0]`, executable, with a shebang. `["python3", "./hooks/x.py"]` silently runs nothing useful.
- Exit codes: before-events (`user_prompt_submit`, `before_tool_call`, `before_llm_call`, `before_spawn_verify`) exit 1 = deny with stdout as the reason; `before_tool_call` exit 2 = replace the tool arguments with the JSON on stdout. After-events: any nonzero exit *with* output = feedback, appended to the tool result as `[hook] <argv>:\n<output>` (capped at 2000 bytes); nonzero with no output = infrastructure error, counts toward the 3-strike breaker.
- `user_prompt_submit` exit 0 + stdout = context prepended to the model input for that turn only.
- `on_turn_end` is documented but never fired by the runtime (no call site in rc.10 or main). Do not register anything on it.
- `write_file`, `read_file`, `shell` are "sensitive": their `arguments` in the payload are redacted down to the path keys, and `result` is `[redacted: sensitive tool output]`. Read the file from disk in the hook instead. `edit_file` / `diff_edit` arguments are visible but truncated at 1 KiB.
- The hook child runs with cwd = workspace root and the parent's environment minus API-key-looking names and the injection denylist (`LD_PRELOAD`, `DYLD_*`, `PYTHONPATH`, ...). `PATH`, `HOME`, `TMPDIR` survive. A budget variable must not contain `KEY`, `TOKEN`, `SECRET` or `PASSWORD` or it is stripped.
- `session_id` / `profile_id` are absent from the `before_llm_call` / `after_llm_call` / `after_tool_call` payloads on both `octos chat` and `octos serve` (rc.10; the serve log line shows the session, the payload does not). The cost guard therefore keys its state on the parent pid: one bucket per `octos chat` process, one bucket per `octos serve` process. Worth an upstream fix: `HookContext` is only populated on some paths.
- `session_cost` in `after_llm_call` is cumulative for the session (verified: 0.0040 → 0.0080 over two iterations).
- `after_tool_call` hooks are debounced per session for the built-in coding checkers' sake; an identical edit within the window can be coalesced.
- A `before_llm_call` deny ends the turn with an error (`octos chat --json` prints `{"error": "LLM call denied by hook: ..."}` and exits 1). The runtime classifies it as an internal harness error in its log; that is cosmetic.

## Where skills load from

- `octos chat`: `<cwd>/.octos/skills`, `<cwd>/.octos/plugins`, `OCTOS_SKILLS_PATH`. Not the profile directory, not `~/.octos/skills` (deprecated, warns once).
- `octos serve` / gateway (octoscode, octoscode-web, the arc-bench adapter): `<profile data dir>/skills`, i.e. `octos skills --profile <id> install ...`.
- `octos chat` does not run the `SkillsLoader`, so `always: true` in `SKILL.md` has no effect there. `prompts.include` fragments from the manifest are injected on both paths; that is why the discipline text is a prompt fragment and not the SKILL.md body.
- `octos serve` merges the built-in coding checkers (`cargo check`, `eslint`, `ruff`, gated on the binary being present) into the same executor; `octos chat` does not. `edit_check.py` therefore overlaps with those only under `serve`, and only for `.rs` / `.js` / `.py` when the tools are installed.

## Operational

- The serve data dir (`--data-dir` / `OCTOS_HOME`) must be a short path: it hosts a unix socket and macOS caps socket paths at 104 bytes.
- `octos skills install <local path>` copies the whole directory, skipping `.git`. Keep work dirs and caches out of the checkout.
- Serve logs land in `<data dir>/logs/serve.<date>.log`; `octos chat -v` prints the same lines to stderr.
