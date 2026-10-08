const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const sockets = [];
const detached = [];
const attached = [];
const debuggerCalls = [];
const pendingCommands = [];
let blockedDetach;
const nativePorts = [];
const reconnectTimers = [];
const alarms = [];
const tabList = Array.from({ length: 102 }, (_, id) => ({ id, title: `Tab ${id}`.repeat(100), url: `https://example.test/${id}` }));
tabList.push({ id: 102, title: "Extension page", url: "chrome://extensions" });
tabList.push({ title: "Invalid tab", url: "https://example.test/invalid" });
let listener;
let mockServerVersion = "0.1.0";
let mockNativeError;
let nativeErrorReads = 0;
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
  tabs: { async query() { return tabList; } },
  runtime: {
    getManifest() { return { version: "0.1.0" }; },
    get lastError() { nativeErrorReads++; return mockNativeError; },
    onMessage: { addListener(callback) { listener = callback; } },
    connectNative(name) { const port = new FakeNativePort(name); nativePorts.push(port); return port; }
  },
  debugger: {
    async attach({ tabId }) { attached.push(tabId); },
    async detach({ tabId }) {
      detached.push(tabId);
      if (blockedDetach?.tabId === tabId) await new Promise(resolve => { blockedDetach.resolve = resolve; });
    },
    async sendCommand(target, method, params) {
      debuggerCalls.push({ target, method, params });
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
  assert.ok(listed.tabs.every(tab => tab.title.length <= 256 && tab.url.length <= 2048));
  assert.deepEqual(attached, [], "listing tabs does not attach any tab");
  native.hostMessage({ type: "pair", request_id: "pair-1", tab_ids: [41, 43] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(attached, [41, 43], "native pairing attaches exactly the requested tabs");
  assert.deepEqual([...new Set(attached)], [41, 43], "native pairing never attaches all tabs");
  assert.deepEqual(debuggerCalls, [], "pairing must not dispatch commands");
  assert.equal(JSON.stringify(native.sent[1]), JSON.stringify({ type: "paired", request_id: "pair-1", targets: ["41", "43"], extension_version: "0.1.0" }));
  native.hostMessage({ type: "command", id: "dom", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.at(-1).method, "DOM.getDocument");
  native.hostMessage({ type: "command", id: "unauthorized", target_id: "42", method: "DOM.getDocument", params: {} });
  native.hostMessage({ type: "command", id: "bad-method", target_id: "41", method: "Browser.getVersion", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-2).error, /not attached/);
  assert.match(native.sent.at(-1).error, /allowlist/);

  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "manual-1", tabIds: [44] })).ok, true);
  assert.deepEqual(attached.slice(-1), [44], "manual pairing can coexist on a distinct tab");
  await message({ type: "release" });
  assert.ok(detached.includes(44));
  assert.ok(!detached.includes(41), "popup release leaves native-owned tabs attached");
  native.hostMessage({ type: "command", id: "still-native", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).id, "still-native", "native command authorization survives popup release");

  native.hostMessage({ type: "command", id: "old-pair-result", target_id: "41", method: "Runtime.evaluate", params: { awaitResult: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const oldPairCommand = pendingCommands.at(-1);
  native.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  native.hostMessage({ type: "pair", request_id: "new-pair", tab_ids: [41] });
  await new Promise(resolve => setTimeout(resolve, 0));
  oldPairCommand({ value: "obsolete result" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.some(reply => reply.id === "old-pair-result"), false,
    "command results from a released pairing are suppressed after same-port re-pairing");

  const detachCountBeforeCollision = detached.length;
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "collision", tabIds: [41] })).ok, false,
    "popup pairing rejects a native-owned tab");
  assert.equal(detached.length, detachCountBeforeCollision, "cross-transport collision does not detach the owner");
  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "manual-2", tabIds: [45] })).ok, true);
  const secondManual = sockets.at(-1);
  native.hostMessage({ type: "pair", request_id: "collision", tab_ids: [45] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).type, "pair_error", "native pairing rejects a manual-owned tab");
  await secondManual.onmessage({ data: JSON.stringify({ type: "command", id: "still-manual", target_id: "45", method: "DOM.getDocument", params: {} }) });
  assert.equal(secondManual.sent.at(-1).id, "still-manual", "failed native collision leaves manual authorization intact");
  native.hostMessage({ type: "pair", request_id: "pair-2", tab_ids: [46] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).type, "paired");
  native.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(46));
  assert.ok(!detached.includes(45), "native release leaves manual-owned tabs attached");
  await secondManual.onmessage({ data: JSON.stringify({ type: "command", id: "manual-after-native-release", target_id: "45", method: "DOM.getDocument", params: {} }) });
  assert.equal(secondManual.sent.at(-1).id, "manual-after-native-release");
  await message({ type: "release" });

  native.hostMessage({ type: "pair", request_id: "pair-release-race", tab_ids: [50] });
  await new Promise(resolve => setTimeout(resolve, 0));
  blockedDetach = { tabId: 50 };
  native.hostMessage({ type: "release" });
  native.hostMessage({ type: "pair", request_id: "pair-after-release", tab_ids: [51] });
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
  oldNative.hostMessage({ type: "pair", request_id: "late-pair", tab_ids: [17] });
  await new Promise(resolve => setTimeout(resolve, 0));
  oldNative.hostMessage({ type: "command", id: "late", target_id: "17", method: "Runtime.evaluate", params: { awaitResult: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  blockedDetach = { tabId: 17 };
  oldNative.disconnect();
  reconnectTimers.at(-1).callback();
  const newNative = nativePorts[4];
  newNative.hostMessage({ type: "ready" });
  newNative.hostMessage({ type: "pair", request_id: "pair-after-disconnect", tab_ids: [53] });
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
  mockServerVersion = "0.0.9";
  const stalePair = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "stale", tabIds: [19] });
  assert.equal(stalePair.ok, false, "stale server handshake must not report connected");
  assert.match(stalePair.error, /Pairing rejected/);
  assert.ok(detached.includes(19), "failed stale pairing must release its debugger attachment");
  console.log("Extension version handshake, pairing cleanup, and command allowlist tests passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
