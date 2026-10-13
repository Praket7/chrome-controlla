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
const sessionStorage = {};
const localStorage = {};
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
const failAttach = new Set();
let tabsUpdated;
let tabsReplaced;
let tabsActivated;
let tabsCreated;
let windowFocusChanged;
let navigationBefore;
let navigationCommitted;
let navigationDomContentLoaded;
const committedListeners = new Set();
const domContentLoadedListeners = new Set();
let slowMediaNextNavigation = false;
let delayedNextCommitMs = 0;
let navigationError;
let debuggerDetached;
let debuggerEvent;
let nextCreatedTab = 500;
let loseNextCreatedTabReply = false;
let mockNow = 1_000;
let advanceClockOnNextDebuggerCall = 0;
let afterNextDebuggerCall;
let attachOopifOnNextAutoAttach = false;
let fallbackFileInputDocument;
let fallbackFrameFileInputDocument;
let identityAxNodes = [];
let rootIdentityElements = [];
let rejectFrameMouseType;
let emitFileChooserOnFrameRelease = false;
let emitFileChooserOnRootRelease = false;
let frameOutcomeVerified = false;
let frameActionable = true;
let childFrameLoaderId = "fixture-loader";
let navigateFrameDuringFileReadback = false;
let fileReadbackEmpty = false;
let childFrameUrl = "https://picker.example.test/";
let navigateFrameOnClick = false;
let networkEventsOnEnable = [];
let fastTypeValue = "";
let fastTypeFocused = true;
let stealFocusAfterKeydown = false;
let stealFocusAfterKeyup = false;
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
  runtime: { getURL(path) { return `chrome-extension://fixture/${path}`; } },
  storage: { local: {
    async get(key) { return { [key]: localStorage[key] }; },
    async set(values) { Object.assign(localStorage, values); },
    async remove(key) { delete localStorage[key]; },
  }, session: {
    async get(key) { return { [key]: sessionStorage[key] }; },
    async set(values) { Object.assign(sessionStorage, values); },
    async remove(key) { delete sessionStorage[key]; },
  } },
  alarms: {
    create(name, info) { alarms.push({ name, info }); },
    onAlarm: { addListener(callback) { this.fire = callback; } },
  },
  tabs: {
    async query() { return tabList; },
    async create({ url, active = true }) { const tab = { id: nextCreatedTab++, title: "New tab", url, status: "complete", active }; tabList.push(tab); if (loseNextCreatedTabReply) { loseNextCreatedTabReply = false; throw new Error("simulated lost tabs.create reply"); } return tab; },
    async update(tabId, { url }) {
      const tab = tabList.find(tab => tab.id === tabId);
      if (!tab) throw new Error("missing tab");
      const delayedCommit = delayedNextCommitMs;
      delayedNextCommitMs = 0;
      const priorUrl = tabUrlOverrides.get(tabId) || tab.url;
      const priorDocumentId = tabDocumentOverrides.get(tabId) || `document-${tabId}`;
      tab.url = url;
      tab.status = delayedCommit ? "complete" : slowMediaNextNavigation ? "loading" : "complete";
      const documentId = `${priorDocumentId}-next`;
      if (delayedCommit) {
        tabUrlOverrides.set(tabId, priorUrl);
        setTimeout(() => {
          tabUrlOverrides.set(tabId, url);
          tabDocumentOverrides.set(tabId, documentId);
          navigationCommitted?.({ tabId, frameId: 0, documentId, url });
          navigationDomContentLoaded?.({ tabId, frameId: 0, documentId, url });
        }, delayedCommit);
        return tab;
      }
      tabUrlOverrides.set(tabId, url);
      tabDocumentOverrides.set(tabId, documentId);
      navigationCommitted?.({ tabId, frameId: 0, documentId, url });
      if (slowMediaNextNavigation) {
        slowMediaNextNavigation = false;
        setTimeout(() => navigationDomContentLoaded?.({ tabId, frameId: 0, documentId, url }), 0);
      } else navigationDomContentLoaded?.({ tabId, frameId: 0, documentId, url });
      return tab;
    },
    async get(tabId) {
      const tab = tabList.find(tab => tab.id === tabId);
      if (!tab) throw new Error("No tab with id: " + tabId);
      return { ...tab, url: tabUrlOverrides.has(tabId) ? tabUrlOverrides.get(tabId) : tab.url };
    },
    onUpdated: { addListener(callback) { tabsUpdated = callback; }, removeListener() {} },
    onReplaced: { addListener(callback) { tabsReplaced = callback; } },
    onActivated: { addListener(callback) { tabsActivated = callback; } },
    onCreated: { addListener(callback) { tabsCreated = callback; } },
  },
  windows: { async get() { return { focused: false }; }, onFocusChanged: { addListener(callback) { windowFocusChanged = callback; } } },
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
    onCommitted: { addListener(callback) { committedListeners.add(callback); navigationCommitted = details => { for (const listener of [...committedListeners]) listener(details); }; }, removeListener(callback) { committedListeners.delete(callback); } },
    onDOMContentLoaded: { addListener(callback) { domContentLoadedListeners.add(callback); navigationDomContentLoaded = details => { for (const listener of [...domContentLoadedListeners]) listener(details); }; }, removeListener(callback) { domContentLoadedListeners.delete(callback); } },
    onErrorOccurred: { addListener(callback) { navigationError = callback; } },
  },
  runtime: {
    getManifest() { return { version: "0.1.0" }; },
    getURL(path) { return `chrome-extension://fixture/${path}`; },
    get lastError() { nativeErrorReads++; return mockNativeError; },
    onMessage: { addListener(callback) { listener = callback; } },
    connectNative(name) { const port = new FakeNativePort(name); nativePorts.push(port); return port; }
  },
  debugger: {
    async attach({ tabId }) {
      if (failAttach.delete(tabId)) throw new Error("fixture attach failure");
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
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(index,expected,prepare,normalize,requireFocus)")) {
        const [index, expected, prepare] = params.arguments.map(argument => argument.value);
        if (prepare) fastTypeFocused = true;
        return { result: { value: { ok: fastTypeFocused && fastTypeValue === expected, reason: fastTypeFocused ? "stale_value" : "focus_or_selection_changed", value: fastTypeValue, rich: false } } };
      }
      if (method === "Input.dispatchKeyEvent") {
        if (params.type === "keyDown" && stealFocusAfterKeydown) { stealFocusAfterKeydown = false; fastTypeFocused = false; }
        if (params.type === "keyUp" && stealFocusAfterKeyup) { stealFocusAfterKeyup = false; fastTypeFocused = false; }
        if (params.type === "char") fastTypeValue += params.text;
        if (params.type === "keyDown" && params.key === "Enter") fastTypeValue += "\n";
        return {};
      }
      if (method === "Network.enable" && networkEventsOnEnable.length) {
        const events = networkEventsOnEnable;
        networkEventsOnEnable = [];
        for (const [eventMethod, eventParams] of events) queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId, sessionId: target.sessionId }, eventMethod, eventParams));
      }
      if (method === "Runtime.evaluate" && params?.expression?.includes("button[data-identity-test]")) {
        const document = {
          querySelectorAll(selector) { return selector === "button[data-identity-test]" ? rootIdentityElements : []; },
          querySelector() { return rootIdentityElements[0] || null; },
        };
        return { result: { value: vm.runInNewContext(params.expression, { document, getComputedStyle: element => element.style }) } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("__controllaFrameOutcome")) {
        return { result: { value: frameOutcomeVerified } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("__controllaFrameActionable")) {
        return { result: { value: frameActionable ? { x: 20, y: 30 } : false } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("ownerDocument.defaultView.innerWidth")) {
        return { result: { value: params.objectId === "fixture-frame-owner" ? { width: 800, height: 600 } : { width: 100, height: 100 } } };
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === rejectFrameMouseType) {
        rejectFrameMouseType = undefined;
        throw new Error("fixture mouse dispatch failure");
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && target.sessionId === "fixture-oopif-session" && emitFileChooserOnFrameRelease) {
        emitFileChooserOnFrameRelease = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId, sessionId: target.sessionId }, "Page.fileChooserOpened", { backendNodeId: 908, frameId: "frame-oopif", mode: "selectSingle" }));
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && !target.sessionId && emitFileChooserOnRootRelease) {
        emitFileChooserOnRootRelease = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId }, "Page.fileChooserOpened", { backendNodeId: 908, frameId: "frame-oopif", mode: "selectSingle" }));
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && navigateFrameOnClick) { navigateFrameOnClick = false; childFrameUrl = "https://picker.example.test/next"; }
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "root-frame", loaderId: "root-loader", url: "https://example.test/root" }, childFrames: [{ frame: { id: "frame-oopif", loaderId: childFrameLoaderId, url: childFrameUrl } }] } };
      if (method === "DOM.getDocument" && target.sessionId === "fixture-oopif-session") return fallbackFrameFileInputDocument || { root: { nodeName: "#document" } };
      if (method === "DOM.getDocument" && fallbackFileInputDocument) return fallbackFileInputDocument;
      if (method === "Target.setAutoAttach" && params?.autoAttach && attachOopifOnNextAutoAttach) {
        attachOopifOnNextAutoAttach = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId }, "Target.attachedToTarget", { sessionId: "fixture-oopif-session", targetInfo: { type: "iframe", targetId: "frame-oopif", url: "https://picker.example.test/" } }));
      }
      if (method === "Accessibility.getFullAXTree" && target.sessionId === "fixture-oopif-session") {
        return { nodes: identityAxNodes.length ? identityAxNodes : [{ nodeId: "1", role: { value: "option" }, name: { value: "School account" }, backendDOMNodeId: 91 }] };
      }
      if (method === "Accessibility.getFullAXTree" && params?.frameId === "frame-oopif") return { nodes: identityAxNodes };
      if (method === "DOM.getFrameOwner") return { backendNodeId: 999 };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 999) return { model: { content: [100, 200, 300, 200, 300, 400, 100, 400] } };
      if (method === "DOM.getBoxModel" && target.sessionId === "fixture-oopif-session") return { model: { content: [10, 20, 30, 20, 30, 40, 10, 40] } };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 777) return { model: { content: [10, 20, 30, 20, 30, 40, 10, 40] } };
      if (method === "DOM.resolveNode") return { object: { objectId: params?.backendNodeId === 999 ? "fixture-frame-owner" : "fixture-file-object" } };
      if (method === "Runtime.callFunctionOn" && params?.objectId === "fixture-file-object") {
        if (navigateFrameDuringFileReadback) { navigateFrameDuringFileReadback = false; childFrameLoaderId = "navigated-loader"; }
        if (fileReadbackEmpty) { fileReadbackEmpty = false; return { result: { value: [] } }; }
        return { result: { value: [{ name: "example", size: 12 }] } };
      }
      const afterCall = afterNextDebuggerCall;
      afterNextDebuggerCall = undefined;
      afterCall?.(target, method, params);
      mockNow += advanceClockOnNextDebuggerCall;
      advanceClockOnNextDebuggerCall = 0;
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration === "guard") return { result: { value: { ok: false } } };
      if (method === "Runtime.evaluate" && params?.awaitResult) {
        return new Promise(resolve => pendingCommands.push(resolve));
      }
      return { accepted: true };
    },
    onDetach: { addListener(callback) { debuggerDetached = callback; } },
    onEvent: { addListener(callback) { debuggerEvent = callback; } },
  },
};
const backgroundSource = fs.readFileSync(new URL("./background.js", `file://${__filename}`), "utf8");
function runBackground() {
  vm.runInNewContext(backgroundSource, {
    chrome, WebSocket: FakeWebSocket, URL, Set, Number, Array, String, Error, Promise, JSON,
    Date: { now() { return mockNow; } },
    performance: { now() { return mockNow; } },
    console: { debug(...args) { commandDiagnostics.push(args); } },
    setTimeout(callback, delay) { reconnectTimers.push({ callback, delay }); return reconnectTimers.length; },
    clearTimeout,
  });
}
runBackground();

function message(payload) {
  return new Promise(resolve => listener(payload, {}, resolve));
}

(async () => {
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(nativePorts[0].name, "chrome_controlla_bridge", "connect native host on worker startup");
  let native = nativePorts[0];
  if (process.argv.includes("--reconnect-only")) {
    let candidate = native;
    candidate.hostMessage({ type: "ready" });
    candidate.disconnect("native host unavailable");
    await new Promise(resolve => setTimeout(resolve, 0));
    runBackground();
    await new Promise(resolve => setTimeout(resolve, 0));
    candidate = nativePorts.at(-1);
    candidate.disconnect("native host still unavailable");
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(reconnectTimers.at(-1).delay, 2000,
      "service-worker restart preserves the bounded reconnect backoff attempt");
    for (let attempt = 2; attempt <= 8; attempt++) {
      reconnectTimers.at(-1).callback();
      candidate = nativePorts.at(-1);
      candidate.disconnect("native host unavailable");
      await new Promise(resolve => setTimeout(resolve, 0));
      const status = await message({ type: "native-status" });
      if (attempt < 8) {
        assert.equal(status.retry_exhausted, false, "native reconnect remains available within its bounded retry budget");
      }
    }
    const exhausted = await message({ type: "native-status" });
    assert.equal(exhausted.retry_exhausted, true,
      "native reconnect reports a terminal state after exhausting its bounded retry budget");
    assert.match(exhausted.error, /reload the extension to retry/);
    const portsAtExhaustion = nativePorts.length;
    chrome.alarms.onAlarm.fire({ name: "controlla-native-reconnect" });
    assert.equal(nativePorts.length, portsAtExhaustion, "a stale service-worker alarm cannot restart exhausted reconnects");
    return;
  }
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
  native.hostMessage({ type: "pair", request_id: "pair-1", tab_ids: [41, 43, 47], expected_urls: expectedUrls([41, 43, 47]), expected_document_ids: expectedDocuments([41, 43, 47]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(attached, [41, 43, 47], "native pairing attaches exactly the requested tabs");
  assert.deepEqual([...new Set(attached)], [41, 43, 47], "native pairing never attaches all tabs");
  assert.deepEqual(debuggerCalls, [], "pairing must not dispatch commands");
  assert.equal(JSON.stringify(native.sent[1]), JSON.stringify({ type: "paired", request_id: "pair-1", lease_id: "pair-1", targets: ["41", "43", "47"], extension_version: "0.1.0", protocol_version: 2, document_identity: true, batch_execution: true, batch_deadline: true }));
  fastTypeValue = ""; fastTypeFocused = true;
  native.hostMessage({ type: "command", id: "fast-type-program", target_id: "41", method: "Controlla.fastType", params: { object_id: "fixture-editor", index: 0, expected: "", text: "Fast\nKeys", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const fastTypeReceipt = native.sent.find(message => message.id === "fast-type-program")?.result;
  assert.equal(fastTypeReceipt.status, "verified", JSON.stringify(fastTypeReceipt));
  assert.equal(fastTypeReceipt.readback, "Fast\nKeys");
  assert.equal(fastTypeReceipt.completed_chars, 9);
  assert.equal(fastTypeReceipt.key_events, 26);
  fastTypeValue = ""; fastTypeFocused = true; stealFocusAfterKeydown = true;
  const eventsBeforeFocusTheft = debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length;
  native.hostMessage({ type: "command", id: "fast-type-focus-theft", target_id: "41", method: "Controlla.fastType", params: { object_id: "fixture-editor", index: 0, expected: "", text: "x", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const focusTheftReceipt = native.sent.find(message => message.id === "fast-type-focus-theft")?.result;
  assert.equal(focusTheftReceipt.status, "unknown");
  assert.equal(focusTheftReceipt.completed_chars, 0);
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length - eventsBeforeFocusTheft, 1,
    "focus movement after keyDown must stop before char and keyUp are sent to a new target");
  fastTypeValue = ""; fastTypeFocused = true; stealFocusAfterKeyup = true;
  const eventsBeforeKeyupFocusTheft = debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length;
  native.hostMessage({ type: "command", id: "fast-type-keyup-focus-theft", target_id: "41", method: "Controlla.fastType", params: { object_id: "fixture-editor", index: 0, expected: "", text: "ab", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const keyupFocusTheft = native.sent.find(message => message.id === "fast-type-keyup-focus-theft")?.result;
  assert.equal(keyupFocusTheft.status, "unknown");
  assert.equal(keyupFocusTheft.completed_chars, 1);
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length - eventsBeforeKeyupFocusTheft, 3,
    "focus movement after keyUp must be detected before the next keyDown is dispatched");
  fastTypeValue = ""; fastTypeFocused = true; rejectNextCommand = "Input.dispatchKeyEvent";
  native.hostMessage({ type: "command", id: "fast-type-delivery-unknown", target_id: "41", method: "Controlla.fastType", params: { object_id: "fixture-editor", index: 0, expected: "", text: "x", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const clickStartedAt = 0;
  tabList.find(tab => tab.id === 41).windowId = 7;
  const popupTab = { id: 900, windowId: 7, openerTabId: 41, url: "https://example.test/story", status: "complete" };
  const unrelatedSameWindowTab = { id: 902, windowId: 7, url: "https://example.test/story", status: "complete" };
  const unrelatedTab = { id: 901, windowId: 8, url: "https://example.test/unrelated", status: "complete" };
  tabList.push(popupTab, unrelatedSameWindowTab, unrelatedTab); tabsCreated(popupTab); tabsCreated(unrelatedSameWindowTab); tabsCreated(unrelatedTab);
  native.hostMessage({ type: "command", id: "verify-click-new-tab", target_id: "41", method: "Controlla.newTabs", params: { since_ms: clickStartedAt, url: "https://example.test/story" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(native.sent.find(message => message.id === "verify-click-new-tab")?.result?.tabs)),
    [{ tab_id: "900", url: "https://example.test/story", opener_match: true }], "only Chrome's opener relationship attributes a tab to the click, excluding same-URL tabs opened independently");
  native.hostMessage({ type: "command", id: "verify-click-new-tab-no-url", target_id: "41", method: "Controlla.newTabs", params: { since_ms: clickStartedAt } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(native.sent.find(message => message.id === "verify-click-new-tab-no-url")?.result?.tabs)),
    [{ tab_id: "900", url: "https://example.test/story", opener_match: true }], "without a selected href, only Chrome's opener relationship is used");
  const failedFastTypeReceipt = native.sent.find(message => message.id === "fast-type-delivery-unknown")?.result;
  assert.equal(failedFastTypeReceipt.status, "unknown");
  assert.equal(failedFastTypeReceipt.cdp_commands, 2, "unknown partial delivery reports the preflight and attempted key command");
  attachOopifOnNextAutoAttach = true;
  native.hostMessage({ type: "command", id: "oopif-ax", target_id: "41", method: "Controlla.frameAccessibility", params: { frame_id: "frame-oopif" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const oopif = native.sent.at(-1);
  assert.equal(oopif.id, "oopif-ax");
  assert.equal(oopif.result.routing, "oopif_child_session");
  assert.equal(oopif.result.nodes[0].backendDOMNodeId, 91);
  assert.ok(debuggerCalls.some(call => call.target.sessionId === "fixture-oopif-session" && call.method === "Accessibility.getFullAXTree"),
    "OOPIF accessibility commands use the attached flattened child session");
  identityAxNodes = [{ nodeId: "2", role: { value: "option" }, name: { value: "student@example.edu" }, backendDOMNodeId: 92,
    properties: [{ name: "selected", value: { type: "boolean", value: false } }] }];
  native.hostMessage({ type: "command", id: "identity-marker-unselected-option", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button.account", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, false, "an unselected account option cannot prove the active account");
  identityAxNodes[0].properties[0].value.value = true;
  const identityAxReadCount = debuggerCalls.filter(call => call.method === "Accessibility.getFullAXTree").length;
  const identityAttachCount = debuggerCalls.filter(call => call.method === "Target.setAutoAttach").length;
  native.hostMessage({ type: "command", id: "identity-marker-ax", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button.account", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, false, "a selected account-picker option cannot prove the active account");
  assert.equal(native.sent.at(-1).result.match_count, 0, "accessibility picker entries never count as active-account proof");
  assert.equal(debuggerCalls.filter(call => call.method === "Accessibility.getFullAXTree").length, identityAxReadCount, "identity authorization never consults the picker accessibility tree");
  assert.equal(debuggerCalls.filter(call => call.method === "Target.setAutoAttach").length, identityAttachCount, "identity authorization never attaches to a picker frame");
  const rootIdentity = (text, visible, parentOpacity = "1", ariaLabel = null, pickerContext = false, selected = null) => ({
    isConnected: true, innerText: text, style: { display: "block", visibility: "visible", opacity: "1" },
    parentElement: { style: { display: "block", visibility: "visible", opacity: parentOpacity }, parentElement: null, hasAttribute() { return false; } },
    getAttribute(name) { return name === "aria-label" ? ariaLabel : name === "aria-selected" ? selected : null; }, hasAttribute() { return false; }, getClientRects() { return visible ? [{}] : []; }, closest(selector) { return pickerContext && selector.includes('[role="menu"]') ? {} : null; },
  });
  identityAxNodes = [];
  rootIdentityElements = [rootIdentity("student@example.edu", false)];
  native.hostMessage({ type: "command", id: "identity-marker-hidden-root", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, false, "a hidden stale root marker cannot authorize an account-sensitive action");
  rootIdentityElements = [rootIdentity("student@example.edu", true), rootIdentity("student@example.edu", true)];
  native.hostMessage({ type: "command", id: "identity-marker-duplicate-root", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.find(message => message.id === "identity-marker-duplicate-root")?.result?.matches, false, JSON.stringify(native.sent.find(message => message.id === "identity-marker-duplicate-root")));
  rootIdentityElements = [rootIdentity("student@example.edu", true, "0")];
  native.hostMessage({ type: "command", id: "identity-marker-transparent-parent", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, false, "an exact marker inside a transparent ancestor is not visible");
  rootIdentityElements = [rootIdentity("student@example.edu", true)];
  native.hostMessage({ type: "command", id: "identity-marker-unique-root", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "student@example.edu" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, true, "one visible exact root marker remains a valid fast path");
  rootIdentityElements = [rootIdentity("", true, "1", "Google Account: Praket G (praketg1@gmail.com)")];
  native.hostMessage({ type: "command", id: "identity-marker-aria-label", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "Google Account: Praket G (praketg1@gmail.com)" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, true, "a unique visible account aria-label supports exact identity matching");
  rootIdentityElements = [rootIdentity("Google Account: Praket G  \n(praketg1@gmail.com)", true)];
  native.hostMessage({ type: "command", id: "identity-marker-wrapped-root", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button[data-identity-test]", expected: "Google Account: Praket G (praketg1@gmail.com)" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, true, "line wrapping in a visible account label does not break exact identity matching");
  rootIdentityElements = [rootIdentity("Google Account: Praket G (praketg1@gmail.com)", true, "1", null, true)];
  native.hostMessage({ type: "command", id: "identity-marker-picker-context", target_id: "41", method: "Controlla.checkIdentityMarker", params: { selector: "button,[role='button']", expected: "Google Account: Praket G (praketg1@gmail.com)" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.matches, false, "an exact-looking control inside an account picker cannot prove the active account");
  rootIdentityElements = [];
  identityAxNodes = [];
  frameOutcomeVerified = true;
  native.hostMessage({ type: "command", id: "oopif-click", target_id: "41", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 91, outcome: { kind: "focused" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_acknowledged, true, "frame control click is dispatched only after child-tree identity validation");
  assert.equal(native.sent.at(-1).result.verified, true, "frame click evaluates the requested postcondition before acknowledging success");
  assert.ok(debuggerCalls.some(call => call.target.sessionId === "fixture-oopif-session" && call.method === "Input.dispatchMouseEvent"), "frame click input is routed through the child target");
  const frameInputCountBeforeInvalidUrl = debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length;
  for (const [index, url] of ["/relative-story", "javascript:alert(1)", "https://user:secret@example.test/", "https://example.test:bad", "https://[:::]/", "https://[gggg::1]/", " https://example.test/", `https://example.test/${"x".repeat(2048)}`].entries()) {
    const id = `oopif-click-invalid-navigation-url-${index}`;
    native.hostMessage({ type: "command", id, target_id: "41", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 91, outcome: { kind: "navigation", url }, timeout_ms: 250 } });
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.ok(native.sent.findLast(message => message.id === id)?.error,
      `invalid navigation URL must be rejected before delivery: ${url}`);
  }
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length, frameInputCountBeforeInvalidUrl,
    "invalid exact URLs must be refused before any frame click can dispatch");
  rejectFrameMouseType = "mouseReleased";
  native.hostMessage({ type: "command", id: "oopif-click-release-error", target_id: "41", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 91, outcome: { kind: "focused" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_may_have_occurred, true, "a release failure after press reports uncertain click delivery");
  assert.equal(native.sent.at(-1).result.verified, false);
  const openRequest = { type: "command", target_id: "41", method: "Controlla.openTab", params: { url: "https://example.test/idempotent", idempotency_key: "client-fixture:operation-1234" } };
  native.hostMessage({ ...openRequest, id: "open-first" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const firstOpen = native.sent.at(-1).result;
  assert.ok(firstOpen, JSON.stringify(native.sent.at(-1)));
  assert.equal(firstOpen.idempotent_reuse, false);
  assert.equal(tabList.find(tab => tab.id === Number(firstOpen.tab_id)).active, true, "open defaults to a foreground tab");
  const countAfterFirstOpen = tabList.filter(tab => tab.url === "https://example.test/idempotent").length;
  native.hostMessage({ ...openRequest, id: "open-retry" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const retryOpen = native.sent.at(-1).result;
  assert.equal(retryOpen.tab_id, firstOpen.tab_id, "retry returns the original Controlla-created tab");
  assert.equal(retryOpen.idempotent_reuse, true);
  assert.equal(tabList.filter(tab => tab.url === "https://example.test/idempotent").length, countAfterFirstOpen,
    "repeating one idempotency key does not create another tab");
  const concurrentKey = "client-fixture:concurrent-1234";
  native.hostMessage({ ...openRequest, id: "open-concurrent-a", params: { ...openRequest.params, idempotency_key: concurrentKey, url: "https://example.test/concurrent" } });
  native.hostMessage({ ...openRequest, id: "open-concurrent-b", params: { ...openRequest.params, idempotency_key: concurrentKey, url: "https://example.test/concurrent" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const concurrent = native.sent.filter(message => ["open-concurrent-a", "open-concurrent-b"].includes(message.id));
  assert.equal(concurrent.length, 2, "both concurrent callers receive a result");
  assert.equal(concurrent[0].result.tab_id, concurrent[1].result.tab_id, "in-flight retries share one tab");
  assert.equal(tabList.filter(tab => tab.url === "https://example.test/concurrent").length, 1,
    "concurrent requests with one key create only one tab");
  native.hostMessage({ type: "command", id: "presentation-state", target_id: "41", method: "Controlla.presentationState", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.tab_active, false, "presentation telemetry reports the tab without activating it");
  assert.equal(native.sent.at(-1).result.window_focused, false, "presentation telemetry reports the window without focusing it");
  const activationEpoch = native.sent.at(-1).result.activation_epoch;
  const focusEpoch = native.sent.at(-1).result.focus_epoch;
  assert.ok(Number.isSafeInteger(activationEpoch) && Number.isSafeInteger(focusEpoch), "presentation telemetry includes transient event counters");
  afterNextDebuggerCall = () => {
    tabsActivated({ tabId: 42 }); tabsActivated({ tabId: 41 });
    windowFocusChanged(9); windowFocusChanged(-1);
  };
  native.hostMessage({ type: "command", id: "background-mutation", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  native.hostMessage({ type: "command", id: "presentation-after-transient", target_id: "41", method: "Controlla.presentationState", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(native.sent.at(-1).result.activation_epoch > activationEpoch, "tab activation then restoration remains observable");
  assert.ok(native.sent.at(-1).result.focus_epoch > focusEpoch, "window focus then restoration remains observable");
  native.hostMessage({ ...openRequest, id: "open-distinct", params: { ...openRequest.params, idempotency_key: "client-fixture:operation-5678" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.notEqual(native.sent.at(-1).result.tab_id, firstOpen.tab_id, "a distinct idempotency key creates a new tab");
  const tabCountBeforeMissingKey = tabList.length;
  native.hostMessage({ type: "command", id: "open-without-key", target_id: "41", method: "Controlla.openTab", params: { url: "https://example.test/no-key" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-1).error, /idempotency_key is required/i, "tab creation refuses a retry-unsafe request without a stable key");
  assert.equal(tabList.length, tabCountBeforeMissingKey, "a missing idempotency key cannot create a tab");
  native.hostMessage({ type: "command", id: "guarded-file", target_id: "41", method: "DOM.setFileInputFiles", params: { files: ["/tmp/example"] } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-1).error, /requires local unrestricted mode/);
  const callsBeforeBatch = debuggerCalls.length;
  native.hostMessage({ type: "batch", id: "guarded-batch", target_id: "41", deadline_ms: 1000, actions: [
    { method: "Runtime.callFunctionOn", params: { functionDeclaration: "guard" }, stop_on_not_ok: true },
    { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } },
  ] });
  await new Promise(resolve => setTimeout(resolve, 0));
  const guardedBatch = native.sent.at(-1);
  assert.equal(guardedBatch.type, "batch_result");
  assert.equal(guardedBatch.result.stopped_before, 1);
  assert.equal(debuggerCalls.length, callsBeforeBatch + 1, "a rejected read guard stops before its mutation");
  rejectNextCommand = "Input.dispatchKeyEvent";
  const callsBeforeFailedKey = debuggerCalls.length;
  native.hostMessage({ type: "batch", id: "partial-key-batch", target_id: "41", deadline_ms: 1000, actions: [
    { method: "Runtime.callFunctionOn", params: { functionDeclaration: "guard" } },
    { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } },
    { method: "Input.dispatchKeyEvent", params: { type: "char", text: "a" } },
  ] });
  await new Promise(resolve => setTimeout(resolve, 0));
  const partialKeyBatch = native.sent.at(-1);
  assert.equal(partialKeyBatch.result.completed, 1, "partial receipt counts only completed commands");
  assert.equal(partialKeyBatch.result.failed_at, 1);
  assert.equal(partialKeyBatch.result.dispatch_may_have_occurred, true,
    "failed key dispatch reports possible delivery instead of implying no input was sent");
  assert.equal(debuggerCalls.length, callsBeforeFailedKey + 2, "failed key dispatch stops the remaining batch");
  advanceClockOnNextDebuggerCall = 2;
  const callsBeforeDeadlineBatch = debuggerCalls.length;
  native.hostMessage({ type: "batch", id: "deadline-batch", target_id: "41", deadline_ms: 1, actions: [
    { method: "DOM.getDocument", params: {} },
    { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } },
  ] });
  await new Promise(resolve => setTimeout(resolve, 0));
  const expiredBatch = native.sent.at(-1);
  assert.equal(expiredBatch.type, "batch_result");
  assert.match(expiredBatch.error, /deadline/);
  assert.equal(expiredBatch.result.completed, 1);
  assert.equal(debuggerCalls.length, callsBeforeDeadlineBatch + 1,
    "an expired batch must stop before sending the next mutation");
  const mixedActions = Array.from({ length: 20 }, (_, index) => index % 2
    ? { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } }
    : { method: "DOM.getDocument", params: {} });
  const callsBeforeMixedBatch = debuggerCalls.length;
  native.hostMessage({ type: "batch", id: "mixed-batch", target_id: "41", deadline_ms: 1000, actions: mixedActions });
  await new Promise(resolve => setTimeout(resolve, 0));
  const mixedBatch = native.sent.at(-1);
  assert.equal(mixedBatch.result.completed, 20, "one bounded batch executes mixed read and mutation commands");
  assert.equal(mixedBatch.result.host_round_trips, 1);
  assert.equal(debuggerCalls.length, callsBeforeMixedBatch + 20);
  const callsBeforeStaleBatch = debuggerCalls.length;
  afterNextDebuggerCall = () => tabDocumentOverrides.set(43, "replaced-document");
  native.hostMessage({ type: "batch", id: "stale-document-batch", target_id: "43", deadline_ms: 1000, actions: [
    { method: "DOM.getDocument", params: {} },
    { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } },
  ] });
  await new Promise(resolve => setTimeout(resolve, 0));
  const staleBatch = native.sent.at(-1);
  assert.match(staleBatch.error, /document identity changed/);
  assert.equal(debuggerCalls.length, callsBeforeStaleBatch + 1, "a changed document stops the next batch mutation");
  const callsBeforeNavigationBatch = debuggerCalls.length;
  afterNextDebuggerCall = () => navigationBefore({ tabId: 47, frameId: 0, url: "https://example.test/47/changed" });
  native.hostMessage({ type: "batch", id: "navigation-batch", target_id: "47", deadline_ms: 1000, actions: [
    { method: "DOM.getDocument", params: {} },
    { method: "Input.dispatchKeyEvent", params: { type: "keyDown", key: "a" } },
  ] });
  await new Promise(resolve => setTimeout(resolve, 0));
  const navigationBatch = native.sent.at(-1);
  assert.match(navigationBatch.error, /navigating/);
  assert.equal(debuggerCalls.length, callsBeforeNavigationBatch + 1, "user navigation stops the next batch mutation");
  commandDiagnostics.splice(0);
  native.hostMessage({ type: "command", id: "open-tab", target_id: "41", method: "Controlla.openTab", params: { url: "https://www.espn.com/", idempotency_key: "fixture:open-tab-12345" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const opened = native.sent.at(-1);
  assert.equal(opened.id, "open-tab");
  assert.equal(opened.result.tab_id, "503");
  assert.equal(opened.result.url, "https://www.espn.com/");
  assert.equal(opened.result.document_id, "document-503-next");
  assert.ok(attached.includes(503), "the opened tab joins the existing native session");
  native.hostMessage({ type: "command", id: "navigate-tab", target_id: "503", method: "Controlla.navigate", params: { url: "https://www.espn.com/nfl/" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const navigated = native.sent.at(-1);
  assert.equal(navigated.result.tab_id, "503");
  assert.equal(navigated.result.url, "https://www.espn.com/nfl/");
  assert.equal(navigated.result.document_id, "document-503-next-next");
  delayedNextCommitMs = 100;
  native.hostMessage({ type: "command", id: "navigate-after-stale-ready", target_id: "503", method: "Controlla.navigate", params: { url: "https://www.espn.com/nfl/team/" } });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(native.sent.some(message => message.id === "navigate-after-stale-ready"), false,
    "paired navigation must wait for the new document, even when the prior document is already DOM-ready");
  await new Promise(resolve => setTimeout(resolve, 120));
  assert.equal(native.sent.find(message => message.id === "navigate-after-stale-ready")?.result?.document_id, "document-503-next-next-next");
  native.hostMessage({ type: "command", id: "unsafe-url", target_id: "41", method: "Controlla.openTab", params: { url: "javascript:alert(1)" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-1).error, /HTTP\(S\)/);
  failAttach.add(504);
  native.hostMessage({ type: "command", id: "failed-open", target_id: "41", method: "Controlla.openTab", params: { url: "https://example.test/failed", idempotency_key: "client-fixture:failed-open-001" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-1).error, /Tab 504 remains open/);
  assert.match(native.sent.at(-1).error, /fixture attach failure/);
  assert.ok(tabList.some(tab => tab.id === 504), "a failed open preserves the page so user edits cannot be lost");
  native.hostMessage({ type: "command", id: "failed-open-retry", target_id: "41", method: "Controlla.openTab", params: { url: "https://example.test/failed", idempotency_key: "client-fixture:failed-open-001" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.at(-1).error, /tab 504 for recovery/i, "a failed idempotent open returns its existing orphan instead of creating another tab");
  assert.equal(tabList.some(tab => tab.id === 505), false, "a failed idempotent open does not create a duplicate retry tab");
  native.hostMessage({ type: "command", id: "dom", target_id: "41", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.at(-1).method, "DOM.getDocument");
  const successTrace = commandDiagnostics.splice(0).map(([, event]) => JSON.parse(JSON.stringify(event)));
  assert.deepEqual(successTrace.map(event => event.phase), ["dispatch", "settled", "reply"]);
  assert.ok(Number.isSafeInteger(successTrace[1].requestBytes) && successTrace[1].requestBytes > 0);
  assert.ok(Number.isSafeInteger(successTrace[1].responseBytes) && successTrace[1].responseBytes > 0);
  const successSettlement = { ...successTrace[1] };
  delete successSettlement.requestBytes;
  delete successSettlement.responseBytes;
  assert.deepEqual(successSettlement, {
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
  assert.ok(tabList.some(tab => tab.id === 44), "releasing debugger control preserves the user's Chrome tab");
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

  native.hostMessage({ type: "pair", request_id: "codex-a", lease_id: "codex-a", tab_ids: [60], expected_urls: expectedUrls([60]), expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  native.hostMessage({ type: "pair", request_id: "codex-b", lease_id: "codex-b", tab_ids: [61], expected_urls: expectedUrls([61]), expected_document_ids: expectedDocuments([61]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.find(message => message.request_id === "codex-a" && message.type === "paired")?.lease_id, "codex-a");
  assert.equal(native.sent.find(message => message.request_id === "codex-b" && message.type === "paired")?.lease_id, "codex-b");
  assert.equal(detached.includes(60), false, "a second Codex lease must preserve the first lease's tab");
  native.hostMessage({ type: "release", lease_id: "codex-a" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(60), "releasing one Codex lease detaches only its tab");
  assert.equal(detached.includes(61), false, "releasing one Codex lease preserves another client's tab");
  native.hostMessage({ type: "command", lease_id: "codex-b", id: "codex-b-still-live", target_id: "61", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).id, "codex-b-still-live", "the remaining Codex lease remains usable");
  native.hostMessage({ type: "release", lease_id: "codex-b" });
  await new Promise(resolve => setTimeout(resolve, 0));

  native.disconnect();
  await new Promise(resolve => setTimeout(resolve, 0));
  const disconnectedStatus = await message({ type: "native-status" });
  assert.equal(disconnectedStatus.connected, false);
  assert.deepEqual([...disconnectedStatus.attached], []);
  assert.equal(disconnectedStatus.error, undefined);
  assert.equal(reconnectTimers.at(-1).delay, 1000, "first reconnect uses base delay");
  assert.equal(alarms.at(-1).name, "controlla-native-reconnect", "reconnect survives service worker suspension");
  reconnectTimers.at(-1).callback();
  assert.equal(nativePorts.length, 2, "disconnect reconnects the native host");
  nativePorts[1].disconnect(`Forbidden\n${"x".repeat(300)}`);
  await new Promise(resolve => setTimeout(resolve, 0));
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
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(reconnectTimers.at(-1).delay, 1000, "successful host connection resets capped backoff after ready");
  chrome.alarms.onAlarm.fire({ name: "controlla-native-reconnect" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(nativePorts.length, 4, "alarm reconnects after the service worker timer is lost");

  const oldNative = nativePorts[3];
  oldNative.hostMessage({ type: "pair", request_id: "late-pair", tab_ids: [17], expected_urls: expectedUrls([17]), expected_document_ids: expectedDocuments([17]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  oldNative.hostMessage({ type: "command", id: "late", target_id: "17", method: "Runtime.evaluate", params: { awaitResult: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  blockedDetach = { tabId: 17 };
  oldNative.disconnect();
  await new Promise(resolve => setTimeout(resolve, 0));
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

  // Local unrestricted pairing accepts numeric tab IDs without a prior discovery receipt.
  const liveNative = nativePorts.at(-1);
  liveNative.hostMessage({ type: "pair", request_id: "pair-urls-ok", tab_ids: [60], expected_urls: { 60: "https://example.test/60" }, expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "matching discovery URL pairs");
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "pair", request_id: "pair-guarded-missing", tab_ids: [66] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "guarded pairing still requires discovery identity");
  liveNative.hostMessage({ type: "pair", request_id: "pair-url-missing", tab_ids: [66], unrestricted: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "pairing without a discovery URL is allowed");
  assert.ok(attached.includes(66), "numeric tab ID attaches without a URL receipt");
  const fileCallsBefore = debuggerCalls.length;
  liveNative.hostMessage({ type: "command", id: "unrestricted-file", target_id: "66", method: "DOM.setFileInputFiles", params: { files: ["/tmp/example"] } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).id, "unrestricted-file");
  assert.equal(debuggerCalls.length, fileCallsBefore + 1, "unrestricted shared file selection reaches Chrome");
  attachOopifOnNextAutoAttach = true;
  liveNative.hostMessage({ type: "command", id: "file-frame-ax", target_id: "66", method: "Controlla.frameAccessibility", params: { frame_id: "frame-oopif" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "command", id: "file-frame-set", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.files)), [{ name: "example", size: 12 }], "frame-scoped upload sets the selected input and returns a file readback");
  assert.ok(debuggerCalls.some(call => call.method === "DOM.setFileInputFiles" && call.target.sessionId === "fixture-oopif-session"), "upload selection routes to the exact frame CDP session");
  const fileSetCalls = debuggerCalls.filter(call => call.method === "DOM.setFileInputFiles").length;
  liveNative.hostMessage({ type: "command", id: "file-frame-stale", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "stale-loader", backend_node_id: 91, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /frame navigated after the chooser opened/i);
  assert.equal(debuggerCalls.filter(call => call.method === "DOM.setFileInputFiles").length, fileSetCalls, "a chooser from an old frame document cannot select a file");
  liveNative.hostMessage({ type: "command", id: "unrestricted-unpaired", target_id: "67", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).id, "unrestricted-unpaired");
  assert.ok(attached.includes(67), "unrestricted native command attaches an unpaired tab ID");
  liveNative.hostMessage({ type: "pair", request_id: "pair-document-missing", tab_ids: [69], expected_urls: expectedUrls([69]), unrestricted: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "pairing without a discovered document ID is allowed");
  assert.ok(attached.includes(69), "numeric tab ID attaches without a document receipt");
  tabUrlOverrides.set(60, "https://example.test/navigated");
  liveNative.hostMessage({ type: "pair", request_id: "pair-urls-moved", tab_ids: [60], expected_urls: { 60: "https://example.test/60" }, expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "pair_error", "a tab that navigated since discovery refuses to pair");
  assert.match(liveNative.sent.at(-1).error, /changed during pairing/);
  assert.ok(!attached.includes(60) || detached.includes(60), "refused pairing leaves no debugger attachment");
  tabUrlOverrides.delete(60);

  // Navigation keeps the selected tab session and refreshes its document binding.
  liveNative.hostMessage({ type: "pair", request_id: "pair-nav", tab_ids: [62], expected_urls: { 62: "https://example.test/62" }, expected_document_ids: expectedDocuments([62]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  assert.ok(!detached.includes(62), "paired tab stays attached before navigation");
  tabsUpdated(62, { url: "https://example.test/62#section" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(!detached.includes(62), "same-URL navigation keeps the pairing");
  tabsUpdated(62, { url: "https://example.test/replaced" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(detached.includes(62), "a URL change without navigation lifecycle events fails closed");

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
  const priorNavigationAttachCount = attached.filter(tabId => tabId === 67).length;
  const priorNavigationDetachCount = detached.filter(tabId => tabId === 67).length;
  tabDocumentOverrides.set(67, "replacement-document-67");
  tabUrlOverrides.set(67, "https://example.test/67/story");
  navigationBefore({ tabId: 67, frameId: 0, url: "https://example.test/67/story" });
  debuggerDetached({ tabId: 67 }, "target_closed");
  navigationCommitted({ tabId: 67, frameId: 0, documentId: "replacement-document-67", url: "https://example.test/67/story" });
  navigationDomContentLoaded({ tabId: 67, frameId: 0, documentId: "replacement-document-67", url: "https://example.test/67/story" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(detached.filter(tabId => tabId === 67).length, priorNavigationDetachCount,
    "a committed navigation keeps the selected tab attached");
  assert.equal(attached.filter(tabId => tabId === 67).length, priorNavigationAttachCount + 1,
    "a debugger detach during navigation is restored only for the same selected tab");
  liveNative.hostMessage({ type: "command", id: "command-after-navigation", target_id: "67", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).id, "command-after-navigation");
  assert.equal(liveNative.sent.at(-1).error, undefined, "commands use the refreshed document identity");

  liveNative.hostMessage({ type: "pair", request_id: "slow-media-reattach", tab_ids: [68], expected_urls: expectedUrls([68]), expected_document_ids: expectedDocuments([68]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  const tab68 = tabList.find(tab => tab.id === 68);
  tab68.status = "loading";
  tabDocumentOverrides.set(68, "document-68-after-commit");
  tabUrlOverrides.set(68, "https://example.test/68/story");
  navigationBefore({ tabId: 68, frameId: 0, url: "https://example.test/68/story" });
  debuggerDetached({ tabId: 68 }, "target_closed");
  navigationCommitted({ tabId: 68, frameId: 0, documentId: "document-68-after-commit", url: "https://example.test/68/story" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 68).length, 1, "reattach waits for DOMContentLoaded instead of the late complete event");
  navigationDomContentLoaded({ tabId: 68, frameId: 0, documentId: "document-68-after-commit", url: "https://example.test/68/story" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 68).length, 2, "paired tab is usable while late media keeps Chrome loading");
  tab68.status = "complete";
  tabDocumentOverrides.delete(68);
  tabUrlOverrides.delete(68);

  liveNative.hostMessage({ type: "pair", request_id: "file-chooser", tab_ids: [66], unrestricted: true,
    expected_urls: expectedUrls([66]), expected_document_ids: expectedDocuments([66]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  liveNative.hostMessage({ type: "command", id: "page-enable-file-chooser-events", target_id: "66", method: "Page.enable", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).error, undefined, "Page.enable is allowlisted so foreground upload can receive chooser events");
  liveNative.hostMessage({ type: "command", id: "chooser-intercept", target_id: "66", method: "Page.setInterceptFileChooserDialog", params: { enabled: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).error, undefined);
  const chooserReply = liveNative.sent.length;
  liveNative.hostMessage({ type: "command", id: "chooser-wait", target_id: "66", method: "Controlla.waitForFileChooser", params: { timeout_ms: 1000 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  debuggerEvent({ tabId: 66 }, "Page.fileChooserOpened", { backendNodeId: 909, frameId: "frame-oopif", mode: "selectSingle" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.length, chooserReply + 1);
  const { sequence: chooserSequence, ...chooserResult } = JSON.parse(JSON.stringify(liveNative.sent.at(-1).result));
  assert.ok(chooserSequence > 0);
  assert.deepEqual(chooserResult, { backend_node_id: 909, frame_id: "frame-oopif", frame_loader_id: "fixture-loader", mode: "selectSingle", document_id: "document-66", root_document_id: "document-66" },
    "chooser event is correlated to the attached tab and observed document");

  fallbackFileInputDocument = { root: { nodeName: "#document", children: [
    { nodeName: "INPUT", backendNodeId: 902, attributes: ["type", "file"] },
  ] } };
  attachOopifOnNextAutoAttach = true;
  liveNative.hostMessage({ type: "command", id: "file-input-baseline", target_id: "66", method: "Controlla.fileInputSnapshot", params: {} });
  await new Promise(resolve => setTimeout(resolve, 10));
  const baseline = JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.inputs));
  assert.deepEqual(baseline, [{ frame_id: "root-frame", frame_loader_id: "root-loader", backend_node_id: 902 }], "pre-trigger snapshot binds existing file controls to their frame and loader");
  liveNative.hostMessage({ type: "command", id: "find-preexisting-input", target_id: "66", method: "Controlla.findFileInput", params: { baseline } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.match(liveNative.sent.at(-1).error, /no new file input/i, "fallback refuses a file input that existed before the trigger");
  fallbackFileInputDocument = { root: { nodeName: "#document", children: [
    { nodeName: "INPUT", backendNodeId: 902, attributes: ["type", "file"] },
    { nodeName: "INPUT", backendNodeId: 903, attributes: ["type", "file"] },
  ] } };
  liveNative.hostMessage({ type: "command", id: "find-lazy-file-input", target_id: "66", method: "Controlla.findFileInput", params: { baseline } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result)), {
    backend_node_id: 903, frame_id: "root-frame", frame_loader_id: "root-loader", document_id: "document-66",
  }, "lazy input fallback returns the sole new document-bound input");
  fallbackFileInputDocument = { root: { nodeName: "#document" } };
  fallbackFrameFileInputDocument = { root: { nodeName: "#document" } };
  liveNative.hostMessage({ type: "command", id: "file-input-frame-baseline", target_id: "66", method: "Controlla.fileInputSnapshot", params: {} });
  await new Promise(resolve => setTimeout(resolve, 10));
  const frameBaseline = JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.inputs));
  fallbackFrameFileInputDocument = { root: { nodeName: "#document", children: [{ nodeName: "INPUT", backendNodeId: 904, attributes: ["type", "file"] }] } };
  liveNative.hostMessage({ type: "command", id: "find-oopif-lazy-file-input", target_id: "66", method: "Controlla.findFileInput", params: { baseline: frameBaseline } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.at(-1).result.frame_id, "frame-oopif", "lazy input fallback discovers a unique file input in a cross-origin frame");
  assert.equal(liveNative.sent.at(-1).result.backend_node_id, 904);
  fallbackFrameFileInputDocument = undefined;
  fallbackFileInputDocument = { root: { nodeName: "#document", children: [
    { nodeName: "INPUT", backendNodeId: 905, attributes: ["type", "file"] },
    { nodeName: "INPUT", backendNodeId: 906, attributes: ["type", "file"] },
  ] } };
  liveNative.hostMessage({ type: "command", id: "find-ambiguous-file-input", target_id: "66", method: "Controlla.findFileInput", params: { baseline: [] } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.match(liveNative.sent.at(-1).error, /multiple new file inputs/i,
    "lazy input fallback refuses ambiguous file controls rather than selecting the wrong upload");
  fallbackFileInputDocument = undefined;

  identityAxNodes = [{ nodeId: "browse", role: { value: "button" }, name: { value: "Browse" }, backendDOMNodeId: 777 }];
  attachOopifOnNextAutoAttach = true;
  emitFileChooserOnFrameRelease = true;
  liveNative.hostMessage({ type: "command", id: "frame-select-file", target_id: "66", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.at(-1).result?.status, "selected", JSON.stringify(liveNative.sent.at(-1)));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.files)), [{ name: "example", size: 12 }]);
  assert.ok(debuggerCalls.some(call => call.target.sessionId === "fixture-oopif-session" && call.method === "Page.setInterceptFileChooserDialog" && call.params.enabled === true));
  assert.ok(debuggerCalls.some(call => call.target.sessionId === "fixture-oopif-session" && call.method === "Page.setInterceptFileChooserDialog" && call.params.enabled === false));
  frameActionable = false;
  const dispatchesBeforeBlockedFrame = debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length;
  liveNative.hostMessage({ type: "command", id: "frame-select-blocked", target_id: "66", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.at(-1).result?.status, "not_dispatched");
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length, dispatchesBeforeBlockedFrame);
  frameActionable = true;
  liveNative.hostMessage({ type: "pair", request_id: "same-process-upload", tab_ids: [95], unrestricted: true, expected_urls: expectedUrls([95]), expected_document_ids: expectedDocuments([95]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  emitFileChooserOnRootRelease = true;
  liveNative.hostMessage({ type: "command", id: "same-process-frame-upload", target_id: "95", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.at(-1).result?.status, "selected", JSON.stringify(liveNative.sent.at(-1)));
  const rootPress = debuggerCalls.findLast(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed");
  assert.equal(rootPress?.params.x, 140, "same-process frame click includes iframe offset and scale");
  assert.equal(rootPress?.params.y, 260);
  frameOutcomeVerified = true;
  liveNative.hostMessage({ type: "command", id: "same-process-frame-click", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: childFrameLoaderId, backend_node_id: 777, outcome: { kind: "focused" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.findLast(message => message.id === "same-process-frame-click")?.result?.verified, true);
  const rootFrameClick = debuggerCalls.findLast(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed");
  assert.equal(rootFrameClick?.params.x, 140);
  assert.equal(rootFrameClick?.params.y, 260);
  frameOutcomeVerified = false;
  navigateFrameOnClick = true;
  liveNative.hostMessage({ type: "command", id: "child-navigation-click", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: childFrameLoaderId, backend_node_id: 777, outcome: { kind: "navigation", url: "https://picker.example.test/next" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.findLast(message => message.id === "child-navigation-click")?.result?.verified, true, "child-frame navigation is checked against the child URL");
  navigateFrameOnClick = true;
  liveNative.hostMessage({ type: "command", id: "child-navigation-canonical-url", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: childFrameLoaderId, backend_node_id: 777, outcome: { kind: "navigation", url: "HTTPS://PICKER.EXAMPLE.TEST:443/next" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.findLast(message => message.id === "child-navigation-canonical-url")?.result?.verified, true, "frame navigation compares browser-canonical URL semantics");
  childFrameUrl = "https://picker.example.test/";
  fileReadbackEmpty = true;
  const uploadUrl = "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&token=fixture-secret";
  networkEventsOnEnable = [
    ["Network.requestWillBeSent", { requestId: "upload-request", type: "XHR", request: { url: uploadUrl, method: "POST" } }],
    ["Network.responseReceived", { requestId: "upload-request", type: "XHR", response: { url: uploadUrl, status: 403, statusText: "Forbidden", mimeType: "application/json" } }],
  ];
  emitFileChooserOnRootRelease = true;
  liveNative.hostMessage({ type: "command", id: "empty-frame-readback", target_id: "95", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  const emptyReadback = liveNative.sent.findLast(message => message.id === "empty-frame-readback")?.result;
  assert.equal(emptyReadback?.status, "unknown", "empty FileList cannot establish Chrome selection");
  assert.equal(emptyReadback?.network_events?.[0]?.status, 403, "failed picker upload response is retained with the selection error");
  assert.equal(emptyReadback?.network_events?.[0]?.host, "www.googleapis.com");
  assert.doesNotMatch(JSON.stringify(emptyReadback?.network_events), /fixture-secret|uploadType/);
  navigateFrameDuringFileReadback = true;
  emitFileChooserOnRootRelease = true;
  liveNative.hostMessage({ type: "command", id: "navigating-frame-upload", target_id: "95", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.findLast(message => message.id === "navigating-frame-upload")?.result?.status, "unknown", "frame navigation during file readback cannot verify selection");
  childFrameLoaderId = "fixture-loader";
  const frameReleasesBeforeOverlap = debuggerCalls.filter(call => call.target.tabId === 66 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseReleased").length;
  liveNative.hostMessage({ type: "command", id: "first-frame-upload", target_id: "66", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  for (let attempt = 0; attempt < 20 && debuggerCalls.filter(call => call.target.tabId === 66 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseReleased").length === frameReleasesBeforeOverlap; attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "command", id: "overlapping-frame-upload", target_id: "66", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/other" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /another file chooser operation/i);
  debuggerEvent({ tabId: 66, sessionId: "fixture-oopif-session" }, "Page.fileChooserOpened", { backendNodeId: 908, frameId: "frame-oopif", mode: "selectSingle" });
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.equal(liveNative.sent.findLast(message => message.id === "first-frame-upload")?.result?.status, "selected", JSON.stringify({sent:liveNative.sent.slice(-3),calls:debuggerCalls.slice(-15),frameReleasesBeforeOverlap}));
  identityAxNodes = [];

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

  liveNative.hostMessage({ type: "pair", request_id: "late-navigation-detach", tab_ids: [72],
    expected_urls: expectedUrls([72]), expected_document_ids: expectedDocuments([72]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  const lateDetachAttachCount = attached.filter(tabId => tabId === 72).length;
  tabDocumentOverrides.set(72, "replacement-document-72");
  tabUrlOverrides.set(72, "https://example.test/72/story");
  navigationBefore({ tabId: 72, frameId: 0, url: "https://example.test/72/story" });
  navigationCommitted({ tabId: 72, frameId: 0, documentId: "replacement-document-72", url: "https://example.test/72/story" });
  debuggerDetached({ tabId: 72 }, "target_closed");
  navigationDomContentLoaded({ tabId: 72, frameId: 0, documentId: "replacement-document-72", url: "https://example.test/72/story" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(!detached.includes(72), "a debugger detach delivered after commit preserves the selected pairing");
  assert.equal(attached.filter(tabId => tabId === 72).length, lateDetachAttachCount + 1,
    "a late detach restores the committed document attachment");

  liveNative.hostMessage({ type: "pair", request_id: "user-detach-during-grace", tab_ids: [73],
    expected_urls: expectedUrls([73]), expected_document_ids: expectedDocuments([73]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  navigationBefore({ tabId: 73, frameId: 0, url: "https://example.test/73/story" });
  navigationCommitted({ tabId: 73, frameId: 0, documentId: "replacement-document-73", url: "https://example.test/73/story" });
  const userDetachAttachCount = attached.filter(tabId => tabId === 73).length;
  debuggerDetached({ tabId: 73 }, "canceled_by_user");
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "command", id: "user-detach-cleared", target_id: "73", method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /not attached/);
  assert.equal(attached.filter(tabId => tabId === 73).length, userDetachAttachCount,
    "user cancellation never triggers automatic reattachment");

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
  assert.match(liveNative.sent.at(-1).error, /navigating/);
  assert.ok(!detached.includes(70), "a navigation race pauses commands without dropping the tab session");

  liveNative.hostMessage({ type: "pair", request_id: "detach-repair", tab_ids: [71],
    expected_urls: expectedUrls([71]), expected_document_ids: expectedDocuments([71]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired");
  blockedDetach = { tabId: 71 };
  navigationBefore({ tabId: 71, frameId: 0 });
  navigationError({ tabId: 71, frameId: 0, error: "net::ERR_ABORTED" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(typeof blockedDetach.resolve, "function", "failed navigation drops the tab session");
  const priorAttachments = attached.filter(tabId => tabId === 71).length;
  liveNative.hostMessage({ type: "pair", request_id: "repair-after-detach", tab_ids: [71],
    expected_urls: expectedUrls([71]), expected_document_ids: expectedDocuments([71]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 71).length, priorAttachments,
    "re-pair waits for an in-flight navigation failure detach");
  blockedDetach.resolve();
  blockedDetach = undefined;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "re-pair proceeds after the failed-navigation detach settles");

  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  let nativePairFrameStarted;
  const nativePairFrame = new Promise(resolve => { nativePairFrameStarted = resolve; });
  blockedFrame = { tabId: 73, started: nativePairFrameStarted };
  liveNative.hostMessage({ type: "pair", request_id: "busy-native-pair", tab_ids: [72, 73],
    expected_urls: { 72: tabUrlOverrides.get(72), 73: "https://example.test/73" },
    expected_document_ids: { 72: tabDocumentOverrides.get(72), 73: "document-73" } });
  await nativePairFrame;
  assert.ok(attached.includes(72), "native pair attaches its first tab before stalling on the next");
  blockedDetach = { tabId: 72 };
  navigationBefore({ tabId: 72, frameId: 0 });
  navigationError({ tabId: 72, frameId: 0, error: "net::ERR_ABORTED" });
  const attachmentsBeforePopupRepair = attached.filter(tabId => tabId === 72).length;
  const popupRepairDuringPair = await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "queued-detach", tabIds: [72] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(typeof blockedDetach.resolve, "function", "failed-navigation detach is pending while the native queue is busy");
  assert.equal(popupRepairDuringPair.ok, false, "popup repair refuses a tab with a native pairing reservation");
  assert.equal(attached.filter(tabId => tabId === 72).length, attachmentsBeforePopupRepair,
    "popup loser does not attach concurrently with native pairing");
  releaseBlockedFrame();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "stalled native pairing completes after its second frame read");
  const popupRepair = message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "queued-detach-retry", tabIds: [72] });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.filter(tabId => tabId === 72).length, attachmentsBeforePopupRepair,
    "popup retry waits for the pending native failure detach");
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

  const lostCreateBaseline = tabList.length;
  const lostCreateRequest = { type: "command", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/lost-create-ack", idempotency_key: "client-fixture:lost-create-ack-123" } };
  runBackground();
  await new Promise(resolve => setTimeout(resolve, 0));
  let restartPort = nativePorts.at(-1);
  restartPort.hostMessage({ type: "ready" });
  restartPort.hostMessage({ type: "pair", request_id: "pair-before-open-recovery", tab_ids: [41],
    expected_urls: expectedUrls([41]), expected_document_ids: expectedDocuments([41]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  loseNextCreatedTabReply = true;
  restartPort.hostMessage({ ...lostCreateRequest, id: "open-lost-create-ack" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(restartPort.sent.at(-1).error, /simulated lost tabs.create reply/);
  restartPort.disconnect("simulate worker restart after tab creation");
  runBackground();
  await new Promise(resolve => setTimeout(resolve, 0));
  restartPort = nativePorts.at(-1);
  restartPort.hostMessage({ type: "ready" });
  restartPort.hostMessage({ type: "pair", request_id: "pair-after-open-recovery-restart", tab_ids: [41],
    expected_urls: expectedUrls([41]), expected_document_ids: expectedDocuments([41]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  restartPort.hostMessage({ ...lostCreateRequest, id: "open-recover-lost-create-ack" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(tabList.length, lostCreateBaseline + 1, "a fresh service worker recovers the tab Chrome created before its reply was lost");
  assert.ok(restartPort.sent.at(-1).result, JSON.stringify(restartPort.sent.at(-1)));
  assert.equal(restartPort.sent.at(-1).result.idempotent_reuse, true);
  const conflictingKey = "client-fixture:conflict-1234";
  restartPort.hostMessage({ ...lostCreateRequest, id: "open-conflict-a", params: { ...lostCreateRequest.params, idempotency_key: conflictingKey, url: "https://example.test/conflict-a" } });
  restartPort.hostMessage({ ...lostCreateRequest, id: "open-conflict-b", params: { ...lostCreateRequest.params, idempotency_key: conflictingKey, url: "https://example.test/conflict-b" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(restartPort.sent.find(message => message.id === "open-conflict-b")?.error?.includes("different URL"),
    "an in-flight idempotency key cannot be reused with a different URL");

  slowMediaNextNavigation = true;
  restartPort.hostMessage({ type: "command", id: "open-before-slow-media", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/slow-media", idempotency_key: "client-fixture:slow-media-1234" } });
  await new Promise(resolve => setTimeout(resolve, 20));
  const slowMediaOpen = restartPort.sent.find(message => message.id === "open-before-slow-media");
  assert.ok(slowMediaOpen?.result, JSON.stringify(slowMediaOpen));
  assert.equal(tabList.find(tab => String(tab.id) === slowMediaOpen.result.tab_id)?.status, "loading",
    "open returns after DOMContentLoaded while late media is still loading");

  slowMediaNextNavigation = true;
  restartPort.hostMessage({ type: "command", id: "open-at-document-commit", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/committed", idempotency_key: "client-fixture:document-commit-1234", wait_until: "document_committed" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const committedOpen = restartPort.sent.find(message => message.id === "open-at-document-commit");
  assert.ok(committedOpen?.result, JSON.stringify(committedOpen));
  assert.equal(tabList.find(tab => String(tab.id) === committedOpen.result.tab_id)?.status, "loading",
    "document_committed readiness attaches without waiting for DOMContentLoaded or late resources");

  delayedNextCommitMs = 100;
  restartPort.hostMessage({ type: "command", id: "open-after-stale-complete", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/delayed-commit", idempotency_key: "client-fixture:delayed-commit-1234" } });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(restartPort.sent.some(message => message.id === "open-after-stale-complete"), false,
    "a stale complete status from the prior document cannot satisfy navigation readiness");
  await new Promise(resolve => setTimeout(resolve, 120));
  assert.ok(restartPort.sent.find(message => message.id === "open-after-stale-complete")?.result,
    "navigation resolves after the new document reaches DOMContentLoaded");

  const backgroundKey = "client-fixture:background-1234";
  const backgroundParams = { url: "https://example.test/background", idempotency_key: backgroundKey, active: false };
  restartPort.hostMessage({ ...openRequest, id: "open-background", params: backgroundParams });
  await new Promise(resolve => setTimeout(resolve, 0));
  const backgroundOpen = restartPort.sent.at(-1).result;
  assert.ok(backgroundOpen, JSON.stringify(restartPort.sent.at(-1)));
  assert.equal(tabList.find(tab => tab.id === Number(backgroundOpen.tab_id)).active, false, "active:false opens in the background");
  restartPort.hostMessage({ ...openRequest, id: "open-background-retry", params: backgroundParams });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(restartPort.sent.at(-1).result.tab_id, backgroundOpen.tab_id, "background retries reuse the original tab");
  assert.equal(tabList.find(tab => tab.id === Number(backgroundOpen.tab_id)).active, false, "a background retry keeps the tab inactive");
  restartPort.hostMessage({ ...openRequest, id: "open-background-mode-mismatch", params: { ...backgroundParams, active: true } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(restartPort.sent.at(-1).error, /activation mode/i, "one idempotency key cannot change activation mode");
  assert.equal(tabList.filter(tab => tab.url === backgroundParams.url).length, 1, "a conflicting retry cannot create a second tab");

  console.log("Extension version handshake, pairing cleanup, chooser event routing, and command allowlist tests passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
