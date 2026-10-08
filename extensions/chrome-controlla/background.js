let socket;
const NATIVE_HOST = "chrome_controlla_bridge";
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 60000;
const RECONNECT_ALARM = "controlla-native-reconnect";
let nativePort;
let nativeReady = false;
let nativeError;
let reconnectTimer;
let reconnectAttempts = 0;
let manualAttachedTabs = new Set();
let nativeAttachedTabs = new Set();
let manualGeneration = 0;
let nativeGeneration = 0;
let nativePortGeneration = 0;
let nativeMutation = Promise.resolve();

function queueNativeMutation(operation) {
  const result = nativeMutation.then(operation, operation);
  nativeMutation = result.catch(() => {});
  return result;
}

function sendNative(port, generation, message) {
  if (nativePort !== port || nativePortGeneration !== generation) return;
  try { port.postMessage(message); } catch {}
}

function safeNativeError(value) {
  if (typeof value !== "string" || !value) return undefined;
  return value.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 256);
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
      sendNative(port, portGeneration, { type: "tabs", request_id: message.request_id, truncated: eligible.length > 100,
        tabs: eligible.slice(0, 100)
          .map(tab => ({ id: tab.id, title: String(tab.title || "").slice(0, 256), url: String(tab.url).slice(0, 2048) })) });
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
    await dispatchCommand(message, nativeAttachedTabs, response => {
      if (nativeGeneration === commandGeneration) sendNative(port, portGeneration, response);
    });
  }
}

async function pairNative(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration) return;
  const requested = message.tab_ids;
  let attemptGeneration;
  try {
    if (typeof message.request_id !== "string" || !message.request_id
        || !Array.isArray(requested) || requested.length === 0
        || requested.some(id => !Number.isSafeInteger(id) || id < 0)
        || new Set(requested).size !== requested.length) {
      throw new Error("pair requires a request_id and unique nonnegative numeric tab_ids.");
    }
    if (requested.some(id => manualAttachedTabs.has(id))) throw new Error("A requested tab is already paired through the popup WebSocket.");
    attemptGeneration = ++nativeGeneration;
    await detachNative(attemptGeneration);
    for (const tabId of requested) {
      await chrome.debugger.attach({ tabId }, "1.3");
      if (port !== nativePort || attemptGeneration !== nativeGeneration) {
        await chrome.debugger.detach({ tabId }).catch(() => {});
        throw new Error("Pairing was replaced.");
      }
      nativeAttachedTabs.add(tabId);
    }
    sendNative(port, portGeneration, { type: "paired", request_id: message.request_id,
      targets: [...nativeAttachedTabs].map(String).sort(), extension_version: chrome.runtime.getManifest().version });
  } catch (error) {
    if (attemptGeneration !== undefined) await detachNative(attemptGeneration);
    sendNative(port, portGeneration, { type: "pair_error", request_id: message.request_id, error: String(error) });
  }
}

async function dispatchCommand(request, authorizedTabs, respond) {
  const tabId = Number(request.target_id);
  if (!Number.isSafeInteger(tabId) || !authorizedTabs.has(tabId)) {
    respond({ type: "result", id: request.id, error: "Target is not attached." });
    return;
  }
  try {
    if (!["Page.getFrameTree", "Runtime.evaluate", "Input.dispatchMouseEvent",
      "DOM.getDocument", "DOM.querySelector", "Accessibility.getPartialAXTree"].includes(request.method)) {
      throw new Error("Command is outside the shared observe/input allowlist.");
    }
    const result = await chrome.debugger.sendCommand({ tabId }, request.method, request.params || {});
    respond({ type: "result", id: request.id, result });
  } catch (error) {
    respond({ type: "result", id: request.id, error: String(error) });
  }
}

async function detachManual(generation) {
  if (generation !== manualGeneration) return;
  const toDetach = [...manualAttachedTabs];
  manualAttachedTabs = new Set();
  await Promise.all(toDetach.map(tabId => chrome.debugger.detach({ tabId }).catch(() => {})));
}

async function detachNative(generation) {
  if (generation !== nativeGeneration) return;
  const toDetach = [...nativeAttachedTabs];
  nativeAttachedTabs = new Set();
  await Promise.all(toDetach.map(tabId => chrome.debugger.detach({ tabId }).catch(() => {})));
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
  const generation = ++manualGeneration;
  const previousSocket = socket;
  socket = undefined;
  previousSocket?.close();
  await detachManual(generation);
  try {
    for (const tabId of tabs) {
      await chrome.debugger.attach({ tabId }, "1.3");
      if (generation !== manualGeneration) {
        await chrome.debugger.detach({ tabId }).catch(() => {});
        throw new Error("Pairing was replaced.");
      }
      manualAttachedTabs.add(tabId);
    }
  } catch (error) {
    await detachManual(generation);
    throw error;
  }
  const next = new WebSocket(endpoint);
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
      const authorizedTabs = new Set(manualAttachedTabs);
      await dispatchCommand(request, authorizedTabs, response => {
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

chrome.debugger.onDetach.addListener(({ tabId }) => {
  if (Number.isInteger(tabId)) { manualAttachedTabs.delete(tabId); nativeAttachedTabs.delete(tabId); }
});

connectNative();
