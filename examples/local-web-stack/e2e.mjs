import assert from "node:assert/strict";
import { mkdirSync } from "node:fs";
import { chromium } from "playwright";

// Machine-specific values. The Matrix room must already contain an earlier
// test message, and the octos profile must be configured with EXPECTED_MODEL.
const BASE = "http://localhost:8080";
const WORKSPACE = "/Users/YOU/octos-clients";
const EXPECTED_MODEL = "deepseek-v4-flash";
const MATRIX_ROOM_MARKER = "REPLACE_WITH_ROOM_ID";
const MATRIX_HISTORY_MARKER = "hello octos";

const ARTIFACTS = new URL("./artifacts/", import.meta.url).pathname;
mkdirSync(ARTIFACTS, { recursive: true });

const browser = await chromium.launch();
const results = [];

async function page() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  return context.newPage();
}

function track(p, name, ignore404 = []) {
  const errors = [];
  p.on("pageerror", (e) => errors.push("pageerror: " + String(e).slice(0, 300)));
  p.on("console", (m) => m.type() === "error" && errors.push("console: " + m.text().slice(0, 300)));
  p.on("response", (r) => r.status() === 404 && !ignore404.some((u) => r.url().includes(u)) && errors.push("404: " + r.url().replace(BASE, "")));
  p.name = name; p.errors = errors;
}

async function report(p) {
  await p.screenshot({ path: `${ARTIFACTS}${p.name}.png`, fullPage: true });
  results.push({ name: p.name, errors: p.errors });
  console.log(`[${p.name}] ${p.errors.length === 0 ? "PASS" : "WARN (" + p.errors.length + ")"}`);
  p.errors.forEach((e) => console.log("  " + e));
}

// launcher
{
  const p = await page();
  track(p, "launch");
  await p.goto(`${BASE}/launch/`, { waitUntil: "domcontentloaded" });
  await p.waitForFunction(() => !document.getElementById("status-text").textContent.includes("checking"));
  assert.equal(await p.locator("a.card[data-target]").count(), 5);
  await report(p); await p.close();
}

// /app solo login
{
  const p = await page();
  track(p, "app");
  await p.goto(`${BASE}/app/login`, { waitUntil: "domcontentloaded" });
  await p.getByRole("button", { name: "Continue without a password" }).click();
  await p.getByText("Octos Home", { exact: true }).first().waitFor({ timeout: 20000 });
  await report(p); await p.close();
}

// /admin auto-login (launcher seeds the token)
{
  const p = await page();
  track(p, "admin");
  await p.goto(`${BASE}/launch/`, { waitUntil: "domcontentloaded" });
  await p.locator('a[data-target="admin"]').click();
  await p.getByText("All Profiles", { exact: true }).first().waitFor({ timeout: 20000 });
  assert.ok(await p.evaluate(() => localStorage.getItem("octos_auth_token")));
  await report(p); await p.close();
}

// /code connect + workspace open
{
  const p = await page();
  track(p, "code");
  await p.goto(`${BASE}/launch/`, { waitUntil: "domcontentloaded" });
  await p.locator('a[data-target="code"]').click();
  await p.waitForURL(/\/code\//);
  assert.equal(await p.getByPlaceholder("https://octos.example.com").inputValue(), BASE);
  await p.getByRole("button", { name: "Connect", exact: true }).click();
  await p.getByText("Choose a workspace", { exact: true }).first().waitFor({ timeout: 20000 });
  // NOTE: `_addButton_3mdup_357` is a CSS-modules hash from the octoscode-web
  // build — it changes every time the frontend is rebuilt. If this step fails
  // right after an update, inspect the "add workspace" button in DevTools and
  // paste the new hashed class here (or give the button a stable data-testid
  // upstream).
  await p.locator("button._addButton_3mdup_357").click();
  await p.getByPlaceholder("/srv/projects/octoscode").fill(WORKSPACE);
  await p.getByRole("button", { name: "Add & Start" }).click();
  await p.waitForFunction(() => document.body.innerText.includes("CODING WORKSPACE"), null, { timeout: 60000 });
  await p.waitForFunction(() => document.body.innerText.includes(EXPECTED_MODEL), null, { timeout: 20000 });
  await report(p); await p.close();
}

// /learn solo login + text-only whiteboard
{
  const p = await page();
  track(p, "learn", ["/api/my/profile/skills", "/api/learn/tts/status"]);
  await p.goto(`${BASE}/learn/login`, { waitUntil: "domcontentloaded" });
  await p.getByRole("button", { name: "Continue without a password" }).click();
  await p.getByText("启用小章鱼学习助手", { exact: true }).waitFor({ timeout: 20000 });
  await p.getByRole("button", { name: "仅用文字" }).click();
  await p.waitForFunction(() => document.body.innerText.includes("OCTOS LEARNING CANVAS"), null, { timeout: 30000 });
  await report(p); await p.close();
}

// /matrix-chat auto-login + live send/reply
{
  const p = await page();
  track(p, "matrix");
  await p.goto(`${BASE}/matrix-chat/`, { waitUntil: "domcontentloaded" });
  await p.waitForFunction(() => document.getElementById("status").textContent.includes("connected"), null, { timeout: 20000 });
  assert.ok((await p.locator("#messages").innerText()).includes(MATRIX_HISTORY_MARKER));
  await p.locator("#input").fill("e2e: reply with OK");
  await p.locator("#input").press("Enter");
  await p.waitForFunction(() => document.getElementById("messages").innerText.includes("OK"), null, { timeout: 90000 });
  await report(p); await p.close();
}

await browser.close();
console.log("\nsummary:", results.map((r) => `${r.name}:${r.errors.length === 0 ? "PASS" : "WARN"}`).join(" "));

// A check that never fails is not a check: any tracked error (pageerror,
// console error, unexpected 404) fails the run so this can gate deploys.
const warned = results.filter((r) => r.errors.length > 0);
if (warned.length > 0) {
  console.error(`\nFAIL: ${warned.length}/${results.length} flows reported errors: ${warned.map((r) => r.name).join(", ")}`);
  process.exitCode = 1;
}
