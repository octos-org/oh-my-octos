---
name: oh-my-octos
description: Curated defaults for Octos as a coding agent - work discipline prompt, deterministic edit checks, project-instruction injection, and a session budget guard. Installs as one skill; nothing to configure.
version: 0.1.0
author: octos-org
always: false
requires_bins: python3
---

# oh-my-octos

One skill that ships the judgment calls most Octos users end up making by hand.
Install it once, keep using `octos chat`, `octoscode`, or `octos serve` exactly as before.

## What it adds

| Piece | Mechanism | Effect |
|---|---|---|
| `prompts/discipline.md` | prompt fragment (`prompts.include`) | Nine work rules appended to the system prompt. Verify before claiming, act on hook feedback, smallest diff, and so on. |
| `hooks/project_context.py` | `user_prompt_submit` | Injects the repo-root `AGENTS.md` or `CLAUDE.md` (when `.octos/AGENTS.md` is absent) plus a one-line git summary at the start of every turn. |
| `hooks/edit_check.py` | `after_tool_call` on `write_file`, `edit_file`, `diff_edit` | Re-reads the edited file from disk and reports syntax errors (py, json, toml, sh, js) and conflict markers back to the model as `[hook]` feedback. Silent when clean. |
| `hooks/cost_guard.py` | `after_llm_call` + `before_llm_call` | Records session spend; denies further model calls once it passes `OMO_SESSION_BUDGET_USD` (default 10). |

## Install

```bash
# for octos serve / octoscode / octoscode-web (per profile)
octos skills --profile <profile-id> install octos-org/oh-my-octos

# for octos chat in one project
cd <project> && octos skills install octos-org/oh-my-octos
```

Remove with `octos skills remove oh-my-octos`. Everything lives in that one directory.

## Notes for agents reading this file

The hooks are deterministic checks, not suggestions. A tool result that ends with `[hook] ...` lists real problems in the file you just wrote; fix them before continuing.
