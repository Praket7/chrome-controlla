let socket;
const NATIVE_HOST = "chrome_controlla_bridge";
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 60000;
const RECONNECT_ALARM = "controlla-native-reconnect";
const SHARED_COMMAND_METHODS = new Set(["Page.getFrameTree", "Runtime.evaluate", "Runtime.callFunctionOn", "Runtime.releaseObject",
  "Input.dispatchMouseEvent", "Input.dispatchKeyEvent", "DOM.getDocument", "DOM.querySelector", "Accessibility.getPartialAXTree"]);
let nativePort;
let nativeReady = false;
let nativeError;
let reconnectTimer;
let reconnectAttempts = 0;
let manualAttachedTabs = new Set();
let nativeAttachedTabs = new Set();
// Tab ID -> URL observed at attach time. The pairing is bound to this page
// identity: a later navigation to a different URL detaches the tab.
const pairedTabUrls = new Map();
const pairedTabDocumentIds = new Map();
const tabNavigationGenerations = new Map();
const navigatingTabs = new Set();
const debuggerDetachedDuringNavigation = new Set();
const committedNavigationTabs = new Map();
const navigationDebuggerAttaches = new Map();
let manualGeneration = 0;
let nativeGeneration = 0;
let nativePortGeneration = 0;
let nativeMutation = Promise.resolve();
const pendingTabDetaches = new Map();
const tabPairReservations = new Map();

function reservePairingTabs(tabIds) {
  if (tabIds.some(tabId => tabPairReservations.has(tabId))) {
    throw new Error("A requested tab is already being paired.");
  }
  const reservation = {};
  for (const tabId of tabIds) tabPairReservations.set(tabId, reservation);
  return reservation;
}

function releasePairingTabs(tabIds, reservation) {
  for (const tabId of tabIds) {
    if (tabPairReservations.get(tabId) === reservation) tabPairReservations.delete(tabId);
  }
}

function detachDebugger(tabId) {
  const previous = pendingTabDetaches.get(tabId) || Promise.resolve();
  const current = previous.catch(() => {}).then(() => performDebuggerDetach(tabId));
  trackTabDetach(tabId, current);
  return current;
}

function trackTabDetach(tabId, current) {
  pendingTabDetaches.set(tabId, current);
  const cleanup = () => { if (pendingTabDetaches.get(tabId) === current) pendingTabDetaches.delete(tabId); };
  current.then(cleanup, cleanup);
}

async function performDebuggerDetach(tabId) {
  try { await chrome.debugger.detach({ tabId }); } catch {}
}

async function attachDebugger(tabId) {
  await (pendingTabDetaches.get(tabId) || Promise.resolve()).catch(() => {});
  return chrome.debugger.attach({ tabId }, "1.3");
}

function queueNativeMutation(operation) {
  const result = nativeMutation.then(operation, operation);
  nativeMutation = result.catch(() => {});
  return result;
}

function sendNative(port, generation, message) {
  if (nativePort !== port || nativePortGeneration !== generation) return false;
  try { port.postMessage(message); return true; } catch { return false; }
}

function safeNativeError(value) {
  if (typeof value !== "string" || !value) return undefined;
  return value.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 256);
}

function safeErrorClass(error) {
  try {
    const name = error?.name;
    return typeof name === "string" && /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(name) ? name : "Error";
  } catch { return "Error"; }
}

function traceSharedCommand(phase, request, tabId, startedAt, details = {}) {
  if (!SHARED_COMMAND_METHODS.has(request.method) || !Number.isSafeInteger(tabId)) return;
  const commandId = typeof request.id === "string" || Number.isSafeInteger(request.id)
    ? String(request.id).replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 128) : "";
  try {
    console.debug("[Chrome Controlla] shared command", {
      phase, commandId, targetId: tabId, method: request.method,
      elapsedMs: Math.max(0, Date.now() - startedAt), ...details
    });
  } catch {}
}

function scheduleNativeReconnect() {
  if (reconnectTimer || nativePort) return;
  const delay = Math.min(RECONNECT_BASE_MS * (2 ** reconnectAttempts++), RECONNECT_MAX_MS);
  chrome.alarms.create(RECONNECT_ALARM, { delayInMinutes: Math.max(delay, 30000) / 60000 });
  reconnectTimer = setTimeout(() => { reconnectTimer = undefined; connectNative(); }, delay);
}

chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm.name !== RECONNECT_ALARM) return;
  if (reconnectTimer) clearTimeout(reconnectTimer);
  reconnectTimer = undefined;
  connectNative();
});

function connectNative() {
  if (nativePort) return;
  try {
    const port = chrome.runtime.connectNative(NATIVE_HOST);
    nativePort = port;
    const portGeneration = ++nativePortGeneration;
    nativeReady = false;
    port.onMessage.addListener(message => { void handleNativeMessage(port, portGeneration, message); });
    port.onDisconnect.addListener(() => {
      if (nativePort !== port) return;
      nativeError = safeNativeError(chrome.runtime.lastError?.message);
      nativePort = undefined;
      nativeReady = false;
      const generation = ++nativeGeneration;
      void queueNativeMutation(() => detachNative(generation));
      scheduleNativeReconnect();
    });
  } catch {
    scheduleNativeReconnect();
  }
}

async function handleNativeMessage(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration || !message || typeof message.type !== "string") return;
  if (message.type === "ready") { nativeReady = true; nativeError = undefined; reconnectAttempts = 0; return; }
  if (message.type === "list_tabs") {
    try {
      const tabs = await chrome.tabs.query({});
      const eligible = tabs.filter(tab => Number.isSafeInteger(tab.id) && /^https?:\/\//i.test(tab.url || ""));
      const listed = await Promise.all(eligible.slice(0, 100).map(async tab => {
        const firstFrame = await chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }).catch(() => undefined);
        const current = await chrome.tabs.get(tab.id).catch(() => undefined);
        const lastFrame = await chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }).catch(() => undefined);
        const stableDocument = current?.status === "complete"
          && /^https?:\/\//i.test(current.url || "")
          && firstFrame?.documentId && firstFrame.documentId === lastFrame?.documentId
          && lastFrame.url === current.url;
        return { id: tab.id, title: String(current?.title || tab.title || "").slice(0, 256),
          url: String(current?.url || tab.url).slice(0, 2048),
          ...(stableDocument ? { document_id: lastFrame.documentId } : {}) };
      }));
      sendNative(port, portGeneration, { type: "tabs", request_id: message.request_id, truncated: eligible.length > 100, tabs: listed });
    } catch (error) {
      sendNative(port, portGeneration, { type: "tabs_error", request_id: message.request_id, error: String(error).slice(0, 512) });
    }
    return;
  }
  if (message.type === "pair") {
    await queueNativeMutation(() => pairNative(port, portGeneration, message));
    return;
  }
  if (message.type === "release") {
    await queueNativeMutation(() => detachNative(++nativeGeneration));
    return;
  }
  if (message.type === "command") {
    const commandGeneration = nativeGeneration;
    const startedAt = Date.now();
    await dispatchCommand(message, tabId => nativeGeneration === commandGeneration && nativeAttachedTabs.has(tabId), response => {
      const sent = nativeGeneration === commandGeneration && sendNative(port, portGeneration, response);
      traceSharedCommand("reply", message, Number(message.target_id), startedAt, {
        outcome: nativeGeneration !== commandGeneration ? "suppressed" : sent ? "sent" : "failed"
      });
    });
  }
}

async function pairNative(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration) return;
  const requested = message.tab_ids;
  const expectedUrls = message.expected_urls && typeof message.expected_urls === "object" ? message.expected_urls : undefined;
  const expectedDocumentIds = message.expected_document_ids && typeof message.expected_document_ids === "object" ? message.expected_document_ids : undefined;
  let attemptGeneration;
  let reservation;
  try {
    if (typeof message.request_id !== "string" || !message.request_id
        || !Array.isArray(requested) || requested.length === 0
        || requested.some(id => !Number.isSafeInteger(id) || id < 0)
        || new Set(requested).size !== requested.length) {
      throw new Error("pair requires a request_id and unique nonnegative numeric tab_ids.");
    }
    if (requested.some(id => manualAttachedTabs.has(id))) throw new Error("A requested tab is already paired through the popup WebSocket.");
    reservation = reservePairingTabs(requested);
    attemptGeneration = ++nativeGeneration;
    await detachNative(attemptGeneration);
    for (const tabId of requested) {
      // Bind the attachment to the page identity selected at discovery: a tab
      // that navigated between discovery and pairing must not hand this session
      // to a different page.
      const navigationGeneration = tabNavigationGenerations.get(tabId) || 0;
      const beforeAttach = await chrome.tabs.get(tabId).catch(() => null);
      const beforeFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (!beforeAttach || beforeAttach.status !== "complete"
          || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)
          || !expectedUrls || typeof expectedUrls[tabId] !== "string"
          || beforeAttach.url !== expectedUrls[tabId]
          || !expectedDocumentIds || typeof expectedDocumentIds[tabId] !== "string"
          || !beforeFrame?.documentId || beforeFrame.documentId !== expectedDocumentIds[tabId]) {
        throw new Error("Tab " + tabId + " navigated since discovery; rediscover tabs and pair again.");
      }
      pairedTabUrls.set(tabId, beforeAttach.url);
      pairedTabDocumentIds.set(tabId, beforeFrame.documentId);
      await attachDebugger(tabId);
      if (port !== nativePort || attemptGeneration !== nativeGeneration) {
        await detachDebugger(tabId);
        throw new Error("Pairing was replaced.");
      }
      nativeAttachedTabs.add(tabId);
      const attached = await chrome.tabs.get(tabId).catch(() => null);
      const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)
          || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url
          || attachedFrame?.documentId !== beforeFrame.documentId) {
        throw new Error("Tab " + tabId + " navigated during pairing; rediscover tabs and pair again.");
      }
    }
    sendNative(port, portGeneration, { type: "paired", request_id: message.request_id,
      targets: [...nativeAttachedTabs].map(String).sort(), extension_version: chrome.runtime.getManifest().version,
      document_identity: true });
  } catch (error) {
    if (attemptGeneration !== undefined) await detachNative(attemptGeneration);
    if (reservation) {
      for (const tabId of requested || []) {
        if (manualAttachedTabs.has(tabId)) continue;
        pairedTabUrls.delete(tabId);
        pairedTabDocumentIds.delete(tabId);
      }
    }
    sendNative(port, portGeneration, { type: "pair_error", request_id: message.request_id, error: String(error) });
  } finally {
    if (reservation) releasePairingTabs(requested, reservation);
  }
}

async function dispatchCommand(request, isAuthorized, respond) {
  const tabId = Number(request.target_id);
  const startedAt = Date.now();
  if (!Number.isSafeInteger(tabId) || !isAuthorized(tabId)) {
    respond({ type: "result", id: request.id, error: "Target is not attached." });
    return;
  }
  if (navigatingTabs.has(tabId)) {
    respond({ type: "result", id: request.id, error: "Tab is navigating; retry after the new document commits." });
    return;
  }
  const nativeRoute = nativeAttachedTabs.has(tabId);
  const routeGeneration = nativeRoute ? nativeGeneration : manualGeneration;
  try {
    const expectedDocumentId = pairedTabDocumentIds.get(tabId);
    const frame = expectedDocumentId
      ? await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined)
      : undefined;
    if (!isAuthorized(tabId)) {
      respond({ type: "result", id: request.id, error: "Target is no longer attached." });
      return;
    }
    if (navigatingTabs.has(tabId)) {
      respond({ type: "result", id: request.id, error: "Tab is navigating; retry after the new document commits." });
      return;
    }
    if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId) {
      respond({ type: "result", id: request.id, error: "Target document identity changed; rediscover and pair again." });
      return;
    }
    if (!frame?.documentId || frame.documentId !== expectedDocumentId) {
      await detachPairedTab(tabId);
      respond({ type: "result", id: request.id, error: "Target document identity changed; pair the tab again." });
      return;
    }
    if (request.method === "Controlla.openTab") {
      const url = validateNavigationUrl(request.params?.url);
      const native = nativeRoute;
      const tab = await chrome.tabs.create({ url: "about:blank", active: true });
      let identity;
      let attachedIdentity;
      try {
        attachedIdentity = await attachNewTab(tab.id, url, native, routeGeneration);
        identity = await nativePageIdentity(tab.id);
      } catch (error) {
        if (attachedIdentity && routeGeneration === (native ? nativeGeneration : manualGeneration)
            && pairedTabDocumentIds.get(tab.id) === attachedIdentity.document_id
            && (native ? nativeAttachedTabs : manualAttachedTabs).has(tab.id)) {
          pairedTabUrls.delete(tab.id);
          pairedTabDocumentIds.delete(tab.id);
          (native ? nativeAttachedTabs : manualAttachedTabs).delete(tab.id);
          await detachDebugger(tab.id);
        }
        throw new Error("Tab " + tab.id + " remains open after the failed open; rediscover and pair it before use. " + String(error));
      }
      respond({ type: "result", id: request.id, result: { tab_id: String(tab.id), ...identity } });
      return;
    }
    if (request.method === "Controlla.navigate") {
      const url = validateNavigationUrl(request.params?.url);
      await navigatePairedTab(tabId, url, nativeRoute, routeGeneration);
      respond({ type: "result", id: request.id, result: { tab_id: String(tabId), ...(await nativePageIdentity(tabId)) } });
      return;
    }
    if (!SHARED_COMMAND_METHODS.has(request.method)) {
      throw new Error("Command is outside the shared observe/input allowlist.");
    }
    traceSharedCommand("dispatch", request, tabId, startedAt);
    const result = await chrome.debugger.sendCommand({ tabId }, request.method, request.params || {});
    traceSharedCommand("settled", request, tabId, startedAt, { outcome: "result" });
    respond({ type: "result", id: request.id, result });
  } catch (error) {
    traceSharedCommand("settled", request, tabId, startedAt, { outcome: "error", errorClass: safeErrorClass(error) });
    respond({ type: "result", id: request.id, error: String(error) });
  }
}

function validateNavigationUrl(value) {
  if (typeof value !== "string" || value.length > 2048) throw new Error("URL must be a bounded HTTP(S) URL.");
  const url = new URL(value);
  if (!(url.protocol === "http:" || url.protocol === "https:") || !url.hostname || url.username || url.password) {
    throw new Error("URL must be HTTP(S) and cannot contain credentials.");
  }
  return url.href;
}

async function nativePageIdentity(tabId) {
  const tab = await chrome.tabs.get(tabId);
  const frame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 });
  if (tab.status !== "complete" || !frame?.documentId || frame.url !== tab.url) throw new Error("Navigation completed without a stable document identity.");
  return { url: tab.url, document_id: frame.documentId };
}

async function waitForTabComplete(tabId) {
  const current = await chrome.tabs.get(tabId);
  if (current.status === "complete") return;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { chrome.tabs.onUpdated.removeListener(listener); reject(new Error("Tab navigation timed out.")); }, 30000);
    const listener = (updatedId, info) => {
      if (updatedId !== tabId || info.status !== "complete") return;
      clearTimeout(timer); chrome.tabs.onUpdated.removeListener(listener); resolve();
    };
    chrome.tabs.onUpdated.addListener(listener);
  });
}

function routeActive(native, generation) {
  return native ? nativeGeneration === generation && !!nativePort : manualGeneration === generation && !!socket && socket.readyState === WebSocket.OPEN;
}

async function attachNewTab(tabId, url, native, generation) {
  await chrome.tabs.update(tabId, { url });
  await waitForTabComplete(tabId);
  if (!routeActive(native, generation)) throw new Error("Shared session ended during tab navigation.");
  const navigationGeneration = tabNavigationGenerations.get(tabId) || 0;
  const identity = await nativePageIdentity(tabId);
  await attachDebugger(tabId);
  const attachedIdentity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!routeActive(native, generation) || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)
      || attachedIdentity?.url !== identity.url || attachedIdentity?.document_id !== identity.document_id) {
    await detachDebugger(tabId);
    throw new Error("Tab changed during attachment; retry from a fresh inventory.");
  }
  pairedTabUrls.set(tabId, attachedIdentity.url);
  pairedTabDocumentIds.set(tabId, attachedIdentity.document_id);
  (native ? nativeAttachedTabs : manualAttachedTabs).add(tabId);
  return attachedIdentity;
}

async function navigatePairedTab(tabId, url, native, generation) {
  (native ? nativeAttachedTabs : manualAttachedTabs).delete(tabId);
  pairedTabUrls.delete(tabId);
  pairedTabDocumentIds.delete(tabId);
  await detachDebugger(tabId);
  await chrome.tabs.update(tabId, { url });
  await waitForTabComplete(tabId);
  if (!routeActive(native, generation)) throw new Error("Shared session ended during tab navigation.");
  const navigationGeneration = tabNavigationGenerations.get(tabId) || 0;
  const identity = await nativePageIdentity(tabId);
  await attachDebugger(tabId);
  const attachedIdentity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!routeActive(native, generation) || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)
      || attachedIdentity?.url !== identity.url || attachedIdentity?.document_id !== identity.document_id) {
    await detachDebugger(tabId);
    throw new Error("Tab changed during attachment; retry from a fresh inventory.");
  }
  pairedTabUrls.set(tabId, attachedIdentity.url);
  pairedTabDocumentIds.set(tabId, attachedIdentity.document_id);
  (native ? nativeAttachedTabs : manualAttachedTabs).add(tabId);
}

async function detachPairedTab(tabId) {
  navigatingTabs.delete(tabId);
  debuggerDetachedDuringNavigation.delete(tabId);
  pairedTabUrls.delete(tabId);
  pairedTabDocumentIds.delete(tabId);
  manualAttachedTabs.delete(tabId);
  nativeAttachedTabs.delete(tabId);
  await detachDebugger(tabId);
}

async function detachManual(generation) {
  if (generation !== manualGeneration) return;
  const toDetach = [...manualAttachedTabs];
  manualAttachedTabs = new Set();
  for (const tabId of toDetach) { navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); }
  await Promise.all(toDetach.map(detachDebugger));
}

async function detachNative(generation) {
  if (generation !== nativeGeneration) return;
  const toDetach = [...nativeAttachedTabs];
  nativeAttachedTabs = new Set();
  for (const tabId of toDetach) { navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); }
  await Promise.all(toDetach.map(detachDebugger));
}

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message?.type === "pair") {
    pair(message.endpoint, message.token, message.tabIds)
      .then(() => respond({ ok: true }))
      .catch(error => respond({ ok: false, error: String(error) }));
    return true;
  }
  if (message?.type === "status") {
    respond({ connected: socket?.readyState === WebSocket.OPEN, attached: [...manualAttachedTabs] });
  }
  if (message?.type === "release") {
    const current = socket;
    const generation = ++manualGeneration;
    socket = undefined;
    current?.close();
    detachManual(generation).then(() => respond({ ok: true }));
    return true;
  }
  if (message?.type === "native-status") {
    respond({ connected: nativeReady, attached: [...nativeAttachedTabs], error: nativeError, transport: "native" });
  }
});

async function pair(endpoint, token, selectedTabIds) {
  if (!Array.isArray(selectedTabIds) || selectedTabIds.length === 0 || !token || !endpoint) {
    throw new Error("Choose tabs and provide the local pairing endpoint and token.");
  }
  const tabs = [...new Set(selectedTabIds.map(Number))];
  if (tabs.some(id => !Number.isInteger(id) || id < 0)) throw new Error("Invalid tab selection.");
  const endpointUrl = new URL(endpoint);
  if (endpointUrl.protocol !== "ws:" || endpointUrl.hostname !== "127.0.0.1"
      || endpointUrl.username || endpointUrl.password || endpointUrl.search || endpointUrl.hash) {
    throw new Error("The pairing endpoint must use ws://127.0.0.1.");
  }
  if (tabs.some(tabId => nativeAttachedTabs.has(tabId))) throw new Error("A selected tab is already paired through Native Messaging.");
  const reservation = reservePairingTabs(tabs);
  return pairManual(endpointUrl, token, tabs).finally(() => releasePairingTabs(tabs, reservation));
}

async function pairManual(endpointUrl, token, tabs) {
  const generation = ++manualGeneration;
  const previousSocket = socket;
  socket = undefined;
  previousSocket?.close();
  await detachManual(generation);
  try {
    for (const tabId of tabs) {
      const navigationGeneration = tabNavigationGenerations.get(tabId) || 0;
      const beforeAttach = await chrome.tabs.get(tabId).catch(() => null);
      const beforeFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (!beforeAttach || beforeAttach.status !== "complete"
          || !beforeFrame?.documentId
          || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)) {
        throw new Error("Tab " + tabId + " is navigating or its document identity is unavailable; wait for it to finish and pair again.");
      }
      pairedTabUrls.set(tabId, beforeAttach.url);
      pairedTabDocumentIds.set(tabId, beforeFrame.documentId);
      await attachDebugger(tabId);
      if (generation !== manualGeneration) {
        await detachDebugger(tabId);
        throw new Error("Pairing was replaced.");
      }
      manualAttachedTabs.add(tabId);
      const attached = await chrome.tabs.get(tabId).catch(() => null);
      const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)
          || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url
          || attachedFrame?.documentId !== beforeFrame.documentId) {
        throw new Error("Tab " + tabId + " navigated during pairing; pair again.");
      }
    }
  } catch (error) {
    await detachManual(generation);
    for (const tabId of tabs) { pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); }
    throw error;
  }
  const next = new WebSocket(endpointUrl.href);
  socket = next;
  try {
    await new Promise((resolve, reject) => {
      next.onopen = resolve;
      next.onerror = () => reject(new Error("Cannot connect to the loopback provider."));
      next.onclose = () => reject(new Error("Loopback pairing was closed."));
    });
    if (generation !== manualGeneration) throw new Error("Pairing was replaced.");
    const extensionVersion = chrome.runtime.getManifest().version;
    next.send(JSON.stringify({
      type: "hello", token, extension_version: extensionVersion,
      document_identity: true,
      targets: [...manualAttachedTabs].map(String).sort()
    }));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Provider did not confirm pairing.")), 5000);
      next.onmessage = event => {
        const reply = JSON.parse(event.data);
        if (reply.type === "ready" && reply.server_version === extensionVersion) { clearTimeout(timer); resolve(); }
        else { clearTimeout(timer); reject(new Error(reply.error || "Pairing rejected.")); }
      };
    });
  } catch (error) {
    next.close();
    if (socket === next) socket = undefined;
    await detachManual(generation);
    throw error;
  }
  if (generation !== manualGeneration) {
    next.close();
    return;
  }
  next.onmessage = async event => {
    if (generation !== manualGeneration || socket !== next) return;
    const request = JSON.parse(event.data);
    if (request.type === "command") {
      await dispatchCommand(request, tabId => generation === manualGeneration && socket === next && manualAttachedTabs.has(tabId), response => {
        if (generation === manualGeneration && socket === next) next.send(JSON.stringify(response));
      });
    }
  };
  next.onclose = () => {
    if (generation !== manualGeneration) return;
    socket = undefined;
    void detachManual(generation);
  };
  next.onerror = () => {
    if (generation !== manualGeneration) return;
    socket = undefined;
    void detachManual(generation);
  };
}

chrome.debugger.onDetach.addListener(({ tabId }, reason) => {
  if (!Number.isInteger(tabId)) return;
  const committed = committedNavigationTabs.get(tabId);
  if (reason === "target_closed" && (navigatingTabs.has(tabId) || committed) && pairedTabDocumentIds.has(tabId)) {
    debuggerDetachedDuringNavigation.add(tabId);
    if (committed && !navigatingTabs.has(tabId)) {
      void reattachAfterNavigation(tabId, committed.native, committed.generation, committed.url, committed.documentId)
        .then(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) debuggerDetachedDuringNavigation.delete(tabId); })
        .catch(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) void detachPairedTab(tabId); });
    }
    return;
  }
  manualAttachedTabs.delete(tabId); nativeAttachedTabs.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId);
  navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); committedNavigationTabs.delete(tabId);
});

async function reattachAfterNavigation(tabId, native, generation, expectedUrl, expectedDocumentId) {
  await waitForTabComplete(tabId);
  const navigationGeneration = tabNavigationGenerations.get(tabId);
  const current = () => navigationGeneration === tabNavigationGenerations.get(tabId)
    && pairedTabUrls.get(tabId) === expectedUrl
    && pairedTabDocumentIds.get(tabId) === expectedDocumentId;
  if (!current()) return;
  if (!routeActive(native, generation) || !(native ? nativeAttachedTabs : manualAttachedTabs).has(tabId)) {
    throw new Error("Shared tab session ended during navigation.");
  }
  let attaching = navigationDebuggerAttaches.get(tabId);
  if (!attaching) {
    attaching = attachDebugger(tabId);
    navigationDebuggerAttaches.set(tabId, attaching);
  }
  try { await attaching; }
  finally { if (navigationDebuggerAttaches.get(tabId) === attaching) navigationDebuggerAttaches.delete(tabId); }
  const identity = await nativePageIdentity(tabId);
  if (!current()) return;
  if (!routeActive(native, generation) || identity.url !== expectedUrl || identity.document_id !== expectedDocumentId) {
    await detachDebugger(tabId);
    return;
  }
  pairedTabUrls.set(tabId, identity.url);
  pairedTabDocumentIds.set(tabId, identity.document_id);
}

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === "loading") {
    tabNavigationGenerations.set(tabId, (tabNavigationGenerations.get(tabId) || 0) + 1);
    if (pairedTabDocumentIds.has(tabId)) navigatingTabs.add(tabId);
  }
  if (changeInfo.status === "complete" && !debuggerDetachedDuringNavigation.has(tabId)) {
    navigatingTabs.delete(tabId);
  }
  const pairedUrl = pairedTabUrls.get(tabId);
  if (pairedUrl === undefined || !changeInfo.url || changeInfo.url === "about:blank" || navigatingTabs.has(tabId)) return;
  const changed = (() => {
    try {
      const before = new URL(pairedUrl), after = new URL(changeInfo.url);
      return before.origin !== after.origin || before.pathname !== after.pathname || before.search !== after.search;
    } catch { return changeInfo.url !== pairedUrl; }
  })();
  if (changed) void detachPairedTab(tabId);
});

chrome.webNavigation.onBeforeNavigate.addListener(details => {
  if (details.frameId !== 0) return;
  tabNavigationGenerations.set(details.tabId, (tabNavigationGenerations.get(details.tabId) || 0) + 1);
  if (pairedTabDocumentIds.has(details.tabId)) navigatingTabs.add(details.tabId);
});

chrome.webNavigation.onCommitted.addListener(details => {
  if (details.frameId !== 0) return;
  tabNavigationGenerations.set(details.tabId, (tabNavigationGenerations.get(details.tabId) || 0) + 1);
  if (!pairedTabDocumentIds.has(details.tabId)) return;
  let url;
  try { url = validateNavigationUrl(details.url); } catch { void detachPairedTab(details.tabId); return; }
  if (typeof details.documentId !== "string" || !details.documentId) { void detachPairedTab(details.tabId); return; }
  pairedTabUrls.set(details.tabId, url);
  pairedTabDocumentIds.set(details.tabId, details.documentId);
  const native = nativeAttachedTabs.has(details.tabId);
  const generation = native ? nativeGeneration : manualGeneration;
  committedNavigationTabs.set(details.tabId, { native, generation, url, documentId: details.documentId });
  setTimeout(() => {
    if (committedNavigationTabs.get(details.tabId)?.documentId === details.documentId) committedNavigationTabs.delete(details.tabId);
  }, 3000);
  if (!debuggerDetachedDuringNavigation.has(details.tabId)) {
    navigatingTabs.delete(details.tabId);
    return;
  }
  void reattachAfterNavigation(details.tabId, native, generation, url, details.documentId)
    .then(() => {
      if (pairedTabDocumentIds.get(details.tabId) === details.documentId) {
        debuggerDetachedDuringNavigation.delete(details.tabId);
        navigatingTabs.delete(details.tabId);
      }
    })
    .catch(() => {
      if (pairedTabDocumentIds.get(details.tabId) === details.documentId) void detachPairedTab(details.tabId);
    });
});

chrome.webNavigation.onErrorOccurred.addListener(details => {
  if (details.frameId === 0 && navigatingTabs.has(details.tabId)) void detachPairedTab(details.tabId);
});

chrome.tabs.onReplaced.addListener((_addedTabId, removedTabId) => {
  tabNavigationGenerations.set(removedTabId, (tabNavigationGenerations.get(removedTabId) || 0) + 1);
  if (!pairedTabUrls.has(removedTabId)) return;
  navigatingTabs.delete(removedTabId);
  debuggerDetachedDuringNavigation.delete(removedTabId);
  pairedTabUrls.delete(removedTabId);
  pairedTabDocumentIds.delete(removedTabId);
  manualAttachedTabs.delete(removedTabId);
  nativeAttachedTabs.delete(removedTabId);
  void detachDebugger(removedTabId);
});

connectNative();
