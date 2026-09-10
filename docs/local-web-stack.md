# Local web stack: one entry, everything behind it

This is a reference deployment for running the whole Octos local web surface as
a small, restartable, auto-started stack on a single machine. It was distilled
from a real macOS arm64 setup and is intentionally template-first: copy the
files under `examples/local-web-stack/`, replace the placeholders, and keep the
secrets out of source control.

The stack serves these at one origin, `http://localhost:8080`:

```text
/launch/        launcher + living documentation for the whole machine
/app/           octos-web (embedded in octos serve)
/admin/         octos admin dashboard (embedded)
/code/          octoscode-web (production build)
/learn/         octos-learn (production build)
/matrix-chat/   one-click local Matrix room (auto-login client)
/_matrix/*      reverse proxy to a local Synapse homeserver
/api/*          reverse proxy to octos serve (REST + WebSocket)
```

One Caddy process is the only public listener; it is bound to loopback.
`octos serve`, Synapse, and ngrok (optional, for Lark callbacks) are supervised
by launchd with `RunAtLoad` and `KeepAlive`, so they survive reboots and crash
loops.

## Prerequisites

- Node.js 22+, pnpm 11.5.2
- Caddy 2.x (`brew install caddy`)
- Rust toolchain only if you rebuild octos/synapse from source
- macOS launchd (Linux systemd equivalents are straightforward translations)

## 1. Install octos and build the web clients

```sh
npm install -g @octos-org/octos @octos-org/octoscode @octos-org/octos-tui
git clone https://github.com/octos-org/octoscode-web ~/octos-clients/octoscode-web
git clone https://github.com/octos-org/octos-learn ~/octos-clients/octos-learn

cd ~/octos-clients/octoscode-web
pnpm install --frozen-lockfile
OCTOSCODE_WEB_BASE_PATH=/code/ pnpm build

cd ~/octos-clients/octos-learn
pnpm install --frozen-lockfile
BASE_URL=/learn/ pnpm build:public
```

`octos-web` does not need a build: a production copy ships inside `octos serve`
at `/app/` (and `/admin/`).

## 2. Caddy

Copy `examples/local-web-stack/Caddyfile` to the brew-managed
`/opt/homebrew/etc/Caddyfile` and adapt the roots. Two details matter:

- Strip `Origin` and `X-Forwarded-*` upstream. Octos 2.0.2 has a hardcoded WS
  Origin allowlist (no config knob yet), so a browser origin such as
  `http://localhost:8080` is rejected at `/api/ui-protocol/ws`. Stripping the
  header at the trusted local proxy is the supported workaround; token auth is
  still enforced. Newer builds accept `OCTOS_APPUI_ALLOWED_ORIGINS`, at which
  point the header deletions can be dropped.
- Bind to loopback. The config binds `127.0.0.1 ::1`; do not expose this to a
  LAN unless you first enable Lark webhook signing and admin auth everywhere.

Validate and start:

```sh
caddy validate --config /opt/homebrew/etc/Caddyfile
brew services start caddy
```

## 3. octos serve under launchd

Install `examples/local-web-stack/plists/octos.serve.plist` as
`~/Library/LaunchAgents/local.octos.serve.plist`. Notes:

- launchd has no shell `PATH`; reference the real `node` binary and the
  `octos.js` script by absolute path, and export a `PATH` in the plist so
  octos can spawn its own children.
- Start with `--solo` for a local single-user box. Add a `--auth-token`
  bootstrap token, log in once as admin, and rotate it to an 8-character
  persistent token (the dashboard wizard enforces the length).
- `launchctl bootstrap gui/$(id -u) <plist>` registers it; `KeepAlive` makes it
  restart on crash. A kill test is part of the e2e checklist below.

## 4. Launcher page

Copy `examples/local-web-stack/launch/index.html` to `~/octos-clients/launch/`
and fill in the placeholders. The page is deliberately self-contained: entry
cards, ports, model/key status, channel status, login behavior, ops commands,
update instructions, and troubleshooting all live in the one file. The code
card seeds `octoscode-web`'s tab storage with a fresh solo token so Connect is
one click; the admin card seeds `octos_auth_token` for auto-login.

## 5. Local Matrix

The `examples/local-web-stack/matrix-chat/index.html` template is a tiny
same-origin client that auto-logs into a room and live-syncs it. The
`/_matrix/*` Caddy route makes it same-origin, so no CORS work is needed.
Copy it to `~/octos-clients/matrix-chat/` and fill in the three placeholders:
`REPLACE_WITH_MATRIX_USER` is the **bare localpart** (`octos`, not
`@octos:localhost` — senders are compared after the server part is stripped),
plus the account password and the room id.

Server side, a Synapse in a venv is the least moving part:

```sh
python3.12 -m venv ~/octos-matrix/venv
~/octos-matrix/venv/bin/pip install 'setuptools<81' matrix-synapse
~/octos-matrix/venv/bin/python -m synapse.app.homeserver \
  --server-name localhost --config-path ~/octos-matrix/homeserver.yaml \
  --generate-config --report-stats=no
```

Set the listener to `127.0.0.1`, point sqlite/media/logs into
`~/octos-matrix/`, keep `registration_shared_secret` for the register CLI, then
install the synapse plist template. Register the bot and a human user, log the
bot in through `/_matrix/client/v3/login`, create a room, and put the resulting
room id and access token into the octos profile channel:

```json
{
  "type": "matrix",
  "homeserver": "http://127.0.0.1:8008",
  "mode": "user",
  "user_id": "@octos:localhost",
  "access_token": "syt_...",
  "device_name": "octos",
  "rooms": ["!room:localhost"],
  "auto_join": "allowlist",
  "auto_join_allowlist": ["!room:localhost"],
  "group_policy": "allowlist",
  "require_mention": false
}
```

In octos 2.0.2 profile channels are typed structs, so the fields sit at the top
level of the channel object (no `settings` wrapper), and Lark international is
selected with `"type": "feishu", "region": "global", "mode": "webhook"`.

## 6. Channels and credentials

- Provider keys: `octos auth set-key <NAME> <VALUE>` stores them in the macOS
  keychain; the profile `env_vars` keeps `"keychain:"` references rather than
  plaintext.
- Telegram: `{"type":"telegram","token_env":"TELEGRAM_BOT_TOKEN"}`; the token
  must be in the serve process environment.
- Lark webhook: run ngrok from launchd (`examples/local-web-stack/plists/ngrok.plist`),
  point it at the Lark callback URL, then finish the console steps (bot
  capability, `im.message.receive_v1`, message permissions, publish).

## 7. End-to-end check

`examples/local-web-stack/e2e.mjs` drives real Chromium with Playwright and
verifies every entry, not just HTTP status codes: launcher cards, `/app` solo
login, `/admin` auto-login, `/code` connect + workspace open, `/learn`
whiteboard, and a Matrix send/reply round trip. Keep it outside the repo so
its `artifacts/` screenshots do not dirty the checkout, and adapt the
machine-specific constants at the top (`WORKSPACE`, `EXPECTED_MODEL`,
`MATRIX_ROOM_MARKER`, `MATRIX_HISTORY_MARKER`). Run it after any change:

```sh
mkdir -p ~/octos-clients/e2e
cp examples/local-web-stack/e2e.mjs ~/octos-clients/e2e/
cd ~/octos-clients/e2e && npm i -D playwright && npx playwright install chromium
node e2e.mjs
```

The script exits non-zero when any flow reports a page error, console error,
or unexpected 404, so it can gate a deploy or a launchd watchdog. One selector
is a CSS-modules hash from the octoscode-web build (`button._addButton_*`);
it changes on every frontend rebuild — the inline comment in e2e.mjs explains
how to refresh it.

Also kill the octos process once and confirm launchd restarts it and `/app/`
recovers, since keep-alive is part of the deployment contract.

## Gotchas collected on the way

- `pnpm dev` is for development; production serves `dist/` statically.
- Vite SPAs need their base baked at build time: `/code/` for octoscode-web,
  `/learn/` for octos-learn, otherwise assets 404 behind a path prefix.
- Synapse >= 1.104 wants a Rust extension; a crates.io mirror makes the build
  tractable, and `setuptools<81` restores `pkg_resources` on Python 3.12.
- The 2.0.2 channel enum has `feishu`, not `lark`; an unknown channel type
  makes the whole profile fail to parse, so validate `profiles/*.json` after
  edits.
