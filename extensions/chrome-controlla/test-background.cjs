const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const sockets = [];
const detached = [];
const attached = [];
const debuggerCalls = [];
const pendingCommands = [];
const commandDiagnostics = [];
let rejectNextCommand;
let blockedDetach;
let blockedFrame;
let releaseBlockedFrame;
const nativePorts = [];
const reconnectTimers = [];
const alarms = [];
const tabList = Array.from({ length: 102 }, (_, id) => ({ id, title: `Tab ${id}`.repeat(100), url: `https://example.test/${id}`, status: "complete" }));
tabList.push({ id: 102, title: "Extension page", url: "chrome://extensions" });
tabList.push({ title: "Invalid tab", url: "https://example.test/invalid" });
let listener;
let mockServerVersion = "0.1.0";
let mockNativeError;
let nativeErrorReads = 0;
const tabUrlOverrides = new Map();
const tabDocumentOverrides = new Map();
const reloadDuringAttach = new Set();
const replaceDuringAttach = new Set();
let tabsUpdated;
let tabsReplaced;
let navigationBefore;
let navigationCommitted;
const expectedUrls = ids => Object.fromEntries(ids.map(id => [id, `https://example.test/${id}`]));
const expectedDocuments = ids => Object.fromEntries(ids.map(id => [id, `document-${id}`]));
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) {
    this.url = url;
    this.sent = [];
    this.readyState = 0;
    sockets.push(this);
    queueMicrotask(() => { this.readyState = 1; this.onopen?.(); });
  }
  send(payload) {
    const message = JSON.parse(payload);
    this.sent.push(message);
    if (message.type === "hello") {
      queueMicrotask(() => this.onmessage?.({ data: JSON.stringify({ type: "ready", server_version: mockServerVersion }) }));
    }
  }
  close() { this.readyState = 3; this.onclose?.(); }
}
class FakeNativePort {
  constructor(name) {
    this.name = name; this.sent = [];
    this.onMessage = { addListener: callback => { this.receive = callback; } };
    this.onDisconnect = { addListener: callback => { this.disconnected = callback; } };
  }
  postMessage(value) { this.sent.push(value); }
  hostMessage(value) { this.receive(value); }
  disconnect(error) { mockNativeError = error ? { message: error } : undefined; this.disconnected(); mockNativeError = undefined; }
}
const chrome = {
  alarms: {
    create(name, info) { alarms.push({ name, info }); },
    onAlarm: { addListener(callback) { this.fire = callback; } },
  },
  tabs: {
    async query() { return tabList; },
    async get(tabId) {
      const tab = tabList.find(tab => tab.id === tabId);
      if (!tab) throw new Error("No tab with id: " + tabId);
      return { ...tab, url: tabUrlOverrides.has(tabId) ? tabUrlOverrides.get(tabId) : tab.url };
    },
    onUpdated: { addListener(callback) { tabsUpdated = callback; } },
    onReplaced: { addListener(callback) { tabsReplaced = callback; } },
  },
  webNavigation: {
    async getFrame({ tabId, frameId }) {
      if (frameId !== 0 || !tabList.some(tab => tab.id === tabId)) return undefined;
      if (blockedFrame?.tabId === tabId) {
        const wait = blockedFrame;
        blockedFrame = undefined;
        wait.started();
        await new Promise(resolve => { releaseBlockedFrame = resolve; });
      }
      const tab = tabList.find(item => item.id === tabId);
      return { documentId: tabDocumentOverrides.get(tabId) || `document-${tabId}`,
        url: tabUrlOverrides.get(tabId) || tab.url };
    },
    onBeforeNavigate: { addListener(callback) { navigationBefore = callback; } },
    onCommitted: { addListener(callback) { navigationCommitted = callback; } },
  },
  runtime: {
    getManifest() { return { version: "0.1.0" }; },
    get lastError() { nativeErrorReads++; return mockNativeError; },
    onMessage: { addListener(callback) { listener = callback; } },
    connectNative(name) { const port = new FakeNativePort(name); nativePorts.push(port); return port; }
  },
  debugger: {
    async attach({ tabId }) {
      attached.push(tabId);
      if (reloadDuringAttach.delete(tabId)) {
        tabsUpdated(tabId, { status: "loading" });
        tabsUpdated(tabId, { status: "complete" });
      }
      if (replaceDuringAttach.delete(tabId)) tabsReplaced(tabId + 1000, tabId);
    },
    async detach({ tabId }) {
      detached.push(tabId);
      if (blockedDetach?.tabId === tabId) await new Promise(resolve => { blockedDetach.resolve = resolve; });
    },
    async sendCommand(target, method, params) {
      debuggerCalls.push({ target, method, params });
      if (rejectNextCommand === method) { rejectNextCommand = undefined; throw new Error("fixture rejection"); }
      if (method === "Runtime.evaluate" && params?.awaitResult) {
        return new Promise(resolve => pendingCommands.push(resolve));
      }
      return { accepted: true };
    },
    onDetach: { addListener() {} },
  },
};
vm.runInNewContext(fs.readFileSync(new URL("./background.js", `file://${__filename}`), "utf8"), {
  chrome, WebSocket: FakeWebSocket, URL, Set, Number, Array, String, Error, Promise, JSON,
  console: { debug(...args) { commandDiagnostics.push(args); } },
  setTimeout(callback, delay) { reconnectTimers.push({ callback, delay }); return reconnectTimers.length; },
  clearTimeout,
});

function message(payload) {
  return new Promise(resolve => listener(payload, {}, resolve));
}

(async () => {
  assert.equal(nativePorts[0].name, "chrome_controlla_bridge", "connect native host on worker startup");
  const native = nativePorts[0];
  native.hostMessage({ type: "list_tabs", request_id: "list-1" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const listed = native.sent[0];
  assert.equal(listed.type, "tabs");
  assert.equal(listed.request_id, "list-1");
  assert.equal(listed.tabs.length, 100, "tab listing is capped");
  assert.equal(listed.truncated, true, "tab listing reports when eligible tabs were omitted");
  assert.ok(listed.tabs.every(tab => Number.isSafeInteger(tab.id) && /^https?:\/\//.test(tab.url)));
  assert.ok(listed.tabs.every(tab => typeof tab.document_id === "string"));
  assert.ok(listed.tabs.every(tab => tab.title.length <= 256 && tab.url.length <= 2048));
  assert.deepEqual(attached, [], "listing tabs does not attach any tab");
  native.hostMessage({ type: "pair", request_id: "pair-1", tab_ids: [41, 43], expected_urls: expectedUrls([41, 43]), expected_document_ids: expectedDocuments([41, 43]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(attached, [41, 43], "native pairing attaches exactly the requested tabs");
  assert.deepEqual([...new Set(attached)], [41, 43], "native pairing never attaches all tabs");
  assert.deepEqual(debuggerCalls, [], "pairing must not dispatch commands");
  assert.equal(JSON.stringify(native.sent[1]), JSON.stringify({ type: "paired", request_id: "pair-1", targets: ["41", "43"], extension_version: "0.1.0", document_identity: true }));
  native.hostMessage({ type: "command", id: "dom", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.at(-1).method, "DOM.getDocument");
  const successTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(successTrace.map(event => event.phase), ["dispatch", "settled", "reply"]);
  assert.deepEqual(successTrace[1], {
    phase: "settled", commandId: "dom", targetId: 41, method: "DOM.getDocument",
    elapsedMs: successTrace[1].elapsedMs, outcome: "result"
  }, "debug diagnostics identify successful dispatch settlement without command parameters");
  assert.ok(Number.isSafeInteger(successTrace[1].elapsedMs) && successTrace[1].elapsedMs >= 0);
  assert.ok(successTrace.every(event => !("params" in event) && !("url" in event)));
  assert.equal(successTrace.at(-1).outcome, "sent");
  native.hostMessage({ type: "command", id: 23, target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  const numericTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(numericTrace.map(event => event.phase), ["dispatch", "settled", "reply"]);
  assert.equal(numericTrace[0].commandId, "23");
  assert.equal(numericTrace[2].commandId, "23");
  rejectNextCommand = "DOM.getDocument";
  native.hostMessage({ type: "command", id: "dom-error", target_id: "41", method: "DOM.getDocument", params: { secret: "never-log" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const errorTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(errorTrace.map(event => event.phase), ["dispatch", "settled", "reply"]);
  assert.deepEqual(errorTrace[1], {
    phase: "settled", commandId: "dom-error", targetId: 41, method: "DOM.getDocument",
    elapsedMs: errorTrace[1].elapsedMs, outcome: "error", errorClass: "Error"
  }, "debug diagnostics classify debugger failures without params or error bodies");
  assert.ok(Number.isSafeInteger(errorTrace[1].elapsedMs) && errorTrace[1].elapsedMs >= 0);
  assert.equal(errorTrace[2].outcome, "sent");
  assert.equal(JSON.stringify(errorTrace).includes("never-log"), false);
  assert.equal(native.sent.at(-1).type, "result", "diagnostics do not change command error response shape");
  assert.match(native.sent.at(-1).error, /fixture rejection/);
  native.hostMessage({ type: "command", id: "unauthorized", target_id: "42", method: "DOM.getDocument", params: {} });
  native.hostMessage({ type: "command", id: "bad-method", target_id: "41", method: "Browser.getVersion", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-2).error, /not attached/);
  assert.match(native.sent.at(-1).error, /allowlist/);

  const firstManualPair = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "manual-1", tabIds: [44] });
  assert.equal(firstManualPair.ok, true, firstManualPair.error);
  assert.deepEqual(attached.slice(-1), [44], "manual pairing can coexist on a distinct tab");
  await message({ type: "release" });
  assert.ok(detached.includes(44));
  assert.ok(!detached.includes(41), "popup release leaves native-owned tabs attached");
  native.hostMessage({ type: "command", id: "still-native", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).id, "still-native", "native command authorization survives popup release");

  commandDiagnostics.splice(0);
  native.hostMessage({ type: "command", id: "old-pair-result", target_id: "41", method: "Runtime.evaluate", params: { awaitResult: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const pendingTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(pendingTrace.map(event => event.phase), ["dispatch"],
    "a still-pending Chrome command logs dispatch without falsely claiming settlement or reply");
  const oldPairCommand = pendingCommands.at(-1);
  native.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  native.hostMessage({ type: "pair", request_id: "new-pair", tab_ids: [41], expected_urls: expectedUrls([41]), expected_document_ids: expectedDocuments([41]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  oldPairCommand({ value: "obsolete result" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.some(reply => reply.id === "old-pair-result"), false,
    "command results from a released pairing are suppressed after same-port re-pairing");
  const suppressedTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(suppressedTrace.map(event => event.phase), ["settled", "reply"]);
  assert.equal(suppressedTrace.at(-1).outcome, "suppressed");

  const detachCountBeforeCollision = detached.length;
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "collision", tabIds: [41] })).ok, false,
    "popup pairing rejects a native-owned tab");
  assert.equal(detached.length, detachCountBeforeCollision, "cross-transport collision does not detach the owner");
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "manual-2", tabIds: [45] })).ok, true);
  const secondManual = sockets.at(-1);
  native.hostMessage({ type: "pair", request_id: "collision", tab_ids: [45], expected_urls: expectedUrls([45]), expected_document_ids: expectedDocuments([45]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).type, "pair_error", "native pairing rejects a manual-owned tab");
  await secondManual.onmessage({ data: JSON.stringify({ type: "command", id: "still-manual", target_id: "45", method: "DOM.getDocument", params: {} }) });
  assert.equal(secondManual.sent.at(-1).id, "still-manual", "failed native collision leaves manual authorization intact");
  native.hostMessage({ type: "pair", request_id: "pair-2", tab_ids: [46], expected_urls: expectedUrls([46]), expected_document_ids: expectedDocuments([46]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).type, "paired");
  native.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(46));
  assert.ok(!detached.includes(45), "native release leaves manual-owned tabs attached");
  await secondManual.onmessage({ data: JSON.stringify({ type: "command", id: "manual-after-native-release", target_id: "45", method: "DOM.getDocument", params: {} }) });
  assert.equal(secondManual.sent.at(-1).id, "manual-after-native-release");
  await message({ type: "release" });

  native.hostMessage({ type: "pair", request_id: "pair-release-race", tab_ids: [50], expected_urls: expectedUrls([50]), expected_document_ids: expectedDocuments([50]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  blockedDetach = { tabId: 50 };
  native.hostMessage({ type: "release" });
  native.hostMessage({ type: "pair", request_id: "pair-after-release", tab_ids: [51], expected_urls: expectedUrls([51]), expected_document_ids: expectedDocuments([51]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.includes(51), false, "next pair waits until release detach finishes");
  blockedDetach.resolve();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).request_id, "pair-after-release");
  assert.deepEqual([...native.sent.at(-1).targets], ["51"]);
  assert.equal(detached.includes(51), false, "queued release does not detach the later pair");

  native.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(41) && detached.includes(43), "release detaches only explicitly paired native targets");

  native.disconnect();
  const disconnectedStatus = await message({ type: "native-status" });
  assert.equal(disconnectedStatus.connected, false);
  assert.deepEqual([...disconnectedStatus.attached], []);
  assert.equal(disconnectedStatus.error, undefined);
  assert.equal(reconnectTimers.at(-1).delay, 1000, "first reconnect uses base delay");
  assert.equal(alarms.at(-1).name, "controlla-native-reconnect", "reconnect survives service worker suspension");
  reconnectTimers.at(-1).callback();
  assert.equal(nativePorts.length, 2, "disconnect reconnects the native host");
  nativePorts[1].disconnect(`Forbidden\n${"x".repeat(300)}`);
  const errorStatus = await message({ type: "native-status" });
  assert.equal(errorStatus.connected, false);
  assert.deepEqual([...errorStatus.attached], []);
  assert.equal(errorStatus.error.length, 256, "native disconnect diagnostic is bounded");
  assert.equal(errorStatus.error.includes("\n"), false, "diagnostic strips control characters");
  assert.ok(nativeErrorReads >= 2, "disconnect consumes Chrome's runtime.lastError synchronously");
  reconnectTimers.at(-1).callback();
  assert.equal(nativePorts.length, 3);
  nativePorts[2].hostMessage({ type: "ready" });
  assert.equal((await message({ type: "native-status" })).error, undefined, "ready clears stale native error");
  nativePorts[2].disconnect();
  assert.equal(reconnectTimers.at(-1).delay, 1000, "successful host connection resets capped backoff after ready");
  chrome.alarms.onAlarm.fire({ name: "controlla-native-reconnect" });
  assert.equal(nativePorts.length, 4, "alarm reconnects after the service worker timer is lost");

  const oldNative = nativePorts[3];
  oldNative.hostMessage({ type: "pair", request_id: "late-pair", tab_ids: [17], expected_urls: expectedUrls([17]), expected_document_ids: expectedDocuments([17]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  oldNative.hostMessage({ type: "command", id: "late", target_id: "17", method: "Runtime.evaluate", params: { awaitResult: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  blockedDetach = { tabId: 17 };
  oldNative.disconnect();
  reconnectTimers.at(-1).callback();
  const newNative = nativePorts[4];
  newNative.hostMessage({ type: "ready" });
  newNative.hostMessage({ type: "pair", request_id: "pair-after-disconnect", tab_ids: [53], expected_urls: expectedUrls([53]), expected_document_ids: expectedDocuments([53]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.includes(53), false, "reconnected host waits for old disconnect cleanup");
  blockedDetach.resolve();
  blockedDetach = undefined;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(newNative.sent.at(-1).request_id, "pair-after-disconnect");
  assert.deepEqual([...newNative.sent.at(-1).targets], ["53"]);
  pendingCommands.at(-1)({ value: "late result" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(newNative.sent.some(reply => reply.id === "late"), false, "late async replies are not sent to the replacement native host");
  assert.equal(oldNative.sent.some(reply => reply.id === "late"), false, "disconnected host does not receive a late async reply");

  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "one", tabIds: [17] })).ok, true);
  assert.equal(sockets[0].sent[0].extension_version, "0.1.0", "extension handshake must identify the loaded version");
  assert.equal(sockets[0].sent[0].document_identity, true, "manual handshake declares durable document identity support");
  const oldSocket = sockets.at(-1);
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "two", tabIds: [18] })).ok, true);
  const detachCountBeforeOldClose = detached.length;
  oldSocket.onclose();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(detached.length, detachCountBeforeOldClose, "an old pairing must not detach newer tabs");

  const activeSocket = sockets.at(-1);
  const callsBeforeBlocked = debuggerCalls.length;
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "blocked", target_id: "18", method: "Browser.getVersion", params: {}
  }) });
  assert.equal(debuggerCalls.length, callsBeforeBlocked, "unlisted CDP methods must not reach chrome.debugger");
  assert.match(activeSocket.sent.at(-1).error, /outside the shared observe\/input allowlist/);

  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "allowed", target_id: "18", method: "Input.dispatchMouseEvent", params: { type: "mousePressed" }
  }) });
  assert.equal(debuggerCalls.length, callsBeforeBlocked + 1);
  assert.equal(debuggerCalls.at(-1).method, "Input.dispatchMouseEvent");
  const callsBeforeTyping = debuggerCalls.length;
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "allowed-key", target_id: "18", method: "Input.dispatchKeyEvent",
    params: { type: "char", text: "x", unmodifiedText: "x" }
  }) });
  assert.equal(debuggerCalls.length, callsBeforeTyping + 1, "shared typing dispatch is allowed on an attached tab");
  assert.equal(debuggerCalls.at(-1).method, "Input.dispatchKeyEvent");
  assert.equal(activeSocket.sent.at(-1).id, "allowed-key");
  for (const method of ["Runtime.callFunctionOn", "Runtime.releaseObject"]) {
    await activeSocket.onmessage({ data: JSON.stringify({
      type: "command", id: `allowed-${method}`, target_id: "18", method, params: {}
    }) });
    assert.equal(debuggerCalls.at(-1).method, method, `shared typing identity method ${method} is allowed`);
    assert.equal(activeSocket.sent.at(-1).id, `allowed-${method}`);
  }
  activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "old-manual-result", target_id: "18", method: "Runtime.evaluate", params: { awaitResult: true }
  }) });
  await new Promise(resolve => setTimeout(resolve, 0));
  const oldManualCommand = pendingCommands.at(-1);
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "replace-manual", tabIds: [20] })).ok, true);
  const callsAfterReplacement = debuggerCalls.length;
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "stale-manual-request", target_id: "20", method: "DOM.getDocument", params: {}
  }) });
  assert.equal(debuggerCalls.length, callsAfterReplacement, "replaced WebSocket cannot dispatch against the new pairing");
  oldManualCommand({ value: "obsolete manual result" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(activeSocket.sent.some(reply => reply.id === "old-manual-result"), false,
    "command results from a replaced WebSocket are suppressed");

  // Page-identity binding: pairing validates the tab URL and document ID observed at discovery.
  const liveNative = nativePorts.at(-1);
  liveNative.hostMessage({ type: "pair", request_id: "pair-urls-ok", tab_ids: [60], expected_urls: { 60: "https://example.test/60" }, expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "matching discovery URL pairs");
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "pair", request_id: "pair-url-missing", tab_ids: [66] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "pairing without a discovery URL fails closed");
  assert.ok(!attached.includes(66), "missing URL binding does not attach the tab");
  liveNative.hostMessage({ type: "pair", request_id: "pair-document-missing", tab_ids: [69], expected_urls: expectedUrls([69]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "pairing without a discovered document ID fails closed");
  assert.ok(!attached.includes(69), "missing document binding does not attach the tab");
  tabUrlOverrides.set(60, "https://example.test/navigated");
  liveNative.hostMessage({ type: "pair", request_id: "pair-urls-moved", tab_ids: [60], expected_urls: { 60: "https://example.test/60" }, expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "a tab that navigated since discovery refuses to pair");
  assert.match(liveNative.sent.at(-1).error, /navigated since discovery/);
  assert.ok(!attached.includes(60) || detached.includes(60), "refused pairing leaves no debugger attachment");
  tabUrlOverrides.delete(60);

  // Mid-session navigation detaches the paired tab.
  liveNative.hostMessage({ type: "pair", request_id: "pair-nav", tab_ids: [62], expected_urls: { 62: "https://example.test/62" }, expected_document_ids: expectedDocuments([62]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  assert.ok(!detached.includes(62), "paired tab stays attached before navigation");
  tabsUpdated(62, { url: "https://example.test/62#section" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(!detached.includes(62), "same-URL navigation keeps the pairing");
  tabsUpdated(62, { url: "https://example.test/replaced" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(62), "navigation to a different document detaches the paired tab");

  // A same-URL reload during debugger attachment must fail closed for the
  // native path, even though the tab URL is unchanged before and after.
  reloadDuringAttach.add(64);
  liveNative.hostMessage({ type: "pair", request_id: "same-url-native", tab_ids: [64], expected_urls: { 64: "https://example.test/64" }, expected_document_ids: expectedDocuments([64]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "native pairing rejects same-URL document replacement during attach");
  assert.match(liveNative.sent.at(-1).error, /navigated during pairing/);
  assert.ok(detached.includes(64), "native retry cleanup detaches the replaced document");

  replaceDuringAttach.add(65);
  liveNative.hostMessage({ type: "pair", request_id: "tab-replaced-native", tab_ids: [65], expected_urls: { 65: "https://example.test/65" }, expected_document_ids: expectedDocuments([65]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "native pairing rejects tab replacement during attach");
  assert.match(liveNative.sent.at(-1).error, /navigated during pairing/);
  assert.ok(detached.includes(65), "native replacement cleanup detaches the retired tab");

  liveNative.hostMessage({ type: "pair", request_id: "document-change", tab_ids: [67],
    expected_urls: expectedUrls([67]), expected_document_ids: expectedDocuments([67]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  tabDocumentOverrides.set(67, "replacement-document-67");
  navigationCommitted({ tabId: 67, frameId: 0, documentId: "replacement-document-67" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(67), "a committed same-URL document replacement detaches the paired tab");

  liveNative.hostMessage({ type: "pair", request_id: "document-command-guard", tab_ids: [68],
    expected_urls: expectedUrls([68]), expected_document_ids: expectedDocuments([68]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  tabDocumentOverrides.set(68, "unreported-replacement-68");
  const callsBeforeDocumentGuard = debuggerCalls.length;
  liveNative.hostMessage({ type: "command", id: "document-guard", target_id: "68", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.length, callsBeforeDocumentGuard, "commands fail closed when the current document ID changed");
  assert.match(liveNative.sent.at(-1).error, /document identity changed/);

  liveNative.hostMessage({ type: "pair", request_id: "document-auth-race", tab_ids: [70],
    expected_urls: expectedUrls([70]), expected_document_ids: expectedDocuments([70]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  let frameReadStarted;
  const frameRead = new Promise(resolve => { frameReadStarted = resolve; });
  blockedFrame = { tabId: 70, started: frameReadStarted };
  const callsBeforeAuthRace = debuggerCalls.length;
  liveNative.hostMessage({ type: "command", id: "navigation-auth-race", target_id: "70", method: "DOM.getDocument", params: {} });
  await frameRead;
  navigationBefore({ tabId: 70, frameId: 0 });
  releaseBlockedFrame();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.length, callsBeforeAuthRace, "navigation during document lookup revokes authorization before dispatch");
  assert.equal(liveNative.sent.at(-1).id, "navigation-auth-race");
  assert.match(liveNative.sent.at(-1).error, /no longer attached/);

  liveNative.hostMessage({ type: "pair", request_id: "detach-repair", tab_ids: [71],
    expected_urls: expectedUrls([71]), expected_document_ids: expectedDocuments([71]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  blockedDetach = { tabId: 71 };
  navigationBefore({ tabId: 71, frameId: 0 });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(typeof blockedDetach.resolve, "function", "navigation began its guarded debugger detach");
  const priorAttachments = attached.filter(tabId => tabId === 71).length;
  liveNative.hostMessage({ type: "pair", request_id: "repair-after-detach", tab_ids: [71],
    expected_urls: expectedUrls([71]), expected_document_ids: expectedDocuments([71]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 71).length, priorAttachments,
    "re-pair waits for an in-flight navigation detach");
  blockedDetach.resolve();
  blockedDetach = undefined;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "re-pair proceeds after the prior detach settles");

  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  let nativePairFrameStarted;
  const nativePairFrame = new Promise(resolve => { nativePairFrameStarted = resolve; });
  blockedFrame = { tabId: 73, started: nativePairFrameStarted };
  liveNative.hostMessage({ type: "pair", request_id: "busy-native-pair", tab_ids: [72, 73],
    expected_urls: expectedUrls([72, 73]), expected_document_ids: expectedDocuments([72, 73]) });
  await nativePairFrame;
  assert.ok(attached.includes(72), "native pair attaches its first tab before stalling on the next");
  blockedDetach = { tabId: 72 };
  tabList.find(tab => tab.id === 72).status = "loading";
  tabsUpdated(72, { status: "loading" });
  tabList.find(tab => tab.id === 72).status = "complete";
  const attachmentsBeforePopupRepair = attached.filter(tabId => tabId === 72).length;
  const popupRepairDuringPair = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "queued-detach", tabIds: [72] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(typeof blockedDetach.resolve, "function", "navigation detach is pending while the native queue is busy");
  assert.equal(popupRepairDuringPair.ok, false, "popup repair refuses a tab with a native pairing reservation");
  assert.equal(attached.filter(tabId => tabId === 72).length, attachmentsBeforePopupRepair,
    "popup loser does not attach concurrently with native pairing");
  releaseBlockedFrame();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "stalled native pairing completes after its second frame read");
  const popupRepair = message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "queued-detach-retry", tabIds: [72] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 72).length, attachmentsBeforePopupRepair,
    "popup retry waits for the pending native navigation detach");
  blockedDetach.resolve();
  blockedDetach = undefined;
  assert.equal((await popupRepair).ok, true, "popup retry proceeds after navigation detach settles");
  await message({ type: "release" });
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));

  let manualPairFrameStarted;
  const manualPairFrame = new Promise(resolve => { manualPairFrameStarted = resolve; });
  blockedFrame = { tabId: 74, started: manualPairFrameStarted };
  const manualWinner = message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "pair-reservation", tabIds: [74] });
  await manualPairFrame;
  const attachedBeforeNativeCollision = attached.filter(tabId => tabId === 74).length;
  liveNative.hostMessage({ type: "pair", request_id: "native-pair-reservation", tab_ids: [74],
    expected_urls: expectedUrls([74]), expected_document_ids: expectedDocuments([74]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "native pairing refuses a tab reserved by a popup pairing");
  assert.equal(attached.filter(tabId => tabId === 74).length, attachedBeforeNativeCollision,
    "native loser does not attach concurrently with the popup winner");
  releaseBlockedFrame();
  assert.equal((await manualWinner).ok, true, "popup pairing completes with its document identity intact");
  await message({ type: "release" });

  let reservedNativePairFrameStarted;
  const reservedNativePairFrame = new Promise(resolve => { reservedNativePairFrameStarted = resolve; });
  blockedFrame = { tabId: 75, started: reservedNativePairFrameStarted };
  liveNative.hostMessage({ type: "pair", request_id: "native-pair-winner", tab_ids: [75],
    expected_urls: expectedUrls([75]), expected_document_ids: expectedDocuments([75]) });
  await reservedNativePairFrame;
  const popupLoser = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "pair-collision", tabIds: [75] });
  assert.equal(popupLoser.ok, false, "popup pairing refuses a tab reserved by an in-progress native pairing");
  assert.match(popupLoser.error, /already being paired/);
  releaseBlockedFrame();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "native pairing completes with its document identity intact");
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));

  // Popup pairings use the same loading-generation guard.
  reloadDuringAttach.add(63);
  const sameUrlManual = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "same-url", tabIds: [63] });
  assert.equal(sameUrlManual.ok, false, "popup pairing rejects same-URL document replacement during attach");
  assert.match(sameUrlManual.error, /navigated during pairing/);
  assert.ok(detached.includes(63), "popup retry cleanup detaches the replaced document");

  mockServerVersion = "0.0.9";
  const stalePair = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "stale", tabIds: [19] });
  assert.equal(stalePair.ok, false, "stale server handshake must not report connected");
  assert.match(stalePair.error, /Pairing rejected/);
  assert.ok(detached.includes(19), "failed stale pairing must release its debugger attachment");
  console.log("Extension version handshake, pairing cleanup, and command allowlist tests passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
