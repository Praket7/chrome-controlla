const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const { webcrypto, createHash } = require("node:crypto");

function makePngBase64(byteLength = 825000, width = 320, height = 180) {
  const png = Buffer.alloc(byteLength);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(png, 0);
  png.writeUInt32BE(13, 8);
  png.write("IHDR", 12, "ascii");
  png.writeUInt32BE(width, 16);
  png.writeUInt32BE(height, 20);
  return png.toString("base64");
}

const runtimeScreenshotParams = process.env.CONTROLLA_SCREENSHOT_PARAMS_JSON
  ? JSON.parse(process.env.CONTROLLA_SCREENSHOT_PARAMS_JSON)
  : { format: "png", fromSurface: true, captureBeyondViewport: false, clip: { x: 0, y: 0, width: 320, height: 180, scale: 1 } };

const sockets = [];
const detached = [];
const attached = [];
const debuggerCalls = [];
const createdWindows = [];
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
const tabList = Array.from({ length: 102 }, (_, id) => ({ id, windowId: id < 50 ? 1 : 2, lastAccessed: id * 100, title: `Tab ${id}`.repeat(100), url: `https://example.test/${id}`, status: "complete", frozen: false }));
let focusedWindowActiveTabId = 42;
let focusedWindowFocused = false;
let pointerGuardValue = { ok: true, x: 10, y: 20, viewport: "fixture-viewport" };
let fixtureScrollTop = 20;
let fixtureScrollAllowed = true;
let fixtureScrollMoves = true;
let fixtureScrollGuardCalls = 0;
let fixtureScrollHitChangesOnRecheck = false;
let fixtureSwipeMoved = true;
let scrollToGuardReceipt = { ok: true, phase: "after", dispatch_may_have_occurred: true, target_visible: true, center_hit: true, viewport: { width: 100, height: 80 }, center: { x: 50, y: 40 } };
let changePointerAfterFirstClickRelease = false;
tabList.push({ id: 102, title: "Extension page", url: "chrome://extensions" });
tabList.push({ title: "Invalid tab", url: "https://example.test/invalid" });
let listener;
let mockServerVersion = "0.1.0";
let mockNativeError;
let nativeErrorReads = 0;
const tabUrlOverrides = new Map();
const tabDocumentOverrides = new Map();
const frameReadCounts = new Map();
const unavailableIdentityReads = new Map();
const transientIdentityAfterAttach = new Set();
const reloadDuringAttach = new Set();
const identityChangeDuringAttach = new Set();
const navigationStartDuringAttach = new Set();
const replaceDuringAttach = new Set();
const failAttach = new Set();
let tabsUpdated;
let tabsReplaced;
let tabsRemoved;
let tabsMoved;
let tabsAttached;
let tabsDetached;
const tabsActivatedListeners = new Set();
const downloadCreatedListeners = new Set();
const downloadChangedListeners = new Set();
const cancelledDownloads = [];
const emitDownloadCreated = item => { for (const callback of downloadCreatedListeners) callback(item); };
const emitDownloadChanged = delta => { for (const callback of downloadChangedListeners) callback(delta); };
const emitTabsActivated = event => { for (const callback of tabsActivatedListeners) callback(event); };
let emitActivationEvents = false;
let afterNextTabActivation;
let onNextActiveWindowQuery;
let tabsCreated;
let windowFocusChanged;
let navigationBefore;
let navigationCommitted;
let navigationDomContentLoaded;
const committedListeners = new Set();
const domContentLoadedListeners = new Set();
let slowMediaNextNavigation = false;
let delayedNextCommitMs = 0;
let overlapSameUrlNextNavigation = false;
let replacePendingOnFrameRead;
let nextWebNavigationTimeStamp = 10_000;
let navigationError;
let screenshotDataOverride;
let navigateDuringScreenshot;
let freezeDuringScreenshot;
let failPresentationEvaluation;
let debuggerDetached;
let debuggerEvent;
let nextCreatedTab = 500;
const createdTabPreviousActive = new Map();
let loseNextCreatedTabReply = false;
let mockNow = 1_000;
let advanceClockOnNextDebuggerCall = 0;
let afterNextDebuggerCall;
let attachOopifOnNextAutoAttach = false;
let attachNestedOopifParentOnNextAutoAttach = false;
let fallbackFileInputDocument;
let fallbackFrameFileInputDocument;
let identityAxNodes = [];
let rootIdentityElements = [];
let rejectFrameMouseType;
let emitFileChooserOnFrameRelease = false;
let emitFileChooserOnRootRelease = false;
let frameOutcomeVerified = false;
let nestedFrameScrollFixture = false;
let staleNestedAncestorFixture = false;
let reparentNestedFrameFixture = false;
let fixtureFrameScrollGuardCalls = 0;
let nestedFrameTreeReads = 0;
let nonAffineNestedTransformFixture = false;
let changingNestedTransformFixture = false;
let nestedOwnerBoxReads = 0;
let frameActionable = true;
let frameShadowRootFixture = false;
let frameFillMockEvidence;
let childFrameLoaderId = "fixture-loader";
let navigateFrameDuringFileReadback = false;
let fileReadbackEmpty = false;
let fileReadbackFiles;
let fileInputPolicyFixture;
let childFrameUrl = "https://picker.example.test/";
let rootFrameSecurityOrigin = "https://example.test";
let changeRootOriginOnPasswordCheck = false;
let switchTabDuringPasswordRevalidation = false;
let navigateFrameOnClick = false;
let networkEventsOnEnable = [];
let fastTypeValue = "";
let fastTypeFocused = true;
let stealFocusAfterKeydown = false;
let stealFocusAfterKeyup = false;
let blockNextHumanKeydown;
let blockNextTopologyCommand;
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
    async get(key) { return Object.fromEntries((Array.isArray(key) ? key : [key]).map(name => [name, localStorage[name]])); },
    async set(values) { Object.assign(localStorage, values); },
    async remove(key) { delete localStorage[key]; },
  }, session: {
    async get(key) { return Object.fromEntries((Array.isArray(key) ? key : [key]).map(name => [name, sessionStorage[name]])); },
    async set(values) { Object.assign(sessionStorage, values); },
    async remove(key) { delete sessionStorage[key]; },
  } },
  alarms: {
    create(name, info) { alarms.push({ name, info }); },
    onAlarm: { addListener(callback) { this.fire = callback; } },
  },
  tabs: {
    async query(query = {}) {
      if (query.active && Number.isSafeInteger(query.windowId) && onNextActiveWindowQuery) { const callback = onNextActiveWindowQuery; onNextActiveWindowQuery = undefined; callback(); }
      if (query.active && query.lastFocusedWindow) return tabList.filter(tab => tab.id === focusedWindowActiveTabId);
      if (query.active && Number.isSafeInteger(query.windowId)) return tabList.filter(tab => tab.active && tab.windowId === query.windowId);
      return tabList;
    },
    async create({ url, active = true, windowId }) { const id = nextCreatedTab++; const tab = { id, windowId: windowId ?? 1, title: "New tab", url, status: "complete", frozen: false, active }; if (active) createdTabPreviousActive.set(id, focusedWindowActiveTabId); tabList.push(tab); if (active) { for (const sibling of tabList) if (sibling.windowId === tab.windowId && sibling.id !== tab.id) sibling.active = false; tab.active = true; focusedWindowActiveTabId = tab.id; if (emitActivationEvents) emitTabsActivated({ tabId: tab.id, windowId: tab.windowId }); } if (loseNextCreatedTabReply) { loseNextCreatedTabReply = false; throw new Error("simulated lost tabs.create reply"); } return tab; },
    async remove(tabId) { const index = tabList.findIndex(tab => tab.id === tabId); if (index >= 0) { const [removed] = tabList.splice(index, 1); if (removed.active && focusedWindowActiveTabId === tabId) { const previous = tabList.find(tab => tab.id === createdTabPreviousActive.get(tabId) && tab.windowId === removed.windowId); if (previous) { for (const sibling of tabList) if (sibling.windowId === removed.windowId) sibling.active = false; previous.active = true; focusedWindowActiveTabId = previous.id; if (emitActivationEvents) emitTabsActivated({ tabId: previous.id, windowId: previous.windowId }); } } } createdTabPreviousActive.delete(tabId); },
    async move(tabId, { windowId }) {
      const tab = tabList.find(item => item.id === tabId);
      if (!tab) throw new Error("missing tab");
      tab.windowId = windowId;
      return { ...tab };
    },
    async update(tabId, { url, active }) {
      const tab = tabList.find(tab => tab.id === tabId);
      if (!tab) throw new Error("missing tab");
      if (active !== undefined) { tab.active = active; if (active) { focusedWindowActiveTabId = tabId; if (emitActivationEvents) { for (const sibling of tabList) if (sibling.windowId === tab.windowId) sibling.active = false; tab.active = true; emitTabsActivated({ tabId, windowId: tab.windowId }); } if (afterNextTabActivation) { const callback = afterNextTabActivation; afterNextTabActivation = undefined; callback(tabId); } } }
      if (url === undefined) return tab;
      const delayedCommit = delayedNextCommitMs;
      delayedNextCommitMs = 0;
      const priorUrl = tabUrlOverrides.get(tabId) || tab.url;
      const priorDocumentId = tabDocumentOverrides.get(tabId) || `document-${tabId}`;
      tab.url = url;
      const currentNavigationStart = nextWebNavigationTimeStamp++;
      navigationBefore?.({ tabId, frameId: 0, url, timeStamp: currentNavigationStart });
      if (overlapSameUrlNextNavigation) {
        overlapSameUrlNextNavigation = false;
        tab.status = "loading";
        const newerNavigationStart = nextWebNavigationTimeStamp++;
        navigationBefore?.({ tabId, frameId: 0, url, timeStamp: newerNavigationStart });
        const staleDocumentId = `${priorDocumentId}-stale-same-url`;
        tabUrlOverrides.set(tabId, url);
        tabDocumentOverrides.set(tabId, staleDocumentId);
        const lateCommitEventTime = newerNavigationStart + 2;
        assert.ok(lateCommitEventTime > newerNavigationStart, "the old commit callback fires after the newer navigation start callback");
        navigationCommitted?.({ tabId, frameId: 0, documentId: staleDocumentId, url, timeStamp: lateCommitEventTime });
        navigationDomContentLoaded?.({ tabId, frameId: 0, documentId: staleDocumentId, url, timeStamp: lateCommitEventTime + 1 });
        return tab;
      }
      tab.status = delayedCommit ? "complete" : slowMediaNextNavigation ? "loading" : "complete";
      const documentId = `${priorDocumentId}-next`;
      if (delayedCommit) {
        tabUrlOverrides.set(tabId, priorUrl);
        setTimeout(() => {
          tabUrlOverrides.set(tabId, url);
          tabDocumentOverrides.set(tabId, documentId);
          navigationCommitted?.({ tabId, frameId: 0, documentId, url, timeStamp: nextWebNavigationTimeStamp++ });
          navigationDomContentLoaded?.({ tabId, frameId: 0, documentId, url });
        }, delayedCommit);
        return tab;
      }
      tabUrlOverrides.set(tabId, url);
      tabDocumentOverrides.set(tabId, documentId);
      navigationCommitted?.({ tabId, frameId: 0, documentId, url, timeStamp: nextWebNavigationTimeStamp++ });
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
    onMoved: { addListener(callback) { tabsMoved = callback; } },
    onAttached: { addListener(callback) { tabsAttached = callback; } },
    onDetached: { addListener(callback) { tabsDetached = callback; } },
    onReplaced: { addListener(callback) { tabsReplaced = callback; } },
    onRemoved: { addListener(callback) { tabsRemoved = callback; } },
    onActivated: { addListener(callback) { tabsActivatedListeners.add(callback); }, removeListener(callback) { tabsActivatedListeners.delete(callback); } },
    onCreated: { addListener(callback) { tabsCreated = callback; } },
  },
  windows: {
    async get(windowId) { return { id:windowId??1, focused:focusedWindowFocused }; },
    async update(windowId, { focused }) { if (focused !== undefined) focusedWindowFocused = focused; return { id: windowId, focused: !!focused }; },
    async create(options) {
      createdWindows.push(options);
      const windowId = 900 + createdWindows.length;
      const tab = await chrome.tabs.create({ url: options.url, active: true, windowId });
      return { id: windowId, focused: options.focused, tabs: [tab] };
    },
    onFocusChanged: { addListener(callback) { windowFocusChanged = callback; } }
  },
  webNavigation: {
    async getFrame({ tabId, frameId }) {
      frameReadCounts.set(tabId, (frameReadCounts.get(tabId) || 0) + 1);
      if (frameId !== 0 || !tabList.some(tab => tab.id === tabId)) return undefined;
      const unavailableReads = unavailableIdentityReads.get(tabId) || 0;
      if (unavailableReads > 0) {
        unavailableIdentityReads.set(tabId, unavailableReads - 1);
        return undefined;
      }
      if (blockedFrame?.tabId === tabId) {
        const wait = blockedFrame;
        blockedFrame = undefined;
        wait.started();
        await new Promise(resolve => { releaseBlockedFrame = resolve; });
      }
      if (replacePendingOnFrameRead?.tabId === tabId) {
        replacePendingOnFrameRead.remaining--;
        if (replacePendingOnFrameRead.remaining === 0) {
          const replace = replacePendingOnFrameRead.replace;
          replacePendingOnFrameRead = undefined;
          replace();
        }
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
      if (transientIdentityAfterAttach.delete(tabId)) unavailableIdentityReads.set(tabId, 5);
      if (reloadDuringAttach.delete(tabId)) {
        tabsUpdated(tabId, { status: "loading" });
        tabsUpdated(tabId, { status: "complete" });
      }
      if (identityChangeDuringAttach.delete(tabId)) tabDocumentOverrides.set(tabId, `interloper-document-${tabId}`);
      if (navigationStartDuringAttach.delete(tabId)) {
        navigationBefore({ tabId, frameId: 0, url: "https://www.espn.com/nfl/other-story/" });
        navigationDomContentLoaded({ tabId, frameId: 0, documentId: tabDocumentOverrides.get(tabId) });
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
      if (method === "Runtime.evaluate" && blockNextTopologyCommand) {
        const blocked = blockNextTopologyCommand;
        blockNextTopologyCommand = undefined;
        blocked.entered();
        await blocked.release;
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && changePointerAfterFirstClickRelease) {
        changePointerAfterFirstClickRelease = false;
        pointerGuardValue = { ...pointerGuardValue, x: pointerGuardValue.x + 1 };
      }
      if (method === "Page.captureScreenshot") {
        if (freezeDuringScreenshot) {
          freezeDuringScreenshot = false;
          const tab = tabList.find(item => item.id === target.tabId);
          tab.frozen = true;
          tabsUpdated(target.tabId, { frozen: true });
        }
        if (navigateDuringScreenshot) {
          const { tabId, documentId, url } = navigateDuringScreenshot;
          navigateDuringScreenshot = undefined;
          tabDocumentOverrides.set(tabId, documentId);
          tabUrlOverrides.set(tabId, url);
          navigationCommitted?.({ tabId, frameId: 0, documentId, url, timeStamp: nextWebNavigationTimeStamp++ });
        }
        return { data: screenshotDataOverride || "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB" };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(index,expected,prepare,normalize,requireFocus)")) {
        const [index, expected, prepare] = params.arguments.map(argument => argument.value);
        if (prepare) fastTypeFocused = true;
        return { result: { value: { ok: fastTypeFocused && fastTypeValue === expected, reason: fastTypeFocused ? "stale_value" : "focus_or_selection_changed", value: fastTypeValue, rich: false } } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(index,scrollRequest)")) {
        assert.equal(params.arguments?.[1]?.value, "scroll-to-target-v1");
        return { result: { value: structuredClone(scrollToGuardReceipt) } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("__fixturePointerGuard")) {
        return { result: { value: pointerGuardValue } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("nearest_scroll_container_at_boundary") && !params?.functionDeclaration?.startsWith("function(dx,dy,expectedViewport,expectedContainer)")) {
        fixtureScrollGuardCalls += 1;
        return { result: { type: "object", objectId: `fixture-scroll-descriptor-${fixtureScrollGuardCalls}` } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(dx,dy,expectedViewport,expectedContainer)")) {
        fixtureFrameScrollGuardCalls += 1;
        return { result: { type: "object", objectId: `fixture-frame-scroll-descriptor-${fixtureFrameScrollGuardCalls}` } };
      }
      if (method === "Runtime.getProperties" && params?.objectId?.startsWith("fixture-frame-scroll-descriptor-")) {
        const property = (name, value) => ({ name, value: typeof value === "object" ? value : { type: typeof value, value } });
        return { result: [property("ok", true), property("x", 25), property("y", 20), property("width", 100), property("height", 60), property("viewport", "100x60@1"), property("left", 0), property("top", fixtureScrollTop), property("container", { type: "object", subtype: "node", objectId: "fixture-frame-scroll-container" })] };
      }
      if (method === "Runtime.callFunctionOn" && params?.objectId === "fixture-frame-scroll-container") {
        return { result: { value: { connected: true, left: 0, top: fixtureScrollTop } } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(index,sx,sy,ex,ey,expectedViewport,phase)")) {
        const [, sx, sy, ex, ey, viewport, phase] = params.arguments.map(argument => argument.value);
        return { result: { value: { ok: true, viewport, start: { x: sx, y: sy }, end: { x: ex, y: ey },
          scroll: { top: phase === "after" && fixtureSwipeMoved ? 40 : 0 }, rect: { left: 0, top: 0, width: 100, height: 100 } } } };
      }
      if (method === "Input.dispatchTouchEvent" && params?.type === "touchMove" && fixtureSwipeMoved) fixtureScrollTop += 20;
      if (method === "Runtime.getProperties" && params?.objectId?.startsWith("fixture-scroll-descriptor-")) {
        const guardCall = Number(params.objectId.split("-").at(-1));
        const property = (name, value) => ({ name, value: typeof value === "object" ? value : { type: typeof value, value } });
        return { result: [property("ok", fixtureScrollAllowed), property("x", 15 + (fixtureScrollHitChangesOnRecheck && guardCall % 2 === 0 ? 1 : 0)), property("y", 25), property("viewport", "fixture-viewport"),
          property("left", 0), property("top", fixtureScrollTop), property("container", { type: "object", subtype: "node", objectId: "fixture-scroll-container" })] };
      }
      if (method === "Runtime.callFunctionOn" && params?.objectId === "fixture-scroll-container") {
        return { result: { value: { connected: true, left: 0, top: fixtureScrollTop } } };
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseWheel" && fixtureScrollMoves) fixtureScrollTop += params.deltaY;
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.startsWith("function(expected,value){const e=this;const blocked")) {
        const [expected, value] = params.arguments.map(argument => argument.value);
        if (frameFillMockEvidence) return { result: { value: frameFillMockEvidence } };
        return { result: { value: { dispatch_may_have_occurred: true, verified: true, matches_requested_value: true,
          length: value.length, reason_code: null, expected_was_checked: typeof expected === "string" } } };
      }
      if (method === "DOM.resolveNode" && params?.backendNodeId === 990) return { object: { objectId: "fixture-password-object" } };
      if (method === "Runtime.callFunctionOn" && params?.objectId === "fixture-password-object") {
        if (params.functionDeclaration.includes("credential_filled")) return { result: { value: { ok: true, credential_filled: true, length: params.arguments?.[0]?.value?.length || 0 } } };
        if (switchTabDuringPasswordRevalidation) { switchTabDuringPasswordRevalidation = false; await chrome.tabs.update(42, { active: true }); }
        if (changeRootOriginOnPasswordCheck) { changeRootOriginOnPasswordCheck = false; rootFrameSecurityOrigin = "https://attacker.example"; }
        return { result: { value: { ok: true } } };
      }
      if (method === "Input.dispatchKeyEvent") {
        if (params.type === "keyDown" && blockNextHumanKeydown) {
          const blocked = blockNextHumanKeydown;
          blockNextHumanKeydown = undefined;
          blocked.entered();
          await blocked.release;
        }
        if (params.type === "keyDown" && stealFocusAfterKeydown) { stealFocusAfterKeydown = false; fastTypeFocused = false; }
        if (params.type === "keyUp" && stealFocusAfterKeyup) { stealFocusAfterKeyup = false; fastTypeFocused = false; }
        if (params.type === "char") fastTypeValue += params.text;
        if (params.type === "keyDown" && params.key === "Enter") fastTypeValue += "\n";
        return {};
      }
      if (method === "Input.insertText") { fastTypeValue += params.text; return {}; }
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
      if (method === "Runtime.evaluate" && params?.expression?.includes("__controllaPresentationState")) {
        if (failPresentationEvaluation) { const failure = failPresentationEvaluation; failPresentationEvaluation = undefined; throw new Error(failure); }
        return { result: { value: { visibility_state: "visible", document_hidden: false, document_has_focus: true,
          viewport: { width: 1280, height: 720, device_scale_factor: 2 }, lifecycle: "active" } } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("__controllaFrameOutcome")) {
        return { result: { value: frameOutcomeVerified } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("__controllaFrameActionable")) {
        assert.ok(params.functionDeclaration.includes("root=root.host.getRootNode()"), "frame action guard validates the open shadow-root chain to its owning document");
        assert.ok(params.functionDeclaration.includes("shadow.elementFromPoint?.(x,y)"), "frame action guard hit-tests through open shadow roots");
        return { result: { value: frameActionable ? { x: 20, y: 30, viewport: { width: 100, height: 100 } } : false } };
      }
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("document.elementFromPoint(x,y)===this")) return { result: { value: true } };
      if (method === "Runtime.callFunctionOn" && params?.functionDeclaration?.includes("ownerDocument.defaultView.innerWidth")) {
        const dimensions = { "fixture-frame-owner": { width: 800, height: 600 }, "fixture-parent-owner": { width: 100, height: 100 }, "fixture-nested-owner": { width: 200, height: 150 } };
        return { result: { value: dimensions[params.objectId] || { width: 100, height: 100 } } };
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === rejectFrameMouseType) {
        rejectFrameMouseType = undefined;
        throw new Error("fixture mouse dispatch failure");
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && target.sessionId === "fixture-oopif-session" && emitFileChooserOnFrameRelease) {
        emitFileChooserOnFrameRelease = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId, sessionId: target.sessionId }, "Page.fileChooserOpened", { backendNodeId: 908, frameId: "frame-oopif", mode: "selectSingle" }));
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && !target.sessionId && (emitFileChooserOnRootRelease || emitFileChooserOnFrameRelease)) {
        emitFileChooserOnRootRelease = false;
        emitFileChooserOnFrameRelease = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId }, "Page.fileChooserOpened", { backendNodeId: 908, frameId: "frame-oopif", mode: "selectSingle" }));
      }
      if (method === "Input.dispatchMouseEvent" && params?.type === "mouseReleased" && navigateFrameOnClick) { navigateFrameOnClick = false; childFrameUrl = "https://picker.example.test/next"; }
      if (method === "Page.getFrameTree") {
        if (nestedFrameScrollFixture) {
          nestedFrameTreeReads += 1;
          const parentFrameId = reparentNestedFrameFixture && nestedFrameTreeReads > 1 ? "frame-other-parent" : "frame-parent";
          return { frameTree: { frame: { id: "root-frame", loaderId: "root-loader", url: "https://example.test/root", securityOrigin: rootFrameSecurityOrigin }, childFrames: [{ frame: { id: parentFrameId, loaderId: staleNestedAncestorFixture && nestedFrameTreeReads > 1 ? "changed-parent-loader" : "parent-loader", url: "https://example.test/parent", securityOrigin: "https://example.test" }, childFrames: [{ frame: { id: "frame-nested", loaderId: "nested-loader", url: "https://example.test/nested", securityOrigin: "https://example.test" } }] }] } };
        }
        return { frameTree: { frame: { id: "root-frame", loaderId: "root-loader", url: "https://example.test/root", securityOrigin: rootFrameSecurityOrigin }, childFrames: [{ frame: { id: "frame-oopif", loaderId: childFrameLoaderId, url: childFrameUrl, securityOrigin: "https://frame.example" } }] } };
      }
      if (method === "DOM.getDocument" && target.sessionId === "fixture-oopif-session") return fallbackFrameFileInputDocument || { root: { nodeName: "#document" } };
      if (method === "DOM.getDocument" && fallbackFileInputDocument) return fallbackFileInputDocument;
      if (method === "Target.setAutoAttach" && params?.autoAttach && attachOopifOnNextAutoAttach) {
        attachOopifOnNextAutoAttach = false;
        queueMicrotask(() => debuggerEvent?.({ tabId: target.tabId }, "Target.attachedToTarget", { sessionId: "fixture-oopif-session", targetInfo: { type: "iframe", targetId: "frame-oopif", url: "https://picker.example.test/" } }));
      }
      if (method === "Target.setAutoAttach" && params?.autoAttach && attachNestedOopifParentOnNextAutoAttach) {
        attachNestedOopifParentOnNextAutoAttach = false;
        queueMicrotask(() => {
          debuggerEvent?.({ tabId: target.tabId }, "Target.attachedToTarget", { sessionId: "fixture-parent-oopif-session", targetInfo: { type: "iframe", targetId: "frame-parent", url: "https://frame.example.test/" } });
          debuggerEvent?.({ tabId: target.tabId }, "Target.attachedToTarget", { sessionId: "fixture-nested-oopif-session", targetInfo: { type: "iframe", targetId: "frame-nested", url: "https://nested.example.test/" } });
        });
      }
      if (method === "Accessibility.getFullAXTree" && ["fixture-oopif-session", "fixture-nested-oopif-session"].includes(target.sessionId)) {
        return { nodes: identityAxNodes.length ? identityAxNodes : [{ nodeId: "1", role: { value: "option" }, name: { value: "School account" }, backendDOMNodeId: 91 }] };
      }
      if (method === "Accessibility.getFullAXTree" && ["frame-oopif", "frame-nested"].includes(params?.frameId)) return { nodes: identityAxNodes };
      if (method === "DOM.getFrameOwner" && params?.frameId === "frame-parent") return { backendNodeId: 998 };
      if (method === "DOM.getFrameOwner" && params?.frameId === "frame-nested") return { backendNodeId: 997 };
      if (method === "DOM.getFrameOwner") return { backendNodeId: 999 };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 999) return { model: { content: [100, 200, 300, 200, 300, 400, 100, 400] } };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 998) return { model: { content: [50, 60, 250, 60, 250, 210, 50, 210] } };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 997) {
        nestedOwnerBoxReads += 1;
        if (nonAffineNestedTransformFixture) return { model: { content: [20, 30, 120, 30, 130, 90, 20, 90] } };
        const shift = changingNestedTransformFixture && nestedOwnerBoxReads >= 2 ? 12 : 0;
        return { model: { content: [20 + shift, 30, 120 + shift, 30, 120 + shift, 90, 20 + shift, 90] } };
      }
      if (method === "DOM.getBoxModel" && target.sessionId === "fixture-oopif-session") return { model: { content: [10, 20, 30, 20, 30, 40, 10, 40] } };
      if (method === "DOM.getBoxModel" && params?.backendNodeId === 777) return { model: { content: [10, 20, 30, 20, 30, 40, 10, 40] } };
      if (method === "DOM.resolveNode") return { object: { objectId: params?.backendNodeId === 990 ? "fixture-password-object" : params?.backendNodeId === 999 ? "fixture-frame-owner" : params?.backendNodeId === 998 ? "fixture-parent-owner" : params?.backendNodeId === 997 ? "fixture-nested-owner" : "fixture-file-object" } };
      if (method === "DOM.setFileInputFiles" && target.sessionId === "fixture-oopif-session") {
        fileReadbackFiles = (params?.files || []).map(path => ({ name: String(path).split("/").at(-1), size: 12 }));
        return {};
      }
      if (method === "Runtime.callFunctionOn" && params?.objectId === "fixture-file-object") {
        if (params?.functionDeclaration?.startsWith("function(names){const e=this")) return { result: { value: fileInputPolicyFixture || { ok: true, accept: [], multiple: true } } };
        if (navigateFrameDuringFileReadback) { navigateFrameDuringFileReadback = false; childFrameLoaderId = "navigated-loader"; }
        if (fileReadbackEmpty) { fileReadbackEmpty = false; return { result: { value: [] } }; }
        return { result: { value: fileReadbackFiles || [{ name: "example", size: 12 }] } };
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
  downloads: {
    onCreated: { addListener(callback) { downloadCreatedListeners.add(callback); } },
    onChanged: { addListener(callback) { downloadChangedListeners.add(callback); } },
    async cancel(downloadId) { cancelledDownloads.push(downloadId); },
  },
};
const backgroundSource = fs.readFileSync(new URL("./background.js", `file://${__filename}`), "utf8");
const extensionBuildAssets = ["background.js", "manifest.json", "open-recovery.html", "popup.html", "popup.js", "secure-input.css", "secure-input.html", "secure-input.js"];
const extensionAssetBytes = new Map(extensionBuildAssets.map(name => [name, fs.readFileSync(new URL(`./${name}`, `file://${__filename}`))]));
const expectedExtensionDigest = createHash("sha256");
for (const name of extensionBuildAssets) expectedExtensionDigest.update(name).update(Buffer.from([0])).update(extensionAssetBytes.get(name)).update(Buffer.from([0]));
const expectedExtensionBuildId = `controlla-chrome-extension:0.1.0:${expectedExtensionDigest.digest("hex")}`;
let backgroundContext;
function runBackground() {
  backgroundContext = {
    chrome, WebSocket: FakeWebSocket, URL, TextEncoder, Uint8Array, DataView, atob, Set, Number, Array, String, Error, Promise, JSON,
    Date: { now() { return mockNow; } }, crypto: webcrypto,
    fetch: async url => {
      const assetName = extensionBuildAssets.find(name => String(url).endsWith(`/${name}`));
      const bytes = assetName ? extensionAssetBytes.get(assetName) : undefined;
      return { ok: !!bytes, async arrayBuffer() { return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); } };
    },
    performance: { now() { return mockNow; } },
    console: { debug(...args) { commandDiagnostics.push(args); } },
    setTimeout(callback, delay) {
      if (delay === 10) return globalThis.setTimeout(callback, delay);
      reconnectTimers.push({ callback, delay });
      return reconnectTimers.length;
    },
    clearTimeout,
  };
  vm.runInNewContext(`${backgroundSource}\nglobalThis.__testHooks = { pendingNavigationMatchesDocument, pendingMainFrameNavigations, uploadInputPolicyFunction, frameActionableGuardFunction, extensionBuildId: CONTROLLA_EXTENSION_BUILD_ID_PROMISE };`, backgroundContext);
}
runBackground();

function testOpenShadowFrameActionGuard() {
  const guard = vm.runInNewContext(`(${backgroundContext.__testHooks.frameActionableGuardFunction()})`);
  const doc = { defaultView: { innerWidth: 100, innerHeight: 80, getComputedStyle: () => ({
    display: "block", visibility: "visible", pointerEvents: "auto", opacity: "1"
  }) } };
  const target = {
    ownerDocument: doc, isConnected: true, disabled: false, parentElement: null,
    getBoundingClientRect() { return { left: 10, top: 12, right: 40, bottom: 32, width: 30, height: 20 }; },
    getRootNode() { return shadow; }, getAttribute() { return null; }, hasAttribute() { return false; }, contains(node) { return node === this; }
  };
  const host = {
    shadowRoot: null, parentElement: null, disabled: false,
    getRootNode() { return doc; }, getAttribute() { return null; }, hasAttribute() { return false; }
  };
  const shadow = { host, elementFromPoint(x, y) { assert.equal(x, 25); assert.equal(y, 22); return target; } };
  host.shadowRoot = shadow;
  doc.elementFromPoint = () => host;
  const observed = guard.call(target);
  assert.equal(observed.x, 25, "open-shadow hit test retains the exact child-frame point");
  assert.equal(observed.y, 22);

  target.getRootNode = () => ({ host: { getRootNode: () => ({ host: { getRootNode: () => ({}) } }) } });
  assert.equal(guard.call(target), false, "shadow ancestry beyond the bounded root chain fails closed");
}
testOpenShadowFrameActionGuard();

function message(payload) {
  return new Promise(resolve => listener(payload, {}, resolve));
}
function messageFrom(payload, sender) { return new Promise(resolve => listener(payload, sender, resolve)); }
async function waitFor(predicate, description) {
  const deadline = Date.now() + 1000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${description}`);
    await new Promise(resolve => setTimeout(resolve, 0));
  }
}

(async () => {
  assert.equal(await backgroundContext.__testHooks.extensionBuildId, expectedExtensionBuildId,
    "extension computes the same deterministic digest over every packaged runtime asset");
  const filePolicy = new Function(`return (${backgroundContext.__testHooks.uploadInputPolicyFunction()})`)();
  const inputPolicy = (accept, multiple, names) => filePolicy.call({
    nodeName: "INPUT", type: "file", multiple,
    getAttribute: name => name === "accept" ? accept : null,
  }, names);
  assert.equal(inputPolicy(".bin", false, ["acceptance-fixture.bin"]).ok, true,
    "extension accept tokens match the complete filename suffix");
  assert.equal(inputPolicy("application/pdf", false, ["report.pdf"]).ok, true,
    "MIME accept tokens match a known filename MIME type");
  assert.equal(inputPolicy("image/*", false, ["picture.png"]).ok, true,
    "wildcard MIME accept tokens match a known image type");
  assert.equal(inputPolicy(".bin", false, ["malware.exe"]).reason, "accept_filter_mismatch",
    "a file outside the accept filter is refused");
  assert.equal(inputPolicy("", false, ["one.bin", "two.bin"]).reason, "multiple_files_not_allowed",
    "multi-file selection is refused when the input lacks the multiple attribute");
  await new Promise(resolve => setTimeout(resolve, 0));
  const nativeStatus = await message({ type: "native-status" });
  assert.equal(nativeStatus.transport, "native", "the background worker registers its message handler using supported Chrome events");
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
    for (let attempt = 2; attempt <= 12; attempt++) {
      reconnectTimers.at(-1).callback();
      candidate = nativePorts.at(-1);
      candidate.disconnect("native host unavailable");
      await new Promise(resolve => setTimeout(resolve, 0));
      const status = await message({ type: "native-status" });
      assert.equal(status.retry_exhausted, false, "native reconnect must keep retrying after the delay cap");
    }
    const retrying = await message({ type: "native-status" });
    assert.equal(retrying.retry_exhausted, false, "a temporary host outage must not require an extension reload");
    assert.equal(reconnectTimers.at(-1).delay, 60000, "reconnect delay remains capped after repeated failures");
    const portsBeforeAlarm = nativePorts.length;
    chrome.alarms.onAlarm.fire({ name: "controlla-native-reconnect" });
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(nativePorts.length, portsBeforeAlarm + 1, "alarm retries after worker suspension even after many failures");
    return;
  }
  native.hostMessage({ type: "list_tabs", request_id: "list-1" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const listed = native.sent[0];
  assert.equal(listed.type, "tabs");
  assert.equal(listed.request_id, "list-1");
  assert.equal(listed.tabs.length, 100, "tab listing is capped");
  assert.equal(listed.truncated, true, "tab listing reports when eligible tabs were omitted");
  assert.equal(listed.last_focused_window_id, 1, "tab inventory identifies the last-focused Chrome window");
  assert.ok(listed.tabs.every(tab => Number.isSafeInteger(tab.id) && /^https?:\/\//.test(tab.url)));
  assert.ok(listed.tabs.every(tab => typeof tab.document_id === "string"));
  assert.ok(listed.tabs.every(tab => tab.title.length <= 256 && tab.url.length <= 2048));
  assert.equal(listed.tabs.find(tab => tab.id === 42).active_for_open, true,
    "the active tab in Chrome's last-focused window is marked as the open anchor");
  assert.equal(listed.tabs.find(tab => tab.id === 41).active_for_open, false,
    "other stable tabs cannot win the default open anchor");
  assert.equal(listed.tabs.find(tab => tab.id === 41).window_id, 1);
  assert.equal(listed.tabs.find(tab => tab.id === 41).last_accessed, 4100);
  const focusedBeyondCap = { id: 104, windowId: 3, lastAccessed: 10400, title: "Focused late tab", url: "https://example.test/focused-late", status: "complete" };
  tabList.push(focusedBeyondCap);
  focusedWindowActiveTabId = focusedBeyondCap.id;
  native.hostMessage({ type: "list_tabs", request_id: "list-focused-beyond-cap" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const focusedInventory = native.sent.at(-1);
  assert.equal(focusedInventory.truncated, true);
  assert.equal(focusedInventory.last_focused_window_id, 3);
  assert.equal(focusedInventory.tabs.length, 100);
  assert.equal(focusedInventory.tabs.find(tab => tab.id === focusedBeyondCap.id)?.active_for_open, true,
    "the active stable tab must survive the inventory cap even when Chrome lists it after 100 tabs");
  assert.equal(new Set(focusedInventory.tabs.map(tab => tab.id)).size, focusedInventory.tabs.length,
    "prioritizing the active tab must not duplicate it in the bounded inventory");
  tabList.pop();
  focusedWindowActiveTabId = 42;
  const unstableActive = { id: 103, windowId: 1, title: "New Tab", url: "about:blank", status: "complete" };
  tabList.push(unstableActive);
  focusedWindowActiveTabId = unstableActive.id;
  native.hostMessage({ type: "list_tabs", request_id: "list-unstable-active" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const stableFallbackInventory = native.sent.at(-1);
  assert.equal(stableFallbackInventory.last_focused_window_id, 1);
  assert.equal(stableFallbackInventory.tabs.some(tab => tab.active_for_open), false,
    "an active about:blank tab must not be misrepresented as a stable open anchor");
  assert.ok(stableFallbackInventory.tabs.some(tab => tab.window_id === 1 && tab.document_id),
    "the focused window still exposes stable tabs that can anchor a background open");
  tabList.pop();
  focusedWindowActiveTabId = 42;
  assert.deepEqual(attached, [], "listing tabs does not attach any tab");
  native.hostMessage({ type: "pair", request_id: "pair-1", tab_ids: [41, 43, 47], expected_urls: expectedUrls([41, 43, 47]), expected_document_ids: expectedDocuments([41, 43, 47]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(attached, [41, 43, 47], "native pairing attaches exactly the requested tabs");
  assert.deepEqual([...new Set(attached)], [41, 43, 47], "native pairing never attaches all tabs");
  assert.deepEqual(debuggerCalls, [], "pairing must not dispatch commands");
  assert.equal(JSON.stringify(native.sent.find(message => message.request_id === "pair-1")), JSON.stringify({ type: "paired", request_id: "pair-1", lease_id: "pair-1", targets: ["41", "43", "47"], extension_version: "0.1.0", extension_build_id: expectedExtensionBuildId, protocol_version: 2, document_identity: true, batch_execution: true, batch_deadline: true }));
  const discardedTab = tabList.find(tab => tab.id === 41);
  discardedTab.discarded = true;
  const captureCountBeforeDiscarded = debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length;
  native.hostMessage({ type: "command", id: "discarded-screenshot", target_id: "41", method: "Page.captureScreenshot", params: runtimeScreenshotParams });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).reason_code, "target_discarded");
  assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length, captureCountBeforeDiscarded, "discarded target is rejected before screenshot dispatch");
  delete discardedTab.discarded;
  discardedTab.frozen = true;
  native.hostMessage({ type: "command", id: "frozen-screenshot", target_id: "41", method: "Page.captureScreenshot", params: runtimeScreenshotParams });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).reason_code, "target_frozen");
  assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length, captureCountBeforeDiscarded, "frozen target is refused before screenshot dispatch");
  discardedTab.frozen = undefined;
  native.hostMessage({ type: "command", id: "unknown-lifecycle-screenshot", target_id: "41", method: "Page.captureScreenshot", params: runtimeScreenshotParams });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).reason_code, "target_lifecycle_unknown", "missing frozen-state evidence fails closed");
  assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length, captureCountBeforeDiscarded, "unknown lifecycle is refused before screenshot dispatch");
  discardedTab.frozen = false;
  const waitForCommandResult = async id => {
    for (let attempt = 0; attempt < 100; attempt++) {
      const message = native.sent.findLast(item => item.id === id);
      if (message) return message;
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    throw new Error(`Timed out waiting for command result ${id}`);
  };
  let topologyCommandEntered, releaseTopologyCommand;
  const topologyCommandBlocked = new Promise(resolve => { topologyCommandEntered = resolve; });
  const topologyCommandRelease = new Promise(resolve => { releaseTopologyCommand = resolve; });
  blockNextTopologyCommand = { entered: topologyCommandEntered, release: topologyCommandRelease };
  native.hostMessage({ type: "command", id: "paired-window-move-inflight", target_id: "41", method: "Runtime.evaluate", params: { expression: "1", returnByValue: true } });
  await topologyCommandBlocked;
  tabsDetached(41, { oldWindowId: 1, oldPosition: 0 });
  tabList.find(tab => tab.id === 41).windowId = 9;
  tabsAttached(41, { newWindowId: 9, newPosition: 0 });
  releaseTopologyCommand();
  const movedCommand = await waitForCommandResult("paired-window-move-inflight");
  assert.equal(movedCommand.result.status, "unknown", JSON.stringify(movedCommand));
  assert.equal(movedCommand.result.reason_code, "tab_topology_changed");
  assert.equal(movedCommand.result.dispatch_may_have_occurred, true);
  let reorderedCommandEntered, releaseReorderedCommand;
  const reorderedCommandBlocked = new Promise(resolve => { reorderedCommandEntered = resolve; });
  const reorderedCommandRelease = new Promise(resolve => { releaseReorderedCommand = resolve; });
  blockNextTopologyCommand = { entered: reorderedCommandEntered, release: reorderedCommandRelease };
  native.hostMessage({ type: "command", id: "paired-tab-reorder-inflight", target_id: "41", method: "Runtime.evaluate", params: { expression: "2", returnByValue: true } });
  await reorderedCommandBlocked;
  tabsMoved(41, { windowId: 9, fromIndex: 0, toIndex: 2 });
  releaseReorderedCommand();
  const reorderedCommand = await waitForCommandResult("paired-tab-reorder-inflight");
  assert.equal(reorderedCommand.result.status, "unknown", JSON.stringify(reorderedCommand));
  assert.equal(reorderedCommand.result.reason_code, "tab_topology_changed");
  screenshotDataOverride = makePngBase64();
  native.hostMessage({ type: "command", id: 97001, target_id: "41", method: "Page.captureScreenshot", params: runtimeScreenshotParams });
  await new Promise(resolve => setTimeout(resolve, 25));
  const screenshotChunks = native.sent.filter(reply => reply.type === "result_chunk" && reply.id === 97001);
  assert.equal(screenshotChunks.length, 3, `large screenshot responses are split into bounded native-message chunks: ${JSON.stringify(native.sent.filter(reply => reply.id === 97001))}`);
  assert.deepEqual(screenshotChunks.map(reply => reply.index), [0, 1, 2]);
  assert.ok(screenshotChunks.every(reply => new TextEncoder().encode(JSON.stringify(reply)).byteLength < 900 * 1024));
  assert.equal(new Set(screenshotChunks.map(reply => reply.transfer_id)).size, 1);
  assert.ok(screenshotChunks.every(reply => reply.base64_sha256 === screenshotChunks[0].base64_sha256));
  assert.ok(screenshotChunks.every(reply => reply.visual_evidence?.document_id === "document-41"), "every screenshot chunk carries its target document identity");
  assert.ok(screenshotChunks.every(reply => reply.visual_evidence?.image_sha256 === screenshotChunks[0].visual_evidence.image_sha256), "chunked pixels share one integrity-bound visual revision");
  const screenshotBytes = Buffer.from(screenshotDataOverride, "base64");
  assert.equal(screenshotChunks[0].visual_evidence.image_sha256, createHash("sha256").update(screenshotBytes).digest("hex"), "image hash covers decoded PNG bytes");
  assert.equal(screenshotChunks[0].visual_evidence.dimensions.width, 320, "dimensions come from the PNG header");
  assert.notEqual(screenshotChunks[0].visual_evidence.image_sha256, createHash("sha256").update(screenshotDataOverride).digest("hex"), "image hash is distinct from the base64 transfer hash");
  assert.equal(screenshotChunks[0].base64_sha256, createHash("sha256").update(screenshotDataOverride).digest("hex"), "chunk hash covers the exact base64 transfer payload");
  assert.ok(Number.isSafeInteger(screenshotChunks[0].visual_evidence?.captured_at_ms), "screenshot evidence carries a capture timestamp");
  assert.deepEqual(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").at(-1)?.params, runtimeScreenshotParams,
    "the exact runtime-generated screenshot parameters reach the extension's bounded validator and Chrome dispatch");
  assert.equal(screenshotChunks[0].visual_evidence.target_lifecycle, "active");
  assert.equal(screenshotChunks[0].visual_evidence.render_revision, null, "pixel integrity is not mislabeled as renderer freshness");
  screenshotDataOverride = undefined;
  const capturesBeforeTransition = debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length;
  freezeDuringScreenshot = true;
  native.hostMessage({ type: "command", id: "freeze-during-screenshot", target_id: "41", method: "Page.captureScreenshot", params: runtimeScreenshotParams });
  const frozenDuringCapture = await waitForCommandResult("freeze-during-screenshot");
  assert.equal(frozenDuringCapture.reason_code, "target_frozen", JSON.stringify(frozenDuringCapture));
  assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length, capturesBeforeTransition + 1, "transition fixture freezes after screenshot dispatch");
  assert.equal(native.sent.some(reply => reply.type === "result_chunk" && reply.id === "freeze-during-screenshot"), false, "pixels are discarded after an observed freeze transition");
  discardedTab.frozen = false;
  tabsUpdated(41, { frozen: false });
  for (const [id, clip, captureBeyondViewport, description] of [
    [97003, { x: 0, y: 0, width: 4096, height: 4096, scale: 1 }, false, "maximum crop"],
    [97004, { x: 0, y: 0, width: 800, height: 600, scale: 2 }, false, "high-DPI crop"],
    [97005, { x: 0, y: 0, width: 1200, height: 3000, scale: 1 }, true, "bounded full-page crop"],
  ]) {
    native.hostMessage({ type: "command", id, target_id: "41", method: "Page.captureScreenshot", params: {
      format: "png", fromSurface: true, captureBeyondViewport, clip
    } });
    await new Promise(resolve => setTimeout(resolve, 25));
    assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").at(-1)?.params.clip.scale, clip.scale,
      `${description} remains inside the shared screenshot contract`);
    assert.ok(native.sent.some(reply => reply.type === "result" && reply.id === id), `${description} returns a result`);
  }
  const screenshotsBeforeOversize = debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length;
  native.hostMessage({ type: "command", id: 97006, target_id: "41", method: "Page.captureScreenshot", params: {
    format: "png", fromSurface: true, captureBeyondViewport: true,
    clip: { x: 0, y: 0, width: 1200, height: 5000, scale: 1 }
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.filter(call => call.method === "Page.captureScreenshot").length, screenshotsBeforeOversize,
    "oversized full-page captures are refused before Chrome dispatch");
  screenshotDataOverride = makePngBase64();
  native.hostMessage({ type: "command", id: 97002, target_id: "41", lease_id: "pair-1", method: "Page.captureScreenshot", params: {
    format: "png", fromSurface: true, captureBeyondViewport: false, clip: { x: 0, y: 0, width: 320, height: 180, scale: 1 }
  } });
  native.hostMessage({ type: "cancel_screenshot", id: 97002, lease_id: "pair-1" });
  await new Promise(resolve => setTimeout(resolve, 25));
  assert.ok(native.sent.some(reply => reply.type === "result_chunk_abort" && reply.id === 97002),
    "canceled screenshot chunk delivery emits an explicit transfer abort for immediate native cleanup");
  assert.equal(native.sent.some(reply => reply.type === "result_chunk" && reply.id === 97002), false,
    "a canceled screenshot transfer emits no image chunks");
  screenshotDataOverride = undefined;
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
  fastTypeValue = ""; fastTypeFocused = true; stealFocusAfterKeydown = true;
  const humanEventsBeforeFocusTheft = debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length;
  native.hostMessage({ type: "command", id: "human-type-character-focus-theft", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "", character: "x", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const humanFocusTheft = native.sent.find(message => message.id === "human-type-character-focus-theft")?.result;
  assert.equal(humanFocusTheft.status, "unknown");
  assert.equal(humanFocusTheft.reason, "focus_changed_keyup_suppressed_cleanup_unconfirmed");
  assert.deepEqual(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").slice(humanEventsBeforeFocusTheft).map(call => call.params.type), ["keyDown"],
    "single-character bridge transaction must suppress char and stale-target keyUp after focus theft");
  fastTypeValue = ""; fastTypeFocused = true;
  native.hostMessage({ type: "command", id: "human-type-character-cleanup", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "", character: "y", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const humanCleanup = native.sent.find(message => message.id === "human-type-character-cleanup")?.result;
  assert.equal(humanCleanup.status, "verified", JSON.stringify(humanCleanup));
  assert.deepEqual(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").slice(-3).map(call => call.params.type), ["keyDown", "char", "keyUp"],
    "same-target character transaction must complete and release its pressed key before replying");
  fastTypeValue = ""; fastTypeFocused = true;
  const unicodeKeyEventsBefore = debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length;
  native.hostMessage({ type: "command", id: "human-type-unicode-scalar", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "", character: "é", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const unicodeHumanReceipt = native.sent.find(message => message.id === "human-type-unicode-scalar")?.result;
  assert.equal(unicodeHumanReceipt.status, "verified", JSON.stringify(unicodeHumanReceipt));
  assert.equal(unicodeHumanReceipt.input_method, "insertText");
  assert.equal(unicodeHumanReceipt.readback, "é");
  assert.equal(debuggerCalls.filter(call => call.method === "Input.insertText").at(-1)?.params?.text, "é");
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length, unicodeKeyEventsBefore,
    "Unicode insertText transaction must not leave any key down or send a later keyUp");
  const callsBeforeControl = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "human-type-control-rejected", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "é", character: "\u0001", normalize_paragraphs: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(native.sent.find(message => message.id === "human-type-control-rejected")?.error);
  assert.equal(debuggerCalls.length, callsBeforeControl, "unsupported control scalar is rejected before CDP dispatch");
  fastTypeValue = ""; fastTypeFocused = true;
  let enteredHumanKeydown;
  let releaseHumanKeydown;
  const humanKeydownEntered = new Promise(resolve => { enteredHumanKeydown = resolve; });
  const humanKeydownRelease = new Promise(resolve => { releaseHumanKeydown = resolve; });
  blockNextHumanKeydown = { entered: enteredHumanKeydown, release: humanKeydownRelease };
  const humanCancelEventsStart = debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").length;
  let cancellationRequested = false, cancellationAcknowledged = false;
  const readNativeResult = async id => {
    for (let attempt = 0; attempt < 100; attempt++) {
      const message = native.sent.findLast(item => item.id === id);
      if (message) return message.result;
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    throw new Error(`Timed out waiting for native result ${id}`);
  };
  const pacedOperation = (async () => {
    native.hostMessage({ type: "command", id: "human-cancel-char-a", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "", character: "a", normalize_paragraphs: false } });
    const firstCharacter = await readNativeResult("human-cancel-char-a");
    if (cancellationRequested) { cancellationAcknowledged = true; return [firstCharacter]; }
    native.hostMessage({ type: "command", id: "human-cancel-char-b", target_id: "41", method: "Controlla.humanTypeCharacter", params: { object_id: "fixture-editor", index: 0, expected: "a", character: "b", normalize_paragraphs: false } });
    return [firstCharacter, await readNativeResult("human-cancel-char-b")];
  })();
  await humanKeydownEntered;
  cancellationRequested = true;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(cancellationAcknowledged, false, "cancellation acknowledgement waits for the actual extension transaction cleanup");
  releaseHumanKeydown();
  const cancelledCharacters = await pacedOperation;
  assert.equal(cancellationAcknowledged, true);
  assert.equal(cancelledCharacters.length, 1, "the queued second character must not be submitted after cancellation");
  assert.equal(cancelledCharacters[0].status, "verified");
  assert.deepEqual(debuggerCalls.filter(call => call.method === "Input.dispatchKeyEvent").slice(humanCancelEventsStart).map(call => call.params.type), ["keyDown", "char", "keyUp"],
    "the in-flight extension transaction finishes key release before cancellation acknowledgement");
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
  const openerBoundCandidate = native.sent.find(message => message.id === "verify-click-new-tab")?.result?.tabs?.[0];
  assert.deepEqual({ tab_id: openerBoundCandidate?.tab_id, url: openerBoundCandidate?.url, window_id: openerBoundCandidate?.window_id,
    opener_tab_id: openerBoundCandidate?.opener_tab_id, opener_match: openerBoundCandidate?.opener_match },
    { tab_id: "900", url: "https://example.test/story", window_id: 7, opener_tab_id: "41", opener_match: true },
    "only Chrome's opener relationship attributes a tab to the click, excluding same-URL tabs opened independently");
  assert.ok(Number.isSafeInteger(openerBoundCandidate?.created_at_ms), "candidate includes its Chrome creation event timestamp");
  native.hostMessage({ type: "command", id: "verify-click-new-tab-no-url", target_id: "41", method: "Controlla.newTabs", params: { since_ms: clickStartedAt } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const noUrlCandidate = native.sent.find(message => message.id === "verify-click-new-tab-no-url")?.result?.tabs?.[0];
  assert.equal(noUrlCandidate?.tab_id, "900", "without a selected href, only Chrome's opener relationship is used");
  assert.equal(noUrlCandidate?.created_at_ms, openerBoundCandidate?.created_at_ms, "candidate creation timestamp is stable across bounded re-observation");
  const ambiguousPopupTab = { id: 903, windowId: 7, openerTabId: 41, url: "https://example.test/second", status: "complete" };
  tabList.push(ambiguousPopupTab); tabsCreated(ambiguousPopupTab);
  native.hostMessage({ type: "command", id: "verify-popup-adoption-ambiguity", target_id: "41", method: "Controlla.newTabs", params: { since_ms: clickStartedAt } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.find(message => message.id === "verify-popup-adoption-ambiguity")?.result?.tabs?.length, 2,
    "the extension preserves every recent opener-bound candidate so runtime adoption can reject ambiguity");
  tabsRemoved(903); tabList.splice(tabList.findIndex(tab => tab.id === 903), 1);
  tabsRemoved(900);
  tabList.splice(tabList.findIndex(tab => tab.id === 900), 1);
  native.hostMessage({ type: "command", id: "verify-closed-popup-not-reused", target_id: "41", method: "Controlla.newTabs", params: { since_ms: clickStartedAt } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(native.sent.find(message => message.id === "verify-closed-popup-not-reused")?.result?.tabs)), [],
    "closed popup/opener candidates are removed from the short-lived target lifecycle registry");
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
  assert.equal(native.sent.at(-1).result.dispatch_acknowledged, true, `frame control click is dispatched only after child-tree identity validation: ${JSON.stringify(native.sent.at(-1).result)}`);
  assert.equal(native.sent.at(-1).result.verified, true, "frame click evaluates the requested postcondition before acknowledging success");
  const rootFrameInput = debuggerCalls.find(call => call.target.tabId === 41 && !call.target.sessionId && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed");
  assert.ok(rootFrameInput, "frame click input is routed through the root viewport coordinate space");
  assert.deepEqual([rootFrameInput.params.x, rootFrameInput.params.y], [140, 260], "OOPIF-local coordinates are translated through the frame owner into root viewport coordinates");
  const frameMouseCallsBeforeTouch = debuggerCalls.filter(call => call.target.tabId === 41 && !call.target.sessionId && call.method === "Input.dispatchMouseEvent").length;
  const frameTouchCallsBeforeTouch = debuggerCalls.filter(call => call.target.tabId === 41 && !call.target.sessionId && call.method === "Input.dispatchTouchEvent").length;
  native.hostMessage({ type: "command", id: "oopif-touch", target_id: "41", method: "Controlla.frameClick", params: {
    frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 91, outcome: { kind: "focused" }, input_mode: "touch", timeout_ms: 250,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.input_mode, "touch");
  assert.equal(native.sent.at(-1).result.verified, true);
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 41 && !call.target.sessionId && call.method === "Input.dispatchTouchEvent").length - frameTouchCallsBeforeTouch, 2,
    "frame touch tap dispatches one touchStart and one touchEnd");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 41 && !call.target.sessionId && call.method === "Input.dispatchMouseEvent").length, frameMouseCallsBeforeTouch,
    "frame touch never falls back to mouse input");
  focusedWindowActiveTabId = 41;
  focusedWindowFocused = true;
  const nextCreatedBeforeSecureInput = nextCreatedTab;
  tabList.find(tab => tab.id === 41).active = true;
  const passwordSentinel = "one-use-password-sentinel";
  native.hostMessage({ type: "command", id: "secure-password", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, label: "School account password", timeout_ms: 20000,
  } });
  for (let attempt = 0; attempt < 50 && !tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")); attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  const secureTab = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
  assert.ok(secureTab, `password input uses a one-use extension-owned page: ${JSON.stringify(native.sent.find(item => item.id === "secure-password"))}`);
  const secureNonce = secureTab.url.split("#")[1];
  assert.equal((await messageFrom({ type: "secure-input-ready", nonce: secureNonce }, { url: `chrome-extension://fixture/secure-input.html.evil#${secureNonce}` })).ok, false,
    "lookalike extension page cannot read or submit a pending secret");
  const securePromptContext = await messageFrom({ type: "secure-input-ready", nonce: secureNonce }, { url: secureTab.url });
  assert.equal(securePromptContext.ok, true);
  assert.equal(securePromptContext.origin, "https://example.test");
  assert.equal(securePromptContext.label, "School account password");
  assert.equal((await messageFrom({ type: "secure-input-submit", nonce: secureNonce, password: passwordSentinel }, { url: secureTab.url })).ok, true);
  await new Promise(resolve => setTimeout(resolve, 0));
  const secureReceipt = native.sent.find(item => item.id === "secure-password")?.result;
  assert.equal(secureReceipt?.status, "verified", "fresh password field receives one value and verifies using redacted evidence");
  assert.equal(secureReceipt?.readback?.credential_filled, true);
  assert.equal(Object.hasOwn(secureReceipt?.readback || {}, "length"), false, "receipt does not disclose password length");
  assert.equal(JSON.stringify(secureReceipt).includes(passwordSentinel), false, "password is excluded from the browser receipt");
  assert.equal(commandDiagnostics.flat(Infinity).join(" ").includes(passwordSentinel), false, "password is excluded from extension trace diagnostics");
  assert.equal(JSON.stringify(sessionStorage).includes(passwordSentinel), false, "password is never persisted in extension session storage");
  assert.equal(tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")), false, "one-use prompt tab is closed");
  assert.equal(focusedWindowActiveTabId, 41, "original target tab is restored after secure entry");
  emitActivationEvents = true;
  focusedWindowActiveTabId = 41;
  const secureTargetWindowId = tabList.find(tab => tab.id === 41).windowId;
  for (const tab of tabList) if (tab.windowId === secureTargetWindowId) tab.active = tab.id === 41;
  focusedWindowFocused = true;
  const switchDispatchStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "secure-password-user-switch", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, timeout_ms: 20000,
  } });
  for (let attempt = 0; attempt < 50 && !tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")); attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  const switchedPrompt = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
  assert.ok(switchedPrompt, "secure input prompt opens before the user switches tabs");
  await chrome.tabs.update(42, { active: true });
  const switchedNonce = switchedPrompt.url.split("#")[1];
  await messageFrom({ type: "secure-input-submit", nonce: switchedNonce, password: "switch-must-not-send-this" }, { url: switchedPrompt.url });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.slice(switchDispatchStart).some(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.includes("credential_filled")), false,
    "switching to another tab during secure entry prevents password dispatch");
  assert.equal(focusedWindowActiveTabId, 42, "secure-entry cleanup preserves the user's newer tab choice");
  assert.equal(tabList.find(tab => tab.id === 41).active, false, "cleanup does not reactivate the original target after user interference");
  assert.equal(tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")), false, "interrupted one-use prompt is still closed");
  focusedWindowActiveTabId = 41;
  for (const tab of tabList) if (tab.windowId === secureTargetWindowId) tab.active = tab.id === 41;
  const revalidationDispatchStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "secure-password-revalidation-switch", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, timeout_ms: 20000,
  } });
  for (let attempt = 0; attempt < 50 && !tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")); attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  const revalidationPrompt = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
  assert.ok(revalidationPrompt, "secure prompt opens before password-field revalidation");
  switchTabDuringPasswordRevalidation = true;
  await messageFrom({ type: "secure-input-submit", nonce: revalidationPrompt.url.split("#")[1], password: "revalidation-switch-must-not-send" }, { url: revalidationPrompt.url });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.slice(revalidationDispatchStart).some(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.includes("credential_filled")), false,
    "a tab switch during field revalidation is checked at the password dispatch boundary");
  assert.equal(focusedWindowActiveTabId, 42, "closing the now-inactive prompt preserves a tab selected during revalidation");
  assert.equal(tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")), false, "revalidation interruption still closes the prompt");
  focusedWindowActiveTabId = 41;
  for (const tab of tabList) if (tab.windowId === secureTargetWindowId) tab.active = tab.id === 41;
  const ownershipRaceDispatchStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "secure-password-activation-race", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, timeout_ms: 20000,
  } });
  for (let attempt = 0; attempt < 50 && !tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")); attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  const ownershipRacePrompt = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
  assert.ok(ownershipRacePrompt, "secure prompt opens before activation ownership is checked");
  onNextActiveWindowQuery = () => { void chrome.tabs.update(42, { active: true }); void chrome.tabs.update(ownershipRacePrompt.id, { active: true }); };
  await messageFrom({ type: "secure-input-submit", nonce: ownershipRacePrompt.url.split("#")[1], password: "activation-race-must-not-send" }, { url: ownershipRacePrompt.url });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.slice(ownershipRaceDispatchStart).some(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.includes("credential_filled")), false,
    "activation away and back during the ownership query invalidates the one-use prompt");
  assert.equal(tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")), false, "activation-raced prompt still closes");
  emitActivationEvents = false;
  focusedWindowActiveTabId = 41;
  for (const tab of tabList) if (tab.windowId === secureTargetWindowId) tab.active = tab.id === 41;
  focusedWindowFocused = true;
  const createdTabsBeforeChatPassword = nextCreatedTab;
  const debuggerCallsBeforeChatPassword = debuggerCalls.length;
  const chatPasswordSentinel = "CHAT-PASSWORD-TEST-SENTINEL";
  native.hostMessage({ type: "command", id: "secure-password-chat-value", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, label: "School account password", timeout_ms: 20000,
    password: chatPasswordSentinel,
  } });
  for (let attempt = 0; attempt < 50 && !native.sent.some(item => item.id === "secure-password-chat-value"); attempt++) {
    const prompt = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
    if (prompt) await messageFrom({ type: "secure-input-submit", nonce: prompt.url.split("#")[1], password: chatPasswordSentinel }, { url: prompt.url });
    await new Promise(resolve => setTimeout(resolve, 0));
  }
  const chatPasswordReceipt = native.sent.find(item => item.id === "secure-password-chat-value")?.result;
  assert.equal(chatPasswordReceipt?.status, "verified", `chat-provided password is dispatched once and verified: ${JSON.stringify(chatPasswordReceipt)}`);
  assert.equal(chatPasswordReceipt?.readback?.credential_filled, true);
  assert.equal(Object.hasOwn(chatPasswordReceipt?.readback || {}, "length"), false, "receipt does not expose password length");
  assert.equal(JSON.stringify(chatPasswordReceipt).includes(chatPasswordSentinel), false, "chat secret is excluded from the receipt");
  assert.equal(commandDiagnostics.flat(Infinity).join(" ").includes(chatPasswordSentinel), false, "chat secret is excluded from extension diagnostics");
  assert.equal(nextCreatedTab, createdTabsBeforeChatPassword, "chat-provided password does not open a second password prompt");
  const chatPasswordDispatch = debuggerCalls.slice(debuggerCallsBeforeChatPassword).find(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.includes("credential_filled"));
  assert.equal(chatPasswordDispatch?.params.arguments?.[0]?.value, chatPasswordSentinel, "the one-shot value reaches only the guarded password-field dispatch");
  rootFrameSecurityOrigin = "https://example.test";
  changeRootOriginOnPasswordCheck = true;
  const originMismatchCallStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "secure-password-origin-change", target_id: "41", method: "Controlla.securePasswordFill", params: {
    frame_id: "root-frame", loader_id: "root-loader", backend_node_id: 990, timeout_ms: 20000,
  } });
  for (let attempt = 0; attempt < 50 && !tabList.some(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#")); attempt++) await new Promise(resolve => setTimeout(resolve, 0));
  const originPrompt = tabList.find(tab => tab.url?.startsWith("chrome-extension://fixture/secure-input.html#"));
  assert.ok(originPrompt, "origin binding is rechecked after the secure prompt");
  const originNonce = originPrompt.url.split("#")[1];
  await messageFrom({ type: "secure-input-submit", nonce: originNonce, password: "origin-bound-sentinel" }, { url: originPrompt.url });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(debuggerCalls.slice(originMismatchCallStart).some(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.includes("credential_filled")), false,
    "a frame that changes origin during secure entry receives no password");
  rootFrameSecurityOrigin = "https://example.test";
  focusedWindowActiveTabId = 42;
  focusedWindowFocused = false;
  tabList.find(tab => tab.id === 41).active = false;
  nextCreatedTab = nextCreatedBeforeSecureInput;
  identityAxNodes = [{ nodeId: "field", role: { value: "textbox" }, name: { value: "Search" }, backendDOMNodeId: 92 }];
  const frameFillCallStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "oopif-fill", target_id: "41", method: "Controlla.frameFill", params: {
    frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 92, expected_value: "old", value: "private-frame-value", timeout_ms: 1000,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const frameFillResult = native.sent.at(-1).result;
  assert.equal(frameFillResult.verified, true, "frame fill requires structured in-frame readback");
  assert.equal(frameFillResult.matches_requested_value, true);
  assert.ok(debuggerCalls.slice(frameFillCallStart).some(call => call.target.sessionId === "fixture-oopif-session" && call.method === "Runtime.callFunctionOn"), "frame fill is evaluated in the owning child target");
  assert.ok(!JSON.stringify(frameFillResult).includes("private-frame-value"), "frame fill receipts never echo field contents");
  frameFillMockEvidence = { dispatch_may_have_occurred: false, verified: false, matches_requested_value: false, reason_code: "stale_value" };
  native.hostMessage({ type: "command", id: "oopif-fill-stale-value", target_id: "41", method: "Controlla.frameFill", params: {
    frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 92, expected_value: "stale", value: "must-not-overwrite", timeout_ms: 1000,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_may_have_occurred, false, "a changed field value remains a pre-dispatch refusal");
  assert.equal(native.sent.at(-1).result.reason_code, "stale_value");
  frameFillMockEvidence = undefined;
  identityAxNodes = [{ nodeId: "field", role: { value: "button" }, name: { value: "Not a text field" }, backendDOMNodeId: 92 }];
  const frameFillCallsBeforeWrongRole = debuggerCalls.filter(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.startsWith("function(expected,value){const e=this;const blocked")).length;
  native.hostMessage({ type: "command", id: "oopif-fill-wrong-role", target_id: "41", method: "Controlla.frameFill", params: {
    frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 92, expected_value: "old", value: "must-not-dispatch", timeout_ms: 1000,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_may_have_occurred, false, "non-text frame controls are rejected before evaluation");
  assert.equal(debuggerCalls.filter(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.startsWith("function(expected,value){const e=this;const blocked")).length, frameFillCallsBeforeWrongRole);
  identityAxNodes = [{ nodeId: "field", role: { value: "textbox" }, name: { value: "Search" }, backendDOMNodeId: 92 }];
  rejectNextCommand = "Runtime.callFunctionOn";
  native.hostMessage({ type: "command", id: "oopif-fill-uncertain", target_id: "41", method: "Controlla.frameFill", params: {
    frame_id: "frame-oopif", loader_id: "fixture-loader", backend_node_id: 92, expected_value: "old", value: "delivery-uncertain", timeout_ms: 1000,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_may_have_occurred, true, "lost fill acknowledgement is reported as uncertain, never safe to replay");
  assert.equal(native.sent.at(-1).result.verified, false);
  const frameFillDispatchesBeforeStale = debuggerCalls.filter(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.startsWith("function(expected,value){const e=this;const blocked")).length;
  native.hostMessage({ type: "command", id: "oopif-fill-stale-loader", target_id: "41", method: "Controlla.frameFill", params: {
    frame_id: "frame-oopif", loader_id: "stale-loader", backend_node_id: 92, expected_value: "old", value: "must-not-dispatch", timeout_ms: 1000,
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.dispatch_may_have_occurred, false, "a changed frame loader is rejected before dispatch");
  assert.equal(debuggerCalls.filter(call => call.method === "Runtime.callFunctionOn" && call.params?.functionDeclaration?.startsWith("function(expected,value){const e=this;const blocked")).length, frameFillDispatchesBeforeStale);
  identityAxNodes = [];
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
  tabList.find(tab => tab.id === Number(firstOpen.tab_id)).active = false;
  focusedWindowFocused = false;
  const activateRequest = { type: "command", target_id: "41", method: "Controlla.activateOwnedTab", params: { owned_tab_id: firstOpen.tab_id, idempotency_key: "activate-fixture-01" } };
  native.hostMessage({ ...activateRequest, id: "activate-created-tab" });
  await new Promise(resolve => setTimeout(resolve, 0));
  const activated = native.sent.find(message => message.id === "activate-created-tab")?.result;
  assert.equal(activated?.status, "activated", JSON.stringify(native.sent.find(message => message.id === "activate-created-tab")));
  assert.equal(activated?.topology_verified, true);
  assert.equal(activated?.tab_active, true);
  assert.equal(activated?.window_focused, true);
  native.hostMessage({ ...activateRequest, id: "activate-created-tab-retry" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.find(message => message.id === "activate-created-tab-retry")?.result?.idempotent_reuse, true,
    "repeated owned activation returns verified idempotent evidence");
  native.hostMessage({ type: "command", id: "activate-user-tab", target_id: "41", method: "Controlla.activateOwnedTab", params: { owned_tab_id: "41", idempotency_key: "activate-user-01" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.find(message => message.id === "activate-user-tab")?.error || "", /no completed Controlla-owned open receipt/,
    "activate refuses a user-owned tab without a persisted Controlla receipt");
  focusedWindowFocused = false;
  native.hostMessage({ type: "command", id: "close-user-tab", target_id: "41", method: "Controlla.closeOwnedTab", params: { owned_tab_id: "41" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.find(message => message.id === "close-user-tab")?.error || "", /no Controlla-owned open receipt/,
    "close_owned refuses any tab without a persisted Controlla open receipt");
  native.hostMessage({ type: "command", id: "close-created-tab", target_id: "41", method: "Controlla.closeOwnedTab", params: { owned_tab_id: firstOpen.tab_id } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const closedCreatedTab = native.sent.find(message => message.id === "close-created-tab")?.result;
  assert.equal(closedCreatedTab?.status, "closed");
  assert.equal(closedCreatedTab?.topology_verified, true);
  assert.equal(tabList.some(tab => String(tab.id) === firstOpen.tab_id), false,
    "close_owned verifies that Chrome no longer lists the created tab");
  native.hostMessage({ type: "command", id: "close-created-tab-retry", target_id: "41", method: "Controlla.closeOwnedTab", params: { owned_tab_id: firstOpen.tab_id } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const repeatedClose = native.sent.find(message => message.id === "close-created-tab-retry");
  assert.equal(repeatedClose?.result?.idempotent_reuse, true,
    `repeating close_owned verifies the already-closed lifecycle without affecting another tab: ${JSON.stringify(repeatedClose)}`);
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
  assert.equal(native.sent.at(-1).result.visibility_state, "visible", "DOM visibility is reported independently from Chrome tab/window focus");
  assert.equal(native.sent.at(-1).result.document_has_focus, true, "page focus is reported as a distinct signal, not used as window focus");
  assert.equal(JSON.stringify(native.sent.at(-1).result.viewport), JSON.stringify({ width: 1280, height: 720, device_scale_factor: 2 }), "presentation includes the page capture geometry");
  assert.equal(native.sent.at(-1).result.lifecycle, "active", "presentation reports the explicit unfrozen tab lifecycle separately from window and page focus");
  const activationEpoch = native.sent.at(-1).result.activation_epoch;
  const focusEpoch = native.sent.at(-1).result.focus_epoch;
  assert.ok(Number.isSafeInteger(activationEpoch) && Number.isSafeInteger(focusEpoch), "presentation telemetry includes transient event counters");
  failPresentationEvaluation = "Target crashed while reading renderer presentation state";
  native.hostMessage({ type: "command", id: "presentation-crashed-renderer", target_id: "41", method: "Controlla.presentationState", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.lifecycle, "crashed", "recognized debugger crash errors are not reported as an unknown live renderer");
  assert.equal(native.sent.at(-1).result.renderer_unavailable, true);
  focusedWindowFocused = true;
  tabList.find(tab => tab.id === 42).windowId = tabList.find(tab => tab.id === 41).windowId;
  tabList.find(tab => tab.id === 42).active = true;
  tabList.find(tab => tab.id === 41).active = false;
  emitActivationEvents = true;
  // Exercise the guarded native route with a foreground owned fixture target.
  tabList.find(tab => tab.id === 42).active = false;
  tabList.find(tab => tab.id === 41).active = true;
  pointerGuardValue = { ok: true, x: 10, y: 20, viewport: "fixture-viewport" };
  const doubleSuccessStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "double-pointer-click-success", target_id: "41", method: "Controlla.pointerClick", params: {
    object_id: "fixture-target", function_declaration: "function(index,expected){return __fixturePointerGuard}",
    index: 0, outcome: { kind: "focused" }, viewport: "fixture-viewport", click_kind: "double_click"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const successfulDoubleEvents = debuggerCalls.slice(doubleSuccessStart).filter(call => call.target?.tabId === 41 && call.method === "Input.dispatchMouseEvent");
  assert.deepEqual(successfulDoubleEvents.map(call => [call.params.type, call.params.button, call.params.clickCount]), [
    ["mouseMoved", undefined, undefined], ["mousePressed", "left", 1], ["mouseReleased", "left", 1],
    ["mousePressed", "left", 2], ["mouseReleased", "left", 2],
  ], "double click is two trusted guarded mouse pairs with distinct click counts");
  pointerGuardValue = { ok: true, x: 10, y: 20, viewport: "fixture-viewport" };
  changePointerAfterFirstClickRelease = true;
  const doubleStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "double-pointer-click-changed", target_id: "41", method: "Controlla.pointerClick", params: {
    object_id: "fixture-target", function_declaration: "function(index,expected){return __fixturePointerGuard}",
    index: 0, outcome: { kind: "focused" }, viewport: "fixture-viewport", click_kind: "double_click"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const doubleEvents = debuggerCalls.slice(doubleStart).filter(call => call.target?.tabId === 41 && call.method === "Input.dispatchMouseEvent");
  assert.deepEqual(doubleEvents.map(call => [call.params.type, call.params.button, call.params.clickCount]), [
    ["mouseMoved", undefined, undefined], ["mousePressed", "left", 1], ["mouseReleased", "left", 1],
  ], "a changed hit point stops a double click before its second trusted pair");
  const stoppedDouble = native.sent.find(message => message.id === "double-pointer-click-changed")?.result;
  assert.equal(stoppedDouble?.status, "unknown", "the first click may have taken effect even when the second is fenced");
  assert.equal(stoppedDouble?.reason_code, "double_click_target_changed");
  assert.equal(stoppedDouble?.completed_clicks, 1);
  pointerGuardValue = { ok: true, x: 10, y: 20, viewport: "fixture-viewport" };
  const contextStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "context-pointer-click", target_id: "41", method: "Controlla.pointerClick", params: {
    object_id: "fixture-target", function_declaration: "function(index,expected){return __fixturePointerGuard}",
    index: 0, outcome: { kind: "focused" }, viewport: "fixture-viewport", click_kind: "context_click"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const contextEvents = debuggerCalls.slice(contextStart).filter(call => call.target?.tabId === 41 && call.method === "Input.dispatchMouseEvent");
  assert.deepEqual(contextEvents.filter(call => call.params.type === "mousePressed" || call.params.type === "mouseReleased")
    .map(call => [call.params.type, call.params.button, call.params.clickCount]), [
      ["mousePressed", "right", 1], ["mouseReleased", "right", 1],
    ], "context click uses the trusted right mouse button");
  const scrollStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "guarded-wheel-scroll", target_id: "41", method: "Controlla.pointerScroll", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,dx,dy,expectedViewport){return {ok:true,nearest_scroll_container_at_boundary:false}}",
    viewport: "fixture-viewport", delta_x: 0, delta_y: 60, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const scrollResult = native.sent.findLast(message => message.id === "guarded-wheel-scroll")?.result;
  assert.equal(scrollResult?.status, "verified", JSON.stringify(scrollResult));
  assert.equal(scrollResult?.same_container, true, "wheel readback is bound to the same retained DOM node");
  const wheelEvent = debuggerCalls.slice(scrollStart).find(call => call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel");
  assert.deepEqual([wheelEvent?.params.x, wheelEvent?.params.y, wheelEvent?.params.deltaX, wheelEvent?.params.deltaY], [15, 25, 0, 60]);
  fixtureScrollAllowed = false;
  const refusedScrollStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "wheel-at-boundary", target_id: "41", method: "Controlla.pointerScroll", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,dx,dy,expectedViewport){return {ok:true,nearest_scroll_container_at_boundary:false}}",
    viewport: "fixture-viewport", delta_x: 0, delta_y: 60, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.findLast(message => message.id === "wheel-at-boundary")?.result?.status, "not_dispatched");
  assert.equal(debuggerCalls.slice(refusedScrollStart).some(call => call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel"), false,
    "nearest nested scroller at its edge refuses before wheel dispatch");
  fixtureScrollAllowed = true;
  fixtureScrollMoves = false;
  native.hostMessage({ type: "command", id: "wheel-no-movement", target_id: "41", method: "Controlla.pointerScroll", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,dx,dy,expectedViewport){return {ok:true,nearest_scroll_container_at_boundary:false}}",
    viewport: "fixture-viewport", delta_x: 0, delta_y: 30, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const noMovement=native.sent.findLast(message=>message.id==="wheel-no-movement")?.result;
  assert.equal(noMovement?.status,"inconclusive","acknowledged wheel delivery without same-container movement is inconclusive");
  assert.equal(noMovement?.dispatch_acknowledged,true);
  fixtureScrollMoves = true;
  fixtureScrollHitChangesOnRecheck = true;
  const changedScrollTargetStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "wheel-target-changed-before-dispatch", target_id: "41", method: "Controlla.pointerScroll", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,dx,dy,expectedViewport){return {ok:true,nearest_scroll_container_at_boundary:false}}",
    viewport: "fixture-viewport", delta_x: 0, delta_y: 30, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.findLast(message=>message.id==="wheel-target-changed-before-dispatch")?.result?.status,"not_dispatched");
  assert.equal(debuggerCalls.slice(changedScrollTargetStart).some(call => call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel"), false,
    "changed hit point is fenced by a second target guard before wheel dispatch");
  fixtureScrollHitChangesOnRecheck = false;

  const swipeStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "guarded-touch-swipe", target_id: "41", method: "Controlla.pointerSwipe", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,sx,sy,ex,ey,expectedViewport,phase){return {ok:true,viewport:'fixture-viewport',start:{x:sx,y:sy},end:{x:ex,y:ey},scroll:{top:phase==='after'?40:0},rect:{left:0,top:0,width:100,height:100}}}",
    viewport: "fixture-viewport", start_x: 20, start_y: 40, end_x: 20, end_y: 10, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const swipeResult = native.sent.findLast(message => message.id === "guarded-touch-swipe")?.result;
  assert.equal(swipeResult?.status, "verified", JSON.stringify(swipeResult));
  const touchEvents = debuggerCalls.slice(swipeStart).filter(call => call.method === "Input.dispatchTouchEvent");
  assert.deepEqual(touchEvents.map(call => call.params.type), ["touchStart", "touchMove", "touchEnd"]);
  assert.deepEqual(touchEvents.map(call => call.params.touchPoints[0] && [call.params.touchPoints[0].x, call.params.touchPoints[0].y]), [[20, 40], [20, 10], undefined]);
  fixtureSwipeMoved = false;
  native.hostMessage({ type: "command", id: "guarded-touch-swipe-no-movement", target_id: "41", method: "Controlla.pointerSwipe", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,sx,sy,ex,ey,expectedViewport,phase){return {ok:true,viewport:'fixture-viewport',start:{x:sx,y:sy},end:{x:ex,y:ey},scroll:{top:phase==='after'&&fixtureSwipeMoved?40:0},rect:{left:0,top:0,width:100,height:100}}}",
    viewport: "fixture-viewport", start_x: 20, start_y: 40, end_x: 20, end_y: 10, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.findLast(message => message.id === "guarded-touch-swipe-no-movement")?.result?.status, "inconclusive");
  fixtureSwipeMoved = true;
  rejectNextCommand = "Input.dispatchTouchEvent";
  const failedSwipeStart = debuggerCalls.length;
  native.hostMessage({ type: "command", id: "guarded-touch-swipe-release-cleanup", target_id: "41", method: "Controlla.pointerSwipe", params: {
    object_id: "fixture-target", index: 0,
    guard_function: "function(index,sx,sy,ex,ey,expectedViewport,phase){return {ok:true,viewport:'fixture-viewport',start:{x:sx,y:sy},end:{x:ex,y:ey},scroll:{top:0},rect:{left:0,top:0,width:100,height:100}}}",
    viewport: "fixture-viewport", start_x: 20, start_y: 40, end_x: 20, end_y: 10, timeout_ms: 3000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.findLast(message => message.id === "guarded-touch-swipe-release-cleanup")?.result?.status, "unknown");
  assert.ok(debuggerCalls.slice(failedSwipeStart).some(call => call.method === "Input.dispatchTouchEvent" && call.params.type === "touchEnd"), "failed touch delivery still attempts touchEnd cleanup");
  tabList.find(tab => tab.id === 42).active = true;
  tabList.find(tab => tab.id === 41).active = false;
  const pointerCommandsBeforeInactiveAttempt = debuggerCalls.filter(call => call.target?.tabId === 41 && call.method === "Input.dispatchMouseEvent").length;
  native.hostMessage({ type: "command", id: "inactive-pointer-click", target_id: "41", method: "Controlla.pointerClick", params: {
    object_id: "fixture-target", function_declaration: "function(index,expected){return __fixturePointerGuard}",
    index: 0, outcome: { kind: "focused" }, viewport: "fixture-viewport"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.status, "not_dispatched", "inactive user-owned tabs are never activated to synthesize background pointer input");
  assert.equal(native.sent.at(-1).result.reason_code, "activation_forbidden");
  assert.equal(native.sent.at(-1).result.temporary_activation, false);
  assert.equal(tabList.find(tab => tab.id === 41).active, false, "temporary activation restores the previously active tab");
  assert.equal(debuggerCalls.filter(call => call.target?.tabId === 41 && call.method === "Input.dispatchMouseEvent").length, pointerCommandsBeforeInactiveAttempt,
    "activation-forbidden pointer click dispatches no trusted input");
  focusedWindowFocused = false;
  emitActivationEvents = true;
  afterNextTabActivation = () => { pointerGuardValue = { ...pointerGuardValue, viewport: "fixture-viewport-after-activation" }; };
  const pointerCallsBeforeUnfocused = debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length;
  native.hostMessage({ type: "command", id: "unfocused-window-pointer-click", target_id: "41", method: "Controlla.pointerClick", params: {
    object_id: "fixture-target", function_declaration: "function(index,expected){return __fixturePointerGuard}",
    index: 0, outcome: { kind: "focused" }, viewport: "fixture-viewport"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(native.sent.at(-1).result.status, "not_dispatched", "an unfocused Chrome window does not make activation of its inactive user tab safe");
  assert.equal(native.sent.at(-1).result.reason_code, "activation_forbidden");
  assert.equal(debuggerCalls.filter(call => call.method === "Input.dispatchMouseEvent").length, pointerCallsBeforeUnfocused,
    "background refusal dispatches no pointer input");
  afterNextTabActivation = undefined;
  assert.equal(focusedWindowFocused, false, "background pointer click does not focus the Chrome window");
  assert.equal(tabList.find(tab => tab.id === 41).active, false, "background pointer click restores the prior active tab");
  afterNextDebuggerCall = () => {
    emitTabsActivated({ tabId: 42 }); emitTabsActivated({ tabId: 41 });
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
  assert.match(staleBatch.error, /document identity changed/, JSON.stringify(staleBatch));
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
  const openTabFrameReadsBefore = frameReadCounts.get(503) || 0;
  transientIdentityAfterAttach.add(503);
  native.hostMessage({ type: "command", id: "open-tab", target_id: "41", method: "Controlla.openTab", params: { url: "https://www.espn.com/", idempotency_key: "fixture:open-tab-12345" } });
  await new Promise(resolve => setTimeout(resolve, 80));
  const opened = native.sent.find(message => message.id === "open-tab");
  assert.ok(opened?.result, JSON.stringify(opened));
  assert.equal(opened.id, "open-tab");
  assert.equal(opened.result.tab_id, "503");
  assert.equal(opened.result.url, "https://www.espn.com/");
  assert.equal(opened.result.document_id, "document-503-next");
  assert.equal(frameReadCounts.get(503) - openTabFrameReadsBefore, 11,
    "a transient missing frame identity is retried until Chrome exposes the committed document");
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
  commandDiagnostics.splice(0);
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
  const overlapAttachCount = attached.filter(tabId => tabId === 60).length;
  native.hostMessage({ type: "pair", request_id: "codex-c-overlap", lease_id: "codex-c", tab_ids: [60], expected_urls: expectedUrls([60]), expected_document_ids: expectedDocuments([60]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(native.sent.find(message => message.request_id === "codex-c-overlap")?.error || "", /already leased/i,
    "a second client cannot acquire a target already owned by another client");
  assert.equal(attached.filter(tabId => tabId === 60).length, overlapAttachCount,
    "overlapping ownership is rejected without a duplicate debugger attach");
  assert.equal(detached.includes(60), false, "a rejected overlapping lease cannot detach the existing owner's target");
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
  await waitFor(() => nativePorts[2].sent.some(message => message.type === "capabilities"), "verified extension build capabilities");
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
  newNative.hostMessage({ type: "pair", request_id: "pair-after-disconnect", lease_id: "lease-after-disconnect", tab_ids: [53], expected_urls: expectedUrls([53]), expected_document_ids: expectedDocuments([53]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(attached.includes(53), false, "reconnected host waits for old disconnect cleanup");
  blockedDetach.resolve();
  blockedDetach = undefined;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(newNative.sent.at(-1).request_id, "pair-after-disconnect");
  assert.deepEqual([...newNative.sent.at(-1).targets], ["53"]);
  const reconnectAttachCount = attached.filter(tabId => tabId === 53).length;
  newNative.hostMessage({ type: "pair", request_id: "pair-after-disconnect", lease_id: "lease-after-disconnect", tab_ids: [53], expected_urls: expectedUrls([53]), expected_document_ids: expectedDocuments([53]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(newNative.sent.at(-1).type, "pair_error", "replayed pairing after reconnect cannot create a duplicate lease");
  assert.equal(attached.filter(tabId => tabId === 53).length, reconnectAttachCount, "replayed pairing does not attach the target a second time");
  pendingCommands.at(-1)({ value: "late result" });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(newNative.sent.some(reply => reply.id === "late"), false, "late async replies are not sent to the replacement native host");
  assert.equal(oldNative.sent.some(reply => reply.id === "late"), false, "disconnected host does not receive a late async reply");

  assert.equal((await message({ type: "pair", endpoint: "ws://127.0.0.1:1234/", token: "one", tabIds: [17] })).ok, true);
  assert.equal(sockets[0].sent[0].extension_version, "0.1.0", "extension handshake must identify the loaded version");
  assert.equal(sockets[0].sent[0].extension_build_id, expectedExtensionBuildId, "extension handshake must identify the exact build");
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

  const screenshotCallsBefore = debuggerCalls.length;
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "bounded-screenshot", target_id: "18", method: "Page.captureScreenshot",
    params: { format: "png", fromSurface: true, captureBeyondViewport: false, clip: { x: 0, y: 0, width: 320, height: 180, scale: 1 } }
  }) });
  assert.equal(debuggerCalls.length, screenshotCallsBefore + 1, "bounded screenshot capture is explicitly allowed for shared visual probes");
  assert.equal(activeSocket.sent.at(-1).result.data, "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB");

  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "allowed", target_id: "18", method: "Input.dispatchMouseEvent", params: { type: "mousePressed" }
  }) });
  assert.equal(debuggerCalls.length, callsBeforeBlocked + 2);
  assert.equal(debuggerCalls.at(-1).method, "Input.dispatchMouseEvent");
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "allowed-html5-drag", target_id: "18", method: "Input.dispatchDragEvent",
    params: { type: "dragEnter", x: 20, y: 30, data: { items: [{ mimeType: "text/plain", data: "fixture" }], files: [], dragOperationsMask: 16 } }
  }) });
  assert.equal(debuggerCalls.at(-1).method, "Input.dispatchDragEvent", "bounded semantic HTML5 drag events are routed through the explicit CDP input allowlist");
  await activeSocket.onmessage({ data: JSON.stringify({
    type: "command", id: "allowed-touch", target_id: "18", method: "Input.dispatchTouchEvent", params: { type: "touchStart", touchPoints: [{ id: 1, x: 1, y: 1 }] }
  }) });
  assert.equal(debuggerCalls.at(-1).method, "Input.dispatchTouchEvent", "shared touch input is allowed only through the explicit CDP touch method");
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
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "pair", request_id: "pair-guarded-upload", tab_ids: [66], expected_urls: expectedUrls([66]), expected_document_ids: expectedDocuments([66]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(liveNative.sent.at(-1).type, "paired", "guarded shared target is bound to the observed tab and document");
  attachOopifOnNextAutoAttach = true;
  liveNative.hostMessage({ type: "command", id: "guarded-file-frame-ax", target_id: "66", method: "Controlla.frameAccessibility", params: { frame_id: "frame-oopif" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "command", id: "guarded-file-frame-set", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_path: "/tmp/staged-artifact" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.files)), [{ name: "staged-artifact", size: 12 }], "guarded sessions may select only through the validated staged-artifact upload route");
  liveNative.hostMessage({ type: "command", id: "guarded-multi-file-set", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_paths: ["/tmp/a.txt", "/tmp/b.csv"] } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.files)), [{ name: "a.txt", size: 12 }, { name: "b.csv", size: 12 }], "guarded staged route selects and reports the complete ordered multi-file set");
  assert.deepEqual(debuggerCalls.findLast(call => call.method === "DOM.setFileInputFiles" && call.target.sessionId === "fixture-oopif-session")?.params.files, ["/tmp/a.txt", "/tmp/b.csv"]);
  fileReadbackFiles = undefined;
  liveNative.hostMessage({ type: "command", id: "guarded-raw-file-refused", target_id: "66", method: "DOM.setFileInputFiles", params: { files: ["/tmp/arbitrary"] } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /local unrestricted mode/, "guarded sessions cannot use the unrestricted raw file-selection command");
  liveNative.hostMessage({ type: "release" });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "pair", request_id: "pair-resume-unrestricted", tab_ids: [66], unrestricted: true });
  await new Promise(resolve => setTimeout(resolve, 0));
  attachOopifOnNextAutoAttach = true;
  liveNative.hostMessage({ type: "command", id: "file-frame-ax", target_id: "66", method: "Controlla.frameAccessibility", params: { frame_id: "frame-oopif" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  liveNative.hostMessage({ type: "command", id: "file-frame-set", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(liveNative.sent.at(-1).result.files)), [{ name: "example", size: 12 }], "frame-scoped upload sets the selected input and returns a file readback");
  fileInputPolicyFixture = { ok: false, reason: "accept_filter_mismatch" };
  const filteredSelectionCount = debuggerCalls.filter(call => call.method === "DOM.setFileInputFiles").length;
  liveNative.hostMessage({ type: "command", id: "file-frame-filter-refused", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_path: "/tmp/example.exe" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /accept_filter_mismatch/, "accept filter mismatch is refused before file selection");
  assert.equal(debuggerCalls.filter(call => call.method === "DOM.setFileInputFiles").length, filteredSelectionCount, "a filtered file never reaches CDP selection");
  fileInputPolicyFixture = { ok: false, reason: "multiple_files_not_allowed" };
  liveNative.hostMessage({ type: "command", id: "file-frame-multiple-refused", target_id: "66", method: "Controlla.setFrameFileInputFiles", params: { frame_id: "frame-oopif", frame_loader_id: "fixture-loader", backend_node_id: 91, file_paths: ["/tmp/a.txt", "/tmp/b.csv"] } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(liveNative.sent.at(-1).error, /multiple_files_not_allowed/, "multiple paths are refused when the input lacks multiple support");
  assert.equal(debuggerCalls.filter(call => call.method === "DOM.setFileInputFiles").length, filteredSelectionCount, "multiple-file refusal happens before CDP selection");
  fileInputPolicyFixture = undefined;
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

  nestedFrameScrollFixture = true;
  identityAxNodes = [{ nodeId: "nested-oopif-control", role: { value: "button" }, name: { value: "Nested control" }, backendDOMNodeId: 777 }];
  attachNestedOopifParentOnNextAutoAttach = true;
  let nestedSessionWaitCursor = reconnectTimers.length;
  const nestedOopifPressCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length;
  liveNative.hostMessage({ type: "command", id: "nested-oopif-frame-click-composed", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, outcome: { kind: "focused" }, timeout_ms: 250 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-oopif-frame-click-composed"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  const nestedClick = liveNative.sent.findLast(message => message.id === "nested-oopif-frame-click-composed")?.result;
  assert.equal(nestedClick?.verified, true, JSON.stringify(liveNative.sent.findLast(message => message.id === "nested-oopif-frame-click-composed")));
  const nestedPress = debuggerCalls.findLast(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed");
  assert.equal(nestedPress?.params.x, 90, "nested OOPIF click composes nested owner and parent owner affine transforms");
  assert.equal(nestedPress?.params.y, 108);
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length, nestedOopifPressCount + 1);
  assert.ok(debuggerCalls.some(call => call.target.sessionId === "fixture-parent-oopif-session" && call.method === "DOM.getFrameOwner" && call.params.frameId === "frame-nested"), "nested frame owner is resolved through its immediate OOPIF parent session");
  debuggerEvent?.({ tabId: 95 }, "Target.detachedFromTarget", { sessionId: "fixture-parent-oopif-session" });
  debuggerEvent?.({ tabId: 95 }, "Target.detachedFromTarget", { sessionId: "fixture-nested-oopif-session" });
  nestedFrameScrollFixture = false;
  identityAxNodes = [{ nodeId: "nested-scroll", role: { value: "generic" }, name: { value: "Nested scroll region" }, backendDOMNodeId: 777 }];

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
  const scrollToDispatchStart = debuggerCalls.length;
  scrollToGuardReceipt = { ok: true, phase: "after", dispatch_may_have_occurred: true, target_visible: true, center_hit: true, viewport: { width: 100, height: 80 }, center: { x: 50, y: 40 } };
  liveNative.hostMessage({ type: "command", id: "scroll-to-target-verified", target_id: "95", method: "Controlla.scrollToTarget", params: {
    object_id: "paired-document-object", index: 3, guard_function: "function(index,scrollRequest){return {ok:true}}",
    scroll_request: "scroll-to-target-v1", timeout_ms: 1500
  } });
  await new Promise(resolve => setTimeout(resolve, 10));
  const scrollToResult = liveNative.sent.findLast(message => message.id === "scroll-to-target-verified")?.result;
  assert.equal(scrollToResult?.status, "verified", JSON.stringify(scrollToResult));
  assert.equal(scrollToResult?.target_visible, true);
  assert.equal(scrollToResult?.center_hit, true);
  assert.equal(debuggerCalls.slice(scrollToDispatchStart).some(call => call.method === "Input.dispatchMouseEvent"), false,
    "semantic scroll-to-target uses guarded in-page scrolling and emits no pointer/wheel input");
  scrollToGuardReceipt = { ok: false, phase: "before", dispatch_may_have_occurred: false, reason: "target_not_visible" };
  liveNative.hostMessage({ type: "command", id: "scroll-to-target-refused", target_id: "95", method: "Controlla.scrollToTarget", params: {
    object_id: "paired-document-object", index: 3, guard_function: "function(index,scrollRequest){return {ok:false}}",
    scroll_request: "scroll-to-target-v1", timeout_ms: 1500
  } });
  await new Promise(resolve => setTimeout(resolve, 10));
  const scrollToRefusal = liveNative.sent.findLast(message => message.id === "scroll-to-target-refused")?.result;
  assert.equal(scrollToRefusal?.status, "not_dispatched", JSON.stringify(scrollToRefusal));
  assert.equal(scrollToRefusal?.dispatch_may_have_occurred, false);
  scrollToGuardReceipt = { ok: true, phase: "after", dispatch_may_have_occurred: true, target_visible: true, center_hit: true };
  emitFileChooserOnRootRelease = true;
  liveNative.hostMessage({ type: "command", id: "same-process-frame-upload", target_id: "95", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.at(-1).result?.status, "selected", JSON.stringify(liveNative.sent.at(-1)));
  const rootPress = debuggerCalls.findLast(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed");
  assert.equal(rootPress?.params.x, 140, "same-process frame click includes iframe offset and scale");
  assert.equal(rootPress?.params.y, 260);
  attachOopifOnNextAutoAttach = true;
  const frameWheelCountBeforeOopif = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length;
  liveNative.hostMessage({ type: "command", id: "oopif-frame-scroll-refused", target_id: "95", method: "Controlla.frameScroll", params: { frame_id: "frame-oopif", loader_id: childFrameLoaderId, backend_node_id: 777, delta_x: 0, delta_y: 120, timeout_ms: 1500 } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  const oopifScrollResult = liveNative.sent.findLast(message => message.id === "oopif-frame-scroll-refused")?.result;
  assert.equal(oopifScrollResult?.status, "not_dispatched", JSON.stringify(oopifScrollResult));
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length, frameWheelCountBeforeOopif, "OOPIF child-frame scroll fails closed before wheel delivery");
  nestedFrameScrollFixture = true;
  await chrome.tabs.update(95, { active: true });
  identityAxNodes = [{ nodeId: "nested-scroll", role: { value: "generic" }, name: { value: "Nested scroll region" }, backendDOMNodeId: 777 }];
  fixtureScrollTop = 20;
  const nestedWheelStart = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length;
  nestedSessionWaitCursor = reconnectTimers.length;
  liveNative.hostMessage({ type: "command", id: "nested-same-process-frame-scroll", target_id: "95", method: "Controlla.frameScroll", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, delta_x: 0, delta_y: 20, timeout_ms: 1500 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-same-process-frame-scroll"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  const nestedScrollResult = liveNative.sent.findLast(message => message.id === "nested-same-process-frame-scroll")?.result;
  assert.equal(nestedScrollResult?.status, "verified", JSON.stringify(nestedScrollResult));
  const nestedWheel = debuggerCalls.findLast(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length, nestedWheelStart + 1);
  assert.equal(nestedWheel?.params.x, 95, "nested affine frame point is composed through both owner transforms");
  assert.equal(nestedWheel?.params.y, 110);
  staleNestedAncestorFixture = true;
  nestedFrameTreeReads = 0;
  const staleNestedWheelCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length;
  nestedSessionWaitCursor = reconnectTimers.length;
  liveNative.hostMessage({ type: "command", id: "nested-stale-loader-refused", target_id: "95", method: "Controlla.frameScroll", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, delta_x: 0, delta_y: 20, timeout_ms: 1500 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-stale-loader-refused"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  const staleNestedResult = liveNative.sent.findLast(message => message.id === "nested-stale-loader-refused")?.result;
  assert.equal(staleNestedResult?.status, "not_dispatched", JSON.stringify(staleNestedResult));
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length, staleNestedWheelCount, "stale nested loader refuses before wheel delivery");
  staleNestedAncestorFixture = false;
  reparentNestedFrameFixture = true;
  nestedFrameTreeReads = 0;
  const reparentClickCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length;
  nestedSessionWaitCursor = reconnectTimers.length;
  liveNative.hostMessage({ type: "command", id: "nested-reparent-refused", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, outcome: { kind: "focused" }, timeout_ms: 250 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-reparent-refused"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  assert.ok(liveNative.sent.some(message => message.id === "nested-reparent-refused"), "reparented frame action settles with an explicit refusal");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length, reparentClickCount, "dynamic frame reparenting refuses before trusted input");
  reparentNestedFrameFixture = false;
  changingNestedTransformFixture = true;
  nestedOwnerBoxReads = 0;
  nestedSessionWaitCursor = reconnectTimers.length;
  const changedTransformWheelCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length;
  liveNative.hostMessage({ type: "command", id: "nested-transform-change-refused", target_id: "95", method: "Controlla.frameScroll", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, delta_x: 0, delta_y: 20, timeout_ms: 1500 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-transform-change-refused"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  assert.equal(liveNative.sent.findLast(message => message.id === "nested-transform-change-refused")?.result?.status, "not_dispatched");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length, changedTransformWheelCount, "owner transform drift refuses before wheel delivery");
  identityAxNodes = [{ nodeId: "nested-control", role: { value: "button" }, name: { value: "Nested control" }, backendDOMNodeId: 777 }];
  nestedOwnerBoxReads = 0;
  const changedTransformClickCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length;
  liveNative.hostMessage({ type: "command", id: "nested-transform-click-refused", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, outcome: { kind: "focused" }, timeout_ms: 250 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-transform-click-refused"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  assert.match(liveNative.sent.findLast(message => message.id === "nested-transform-click-refused")?.error || "", /geometry changed before dispatch/i);
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length, changedTransformClickCount, "frame click refuses owner transform drift before trusted input");
  identityAxNodes = [{ nodeId: "nested-scroll", role: { value: "generic" }, name: { value: "Nested scroll region" }, backendDOMNodeId: 777 }];
  changingNestedTransformFixture = false;
  nonAffineNestedTransformFixture = true;
  nestedOwnerBoxReads = 0;
  nestedSessionWaitCursor = reconnectTimers.length;
  liveNative.hostMessage({ type: "command", id: "nested-non-affine-refused", target_id: "95", method: "Controlla.frameScroll", params: { frame_id: "frame-nested", loader_id: "nested-loader", backend_node_id: 777, delta_x: 0, delta_y: 20, timeout_ms: 1500 } });
  for (let attempt = 0; attempt < 8 && !liveNative.sent.some(message => message.id === "nested-non-affine-refused"); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 0));
    for (const timer of reconnectTimers.slice(nestedSessionWaitCursor).filter(timer => timer.delay === 400)) timer.callback();
    nestedSessionWaitCursor = reconnectTimers.length;
  }
  assert.equal(liveNative.sent.findLast(message => message.id === "nested-non-affine-refused")?.result?.status, "not_dispatched");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mouseWheel").length, changedTransformWheelCount, "non-affine owner geometry refuses before wheel delivery");
  nonAffineNestedTransformFixture = false;
  nestedFrameScrollFixture = false;
  identityAxNodes = [{ nodeId: "browse", role: { value: "button" }, name: { value: "Browse" }, backendDOMNodeId: 777 }];
  frameShadowRootFixture = true;
  const shadowRoutePressCount = debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length;
  liveNative.hostMessage({ type: "command", id: "shadow-root-frame-click", target_id: "95", method: "Controlla.frameClick", params: { frame_id: "frame-oopif", loader_id: childFrameLoaderId, backend_node_id: 777, outcome: { kind: "focused" }, timeout_ms: 250 } });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(liveNative.sent.findLast(message => message.id === "shadow-root-frame-click")?.result?.verified, true,
    "fresh, visible open-shadow controls in an attached frame can be clicked through the composed hit-test guard");
  assert.equal(debuggerCalls.filter(call => call.target.tabId === 95 && call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed").length, shadowRoutePressCount + 1,
    "open-shadow frame action dispatches one trusted click after the shadow-chain guard");
  frameShadowRootFixture = false;
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
  networkEventsOnEnable = [
    ["Network.requestWillBeSent", { requestId: "upload-stalled-request", type: "XHR", request: { url: uploadUrl, method: "POST" } }],
  ];
  emitFileChooserOnRootRelease = true;
  const uploadStallWaitCursor = reconnectTimers.length;
  liveNative.hostMessage({ type: "command", id: "upload-progress-stalled", target_id: "95", method: "Controlla.frameSelectFile", params: { frame_id: "frame-oopif", frame_loader_id: childFrameLoaderId, trigger_backend_node_id: 777, file_path: "/tmp/example" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  reconnectTimers.slice(uploadStallWaitCursor).findLast(timer => timer.delay === 400)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  reconnectTimers.findLast(timer => timer.delay === 1200)?.callback();
  await new Promise(resolve => setTimeout(resolve, 10));
  const stalledUpload = liveNative.sent.findLast(message => message.id === "upload-progress-stalled")?.result;
  assert.equal(stalledUpload?.status, "unknown", JSON.stringify(stalledUpload));
  assert.equal(stalledUpload?.reason_code, "upload_network_stalled");
  assert.equal(stalledUpload?.network_observation?.status, "stalled");
  assert.equal(stalledUpload?.network_observation?.reason, "upload_response_not_observed");
  assert.equal(stalledUpload?.network_observation?.pending_requests, 1);
  assert.equal(stalledUpload?.network_observation?.observation_bound_ms, 1500);
  assert.ok(stalledUpload?.network_observation?.observation_window_ms <= 1500);
  assert.notEqual(stalledUpload?.status, "selected", "a pending upload request cannot be reported as selection success");
  assert.doesNotMatch(JSON.stringify(stalledUpload?.network_events), /fixture-secret|uploadType/);
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

  delayedNextCommitMs = 100;
  restartPort.hostMessage({ type: "command", id: "open-independent-slow", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/independent-slow", idempotency_key: "client-fixture:independent-slow-1" } });
  restartPort.hostMessage({ type: "command", id: "open-independent-fast", target_id: "41", method: "Controlla.openTab",
    params: { url: "https://example.test/independent-fast", idempotency_key: "client-fixture:independent-fast-1" } });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.ok(restartPort.sent.find(message => message.id === "open-independent-fast")?.result,
    "a distinct open should complete while another tab is still waiting for document readiness");
  assert.equal(restartPort.sent.some(message => message.id === "open-independent-slow"), false,
    "the slow open should remain pending until its own navigation commits");
  await new Promise(resolve => setTimeout(resolve, 110));
  assert.ok(restartPort.sent.find(message => message.id === "open-independent-slow")?.result);

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

  overlapSameUrlNextNavigation = true;
  restartPort.hostMessage({ ...openRequest, id: "open-overlapping-same-url", params: { url: "https://example.test/overlap-same-url", idempotency_key: "client-fixture:overlap-same-url-1234" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(restartPort.sent.at(-1).error, /Tab changed during attachment/, JSON.stringify(restartPort.sent.at(-1)) +
    "an older same-URL commit and DOMContentLoaded cannot satisfy a newer navigation attachment");
  const overlappingTab = tabList.find(tab => tab.url === "https://example.test/overlap-same-url");
  assert.ok(overlappingTab, "the failed open leaves the exact tab available for safe recovery");
  assert.equal(await backgroundContext.__testHooks.pendingNavigationMatchesDocument(overlappingTab.id, `document-${overlappingTab.id}-stale-same-url`), false,
    "an ambiguous navigation cannot attach while Chrome still reports it loading");
  overlappingTab.status = "complete";
  const recoveredDocumentId = `document-${overlappingTab.id}-recovered`;
  tabDocumentOverrides.set(overlappingTab.id, recoveredDocumentId);
  assert.equal(await backgroundContext.__testHooks.pendingNavigationMatchesDocument(overlappingTab.id, recoveredDocumentId), true,
    "same-tab retry can recover once Chrome reports one stable current document identity");
  assert.equal(await backgroundContext.__testHooks.pendingNavigationMatchesDocument(overlappingTab.id, `document-${overlappingTab.id}-stale-same-url`), false,
    "a delayed same-URL callback cannot replace the recovered live document identity");
  let recoveryRaceInjected = 0;
  replacePendingOnFrameRead = {
    tabId: overlappingTab.id,
    remaining: 2,
    replace() {
      recoveryRaceInjected++;
      navigationBefore?.({ tabId: overlappingTab.id, frameId: 0, url: "https://example.test/overlap-same-url", timeStamp: nextWebNavigationTimeStamp++ });
    },
  };
  assert.equal(await backgroundContext.__testHooks.pendingNavigationMatchesDocument(overlappingTab.id, recoveredDocumentId), false,
    "a newer same-URL navigation during the second frame read invalidates recovery");
  assert.equal(recoveryRaceInjected, 1, "the test replaces the pending record during the second read");
  assert.equal(replacePendingOnFrameRead, undefined, "the race fixture was consumed by the second frame read");
  tabsRemoved(overlappingTab.id);

  const redirectedTab = tabList.find(tab => tab.id === 74);
  const redirectStartUrl = "https://example.test/redirect-start";
  const redirectFinalUrl = "https://example.test/redirect-final";
  const redirectDocumentId = "document-74-after-redirect";
  navigationBefore({ tabId: redirectedTab.id, frameId: 0, url: redirectStartUrl, timeStamp: nextWebNavigationTimeStamp++ });
  navigationError({ tabId: redirectedTab.id, frameId: 0, url: redirectStartUrl, timeStamp: nextWebNavigationTimeStamp++, error: "net::ERR_ABORTED" });
  navigationBefore({ tabId: redirectedTab.id, frameId: 0, url: redirectFinalUrl, timeStamp: nextWebNavigationTimeStamp++ });
  redirectedTab.url = redirectFinalUrl;
  redirectedTab.status = "complete";
  tabUrlOverrides.set(redirectedTab.id, redirectFinalUrl);
  tabDocumentOverrides.set(redirectedTab.id, redirectDocumentId);
  navigationCommitted({ tabId: redirectedTab.id, frameId: 0, documentId: redirectDocumentId, url: redirectFinalUrl, timeStamp: nextWebNavigationTimeStamp++ });
  navigationDomContentLoaded({ tabId: redirectedTab.id, frameId: 0, documentId: redirectDocumentId, url: redirectFinalUrl, timeStamp: nextWebNavigationTimeStamp++ });
  assert.equal(backgroundContext.__testHooks.pendingMainFrameNavigations.has(redirectedTab.id), false,
    "a server redirect's committed destination clears its matching pending navigation");
  assert.equal(await backgroundContext.__testHooks.pendingNavigationMatchesDocument(redirectedTab.id, redirectDocumentId), true,
    "the final document in a navigation redirect sequence remains eligible for attachment");

  const redirectRetryTab = tabList.find(tab => tab.id === 75);
  const retryStartUrl = "https://example.test/retry-start";
  const retryDestinationUrl = "https://example.test/retry-destination";
  navigationBefore({ tabId: redirectRetryTab.id, frameId: 0, url: retryStartUrl, timeStamp: nextWebNavigationTimeStamp++ });
  navigationError({ tabId: redirectRetryTab.id, frameId: 0, url: retryStartUrl, timeStamp: nextWebNavigationTimeStamp++, error: "net::ERR_ABORTED" });
  navigationBefore({ tabId: redirectRetryTab.id, frameId: 0, url: retryDestinationUrl, timeStamp: nextWebNavigationTimeStamp++ });
  redirectRetryTab.url = retryDestinationUrl;
  tabUrlOverrides.set(redirectRetryTab.id, retryDestinationUrl);
  tabDocumentOverrides.set(redirectRetryTab.id, "document-75-after-failed-redirect");
  navigationError({ tabId: redirectRetryTab.id, frameId: 0, url: retryDestinationUrl, timeStamp: nextWebNavigationTimeStamp++, error: "net::ERR_NAME_NOT_RESOLVED" });
  await new Promise(resolve => setTimeout(resolve, 0));
  navigationBefore({ tabId: redirectRetryTab.id, frameId: 0, url: retryStartUrl, timeStamp: nextWebNavigationTimeStamp++ });
  const retriedNavigation = backgroundContext.__testHooks.pendingMainFrameNavigations.get(redirectRetryTab.id);
  assert.equal(retriedNavigation.ambiguous, false,
    "a failed redirect destination must release the chain so retrying the original URL is matchable");

  const staleRedirectErrorTab = tabList.find(tab => tab.id === 76);
  const chainStartUrl = "https://example.test/chain-start";
  const chainMiddleUrl = "https://example.test/chain-middle";
  const chainFinalUrl = "https://example.test/chain-final";
  const chainStartAt = nextWebNavigationTimeStamp++;
  navigationBefore({ tabId: staleRedirectErrorTab.id, frameId: 0, url: chainStartUrl, timeStamp: chainStartAt });
  const middleNavigationAt = nextWebNavigationTimeStamp++;
  navigationBefore({ tabId: staleRedirectErrorTab.id, frameId: 0, url: chainMiddleUrl, timeStamp: middleNavigationAt });
  const finalNavigationAt = nextWebNavigationTimeStamp++;
  navigationBefore({ tabId: staleRedirectErrorTab.id, frameId: 0, url: chainFinalUrl, timeStamp: finalNavigationAt });
  navigationError({ tabId: staleRedirectErrorTab.id, frameId: 0, url: chainMiddleUrl, timeStamp: middleNavigationAt + 0.5, error: "net::ERR_ABORTED" });
  assert.equal(Boolean(backgroundContext.__testHooks.pendingMainFrameNavigations.get(staleRedirectErrorTab.id).failed), false,
    "a delayed error from an intermediate redirect must not fail the later navigation");
  const chainDocumentId = "document-76-after-redirect-chain";
  staleRedirectErrorTab.url = chainFinalUrl;
  staleRedirectErrorTab.status = "complete";
  tabUrlOverrides.set(staleRedirectErrorTab.id, chainFinalUrl);
  tabDocumentOverrides.set(staleRedirectErrorTab.id, chainDocumentId);
  navigationCommitted({ tabId: staleRedirectErrorTab.id, frameId: 0, documentId: chainDocumentId, url: chainFinalUrl, timeStamp: nextWebNavigationTimeStamp++ });
  navigationDomContentLoaded({ tabId: staleRedirectErrorTab.id, frameId: 0, documentId: chainDocumentId, url: chainFinalUrl, timeStamp: nextWebNavigationTimeStamp++ });
  assert.equal(backgroundContext.__testHooks.pendingMainFrameNavigations.has(staleRedirectErrorTab.id), false,
    "the continued redirect chain still commits and clears normally after a stale intermediate error");

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

  // A duplicate status notification after DOMContentLoaded must not invalidate
  // a stable document, but a genuinely newer navigation start during attach
  // must survive a stale DOMContentLoaded notification for the prior document.
  const backgroundTabId = Number(backgroundOpen.tab_id);
  reloadDuringAttach.add(backgroundTabId);
  restartPort.hostMessage({ type: "command", id: "navigate-late-loading-event", target_id: String(backgroundTabId), method: "Controlla.navigate", params: { url: "https://example.test/background/stable" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(restartPort.sent.at(-1).result?.url, "https://example.test/background/stable",
    "a late status event with stable URL/document identity does not fail navigation");
  restartPort.hostMessage({ type: "command", id: "snapshot-after-late-loading-event", target_id: String(backgroundTabId), method: "DOM.getDocument", params: {} });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(restartPort.sent.at(-1).id, "snapshot-after-late-loading-event",
    "the tab remains paired after the benign late loading event");

  const pendingRaceOpen = { ...openRequest, id: "open-for-pending-race", params: { url: "https://example.test/pending-race", idempotency_key: "client-fixture:pending-race-1234" } };
  restartPort.hostMessage(pendingRaceOpen);
  await new Promise(resolve => setTimeout(resolve, 0));
  const pendingRaceTabId = Number(restartPort.sent.at(-1).result.tab_id);
  navigationStartDuringAttach.add(pendingRaceTabId);
  restartPort.hostMessage({ type: "command", id: "navigation-started-during-attach", target_id: String(pendingRaceTabId), method: "Controlla.navigate", params: { url: "https://example.test/pending-race/first" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const navigationAttachResult = restartPort.sent.findLast(message => message.id === "navigation-started-during-attach");
  assert.match(navigationAttachResult.error, /Tab changed during attachment/, JSON.stringify(navigationAttachResult) +
    " a newer main-frame navigation survives an old-document DOMContentLoaded and fails closed");
  tabsRemoved(pendingRaceTabId);

  const identityRaceOpen = { ...openRequest, id: "open-for-identity-race", params: { url: "https://example.test/identity-race", idempotency_key: "client-fixture:identity-race-1234" } };
  restartPort.hostMessage(identityRaceOpen);
  await new Promise(resolve => setTimeout(resolve, 0));
  const identityRaceTabId = Number(restartPort.sent.at(-1).result.tab_id);
  identityChangeDuringAttach.add(identityRaceTabId);
  restartPort.hostMessage({ type: "command", id: "navigation-identity-changed-during-attach", target_id: String(identityRaceTabId), method: "Controlla.navigate", params: { url: "https://example.test/identity-race/next" } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(restartPort.sent.at(-1).error, /Tab changed during attachment/,
    "a committed document identity change during debugger attach still fails closed");
  assert.ok(detached.includes(identityRaceTabId), "a mismatched document is detached instead of paired to the stale navigation");

  localStorage.openReceipts["expired:receipt"] = { status: "complete", tab_id: "1", url: "https://example.test/old", document_id: "old", created_ms: -1_000_000_000_000, active: false };
  restartPort.hostMessage({ ...openRequest, id: "open-prunes-expired-receipts", params: { url: "https://example.test/pruned", idempotency_key: "client-fixture:prune-expired-1", active: false } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(localStorage.openReceipts["expired:receipt"], undefined, "expired completed open receipts are pruned before new work");

  restartPort.hostMessage({ ...openRequest, id: "open-owned-background-window", params: {
    url: "https://example.test/owned-window", idempotency_key: "client-fixture:owned-window-123",
    active: false, create_background_window: true, window_left: 80, window_top: 60, window_width: 1000, window_height: 720
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const ownedWindowResult = restartPort.sent.at(-1).result;
  assert.equal(createdWindows.at(-1).focused, false, "owned automation window is created unfocused");
  assert.deepEqual([createdWindows.at(-1).left, createdWindows.at(-1).top, createdWindows.at(-1).width, createdWindows.at(-1).height], [80, 60, 1000, 720]);
  assert.equal(ownedWindowResult.created_by_controlla, true);
  assert.equal(ownedWindowResult.window_id, 901);
  assert.equal(tabList.find(tab => tab.id === Number(ownedWindowResult.tab_id)).windowId, ownedWindowResult.window_id);
  assert.equal(focusedWindowFocused, false, "creating the owned window does not focus Chrome");

  restartPort.hostMessage({ ...openRequest, id: "open-tab-before-move", params: {
    url: "https://example.test/move-owned", idempotency_key: "client-fixture:move-owned-1234", active: false
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const moveSourceTabId = Number(restartPort.sent.at(-1).result.tab_id);
  restartPort.hostMessage({ type: "command", id: "move-owned-tab", target_id: "41", method: "Controlla.moveOwnedTab", params: {
    owned_tab_id: String(moveSourceTabId), target_window_id: ownedWindowResult.window_id
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const movedReceipt = restartPort.sent.at(-1).result;
  assert.equal(movedReceipt.status, "moved");
  assert.equal(movedReceipt.topology_verified, true);
  assert.equal(movedReceipt.window_id, ownedWindowResult.window_id);
  assert.equal(tabList.find(tab => tab.id === moveSourceTabId).windowId, ownedWindowResult.window_id,
    "owned tab moves into the selected automation window with its document identity intact");

  restartPort.hostMessage({ type: "command", id: "open-in-existing-owned-window", target_id: "41", method: "Controlla.openTab", params: {
    url: "https://example.test/reuse-owned-window", idempotency_key: "client-fixture:reuse-owned-window-123",
    active: true, target_window_id: ownedWindowResult.window_id
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const selectedWindowResult = restartPort.sent.at(-1).result;
  assert.equal(selectedWindowResult.window_id, ownedWindowResult.window_id, "caller-selected automation window receives the new tab");
  assert.equal(selectedWindowResult.window_focused, false, "selected background automation window stays unfocused");
  assert.equal(tabList.find(tab => tab.id === Number(selectedWindowResult.tab_id)).active, true, "new tab is active inside the selected automation window");
  assert.equal(focusedWindowFocused, false, "activating a tab inside the automation window does not focus Chrome");
  restartPort.hostMessage({ type: "command", id: "open-in-unowned-window", target_id: "41", method: "Controlla.openTab", params: {
    url: "https://example.test/unowned-window", idempotency_key: "client-fixture:unowned-window-123",
    active: true, target_window_id: 2
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(restartPort.sent.at(-1).error, /no completed Controlla automation-window ownership receipt/,
    "user-owned windows cannot be selected as automation windows");

  restartPort.hostMessage({ ...openRequest, id: "open-for-screenshot-navigation", params: {
    url: "https://example.test/screenshot-capture", idempotency_key: "client-fixture:screenshot-navigation-123"
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  const screenshotNavigationTabId = Number(restartPort.sent.at(-1).result.tab_id);
  navigateDuringScreenshot = { tabId: screenshotNavigationTabId, documentId: "screenshot-navigation-interloper", url: "https://example.test/after-capture" };
  restartPort.hostMessage({ type: "command", id: "screenshot-navigated-during-capture", target_id: String(screenshotNavigationTabId), method: "Page.captureScreenshot", params: {
    format: "png", captureBeyondViewport: false, fromSurface: true, clip: { x: 0, y: 0, width: 100, height: 100, scale: 1 }
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(restartPort.sent.at(-1).reason_code, "screenshot_document_changed", "capture bytes from a replaced document are discarded");

  restartPort.hostMessage({ type: "pair", request_id: "download-observer-pair", tab_ids: [98], unrestricted: true,
    expected_urls: expectedUrls([98]), expected_document_ids: expectedDocuments([98]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  const downloadUrl = "https://example.test/files/report.csv?fixture=1";
  const sendDownloadCommand = async (id, params, tabId = 98) => {
    restartPort.hostMessage({ type: "command", id, target_id: String(tabId), method: "Controlla.downloadLifecycle", params });
    await new Promise(resolve => setTimeout(resolve, 5));
    return restartPort.sent.findLast(message => message.id === id);
  };
  const waitForNativeMessage = id => new Promise((resolve, reject) => {
    const deadline = Date.now() + 3000;
    const poll = () => {
      const message = restartPort.sent.findLast(candidate => candidate.id === id);
      if (message) resolve(message);
      else if (Date.now() >= deadline) reject(new Error(`Timed out waiting for fixture message ${id}: ${JSON.stringify(restartPort.sent.filter(candidate => candidate.id === id))}`));
      else setTimeout(poll, 1);
    };
    poll();
  });
  const staleDownloadArm = await sendDownloadCommand("download-stale-arm", {
    action: "arm", expected_url: downloadUrl, expected_document_id: "wrong-document"
  });
  assert.match(staleDownloadArm.error, /current paired document identity/);
  const armedDownload = await sendDownloadCommand("download-arm", {
    action: "arm", expected_url: downloadUrl, expected_document_id: expectedDocuments([98])[98]
  });
  assert.equal(armedDownload.result.status, "armed");
  const runtimeBoundArm = await sendDownloadCommand("download-runtime-bound-arm", {
    action: "arm", expected_url: "https://example.test/files/runtime-bound.csv"
  });
  assert.equal(runtimeBoundArm.result.status, "armed");
  assert.equal(runtimeBoundArm.result.document_id, expectedDocuments([98])[98],
    "runtime can ask the extension to bind to its current paired document identity");
  await sendDownloadCommand("download-runtime-bound-cleanup", {
    action: "cancel", observer_id: runtimeBoundArm.result.observer_id
  });
  const observerId = armedDownload.result.observer_id;
  const crossTargetWait = await sendDownloadCommand("download-cross-target", {
    action: "wait", observer_id: observerId, timeout_ms: 100
  }, 99);
  assert.match(crossTargetWait.error, /stale or belongs to another target document/);
  void sendDownloadCommand("download-wait-start", { action: "wait", observer_id: observerId, timeout_ms: 1000 });
  emitDownloadCreated({ id: 9101, tabId: 98, url: downloadUrl, finalUrl: downloadUrl,
    filename: "/Users/fixture/Downloads/report.csv", state: "in_progress", bytesReceived: 0, totalBytes: -1 });
  const startedDownload = await waitForNativeMessage("download-wait-start");
  assert.equal(startedDownload.result.status, "started");
  assert.equal(startedDownload.result.filename, "report.csv", "download path is reduced to a safe basename");
  assert.equal(startedDownload.result.bytes_received, 0);
  assert.equal(startedDownload.result.file_path, undefined, "in-progress events never expose a file path for acquisition");
  assert.equal(startedDownload.result.url_identity.matches_expected, true);
  assert.equal(startedDownload.result.url_identity.url_sha256.length, 64);
  assert.doesNotMatch(JSON.stringify(startedDownload.result), /fixture=1/, "signed/query URL text is not exposed");
  void sendDownloadCommand("download-wait-complete", { action: "wait", observer_id: observerId, timeout_ms: 1000 });
  emitDownloadChanged({ id: 9101, state: { current: "complete" }, bytesReceived: { current: 18 },
    totalBytes: { current: 18 }, filename: { current: "/Users/fixture/Downloads/report.csv" } });
  const completedDownload = await waitForNativeMessage("download-wait-complete");
  assert.equal(completedDownload.result.status, "completed");
  assert.equal(completedDownload.result.bytes_received, 18);
  assert.equal(completedDownload.result.total_bytes, 18);
  assert.equal(completedDownload.result.download_id, 9101, "completion is bound to the matched Chrome download ID");
  assert.equal(completedDownload.result.file_path, "/Users/fixture/Downloads/report.csv",
    "only the internally consumed completed event carries Chrome's file path");

  const interruptedArm = await sendDownloadCommand("download-interrupted-arm", {
    action: "arm", expected_url: "https://example.test/files/fail.bin", expected_document_id: expectedDocuments([98])[98]
  });
  const interruptedObserver = interruptedArm.result.observer_id;
  void sendDownloadCommand("download-interrupted-wait", {
    action: "wait", observer_id: interruptedObserver, timeout_ms: 1000
  });
  emitDownloadCreated({ id: 9102, tabId: 98, url: "https://example.test/files/fail.bin", finalUrl: "https://example.test/files/fail.bin",
    filename: "/tmp/fail.bin", state: "in_progress", bytesReceived: 4, totalBytes: 12 });
  await new Promise(resolve => setTimeout(resolve, 5));
  await waitForNativeMessage("download-interrupted-wait");
  void sendDownloadCommand("download-interrupted-terminal-wait", {
    action: "wait", observer_id: interruptedObserver, timeout_ms: 1000
  });
  emitDownloadChanged({ id: 9102, state: { current: "interrupted" }, error: { current: "NETWORK_FAILED" }, bytesReceived: { current: 4 } });
  assert.equal((await waitForNativeMessage("download-interrupted-terminal-wait")).result.status, "interrupted");

  const timeoutArm = await sendDownloadCommand("download-timeout-arm", {
    action: "arm", expected_url: "https://example.test/files/never.bin", expected_document_id: expectedDocuments([98])[98]
  });
  await sendDownloadCommand("download-timeout-wait", {
    action: "wait", observer_id: timeoutArm.result.observer_id, timeout_ms: 50
  });
  reconnectTimers.findLast(timer => timer.delay === 50)?.callback();
  const timedOut = await waitForNativeMessage("download-timeout-wait");
  assert.equal(timedOut.result?.status, "timed_out", JSON.stringify(timedOut));
  const timedOutObserverReuse = await sendDownloadCommand("download-timeout-cleanup", {
    action: "wait", observer_id: timeoutArm.result.observer_id, timeout_ms: 5
  });
  assert.match(timedOutObserverReuse.error, /stale or belongs to another target document/);
  const cancelArm = await sendDownloadCommand("download-cancel-arm", {
    action: "arm", expected_url: "https://example.test/files/cancel.bin", expected_document_id: expectedDocuments([98])[98]
  });
  emitDownloadCreated({ id: 9103, tabId: 98, url: "https://example.test/files/cancel.bin", finalUrl: "https://example.test/files/cancel.bin",
    filename: "/tmp/cancel.bin", state: "in_progress", bytesReceived: 0, totalBytes: 20 });
  await new Promise(resolve => setTimeout(resolve, 5));
  const cancelled = await sendDownloadCommand("download-cancel", { action: "cancel", observer_id: cancelArm.result.observer_id });
  assert.equal(cancelled.result.status, "observer_cancelled");
  assert.ok(cancelledDownloads.includes(9103));

  const detachArm = await sendDownloadCommand("download-detach-arm", {
    action: "arm", expected_url: "https://example.test/files/detach.bin", expected_document_id: expectedDocuments([98])[98]
  });
  void sendDownloadCommand("download-detach-wait", { action: "wait", observer_id: detachArm.result.observer_id, timeout_ms: 1000 });
  await new Promise(resolve => setTimeout(resolve, 0));
  emitDownloadCreated({ id: 9104, tabId: 98, url: "https://example.test/files/detach.bin", finalUrl: "https://example.test/files/detach.bin",
    filename: "/tmp/detach.bin", state: "in_progress", bytesReceived: 1, totalBytes: 9 });
  debuggerDetached?.({ tabId: 98 }, "canceled_by_user");
  const detachResult = await waitForNativeMessage("download-detach-wait");
  assert.match(detachResult.error, /stale target/);
  const detachedObserverReuse = await sendDownloadCommand("download-detach-reuse", {
    action: "wait", observer_id: detachArm.result.observer_id, timeout_ms: 5
  });
  assert.match(detachedObserverReuse.error, /stale or belongs to another target document/, "an in-flight URL hash cannot resurrect an observer after detach");

  restartPort.hostMessage({ type: "pair", request_id: "download-close-pair", tab_ids: [97], unrestricted: true,
    expected_urls: expectedUrls([97]), expected_document_ids: expectedDocuments([97]) });
  await new Promise(resolve => setTimeout(resolve, 0));
  restartPort.hostMessage({ type: "command", id: "download-close-arm", target_id: "97", method: "Controlla.downloadLifecycle", params: {
    action: "arm", expected_url: "https://example.test/files/close.bin", expected_document_id: expectedDocuments([97])[97]
  } });
  await new Promise(resolve => setTimeout(resolve, 5));
  const closeObserver = restartPort.sent.findLast(message => message.id === "download-close-arm").result.observer_id;
  restartPort.hostMessage({ type: "command", id: "download-close-wait", target_id: "97", method: "Controlla.downloadLifecycle", params: {
    action: "wait", observer_id: closeObserver, timeout_ms: 1000
  } });
  await new Promise(resolve => setTimeout(resolve, 0));
  tabsRemoved?.(97);
  const closeResult = await waitForNativeMessage("download-close-wait");
  assert.match(closeResult.error, /stale target/);

  console.log("Extension version handshake, pairing cleanup, chooser/download event routing, and command allowlist tests passed.");
})().catch(error => { console.error(error); process.exitCode = 1; });
