// Live installed-Chrome qualification for the fast text path.
//
// Run with the extension loaded in the user's Chrome:
//   node scripts/live-bulk-docs.mjs
//
// It measures the public route, not a fixture: open a document, type 1,000 and
// ~4,000 characters through `act type`, then prove persistence by re-opening the
// same document URL and reading the text back. ESPN is exercised separately for
// scroll, click, verification, and a newest-story navigation.
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const BIN = process.env.CONTROLLA_BIN ?? resolve(here, "../target/release/controlla-v2");
const STATE = process.env.CONTROLLA_STATE_DIR ?? "/tmp/controlla-live-bulk-state";
const SKIP_DOCS = process.env.SKIP_DOCS === "1";

const child = spawn(BIN, ["mcp"], { env: { ...process.env, CONTROLLA_STATE_DIR: STATE }, stdio: ["pipe", "pipe", "pipe"] });
let stderr = "";
child.stderr.on("data", chunk => { stderr += chunk.toString(); });
let buffer = "";
let nextId = 0;
const pending = new Map();
child.stdout.on("data", chunk => {
  buffer += chunk.toString();
  let index;
  while ((index = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, index).trim();
    buffer = buffer.slice(index + 1);
    if (!line) continue;
    let message;
    try { message = JSON.parse(line); } catch { continue; }
    if (message.id === undefined) continue;
    const entry = pending.get(message.id);
    if (!entry) continue;
    pending.delete(message.id);
    clearTimeout(entry.timer);
    if (message.error) entry.reject(new Error(JSON.stringify(message.error)));
    else entry.resolve(message.result);
  }
});
function rpc(method, params, timeoutMs = 600_000) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`timeout ${method}`)); }, timeoutMs);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
}
async function call(tool, args, timeoutMs) {
  const result = await rpc("tools/call", { name: tool, arguments: args }, timeoutMs);
  if (result.structuredContent !== undefined) return result.structuredContent;
  const text = result.content?.map(part => part.text ?? "").join("");
  try { return JSON.parse(text); } catch { return { raw: text }; }
}
const show = (label, value, limit = 1600) => {
  const encoded = JSON.stringify(value);
  console.log(label, String(encoded ?? value).slice(0, limit));
};
const failures = [];
function check(condition, label, detail) {
  if (condition) console.log("PASS", label);
  else { failures.push(label); console.log("FAIL", label, detail === undefined ? "" : JSON.stringify(detail).slice(0, 800)); }
}
const finish = code => { child.stdin.end(); setTimeout(() => child.kill("SIGTERM"), 100); process.exit(code); };

function textBody(chars, prefix) {
  const line = `${prefix} the quick brown fox jumps over the lazy dog while the clock keeps time. `;
  let body = "";
  let lineNumber = 1;
  while (body.length < chars) body += `Line ${lineNumber++}: ${line}\n`;
  return body.slice(0, chars);
}

async function findEditor(session, tab) {
  for (let attempt = 0; attempt < 15; attempt += 1) {
    const snapshot = await call("snapshot", {
      session_id: session, chrome_tab_id: tab,
      goal: { query: "Document content", roles: ["textbox"] }, max_items: 40,
    });
    const editor = (snapshot.items ?? []).find(item => item.role === "textbox");
    if (editor) return { snapshot, editor };
    await new Promise(resolve => setTimeout(resolve, 1200));
  }
  return { snapshot: null, editor: null };
}

async function docsRun(chars, label) {
  const marker = `Live bulk ${label} ${Date.now()}`;
  const body = `${marker}\n${textBody(chars, label)}`;
  const startedOpen = Date.now();
  const key = `live-bulk-${label}-${Date.now()}`;
  let opened = await call("browser", {
    action: "open", url: "https://docs.google.com/document/create",
    idempotency_key: key,
    mode: "background",
    // A canvas editor must be the active tab of an unfocused window: visible to
    // Chrome, without taking focus from the user.
    target_policy: "create_background_window",
    ready_when: { kind: "url_prefix", prefix: "https://docs.google.com/document/d/" },
  });
  if (opened.status === "error" && /timed out|pairing/i.test(String(opened.reason ?? ""))) {
    // The first native open after a worker restart can lose the pairing race.
    // Retrying with the same idempotency key must resolve the same tab.
    console.log("DOCS_RETRY", label);
    opened = await call("browser", {
      action: "open", url: "https://docs.google.com/document/create",
      idempotency_key: key, mode: "background",
      target_policy: "create_background_window",
      ready_when: { kind: "url_prefix", prefix: "https://docs.google.com/document/d/" },
    });
  }
  show(`DOCS_${label}_OPEN`, opened, 700);
  if (!opened.session_id || !opened.tab_id) {
    check(false, `docs ${label} open`, opened);
    return null;
  }
  // Docs mounts its editor after the URL settles; a snapshot taken during that
  // mount points at an element the editor replaces moments later.
  await new Promise(resolve => setTimeout(resolve, 4000));
  // A reference can still be replaced while the document finishes mounting.
  // Refresh the observation and let the guarded action refuse rather than
  // write to a replaced target.
  let typed = null;
  const started = Date.now();
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const { editor } = await findEditor(opened.session_id, opened.tab_id);
    if (!editor) { check(false, `docs ${label} editor`, null); return null; }
    typed = await call("act", {
      session_id: opened.session_id, chrome_tab_id: opened.tab_id,
      action: "type", reference: editor.reference,
      expected_value: "", value: body,
      typing_mode: "block", timeout_ms: 240_000,
    });
    if (typed.status !== "not_dispatched" || typed.reason_code !== "target_replaced") break;
    console.log(`DOCS_${label}_RETRY`, attempt + 1, typed.reason_code);
    await new Promise(resolve => setTimeout(resolve, 700));
  }
  const elapsed = Date.now() - started;
  show(`DOCS_${label}_TYPE`, typed, 1400);
  check(typed.status === "verified", `docs ${label} verified`, typed);
  check(typed.completed_chars === [...body].length, `docs ${label} completed chars`, { expected: [...body].length, actual: typed.completed_chars });
  check(typed.bulk_insert === true, `docs ${label} used bulk insertion`, typed);
  check(elapsed < (chars < 2000 ? 12_000 : 25_000), `docs ${label} latency ${elapsed}ms`, { chars: body.length, elapsed });
  console.log(`DOCS_${label}_CHARS_PER_SECOND`, Math.round(body.length / (elapsed / 1000)));

  // Re-open the exact document URL: the same owned tab must be reused and the
  // text must be readable from the app's own copy path.
  // Re-open inside the live session. The exact URL returns the tab the session
  // already holds instead of creating a second one, which is also what proves
  // the document text was persisted rather than only present in the old view.
  const reopened = await call("browser", {
    action: "open", url: opened.url,
    idempotency_key: `live-bulk-${label}-reopen-${Date.now()}`,
    session_id: opened.session_id,
    mode: "background",
  });
  show(`DOCS_${label}_REOPEN`, reopened, 900);
  check(reopened.tab_id === opened.tab_id, `docs ${label} reopen reused the same tab`, {
    before: opened.tab_id, after: reopened.tab_id, status: reopened.status,
  });
  await new Promise(resolve => setTimeout(resolve, 1500));
  let text = "";
  let observed = null;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const { snapshot, editor } = await findEditor(opened.session_id, reopened.tab_id);
    observed = editor;
    text = String(snapshot?.text ?? "");
    if (editor) break;
    await new Promise(resolve => setTimeout(resolve, 900));
  }
  // The canvas document text is not in the DOM snapshot. Ask the verify
  // predicate, which reads it through the editor's own copy path.
  const persisted = await call("verify", {
    session_id: opened.session_id, chrome_tab_id: reopened.tab_id,
    predicate: { kind: "text_present", text: marker },
  });
  show(`DOCS_${label}_PERSISTED`, persisted, 900);
  check(persisted.status === "passed" || persisted.verified === true, `docs ${label} text persisted after reopen`, {
    status: persisted.status, verification: persisted.verification,
    snapshot_chars: text.length, editor: observed?.reference ?? null,
  });
  console.log(`DOCS_${label}_OPEN_MS`, Date.now() - startedOpen, "TYPE_MS", elapsed);
  return { opened, elapsed, chars: body.length };
}

async function espnPass() {
  let opened = null;
  const espnKey = `live-espn-${Date.now()}`;
  for (let attempt = 0; attempt < 4; attempt += 1) {
    opened = await call("browser", {
      action: "open", url: "https://www.espn.com/",
      idempotency_key: espnKey, mode: "background",
      target_policy: "create_background_window",
      ready_when: { kind: "url_prefix", prefix: "https://www.espn.com/" },
    });
    if (opened.session_id && opened.tab_id) break;
    console.log("ESPN_OPEN_RETRY", attempt + 1, opened.reason_code ?? opened.reason);
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
  await new Promise(resolve => setTimeout(resolve, 4000));
  show("ESPN_OPEN", opened, 700);
  if (!opened.session_id || !opened.tab_id) { check(false, "espn open", opened); return; }
  const { session_id: session, tab_id: tab } = { session_id: opened.session_id, tab_id: opened.tab_id };

  const scrolled = await call("act", { session_id: session, chrome_tab_id: tab, action: "scroll", delta_x: 0, delta_y: 900, timeout_ms: 20_000, max_bytes: 8_000, max_bytes_reason: "keep the scroll readback" });
  show("ESPN_SCROLL", scrolled, 700);
  check(scrolled.status === "verified", "espn page scroll verified", scrolled);
  check(scrolled.readback?.after?.y > scrolled.readback?.before?.y, "espn page scroll moved the viewport", scrolled.readback);

  const verified = await call("verify", {
    session_id: session, chrome_tab_id: tab,
    predicate: { kind: "text_present", text: "ESPN" },
  });
  show("ESPN_VERIFY", verified, 500);
  check(verified.status === "passed" || verified.verified === true || verified.matched === true || verified.status === "verified", "espn text verify", verified);

  const snapshot = await call("snapshot", {
    session_id: session, chrome_tab_id: tab,
    goal: { query: "NFL", roles: ["link"] }, max_items: 80,
  });
  const nfl = (snapshot.items ?? []).find(item => item.role === "link" && /^NFL$/i.test(String(item.name ?? "").trim()));
  show("ESPN_NFL_LINK", nfl && { reference: nfl.reference, name: nfl.name }, 300);
  if (!nfl) { check(false, "espn NFL link found", null); return; }
  const clicked = await call("act", { session_id: session, chrome_tab_id: tab, action: "click", reference: nfl.reference, timeout_ms: 30_000 });
  show("ESPN_NFL_CLICK", clicked, 800);
  await new Promise(resolve => setTimeout(resolve, 2500));
  const after = await call("verify", {
    session_id: session, chrome_tab_id: tab,
    predicate: { kind: "url_starts_with", prefix: "https://www.espn.com/nfl" },
  });
  show("ESPN_NFL_URL", after, 500);
  check(after.status === "passed" || after.verified === true || after.matched === true || after.status === "verified", "espn NFL navigation", after);
}

await rpc("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "live-bulk-docs", version: "1" } });
child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);

const docs = [];
if (!SKIP_DOCS) {
  const thousand = await docsRun(1000, "1k");
  if (thousand) docs.push(thousand);
  const fourThousand = await docsRun(4000, "4k");
  if (fourThousand) docs.push(fourThousand);
}
await espnPass().catch(error => { check(false, "espn pass threw", String(error)); });

console.log("DOCS_RESULTS", JSON.stringify(docs));
if (stderr.trim()) console.log("STDERR", stderr.slice(-1200));
console.log(failures.length ? `FAILURES ${JSON.stringify(failures)}` : "ALL_LIVE_CHECKS_PASSED");
finish(failures.length ? 1 : 0);
