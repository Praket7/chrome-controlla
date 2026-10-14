let socket;
const NATIVE_HOST = "chrome_controlla_bridge";
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 60000;
const MAX_RECONNECT_ATTEMPTS = 8;
const RECONNECT_ALARM = "controlla-native-reconnect";
const RECONNECT_STORAGE_KEY = "nativeReconnectAttempts";
const OPEN_RECEIPTS_KEY = "openReceipts";
const OPEN_RECEIPT_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const OPEN_RECEIPT_MAX_COUNT = 256;
const MAX_BATCH_ACTIONS = 64;
const SHARED_COMMAND_METHODS = new Set(["Page.getFrameTree", "Runtime.evaluate", "Runtime.callFunctionOn", "Runtime.releaseObject",
  "Input.dispatchMouseEvent", "Input.dispatchTouchEvent", "Input.dispatchKeyEvent", "Input.imeSetComposition", "Input.insertText", "DOM.getDocument", "DOM.querySelector", "DOM.enable", "DOM.describeNode", "DOM.resolveNode", "DOM.setFileInputFiles", "Page.enable", "Page.setInterceptFileChooserDialog", "Accessibility.getPartialAXTree", "Accessibility.getFullAXTree", "Controlla.waitForFileChooser"]);
let nativePort;
let nativeReady = false;
let nativeError;
let reconnectExhausted = false;
let reconnectTimer;
let reconnectAttempts = 0;
const reconnectStateReady = chrome.storage.session.get(RECONNECT_STORAGE_KEY).then(state => {
  reconnectAttempts = Number.isSafeInteger(state[RECONNECT_STORAGE_KEY])
    ? Math.max(0, Math.min(state[RECONNECT_STORAGE_KEY], MAX_RECONNECT_ATTEMPTS)) : 0;
  reconnectExhausted = false;
}).catch(() => {
  reconnectAttempts = 0;
  reconnectExhausted = false;
  nativeError = "Native host reconnect state unavailable; retrying with bounded backoff.";
});
let manualAttachedTabs = new Set();
let nativeAttachedTabs = new Set();
const nativeLeases = new Map();
const nativeTabLeases = new Map();
const nativeLeaseGenerations = new Set();
const pairedTabUrls = new Map();
const pairedTabDocumentIds = new Map();
const domReadyDocuments = new Map();
const tabNavigationGenerations = new Map();
const pendingMainFrameNavigations = new Map();
const navigatingTabs = new Set();
const debuggerDetachedDuringNavigation = new Set();
const committedNavigationTabs = new Map();
const navigationDebuggerAttaches = new Map();
let manualGeneration = 0;
let nativeGeneration = 0;
let nextNativeLeaseGeneration = 0;
let nativePortGeneration = 0;
let nativeMutation = Promise.resolve();
const pendingTabDetaches = new Map();
const tabPairReservations = new Map();
const fileChooserEvents = new Map();
const fileChooserWaiters = new Map();
const fileChooserOperations = new Map();
const recentlyCreatedTabs = new Map();
let fileChooserEventSequence = 0;
const frameSessions = new Map();
const frameSessionWaiters = new Map();
const pendingSecureInputs = new Map();
const openTabInflight = new Map();
let openTabQueue = Promise.resolve();
let activationEpoch = 0;
let focusEpoch = 0;
chrome.tabs.onActivated.addListener(() => { activationEpoch++; });
chrome.tabs.onCreated.addListener(tab => { if (Number.isSafeInteger(tab.id)) { recentlyCreatedTabs.set(tab.id, { opener_tab_id: tab.openerTabId, window_id: tab.windowId, created_at_ms: Date.now() }); for (const [id, value] of recentlyCreatedTabs) if (value.created_at_ms < Date.now() - 60000) recentlyCreatedTabs.delete(id); } });
chrome.windows.onFocusChanged.addListener(() => { focusEpoch++; });
function recordFrameSession(tabId, frameId, sessionId) {
  let frames = frameSessions.get(tabId);
  if (!frames) { frames = new Map(); frameSessions.set(tabId, frames); }
  frames.set(frameId, sessionId);
  const waiter = frameSessionWaiters.get(`${tabId}:${frameId}`);
  if (waiter) { clearTimeout(waiter.timer); frameSessionWaiters.delete(`${tabId}:${frameId}`); waiter.resolve(sessionId); }
}
function isExactHttpUrl(value) {
  if (typeof value !== "string" || !value || value.length > 2048 || /\s/.test(value)) return false;
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && !!url.hostname && !url.username && !url.password;
  } catch { return false; }
}
function waitForFrameSession(tabId, frameId, timeoutMs) {
  const existing = frameSessions.get(tabId)?.get(frameId);
  if (existing) return Promise.resolve(existing);
  const key = `${tabId}:${frameId}`;
  if (frameSessionWaiters.has(key)) return frameSessionWaiters.get(key).promise;
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  const waiter = { promise, resolve, timer: setTimeout(() => { frameSessionWaiters.delete(key); resolve(undefined); }, timeoutMs) };
  frameSessionWaiters.set(key, waiter);
  return promise;
}
function clearFrameSessions(tabId) {
  frameSessions.delete(tabId);
  for (const [key, waiter] of frameSessionWaiters) if (key.startsWith(`${tabId}:`)) { clearTimeout(waiter.timer); frameSessionWaiters.delete(key); waiter.resolve(undefined); }
}

function clearFileChooserState(tabId, error = "Target detached before the file chooser opened.") {
  fileChooserEvents.delete(tabId);
  const waiter = fileChooserWaiters.get(tabId);
  if (waiter) { clearTimeout(waiter.timer); fileChooserWaiters.delete(tabId); waiter.reject(new Error(error)); }
}
function waitForFileChooser(tabId, documentId, timeoutMs, afterSequence = 0) {
  const queued = fileChooserEvents.get(tabId) || [];
  const index = queued.findIndex(event => event.root_document_id === documentId && event.sequence > afterSequence);
  if (index >= 0) { const [event] = queued.splice(index, 1); return Promise.resolve(event); }
  if (fileChooserWaiters.has(tabId)) return Promise.reject(new Error("A file chooser wait is already active for this tab."));
  return new Promise((resolve, reject) => {
    const waiter = { documentId, afterSequence, resolve, reject, timer: setTimeout(() => {
      if (fileChooserWaiters.get(tabId) === waiter) fileChooserWaiters.delete(tabId);
      reject(new Error("No file chooser opened before the bounded wait expired."));
    }, timeoutMs) };
    fileChooserWaiters.set(tabId, waiter);
  });
}
async function frameActionablePoint(route, objectId) {
  const result = await chrome.debugger.sendCommand(route, "Runtime.callFunctionOn", { objectId, functionDeclaration: "function __controllaFrameActionable(){const e=this,r=e.getBoundingClientRect(),v=e.ownerDocument.defaultView;if(!e.isConnected||!r.width||!r.height||r.left<0||r.top<0||r.right>v.innerWidth||r.bottom>v.innerHeight)return false;for(let n=e;n;n=n.parentElement){const s=v.getComputedStyle(n);if(n.disabled||n.getAttribute('aria-disabled')==='true'||n.hasAttribute('inert')||n.hasAttribute('hidden')||s.display==='none'||s.visibility!=='visible'||s.pointerEvents==='none'||Number.parseFloat(s.opacity||'1')<=0)return false}const x=r.left+r.width/2,y=r.top+r.height/2,root=e.getRootNode(),h=(root.elementFromPoint?.(x,y)||e.ownerDocument.elementFromPoint(x,y));return !!h&&(h===e||e.contains(h))?{x,y}:false}", returnByValue: true });
  const point = result.result?.value;
  if (!(point?.x >= 0) || !(point?.y >= 0)) throw new Error("Frame control is disabled, outside the viewport, or obscured.");
  return point;
}
async function rootPointForFrame(tabId, frameId, objectId, x, y) {
  const root = { tabId }, tree = await chrome.debugger.sendCommand(root, "Page.getFrameTree", {});
  const path = (node, ancestors = []) => node?.frame?.id === frameId ? ancestors : (node?.childFrames || []).map(child => path(child, [...ancestors, child.frame.id])).find(Boolean);
  const frames = path(tree.frameTree);
  if (!frames?.length || frames.length > 8) throw new Error("Same-process frame ancestry is unavailable or too deep.");
  let viewport = await chrome.debugger.sendCommand(root, "Runtime.callFunctionOn", { objectId, functionDeclaration: "function(){return {width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight}}", returnByValue: true });
  viewport = viewport.result?.value;
  for (const childFrameId of frames.reverse()) {
    const owner = await chrome.debugger.sendCommand(root, "DOM.getFrameOwner", { frameId: childFrameId });
    const box = await chrome.debugger.sendCommand(root, "DOM.getBoxModel", { backendNodeId: owner.backendNodeId });
    const q = box.model?.content;
    if (!Array.isArray(q) || q.length !== 8 || !(viewport?.width > 0) || !(viewport?.height > 0)) throw new Error("Same-process frame geometry is unavailable.");
    const u = x / viewport.width, v = y / viewport.height;
    x = q[0] + u * (q[2] - q[0]) + v * (q[6] - q[0]);
    y = q[1] + u * (q[3] - q[1]) + v * (q[7] - q[1]);
    const parent = await chrome.debugger.sendCommand(root, "DOM.resolveNode", { backendNodeId: owner.backendNodeId });
    if (!parent.object?.objectId) throw new Error("Same-process frame owner could not be resolved.");
    const size = await chrome.debugger.sendCommand(root, "Runtime.callFunctionOn", { objectId: parent.object.objectId, functionDeclaration: "function(){return {width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight}}", returnByValue: true });
    viewport = size.result?.value;
  }
  return { x, y };
}
function fileInputNodes(node, found = []) {
  if (!node || typeof node !== "object") return found;
  if (node.nodeName === "INPUT") {
    const attributes = node.attributes || [];
    for (let index = 0; index < attributes.length; index += 2) {
      if (String(attributes[index]).toLowerCase() === "type" && String(attributes[index + 1]).toLowerCase() === "file" && Number.isSafeInteger(node.backendNodeId)) {
        found.push(node.backendNodeId);
        break;
      }
    }
  }
  for (const child of [...(node.children || []), ...(node.shadowRoots || []), ...(node.contentDocument ? [node.contentDocument] : [])]) fileInputNodes(child, found);
  return found;
}
async function fileInputsInFrames(tabId, frameTree) {
  const root = frameTree?.frame;
  if (!root?.id || !root.loaderId) throw new Error("Main document frame identity is unavailable for file-input discovery.");
  const frames = [];
  const visit = node => { for (const child of node?.childFrames || []) { if (child.frame?.id && child.frame.loaderId) frames.push(child.frame); visit(child); } };
  visit(frameTree);
  if (frames.length > 128) throw new Error("Too many frames for bounded file-input discovery.");
  await chrome.debugger.sendCommand({ tabId }, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
  try {
    const sessions = await Promise.all(frames.map(async child => [child, frameSessions.get(tabId)?.get(child.id) || await waitForFrameSession(tabId, child.id, 400)]));
    const routes = [{ frameId: root.id, loaderId: root.loaderId, target: { tabId } }, ...sessions.filter(([, sessionId]) => sessionId).map(([child, sessionId]) => ({ frameId: child.id, loaderId: child.loaderId, target: { tabId, sessionId } }))];
    const inputs = [];
    const documents = await Promise.all(routes.map(route => chrome.debugger.sendCommand(route.target, "DOM.getDocument", { depth: -1, pierce: true })));
    for (let index = 0; index < routes.length; index++) {
      const route = routes[index];
      inputs.push(...fileInputNodes(documents[index].root).map(backendNodeId => ({ frame_id: route.frameId, frame_loader_id: route.loaderId, backend_node_id: backendNodeId })));
      if (inputs.length > 256) throw new Error("Too many file inputs for bounded upload discovery.");
    }
    return inputs;
  } finally {
    await chrome.debugger.sendCommand({ tabId }, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
  }
}
chrome.debugger.onEvent.addListener((source, method, params) => {
  if (Number.isSafeInteger(source?.tabId)) {
    const tabId = source.tabId;
    captureUploadNetworkEvent(tabId, source, method, params || {});
    if (method === "Target.attachedToTarget" && params?.targetInfo?.type === "iframe" && typeof params.sessionId === "string") {
      recordFrameSession(tabId, params.targetInfo.targetId, params.sessionId);
      void chrome.debugger.sendCommand({ tabId, sessionId: params.sessionId }, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] }).catch(() => {});
    } else if (method === "Page.frameNavigated" && typeof source.sessionId === "string" && typeof params?.frame?.id === "string") {
      recordFrameSession(tabId, params.frame.id, source.sessionId);
    } else if (method === "Target.detachedFromTarget") {
      const frames = frameSessions.get(tabId);
      if (frames) for (const [frameId, sessionId] of frames) if (sessionId === params?.sessionId) frames.delete(frameId);
    }
  }
  if (method !== "Page.fileChooserOpened" || !Number.isSafeInteger(source?.tabId)) return;
  const tabId = source.tabId, documentId = pairedTabDocumentIds.get(tabId);
  if (!documentId || !(nativeAttachedTabs.has(tabId) || manualAttachedTabs.has(tabId))) return;
  const backendNodeId = params?.backendNodeId, frameId = params?.frameId;
  if (!Number.isSafeInteger(backendNodeId) || typeof frameId !== "string") return;
  const sequence = ++fileChooserEventSequence;
  const operation = fileChooserOperations.get(tabId);
  if (operation && (!operation.pressing || operation.frameId !== frameId)) operation.conflicted = true;
  void chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {}).then(tree => {
    const find = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(find).find(Boolean);
    const frame = find(tree.frameTree);
    if (!frame?.loaderId || pairedTabDocumentIds.get(tabId) !== documentId) return;
    const event = { backend_node_id: backendNodeId, frame_id: frameId, frame_loader_id: frame.loaderId, mode: params?.mode, document_id: documentId, root_document_id: documentId, sequence };
    const waiter = fileChooserWaiters.get(tabId);
    if (waiter && waiter.documentId === documentId && sequence > waiter.afterSequence) { clearTimeout(waiter.timer); fileChooserWaiters.delete(tabId); waiter.resolve(event); return; }
    const queued = fileChooserEvents.get(tabId) || [];
    queued.push(event); if (queued.length > 2) queued.shift(); fileChooserEvents.set(tabId, queued);
  }).catch(() => {});
});

function reservePairingTabs(tabIds) {
  if (tabIds.some(tabId => tabPairReservations.has(tabId))) throw new Error("A requested tab is already being paired.");
  const reservation = {};
  for (const tabId of tabIds) tabPairReservations.set(tabId, reservation);
  return reservation;
}
function releasePairingTabs(tabIds, reservation) { for (const tabId of tabIds) if (tabPairReservations.get(tabId) === reservation) tabPairReservations.delete(tabId); }
function detachDebugger(tabId) {
  const previous = pendingTabDetaches.get(tabId) || Promise.resolve();
  const current = previous.catch(() => {}).then(() => performDebuggerDetach(tabId));
  trackTabDetach(tabId, current); return current;
}
function trackTabDetach(tabId, current) {
  pendingTabDetaches.set(tabId, current);
  const cleanup = () => { if (pendingTabDetaches.get(tabId) === current) pendingTabDetaches.delete(tabId); };
  current.then(cleanup, cleanup);
}
async function performDebuggerDetach(tabId) { try { await chrome.debugger.detach({ tabId }); } catch {} }
async function attachDebugger(tabId) { await (pendingTabDetaches.get(tabId) || Promise.resolve()).catch(() => {}); return chrome.debugger.attach({ tabId }, "1.3"); }
function queueNativeMutation(operation) { const result = nativeMutation.then(operation, operation); nativeMutation = result.catch(() => {}); return result; }
function sendNative(port, generation, message) { if (nativePort !== port || nativePortGeneration !== generation) return false; try { port.postMessage(message); return true; } catch { return false; } }
function resolveNativeLease(leaseId, tabId) { if (leaseId) return nativeLeases.get(leaseId); const owner = nativeTabLeases.get(tabId); if (owner) return nativeLeases.get(owner); return nativeLeases.size === 1 ? nativeLeases.values().next().value : undefined; }
function safeNativeError(value) { if (typeof value !== "string" || !value) return undefined; return value.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 256); }
function safeErrorClass(error) { try { const name = error?.name; return typeof name === "string" && /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(name) ? name : "Error"; } catch { return "Error"; } }
function traceSharedCommand(phase, request, tabId, startedAt, details = {}) {
  if ((!SHARED_COMMAND_METHODS.has(request.method) && request.method !== "Controlla.fastType") || !Number.isSafeInteger(tabId)) return;
  const commandId = typeof request.id === "string" || Number.isSafeInteger(request.id) ? String(request.id).replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 128) : "";
  try { console.debug("[Chrome Controlla] shared command", { phase, commandId, targetId: tabId, method: request.method, elapsedMs: Math.max(0, Date.now() - startedAt), ...details }); } catch {}
}
const FAST_TYPE_GUARD = "function(index,expected,prepare,normalize,requireFocus){const e=this.nodes[index];if(!e||!e.isConnected)return {ok:false,reason:'target_replaced'};const rich=e instanceof HTMLElement&&!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)&&(e.isContentEditable||e.getAttribute('role')==='textbox'),plain=e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type),trustedBoundary=e.closest('[data-requires-trusted]');if(!rich&&!plain||e.matches(':disabled,[aria-disabled=\"true\"]')||e.readOnly||e.getAttribute('aria-readonly')==='true'||e.getAttribute('contenteditable')?.toLowerCase()==='false'||e.closest('[data-masked]')||trustedBoundary&&trustedBoundary!==e)return {ok:false,reason:'blocked'};const value=()=>{const raw=rich?(e.innerText??e.textContent??'').replace(/\\r\\n/g,'\\n').replace(/\\u00a0/g,' '):e.value;return rich?(raw==='\\n'?'':normalize?raw.replace(/\\n{2,}/g,b=> '\\n'.repeat(Math.ceil(b.length/2))):raw):raw};if(value()!==(rich&&expected==='\\n'?'':expected))return {ok:false,reason:'stale_value',value:value()};if(prepare){e.focus();if(rich){const s=e.ownerDocument.getSelection(),r=e.ownerDocument.createRange();r.selectNodeContents(e);r.collapse(false);s.removeAllRanges();s.addRange(r)}else e.setSelectionRange(e.value.length,e.value.length)}const s=e.ownerDocument.getSelection(),focused=e.ownerDocument.activeElement===e,atEnd=rich?s?.isCollapsed&&s.anchorNode&&e.contains(s.anchorNode)&&(()=>{const r=e.ownerDocument.createRange();r.selectNodeContents(e);r.setStart(s.anchorNode,s.anchorOffset);return r.toString()===''})():e.selectionStart===e.selectionEnd&&e.selectionEnd===e.value.length;if(requireFocus&&(!focused||!atEnd))return {ok:false,reason:'focus_or_selection_changed',value:value(),focused,atEnd};return {ok:true,value:value(),rich}}";
async function fastTypeProgram(tabId, params, isAuthorized) {
  const { object_id: objectId, index, expected, text, normalize_paragraphs: normalizeParagraphs } = params || {};
  if (typeof objectId !== "string" || objectId.length < 1 || objectId.length > 1024 || !Number.isSafeInteger(index) || index < 0 || typeof expected !== "string" || expected.length > 16384 || typeof text !== "string" || text.length < 1 || text.length > 16384 || !/^[\x00-\x7f]*$/.test(text) || typeof normalizeParagraphs !== "boolean") throw new Error("fast_type requires a bounded retained editor, ASCII text, and newline policy.");
  if (text.replace(/\r\n/g, "\n").includes("\r")) throw new Error("fast_type text must use normalized newlines.");
  const deadline = performance.now() + Math.min(60000, Math.max(1000, Number.isSafeInteger(params.deadline_ms) ? params.deadline_ms : 30000));
  let progress = expected, completedChars = 0, keyEvents = 0, guardChecks = 0;
  const guard = async (value, prepare = false) => {
    if (!isAuthorized(tabId) || performance.now() >= deadline) return { ok: false, reason: "deadline_or_session_changed" };
    guardChecks++;
    try {
      const response = await chrome.debugger.sendCommand({ tabId }, "Runtime.callFunctionOn", { objectId, functionDeclaration: FAST_TYPE_GUARD, arguments: [{ value: index }, { value }, { value: prepare }, { value: normalizeParagraphs }, { value: true }], returnByValue: true });
      return response?.result?.value || { ok: false, reason: "guard_response_missing" };
    } catch { return { ok: false, reason: "guard_read_failed" }; }
  };
  const before = await guard(progress, true);
  if (before.ok !== true) return { status: "not_dispatched", completed_chars: 0, key_events: 0, cdp_commands: guardChecks, readback: before.value, reason: before.reason || "editor_guard_rejected" };
  for (const character of text) {
    if (!isAuthorized(tabId) || performance.now() >= deadline) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "typing_deadline_or_session_change" };
    if (completedChars > 0) {
      const beforeNextKey = await guard(progress);
      if (beforeNextKey.ok !== true) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "focus_or_value_changed_before_next_keydown", guard: beforeNextKey };
    }
    const enter = character === "\n", key = enter ? "Enter" : character;
    const keyDown = enter ? { type: "keyDown", key, code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36, text: "\r", unmodifiedText: "\r" } : { type: "keyDown", key };
    keyEvents++;
    try { await chrome.debugger.sendCommand({ tabId }, "Input.dispatchKeyEvent", keyDown); }
    catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "key_down_delivery_unknown", error: safeNativeError(String(error)) }; }
    if (enter) progress += character;
    const afterDown = await guard(progress);
    if (afterDown.ok !== true) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "focus_or_value_changed_after_keydown", guard: afterDown };
    if (!enter) {
      keyEvents++;
      try { await chrome.debugger.sendCommand({ tabId }, "Input.dispatchKeyEvent", { type: "char", key, text: character, unmodifiedText: character }); }
      catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "character_delivery_unknown", error: safeNativeError(String(error)) }; }
      progress += character;
    }
    const afterInput = enter ? afterDown : await guard(progress);
    if (afterInput.ok !== true) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "focus_or_value_changed_after_character", guard: afterInput };
    keyEvents++;
    try { await chrome.debugger.sendCommand({ tabId }, "Input.dispatchKeyEvent", { type: "keyUp", key, ...(enter ? { code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36 } : {}) }); }
    catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "key_up_delivery_unknown", error: safeNativeError(String(error)) }; }
    completedChars++;
  }
  const final = await guard(progress);
  return final.ok === true
    ? { status: "verified", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress }
    : { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "final_editor_guard_failed", guard: final };
}
async function scheduleNativeReconnect() {
  if (reconnectTimer || nativePort) return;
  const delay = Math.min(RECONNECT_BASE_MS * (2 ** reconnectAttempts), RECONNECT_MAX_MS);
  reconnectAttempts = Math.min(reconnectAttempts + 1, MAX_RECONNECT_ATTEMPTS);
  try { await chrome.storage.session.set({ [RECONNECT_STORAGE_KEY]: reconnectAttempts }); }
  catch { nativeError = "Native host reconnect state could not be saved; retrying with bounded backoff."; }
  chrome.alarms.create(RECONNECT_ALARM, { delayInMinutes: Math.max(delay, 30000) / 60000 });
  reconnectTimer = setTimeout(() => { reconnectTimer = undefined; connectNative(); }, delay);
}
chrome.alarms.onAlarm.addListener(alarm => { if (alarm.name !== RECONNECT_ALARM) return; void reconnectStateReady.then(() => { if (reconnectTimer) clearTimeout(reconnectTimer); reconnectTimer = undefined; connectNative(); }); });
function connectNative() {
  if (nativePort || reconnectExhausted) return;
  try {
    const port = chrome.runtime.connectNative(NATIVE_HOST); nativePort = port; const portGeneration = ++nativePortGeneration; nativeReady = false;
    port.onMessage.addListener(message => { void handleNativeMessage(port, portGeneration, message); });
    port.onDisconnect.addListener(() => {
      if (nativePort !== port) return; nativeError = safeNativeError(chrome.runtime.lastError?.message); nativePort = undefined; nativeReady = false;
      const generation = ++nativeGeneration; void queueNativeMutation(() => detachNative(generation)); scheduleNativeReconnect();
    });
  } catch { scheduleNativeReconnect(); }
}
async function handleNativeMessage(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration || !message || typeof message.type !== "string") return;
  if (message.type === "ready") { nativeReady = true; nativeError = undefined; reconnectAttempts = 0; reconnectExhausted = false; void chrome.storage.session.remove(RECONNECT_STORAGE_KEY).catch(() => {}); sendNative(port, portGeneration, { type: "capabilities", protocol_version: 2, multi_session_leases: true }); return; }
  if (message.type === "list_tabs") {
    try {
      const tabs = await chrome.tabs.query({});
      const focusedWindowTabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true }).catch(() => []);
      const lastFocusedActive = focusedWindowTabs[0]?.id;
      const lastFocusedWindowId = Number.isSafeInteger(focusedWindowTabs[0]?.windowId) ? focusedWindowTabs[0].windowId : undefined;
      const eligible = tabs.filter(tab => Number.isSafeInteger(tab.id) && /^https?:\/\//i.test(tab.url || ""));
      // Preserve the focused browser context before applying the bounded inventory cap.
      // Otherwise Chrome's query order can omit the active tab or every stable fallback
      // in its window, causing native code to anchor a background open to another context.
      const prioritized = [...eligible].sort((left, right) => {
        const leftFocused = left.windowId === lastFocusedWindowId;
        const rightFocused = right.windowId === lastFocusedWindowId;
        if (leftFocused !== rightFocused) return rightFocused ? 1 : -1;
        const leftActive = left.id === lastFocusedActive;
        const rightActive = right.id === lastFocusedActive;
        if (leftActive !== rightActive) return rightActive ? 1 : -1;
        const accessDelta = (Number.isFinite(right.lastAccessed) ? right.lastAccessed : 0)
          - (Number.isFinite(left.lastAccessed) ? left.lastAccessed : 0);
        return accessDelta || left.id - right.id;
      });
      const listed = await Promise.all(prioritized.slice(0, 100).map(async tab => {
        const firstFrame = await chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }).catch(() => undefined);
        const current = await chrome.tabs.get(tab.id).catch(() => undefined);
        const lastFrame = await chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }).catch(() => undefined);
        const stableDocument = current?.status === "complete" && /^https?:\/\//i.test(current.url || "") && firstFrame?.documentId && firstFrame.documentId === lastFrame?.documentId && lastFrame.url === current.url;
        return { id: tab.id, window_id: current?.windowId, last_accessed: Number.isFinite(current?.lastAccessed) ? current.lastAccessed : undefined, title: String(current?.title || tab.title || "").slice(0, 256), url: String(current?.url || tab.url).slice(0, 2048), active_for_open: current?.id === lastFocusedActive, ...(stableDocument ? { document_id: lastFrame.documentId } : {}) };
      }));
      sendNative(port, portGeneration, { type: "tabs", request_id: message.request_id, last_focused_window_id: lastFocusedWindowId, truncated: eligible.length > listed.length, tabs: listed });
    } catch (error) { sendNative(port, portGeneration, { type: "tabs_error", request_id: message.request_id, error: String(error).slice(0, 512) }); }
    return;
  }
  if (message.type === "pair") { await queueNativeMutation(() => pairNative(port, portGeneration, message)); return; }
  if (message.type === "release") { await queueNativeMutation(() => message.lease_id ? detachNativeLease(message.lease_id) : detachNative(++nativeGeneration)); return; }
  if (message.type === "command" || message.type === "batch") {
    const commandGeneration = nativeGeneration; const leaseId = message.lease_id; const startedAt = Date.now();
    const lease = resolveNativeLease(leaseId, Number(message.target_id));
    const authorize = tabId => nativeGeneration === commandGeneration && (lease ? lease.tabs.has(tabId) || lease.unrestricted : nativeAttachedTabs.has(tabId));
    const reply = response => {
      const sent = nativeGeneration === commandGeneration && sendNative(port, portGeneration, { ...response, ...(leaseId ? { lease_id: leaseId } : {}) });
      traceSharedCommand("reply", message, Number(message.target_id), startedAt, { outcome: nativeGeneration !== commandGeneration ? "suppressed" : sent ? "sent" : "failed", responseBytes: (() => { try { return JSON.stringify(response).length; } catch { return 0; } })() });
    };
    if (message.type === "batch") await dispatchBatch(message, authorize, reply, true); else await dispatchCommand(message, authorize, reply, true);
  }
}
async function pairNative(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration) return;
  const requested = message.tab_ids;
  const expectedUrls = message.expected_urls && typeof message.expected_urls === "object" ? message.expected_urls : undefined;
  const expectedDocumentIds = message.expected_document_ids && typeof message.expected_document_ids === "object" ? message.expected_document_ids : undefined;
  let attemptGeneration; let reservation;
  try {
    if (typeof message.request_id !== "string" || !message.request_id || !Array.isArray(requested) || requested.length === 0 || requested.some(id => !Number.isSafeInteger(id) || id < 0) || new Set(requested).size !== requested.length) throw new Error("pair requires a request_id and unique nonnegative numeric tab_ids.");
    if (!message.lease_id) await detachNative(nativeGeneration);
    if (requested.some(id => manualAttachedTabs.has(id))) throw new Error("A requested tab is already paired through the popup WebSocket.");
    const leaseId = typeof message.lease_id === "string" && message.lease_id ? message.lease_id : message.request_id;
    if (nativeLeases.has(leaseId)) throw new Error("A native lease with this ID is already active.");
    if (requested.some(id => nativeTabLeases.has(id) || manualAttachedTabs.has(id))) throw new Error("A requested tab is already leased by another Controlla session.");
    reservation = reservePairingTabs(requested); attemptGeneration = ++nextNativeLeaseGeneration;
    const lease = { id: leaseId, generation: attemptGeneration, tabs: new Set(), unrestricted: message.unrestricted === true };
    nativeLeases.set(leaseId, lease);
    nativeLeaseGenerations.add(attemptGeneration);
    for (const tabId of requested) {
      const navigationGeneration = tabNavigationGenerations.get(tabId) || 0;
      const beforeAttach = await chrome.tabs.get(tabId).catch(() => null);
      const beforeFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (!beforeAttach || beforeAttach.status !== "complete" || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0) || !beforeFrame?.documentId || (!message.unrestricted && (!expectedUrls || typeof expectedUrls[tabId] !== "string" || !expectedDocumentIds || typeof expectedDocumentIds[tabId] !== "string")) || (typeof expectedUrls?.[tabId] === "string" && beforeAttach.url !== expectedUrls[tabId]) || (typeof expectedDocumentIds?.[tabId] === "string" && beforeFrame.documentId !== expectedDocumentIds[tabId])) throw new Error("Tab " + tabId + " changed during pairing or lacks a required discovery receipt.");
      pairedTabUrls.set(tabId, beforeAttach.url); pairedTabDocumentIds.set(tabId, beforeFrame.documentId); await attachDebugger(tabId);
      if (port !== nativePort || nativeLeases.get(leaseId) !== lease) { await detachDebugger(tabId); throw new Error("Pairing was released during attachment."); }
      lease.tabs.add(tabId); nativeTabLeases.set(tabId, leaseId); nativeAttachedTabs.add(tabId);
      const attached = await chrome.tabs.get(tabId).catch(() => null); const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0) || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url || attachedFrame?.documentId !== beforeFrame.documentId) throw new Error("Tab " + tabId + " navigated during pairing; rediscover tabs and pair again.");
    }
    sendNative(port, portGeneration, { type: "paired", request_id: message.request_id, lease_id: leaseId, targets: [...lease.tabs].map(String).sort(), extension_version: chrome.runtime.getManifest().version, protocol_version: 2, document_identity: true, batch_execution: true, batch_deadline: true });
  } catch (error) {
    if (attemptGeneration !== undefined) await detachNativeLease(typeof message.lease_id === "string" && message.lease_id ? message.lease_id : message.request_id);
    if (reservation) for (const tabId of requested || []) { if (manualAttachedTabs.has(tabId)) continue; pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); }
    sendNative(port, portGeneration, { type: "pair_error", request_id: message.request_id, error: String(error) });
  } finally { if (reservation) releasePairingTabs(requested, reservation); }
}
async function dispatchBatch(request, isAuthorized, respond, nativeRoute = false) {
  const tabId = Number(request.target_id); const actions = request.actions; const startedAt = performance.now();
  const lease = nativeRoute ? nativeLeases.get(request.lease_id) : undefined;
  if (nativeRoute && lease?.unrestricted && !lease.tabs.has(tabId)) { try { await attachUnrestrictedNativeTab(tabId, lease.id); } catch (error) { respond({ type: "batch_result", id: request.id, error: String(error) }); return; } }
  if (!Number.isSafeInteger(tabId) || !isAuthorized(tabId)) { respond({ type: "batch_result", id: request.id, error: "Target is not attached." }); return; }
  if (!Array.isArray(actions) || actions.length === 0 || actions.length > MAX_BATCH_ACTIONS) { respond({ type: "batch_result", id: request.id, error: "Batch requires 1..64 actions." }); return; }
  if (!Number.isSafeInteger(request.deadline_ms) || request.deadline_ms < 1 || request.deadline_ms > 60000) { respond({ type: "batch_result", id: request.id, error: "Batch deadline must be within 1..60000ms." }); return; }
  const receipts = [];
  const acknowledgedKeyEvent = () => receipts.some(receipt => receipt.method === "Input.dispatchKeyEvent" && !receipt.error);
  for (let index = 0; index < actions.length; index++) {
    if (performance.now() - startedAt >= request.deadline_ms) { respond({ type: "batch_result", id: request.id, result: { receipts, completed: index, stopped_before: index, dispatch_may_have_occurred: acknowledgedKeyEvent(), host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Batch deadline elapsed before the next action; inspect state and do not retry blindly." }); return; }
    const action = actions[index];
    if (!action || typeof action.method !== "string" || action.method.length > 128 || (action.target_id !== undefined && Number(action.target_id) !== tabId) || (action.stop_on_not_ok !== undefined && typeof action.stop_on_not_ok !== "boolean")) {
      respond({ type: "batch_result", id: request.id, result: { receipts, completed: index, stopped_before: index, dispatch_may_have_occurred: acknowledgedKeyEvent(), host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Invalid batch action or target mismatch." }); return;
    }
    let settled;
    await dispatchCommand({ type: "command", id: `${String(request.id).slice(0, 96)}:${index}`, lease_id: request.lease_id, target_id: String(tabId), method: action.method, params: action.params || {} }, isAuthorized, response => { settled = response; }, nativeRoute);
    const receipt = { index, method: action.method };
    if (settled?.error) receipt.error = settled.error; else receipt.result = settled?.result;
    receipts.push(receipt);
    if (settled?.error) { respond({ type: "batch_result", id: request.id, result: { receipts, completed: index, failed_at: index, stopped_before: index, dispatch_may_have_occurred: action.method === "Input.dispatchKeyEvent" || acknowledgedKeyEvent(), host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: settled.error }); return; }
    if (action.stop_on_not_ok && settled?.result?.result?.value?.ok !== true) { respond({ type: "batch_result", id: request.id, result: { receipts, completed: index + 1, stopped_before: index + 1, dispatch_may_have_occurred: acknowledgedKeyEvent(), host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Batch read guard rejected before the next action." }); return; }
    if (index + 1 < actions.length && performance.now() - startedAt >= request.deadline_ms) { respond({ type: "batch_result", id: request.id, result: { receipts, completed: index + 1, stopped_before: index + 1, dispatch_may_have_occurred: acknowledgedKeyEvent(), host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Batch deadline elapsed before the next action; inspect state and do not retry blindly." }); return; }
  }
  respond({ type: "batch_result", id: request.id, result: { receipts, completed: actions.length, stopped_before: null, host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt } });
}
function boundedUploadNetworkUrl(value) {
  try {
    const url = new URL(value);
    if (!/(^|\.)(google\.com|googleapis\.com|googleusercontent\.com)$/.test(url.hostname)) return null;
    return { host: url.hostname, path: url.pathname.replace(/[A-Za-z0-9_-]{24,}/g, "<redacted>").slice(0, 160) };
  } catch { return null; }
}
function captureUploadNetworkEvent(tabId, source, method, params) {
  const operation = fileChooserOperations.get(tabId);
  if (!operation || operation.networkSessionId !== (source.sessionId || null) || !method.startsWith("Network.")) return;
  let request;
  if (method === "Network.requestWillBeSent") {
    if (!(params.type === "XHR" || params.type === "Fetch")) return;
    const url = boundedUploadNetworkUrl(params.request?.url);
    if (!url || !params.request?.method) return;
    request = { ...url, method: params.request.method, type: params.type };
    operation.networkRequests.set(params.requestId, request);
  } else if (method === "Network.responseReceived") {
    request = operation.networkRequests.get(params.requestId);
    if (!request || !(params.type === "XHR" || params.type === "Fetch")) return;
    operation.networkEvents.push({ ...request, status: params.response?.status, status_text: params.response?.statusText, mime_type: params.response?.mimeType });
  } else if (method === "Network.loadingFailed") {
    request = operation.networkRequests.get(params.requestId);
    if (!request) return;
    operation.networkEvents.push({ ...request, error: params.errorText, blocked_reason: params.blockedReason, cors_error: params.corsErrorStatus?.corsError });
  } else return;
  if (operation.networkEvents.length > 24) operation.networkEvents.shift();
  operation.networkChanged?.(operation.networkEvents.at(-1));
}
function waitForUploadNetwork(operation, maxWaitMs = 10000) {
  return new Promise(resolve => {
    const started = Date.now();
    let quietTimer;
    const finish = () => {
      clearTimeout(maxTimer);
      clearTimeout(quietTimer);
      operation.networkChanged = undefined;
      resolve(Date.now() - started);
    };
    const maxTimer = setTimeout(finish, maxWaitMs);
    operation.networkChanged = event => {
      if (event?.status != null || event?.error) return finish();
      clearTimeout(quietTimer);
      quietTimer = setTimeout(finish, 1200);
    };
    if (operation.networkEvents.some(event => event.status != null || event.error)) finish();
    else quietTimer = setTimeout(finish, operation.networkRequests.size ? 1200 : 2500);
  });
}
async function dispatchCommand(request, isAuthorized, respond, nativeMessage = false) {
  const tabId = Number(request.target_id); const startedAt = Date.now();
  const nativeLease = nativeMessage ? resolveNativeLease(request.lease_id, tabId) : undefined;
  if (nativeMessage && nativeLease?.unrestricted && !nativeLease.tabs.has(tabId)) { try { await attachUnrestrictedNativeTab(tabId, nativeLease.id); } catch (error) { respond({ type: "result", id: request.id, error: String(error) }); return; } }
  if (!Number.isSafeInteger(tabId) || !isAuthorized(tabId)) { respond({ type: "result", id: request.id, error: "Target is not attached." }); return; }
  if (navigatingTabs.has(tabId)) { respond({ type: "result", id: request.id, error: "Tab is navigating; retry after the new document commits." }); return; }
  const nativeRoute = nativeMessage || nativeAttachedTabs.has(tabId); const routeGeneration = nativeRoute ? nativeLease?.generation ?? nativeGeneration : manualGeneration;
  if (request.method === "DOM.setFileInputFiles" && !(nativeRoute && nativeLease?.unrestricted)) { respond({ type: "result", id: request.id, error: "File selection requires local unrestricted mode." }); return; }
  try {
    const expectedDocumentId = pairedTabDocumentIds.get(tabId);
    const frame = expectedDocumentId ? await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined) : undefined;
    if (!isAuthorized(tabId)) { respond({ type: "result", id: request.id, error: "Target is no longer attached." }); return; }
    if (navigatingTabs.has(tabId)) { respond({ type: "result", id: request.id, error: "Tab is navigating; retry after the new document commits." }); return; }
    if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId) { respond({ type: "result", id: request.id, error: "Target document identity changed; rediscover and pair again." }); return; }
    if (!frame?.documentId || frame.documentId !== expectedDocumentId) { await detachPairedTab(tabId); respond({ type: "result", id: request.id, error: "Target document identity changed; pair the tab again." }); return; }
    if (request.method === "Controlla.openTab") {
      const url = validateNavigationUrl(request.params?.url); const native = nativeRoute; const waitUntil = request.params?.wait_until ?? "dom_content_loaded";
      const active = request.params?.active ?? true;
      if (waitUntil !== "document_committed" && waitUntil !== "dom_content_loaded") throw new Error("wait_until must be document_committed or dom_content_loaded.");
      if (typeof active !== "boolean") throw new Error("active must be a boolean.");
      const result = await openControllaTab(url, request.params?.idempotency_key, native, routeGeneration, waitUntil, active);
      respond({ type: "result", id: request.id, result }); return;
    }
    if (request.method === "Controlla.newTabs") {
      const since = request.params?.since_ms;
      if (!Number.isSafeInteger(since) || since < 0) throw new Error("New-tab verification requires a valid click timestamp.");
      const expectedUrl = request.params?.url;
      if (expectedUrl != null && validateNavigationUrl(expectedUrl) !== expectedUrl) throw new Error("New-tab expected URL must be an exact absolute HTTP(S) URL.");
      const tabs = await chrome.tabs.query({});
      const created = tabs.filter(tab => {
        const event = recentlyCreatedTabs.get(tab.id);
        const openerMatch = event?.opener_tab_id === tabId;
        return event?.created_at_ms >= since && openerMatch && (!expectedUrl || tab.url === expectedUrl) && /^https?:\/\//i.test(tab.url || "");
      }).slice(0, 8).map(tab => ({ tab_id: String(tab.id), url: String(tab.url).slice(0, 2048), opener_match: recentlyCreatedTabs.get(tab.id)?.opener_tab_id === tabId }));
      respond({ type: "result", id: request.id, result: { tabs: created } }); return;
    }
    if (request.method === "Controlla.fastType") {
      const result = await fastTypeProgram(tabId, request.params, isAuthorized);
      traceSharedCommand("settled", request, tabId, startedAt, { outcome: result.status, completedChars: result.completed_chars, keyEvents: result.key_events, cdpCommands: result.cdp_commands });
      respond({ type: "result", id: request.id, result }); return;
    }
    if (request.method === "Controlla.navigate") { const url = validateNavigationUrl(request.params?.url); await navigatePairedTab(tabId, url, nativeRoute, routeGeneration); respond({ type: "result", id: request.id, result: { tab_id: String(tabId), ...(await nativePageIdentity(tabId)) } }); return; }
    if (request.method === "Controlla.presentationState") {
      const tab = await chrome.tabs.get(tabId);
      const win = await chrome.windows.get(tab.windowId);
      respond({ type: "result", id: request.id, result: { tab_active: !!tab.active, window_focused: !!win.focused, window_id: tab.windowId, activation_epoch: activationEpoch, focus_epoch: focusEpoch } }); return;
    }
    if (request.method === "Controlla.pointerClick") {
      const params = request.params || {}, objectId = params.object_id, index = params.index, outcome = params.outcome;
      const guard = params.function_declaration, expectedViewport = params.viewport;
      if (typeof objectId !== "string" || !objectId || objectId.length > 512 || !Number.isSafeInteger(index) || index < 0 ||
          typeof guard !== "string" || !guard.startsWith("function(index,") || guard.length > 12000 ||
          (expectedViewport != null && (typeof expectedViewport !== "string" || expectedViewport.length > 256))) throw new Error("pointer click requires a bounded guarded target.");
      const target = await chrome.tabs.get(tabId), window = await chrome.windows.get(target.windowId);
      const wasActive = !!target.active;
      const previous = wasActive ? target : (await chrome.tabs.query({ active:true, windowId:target.windowId }))[0];
      if (!previous?.id) { respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,reason_code:"active_tab_unavailable",reason:"Could not identify the tab to restore after pointer input."} }); return; }
      const documentId = pairedTabDocumentIds.get(tabId), generation = routeGeneration, activationEpochBefore = activationEpoch;
      let activated = false, restoredPreviousTab = false, pressAttempted = false, acknowledged = false, finalChecked;
      try {
        if (!wasActive) {
          await chrome.tabs.update(tabId, { active:true });
          activated = true;
          const activeTarget = await chrome.tabs.get(tabId);
          if (!isAuthorized(tabId) || (nativeRoute && resolveNativeLease(request.lease_id, tabId)?.generation !== generation) ||
              pairedTabDocumentIds.get(tabId) !== documentId || !activeTarget.active) {
            respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,reason_code:"activation_revalidation_failed",reason:"Target identity changed during temporary tab activation."} }); return;
          }
        }
        const checked = await chrome.debugger.sendCommand({tabId}, "Runtime.callFunctionOn", {objectId,
          functionDeclaration:guard, arguments:[{value:index},{value:outcome}], returnByValue:true});
        finalChecked = checked?.result?.value;
        if (!finalChecked?.ok || (!activated && expectedViewport != null && finalChecked.viewport !== expectedViewport) || !Number.isFinite(finalChecked.x) || !Number.isFinite(finalChecked.y)) {
          respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,reason_code:"final_target_revalidation_failed",reason:finalChecked?.reason || "Target or viewport changed before pointer input."} }); return;
        }
        const point = {x:finalChecked.x,y:finalChecked.y};
        await chrome.debugger.sendCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseMoved",...point});
        pressAttempted = true;
        await chrome.debugger.sendCommand({tabId}, "Input.dispatchMouseEvent", {type:"mousePressed",button:"left",clickCount:1,...point});
        await chrome.debugger.sendCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseReleased",button:"left",clickCount:1,...point});
        acknowledged = true;
      } catch (error) {
        if (pressAttempted) await chrome.debugger.sendCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseReleased",button:"left",clickCount:1,x:finalChecked?.x,y:finalChecked?.y}).catch(() => {});
        respond({ type:"result", id:request.id, result:{status:pressAttempted?"unknown":"not_dispatched",dispatch_acknowledged:false,reason_code:pressAttempted?"pointer_delivery_unknown":"pointer_dispatch_failed",reason:safeNativeError(String(error))} });
        return;
      } finally {
        if (activated && previous.id !== tabId) {
          const current = await chrome.tabs.get(tabId).catch(() => undefined);
          if (current?.active && activationEpoch <= activationEpochBefore + 1 && isAuthorized(tabId) && pairedTabDocumentIds.get(tabId) === documentId) {
            restoredPreviousTab = !!(await chrome.tabs.update(previous.id, {active:true}).catch(() => undefined));
          }
        }
      }
      respond({ type:"result", id:request.id, result:{status:acknowledged?"dispatched":"unknown",dispatch_acknowledged:acknowledged,
        temporary_activation:activated,restored_previous_tab:restoredPreviousTab,
        reason:activated?(window.focused?"Pointer input used a brief tab activation in the focused Chrome window.":"Pointer input used a brief tab activation while Chrome remained behind another window."):undefined} }); return;
    }
    if (request.method === "Controlla.securePasswordFill") {
      const frameId = request.params?.frame_id, loaderId = request.params?.loader_id, backendNodeId = request.params?.backend_node_id;
      const label = typeof request.params?.label === "string" ? request.params.label.slice(0, 120) : "Password";
      const timeoutMs = request.params?.timeout_ms ?? 20000;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256 || typeof loaderId !== "string" || !loaderId || loaderId.length > 256 || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0 || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 22000) throw new Error("secure password fill requires bounded frame identity and timeout.");
      const target = await chrome.tabs.get(tabId), window = await chrome.windows.get(target.windowId);
      if (!target.active || !window.focused) {
        respond({ type: "result", id: request.id, result: { status: "not_dispatched", dispatch_may_have_occurred: false, reason_code: "secure_input_requires_active_target", reason: "Secure password entry requires the target tab to be active in the focused Chrome window." } }); return;
      }
      const root = { tabId }, frameTree = await chrome.debugger.sendCommand(root, "Page.getFrameTree", {});
      const findFrame = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(findFrame).find(Boolean);
      const frame = findFrame(frameTree.frameTree);
      if (!frame || frame.loaderId !== loaderId) throw new Error("Password field frame changed since its fresh snapshot.");
      await chrome.debugger.sendCommand(root, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      let promptTab;
      const route = () => { const sessionId = frameSessions.get(tabId)?.get(frameId); return sessionId ? { tabId, sessionId } : root; };
      const checkPassword = async () => {
        if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || !isAuthorized(tabId)) throw new Error("Target document or Controlla lease changed during secure entry.");
        const current = await chrome.debugger.sendCommand(root, "Page.getFrameTree", {});
        if (findFrame(current.frameTree)?.loaderId !== loaderId) throw new Error("Password field frame navigated during secure entry.");
        const resolved = await chrome.debugger.sendCommand(route(), "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Password field is no longer available.");
        const result = await chrome.debugger.sendCommand(route(), "Runtime.callFunctionOn", { objectId,
          functionDeclaration: "function(){const e=this;const r=e.getBoundingClientRect(),s=getComputedStyle(e);const visible=e.isConnected&&r.width>0&&r.height>0&&r.left>=0&&r.top>=0&&r.right<=innerWidth&&r.bottom<=innerHeight&&s.display!=='none'&&s.visibility!=='hidden'&&Number.parseFloat(s.opacity||'1')>0&&s.pointerEvents!=='none'&&document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)===e;return {ok:e instanceof HTMLInputElement&&e.type==='password'&&visible&&!e.disabled&&!e.readOnly&&e.getAttribute('aria-disabled')!=='true'&&e.getAttribute('aria-readonly')!=='true'&&!e.hasAttribute('data-masked')&&!e.hasAttribute('data-requires-trusted')&&e.value.length===0,reason:e.value.length?'not_empty':'not_actionable'}}",
          returnByValue: true });
        if (result.result?.value?.ok !== true) throw new Error("Password field is stale, non-empty, hidden, disabled, masked, or not an ordinary password input.");
        return { route: route(), objectId };
      };
      const expectedDocumentId = pairedTabDocumentIds.get(tabId);
      if (!expectedDocumentId) throw new Error("Target document identity is unavailable.");
      try {
        const field = await checkPassword();
        const nonceBytes = new Uint8Array(32); crypto.getRandomValues(nonceBytes);
        const nonce = [...nonceBytes].map(value => value.toString(16).padStart(2, "0")).join("");
        let origin;
        try { origin = new URL(frame.securityOrigin).origin; } catch { throw new Error("Password field has no trustworthy HTTP(S) frame origin."); }
        if (!/^https?:$/.test(new URL(origin).protocol)) throw new Error("Password field has no trustworthy HTTP(S) frame origin.");
        const hasChatPassword = Object.hasOwn(request.params || {}, "password");
        let chatPassword = hasChatPassword ? request.params.password : undefined;
        if (hasChatPassword) request.params.password = "";
        if (hasChatPassword && (typeof chatPassword !== "string" || chatPassword.length < 1 || chatPassword.length > 4096)) {
          throw new Error("Chat-provided password is empty or exceeds the allowed length.");
        }
        let submitted;
        if (hasChatPassword) {
          submitted = { value: chatPassword };
          chatPassword = undefined;
        } else {
          const pending = {};
          const input = new Promise((resolve, reject) => {
            pending.resolve = resolve; pending.reject = reject;
            pending.timer = setTimeout(() => { pendingSecureInputs.delete(nonce); reject(new Error("Secure password entry timed out; no password was dispatched.")); }, timeoutMs);
          });
          pending.tabId = tabId; pending.origin = origin; pending.label = label; pendingSecureInputs.set(nonce, pending);
          promptTab = await chrome.tabs.create({ url: chrome.runtime.getURL(`secure-input.html#${nonce}`), active: true, windowId: target.windowId });
          submitted = await input;
        }
        if (!submitted || typeof submitted.value !== "string" || submitted.value.length < 1 || submitted.value.length > 4096) throw new Error("Secure password entry was cancelled or exceeded its bound.");
        let password = submitted.value;
        let passwordDispatchStarted = false;
        try {
          const latest = await checkPassword();
          if (latest.objectId !== field.objectId) throw new Error("Password field identity changed during secure entry.");
          passwordDispatchStarted = true;
          const result = await chrome.debugger.sendCommand(latest.route, "Runtime.callFunctionOn", { objectId: latest.objectId,
            functionDeclaration: "function(value){const e=this;if(!(e instanceof HTMLInputElement)||e.type!=='password'||!e.isConnected||e.value.length!==0||e.disabled||e.readOnly||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return {ok:false,reason:'field_changed'};const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(!setter)return {ok:false,reason:'setter_unavailable'};setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:null}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:e.value===value,credential_filled:e.value===value,length:e.value.length}}",
            arguments: [{ value: password }], returnByValue: true });
          const evidence = result.result?.value;
          if (evidence?.ok !== true) throw new Error("Password fill did not pass its independent redacted readback.");
          respond({ type: "result", id: request.id, result: { status: "verified", dispatch_acknowledged: true, dispatch_may_have_occurred: true, verified: true, action: "secure_fill", readback: { credential_filled: true } } });
        } catch (error) {
          respond({ type: "result", id: request.id, result: passwordDispatchStarted
            ? { status: "unknown", dispatch_acknowledged: false, dispatch_may_have_occurred: true, verified: false, action: "secure_fill", reason: "Password dispatch or readback is uncertain; inspect the field before retrying." }
            : { status: "not_dispatched", dispatch_acknowledged: false, dispatch_may_have_occurred: false, verified: false, action: "secure_fill", reason: "Password target changed before dispatch; take a fresh snapshot." } });
        } finally {
          password = "";
          if (submitted) submitted.value = "";
        }
      } finally {
        for (const [nonce, pending] of pendingSecureInputs) if (pending.tabId === tabId) { clearTimeout(pending.timer); pendingSecureInputs.delete(nonce); }
        if (promptTab?.id != null) await chrome.tabs.remove(promptTab.id).catch(() => {});
        await chrome.tabs.update(tabId, { active: true }).catch(() => {});
        await chrome.debugger.sendCommand(root, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (request.method === "Page.setInterceptFileChooserDialog" && request.params?.enabled === true) clearFileChooserState(tabId);
    if (request.method === "Controlla.waitForFileChooser") {
      const timeout = request.params?.timeout_ms;
      if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 15000) throw new Error("File chooser timeout_ms must be 1..15000.");
      const event = await waitForFileChooser(tabId, expectedDocumentId, timeout);
      if (!routeActive(nativeRoute, routeGeneration) || pairedTabDocumentIds.get(tabId) !== event.document_id) throw new Error("File chooser belongs to a stale tab document.");
      respond({ type: "result", id: request.id, result: event }); return;
    }
    if (request.method === "Controlla.checkIdentityMarker") {
      const { selector, expected } = request.params || {};
      if (typeof selector !== "string" || !selector || selector.length > 512 || typeof expected !== "string" || !expected || expected.length > 512) throw new Error("Identity marker requires bounded selector and expected text.");
      const direct = await chrome.debugger.sendCommand({ tabId }, "Runtime.evaluate", { expression: `(()=>{const norm=s=>String(s||'').replace(/\\s+/g,' ').trim(),visible=[];for(const e of document.querySelectorAll(${JSON.stringify(selector)})){if(!e.isConnected||e.getClientRects().length===0||e.getAttribute('aria-selected')==='true'||e.getAttribute('aria-checked')==='true'||e.closest('[role="menu"],[role="listbox"],[role="option"],[role="menuitem"],[role="dialog"],[aria-modal="true"]'))continue;let shown=true;for(let n=e;n;n=n.parentElement){const s=getComputedStyle(n);if(s.display==='none'||s.visibility!=='visible'||Number.parseFloat(s.opacity||'1')<=0||n.hasAttribute('hidden')||n.hasAttribute('inert')){shown=false;break}}if(shown)visible.push(e);if(visible.length===2)break}const e=visible[0];const labelled=(e?.getAttribute('aria-labelledby')||'').split(/\\s+/).filter(Boolean).map(id=>document.getElementById(id)?.innerText||'').join(' ').trim();const name=labelled||e?.getAttribute('aria-label')||e?.innerText||e?.getAttribute('alt')||Array.from(e?.labels||[]).map(label=>label.innerText).join(' ')||e?.getAttribute('title')||e?.getAttribute('content')||'';return visible.length===1&&norm(name)===norm(${JSON.stringify(expected)})})()`, returnByValue: true });
      if (direct.result?.value === true) {
        const currentFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
        if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || currentFrame?.documentId !== expectedDocumentId) throw new Error("Identity marker document changed during the root DOM check.");
        respond({ type: "result", id: request.id, result: { matches: true, match_count: 1, route: "root_dom" } }); return;
      }
      respond({ type: "result", id: request.id, result: { matches: false, match_count: 0, route: "none" } });
      return;
    }
    if (request.method === "Controlla.fileInputSnapshot") {
      const tree = await chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {});
      const inputs = await fileInputsInFrames(tabId, tree.frameTree);
      const current = await chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {});
      if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || current.frameTree?.frame?.id !== tree.frameTree?.frame?.id || current.frameTree?.frame?.loaderId !== tree.frameTree?.frame?.loaderId) throw new Error("Upload target document changed during file-input baseline discovery.");
      respond({ type: "result", id: request.id, result: { inputs, document_id: expectedDocumentId } }); return;
    }
    if (request.method === "Controlla.findFileInput") {
      const baseline = request.params?.baseline;
      if (!Array.isArray(baseline) || baseline.length > 256 || baseline.some(item => typeof item?.frame_id !== "string" || typeof item?.frame_loader_id !== "string" || !Number.isSafeInteger(item?.backend_node_id))) throw new Error("Lazy-input fallback requires a bounded pre-trigger file-input baseline.");
      const tree = await chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {});
      const all = await fileInputsInFrames(tabId, tree.frameTree);
      const existed = new Set(baseline.map(item => `${item.frame_id}:${item.frame_loader_id}:${item.backend_node_id}`));
      const candidates = all.filter(item => !existed.has(`${item.frame_id}:${item.frame_loader_id}:${item.backend_node_id}`));
      if (candidates.length === 0) throw new Error("No new file input appeared after the upload trigger.");
      if (candidates.length !== 1) throw new Error("Multiple new file inputs appeared after the upload trigger; refusing ambiguous selection.");
      const selected = candidates[0];
      const current = await chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {});
      const currentFrame = (() => { const find = node => node?.frame?.id === selected.frame_id ? node.frame : (node?.childFrames || []).map(find).find(Boolean); return find(current.frameTree); })();
      if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || current.frameTree?.frame?.id !== tree.frameTree?.frame?.id || current.frameTree?.frame?.loaderId !== tree.frameTree?.frame?.loaderId || currentFrame?.loaderId !== selected.frame_loader_id) throw new Error("Upload target document changed during lazy file-input discovery.");
      respond({ type: "result", id: request.id, result: { ...selected, document_id: expectedDocumentId } }); return;
    }
    if (request.method === "Controlla.setFrameFileInputFiles") {
      if (!nativeRoute || !nativeLease?.unrestricted) throw new Error("File selection requires local unrestricted mode.");
      const { frame_id: frameId, frame_loader_id: loaderId, backend_node_id: backendNodeId, file_path: filePath } = request.params || {};
      if (typeof frameId !== "string" || !frameId || typeof loaderId !== "string" || !loaderId || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0 || typeof filePath !== "string" || !filePath || filePath.length > 4096) throw new Error("Frame upload target or local path is invalid.");
      const readTree = () => chrome.debugger.sendCommand({ tabId }, "Page.getFrameTree", {});
      const find = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(find).find(Boolean);
      if (find((await readTree()).frameTree)?.loaderId !== loaderId) throw new Error("Upload frame navigated after the chooser opened.");
      const childSessionId = frameSessions.get(tabId)?.get(frameId);
      const route = childSessionId ? { tabId, sessionId: childSessionId } : { tabId };
      await chrome.debugger.sendCommand(route, "DOM.setFileInputFiles", { files: [filePath], backendNodeId });
      const resolved = await chrome.debugger.sendCommand(route, "DOM.resolveNode", { backendNodeId });
      const objectId = resolved.object?.objectId;
      if (!objectId) throw new Error("Chooser input could not be resolved for file readback.");
      const readback = await chrome.debugger.sendCommand(route, "Runtime.callFunctionOn", { objectId, functionDeclaration: "function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}))}", returnByValue: true });
      if (find((await readTree()).frameTree)?.loaderId !== loaderId) throw new Error("Upload frame navigated during file selection.");
      respond({ type: "result", id: request.id, result: { frame_id: frameId, frame_loader_id: loaderId, files: readback.result?.value } }); return;
    }
    if (request.method === "Controlla.frameSelectFile") {
      if (!nativeRoute || !nativeLease?.unrestricted) throw new Error("File selection requires local unrestricted mode.");
      const { frame_id: frameId, frame_loader_id: loaderId, trigger_backend_node_id: triggerId, file_path: filePath } = request.params || {};
      if (typeof frameId !== "string" || !frameId || typeof loaderId !== "string" || !loaderId || !Number.isSafeInteger(triggerId) || triggerId <= 0 || typeof filePath !== "string" || !filePath || filePath.length > 4096) throw new Error("Frame upload trigger or local path is invalid.");
      const root = { tabId };
      const readFrame = async () => {
        const tree = await chrome.debugger.sendCommand(root, "Page.getFrameTree", {});
        const find = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(find).find(Boolean);
        return find(tree.frameTree);
      };
      if ((await readFrame())?.loaderId !== loaderId) throw new Error("Upload frame changed before selection.");
      if (fileChooserOperations.has(tabId) || fileChooserWaiters.has(tabId)) throw new Error("Another file chooser operation is active for this tab.");
      const operation = { frameId, pressing: false, conflicted: false, networkRequests: new Map(), networkEvents: [] };
      fileChooserOperations.set(tabId, operation);
      let route, interceptionEnabled = false, networkEnabled = false, pressAttempted = false, dispatched = false, selectionResult;
      try {
        await chrome.debugger.sendCommand(root, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
        const sessionId = frameSessions.get(tabId)?.get(frameId) || await waitForFrameSession(tabId, frameId, 400);
        route = sessionId ? { tabId, sessionId } : root;
        operation.networkSessionId = sessionId || null;
        await chrome.debugger.sendCommand(route, "Network.enable", {});
        networkEnabled = true;
        const ax = await chrome.debugger.sendCommand(route, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const trigger = ax.nodes?.find(node => node.backendDOMNodeId === triggerId);
        if (!trigger || !["button", "link", "option", "menuitem"].includes(trigger.role?.value)) throw new Error("Frame upload trigger is stale or is not actionable.");
        const resolvedTrigger = await chrome.debugger.sendCommand(route, "DOM.resolveNode", { backendNodeId: triggerId });
        if (!resolvedTrigger.object?.objectId) throw new Error("Frame upload trigger could not be resolved.");
        const localPoint = await frameActionablePoint(route, resolvedTrigger.object.objectId);
        const box = await chrome.debugger.sendCommand(route, "DOM.getBoxModel", { backendNodeId: triggerId });
        const quad = box.model?.content;
        if (!Array.isArray(quad) || quad.length !== 8 || (await readFrame())?.loaderId !== loaderId) throw new Error("Frame upload trigger changed before dispatch.");
        clearFileChooserState(tabId);
        await chrome.debugger.sendCommand(route, "Page.enable", {});
        await chrome.debugger.sendCommand(route, "Page.setInterceptFileChooserDialog", { enabled: true });
        interceptionEnabled = true;
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4, y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
        if (!sessionId) ({ x, y } = await rootPointForFrame(tabId, frameId, resolvedTrigger.object.objectId, localPoint.x, localPoint.y));
        if ((await readFrame())?.loaderId !== loaderId) throw new Error("Upload frame changed before dispatch.");
        if (operation.conflicted) throw new Error("Another file chooser opened before the upload trigger was pressed.");
        // shortcut: CDP does not identify a chooser's initiating click; add causal page instrumentation if same-frame human interaction must be distinguished during dispatch.
        operation.eventCursor = fileChooserEventSequence;
        operation.pressing = true;
        await chrome.debugger.sendCommand(route, "Input.dispatchMouseEvent", { type: "mouseMoved", button: "none", x, y });
        pressAttempted = true;
        await chrome.debugger.sendCommand(route, "Input.dispatchMouseEvent", { type: "mousePressed", button: "left", clickCount: 1, x, y });
        await chrome.debugger.sendCommand(route, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y });
        dispatched = true;
        const chooser = await waitForFileChooser(tabId, expectedDocumentId, 4000, operation.eventCursor);
        if (operation.conflicted || chooser.frame_id !== frameId || chooser.frame_loader_id !== loaderId || (await readFrame())?.loaderId !== loaderId || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("File chooser belongs to another frame, document, or overlapping operation.");
        await chrome.debugger.sendCommand(route, "DOM.setFileInputFiles", { files: [filePath], backendNodeId: chooser.backend_node_id });
        const resolved = await chrome.debugger.sendCommand(route, "DOM.resolveNode", { backendNodeId: chooser.backend_node_id });
        if (!resolved.object?.objectId) throw new Error("Chooser input could not be resolved for file readback.");
        const readback = await chrome.debugger.sendCommand(route, "Runtime.callFunctionOn", { objectId: resolved.object.objectId, functionDeclaration: "function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}))}", returnByValue: true });
        if ((await readFrame())?.loaderId !== loaderId || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("Upload frame or root document changed during file selection.");
        if (!Array.isArray(readback.result?.value) || readback.result.value.length !== 1 || !readback.result.value[0]?.name) throw new Error("File input did not retain one selected file for readback.");
        selectionResult = { status: "selected", files: readback.result?.value, frame_id: frameId, frame_loader_id: loaderId, document_id: expectedDocumentId, dispatch_acknowledged: true };
      } catch (error) {
        selectionResult = { status: pressAttempted ? "unknown" : "not_dispatched", dispatch_acknowledged: dispatched, reason: String(error) };
      } finally {
        if (networkEnabled && selectionResult?.reason?.includes("File input did not retain")) {
          selectionResult.network_observation_window_ms = await waitForUploadNetwork(operation);
        }
        if (networkEnabled) await chrome.debugger.sendCommand(route, "Network.disable", {}).catch(() => {});
        if (selectionResult) selectionResult.network_events = operation.networkEvents;
        if (fileChooserOperations.get(tabId) === operation) fileChooserOperations.delete(tabId);
        if (interceptionEnabled) await chrome.debugger.sendCommand(route, "Page.setInterceptFileChooserDialog", { enabled: false }).catch(() => {});
        await chrome.debugger.sendCommand(root, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      respond({ type: "result", id: request.id, result: selectionResult });
      return;
    }
    if (request.method === "Controlla.frameClick") {
      const frameId = request.params?.frame_id, loaderId = request.params?.loader_id, backendNodeId = request.params?.backend_node_id;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256 || typeof loaderId !== "string" || !loaderId || loaderId.length > 256 || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0) throw new Error("frame click requires bounded frame, loader, and backend node identities.");
      const outcome = request.params?.outcome, timeoutMs = request.params?.timeout_ms, inputMode = request.params?.input_mode ?? "mouse";
      if (!(["mouse", "touch"].includes(inputMode))) throw new Error("frame click input_mode must be mouse or touch.");
      if (!outcome || !["navigation", "focused", "visible", "text", "expanded", "selected"].includes(outcome.kind) || !Number.isSafeInteger(timeoutMs) || timeoutMs < 250 || timeoutMs > 10000) throw new Error("frame click requires a supported postcondition and a 250..10000ms timeout.");
      if (["visible", "text"].includes(outcome.kind) && (typeof outcome.selector !== "string" || !outcome.selector || outcome.selector.length > 512) || outcome.kind === "text" && (typeof outcome.text !== "string" || outcome.text.length > 512) || outcome.kind === "navigation" && outcome.url != null && !isExactHttpUrl(outcome.url) || ["expanded", "selected"].includes(outcome.kind) && outcome.selector != null && (typeof outcome.selector !== "string" || !outcome.selector || outcome.selector.length > 512)) throw new Error("frame click postcondition is invalid or exceeds its bound.");
      const route = { tabId };
      const frameLoader = async () => {
        const tree = await chrome.debugger.sendCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.loaderId : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      const frameUrl = async () => {
        const tree = await chrome.debugger.sendCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.url : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      try {
        if (await frameLoader() !== loaderId) throw new Error("Frame document changed since the accessibility snapshot.");
        let sessionId = frameSessions.get(tabId)?.get(frameId);
        if (!sessionId) sessionId = await waitForFrameSession(tabId, frameId, 400);
        const child = sessionId ? { tabId, sessionId } : route;
        const beforeUrl = await frameUrl();
        const ax = await chrome.debugger.sendCommand(child, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const node = ax.nodes?.find(item => item.backendDOMNodeId === backendNodeId);
        if (!node || !["button", "link", "checkbox", "radio", "option", "menuitem", "tab", "switch"].includes(node.role?.value)) throw new Error("Frame control is stale or is not an actionable control.");
        const box = await chrome.debugger.sendCommand(child, "DOM.getBoxModel", { backendNodeId });
        const quad = box.model?.content;
        if (!Array.isArray(quad) || quad.length !== 8 || (sessionId && frameSessions.get(tabId)?.get(frameId) !== sessionId) || await frameLoader() !== loaderId) throw new Error("Frame control changed before dispatch.");
        const resolved = await chrome.debugger.sendCommand(child, "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Frame click target could not be resolved for outcome verification.");
        const localPoint = await frameActionablePoint(child, objectId);
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4, y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
        if (!sessionId) ({ x, y } = await rootPointForFrame(tabId, frameId, objectId, localPoint.x, localPoint.y));
        if (await frameLoader() !== loaderId) throw new Error("Frame control changed before dispatch.");
        let pressAttempted = false;
        try {
          if (inputMode === "mouse") await chrome.debugger.sendCommand(child, "Input.dispatchMouseEvent", { type: "mouseMoved", button: "none", x, y });
          pressAttempted = true;
          if (inputMode === "touch") {
            await chrome.debugger.sendCommand(child, "Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x, y, radiusX: 1, radiusY: 1, force: 1 }] });
            await chrome.debugger.sendCommand(child, "Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
          } else {
            await chrome.debugger.sendCommand(child, "Input.dispatchMouseEvent", { type: "mousePressed", button: "left", clickCount: 1, x, y });
            await chrome.debugger.sendCommand(child, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y });
          }
        } catch (error) {
          if (pressAttempted && inputMode === "touch") await chrome.debugger.sendCommand(child, "Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] }).catch(() => {});
          if (pressAttempted && inputMode === "mouse") await chrome.debugger.sendCommand(child, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y }).catch(() => {});
          respond({ type: "result", id: request.id, result: { frame_id: frameId, backend_node_id: backendNodeId, dispatch_acknowledged: false, dispatch_may_have_occurred: pressAttempted, verified: false, reason: String(error) } });
          return;
        }
        const verifyOutcome = async () => {
          if (outcome.kind === "navigation") {
            const currentUrl = await frameUrl();
            return outcome.url == null ? currentUrl !== beforeUrl : new URL(currentUrl).href === new URL(outcome.url).href;
          }
          const result = await chrome.debugger.sendCommand(child, "Runtime.callFunctionOn", {
            objectId,
            functionDeclaration: "function __controllaFrameOutcome(outcome){const e=this;const q=s=>s==null?e:document.querySelector(s);switch(outcome.kind){case 'focused':return document.activeElement===e||e.contains(document.activeElement);case 'visible':{const n=q(outcome.selector);if(!n)return false;const s=getComputedStyle(n);return n.isConnected&&n.getClientRects().length>0&&s.display!=='none'&&s.visibility==='visible'&&Number.parseFloat(s.opacity||'1')>0}case 'text':{const n=q(outcome.selector);return !!n&&n.isConnected&&(n.innerText||n.textContent||'').includes(outcome.text)}case 'expanded':{const n=q(outcome.selector);return !!n&&(n.getAttribute('aria-expanded')==='true'||n.open===true)}case 'selected':{const n=q(outcome.selector);return !!n&&(n.getAttribute('aria-selected')==='true'||n.getAttribute('aria-checked')==='true'||n.checked===true||n.selected===true)}default:return false}}",
            arguments: [{ value: outcome }], returnByValue: true,
          });
          return result.result?.value === true;
        };
        const deadline = Date.now() + timeoutMs;
        let verified = false;
        do { verified = await verifyOutcome().catch(() => false); if (verified || Date.now() >= deadline) break; await new Promise(resolve => setTimeout(resolve, 50)); } while (Date.now() < deadline);
        respond({ type: "result", id: request.id, result: { frame_id: frameId, backend_node_id: backendNodeId, input_mode: inputMode, dispatch_acknowledged: true, dispatch_may_have_occurred: true, verified, reason: verified ? undefined : "requested frame click outcome was not observed before its deadline" } });
      } finally {
        await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (request.method === "Controlla.frameFill") {
      const frameId = request.params?.frame_id, loaderId = request.params?.loader_id, backendNodeId = request.params?.backend_node_id;
      const expected = request.params?.expected_value, value = request.params?.value, timeoutMs = request.params?.timeout_ms;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256 || typeof loaderId !== "string" || !loaderId || loaderId.length > 256 || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0) throw new Error("frame fill requires bounded frame, loader, and backend node identities.");
      if (typeof expected !== "string" || typeof value !== "string" || expected.length > 16384 || value.length > 16384 || !Number.isSafeInteger(timeoutMs) || timeoutMs < 250 || timeoutMs > 10000) throw new Error("frame fill values or timeout exceed their bounds.");
      const route = { tabId };
      const frameLoader = async () => {
        const tree = await chrome.debugger.sendCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.loaderId : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      let dispatchAttempted = false;
      try {
        if (await frameLoader() !== loaderId) throw new Error("Frame document changed since the accessibility snapshot.");
        let sessionId = frameSessions.get(tabId)?.get(frameId);
        if (!sessionId) sessionId = await waitForFrameSession(tabId, frameId, 400);
        const child = sessionId ? { tabId, sessionId } : route;
        const ax = await chrome.debugger.sendCommand(child, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const node = ax.nodes?.find(item => item.backendDOMNodeId === backendNodeId);
        if (!node || !["textbox", "searchbox", "combobox"].includes(node.role?.value)) throw new Error("Frame field is stale or is not an editable text control.");
        const resolved = await chrome.debugger.sendCommand(child, "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Frame field could not be resolved for guarded fill.");
        if ((sessionId && frameSessions.get(tabId)?.get(frameId) !== sessionId) || await frameLoader() !== loaderId) throw new Error("Frame document changed before fill dispatch.");
        const functionDeclaration = "function(expected,value){const e=this;const blocked=r=>({dispatch_may_have_occurred:false,verified:false,matches_requested_value:false,reason_code:r});if(!e||!e.isConnected)return blocked('target_stale');const input=e instanceof HTMLInputElement,area=e instanceof HTMLTextAreaElement;if(!input&&!area)return blocked('unsupported_control');if(input&&['password','file','hidden','checkbox','radio','submit','button','reset'].includes((e.type||'').toLowerCase()))return blocked('protected_or_nontext_control');if(e.disabled||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return blocked('field_blocked');const r=e.getBoundingClientRect(),s=getComputedStyle(e),x=Math.min(innerWidth-1,Math.max(0,r.left+r.width/2)),y=Math.min(innerHeight-1,Math.max(0,r.top+r.height/2));if(r.width<=0||r.height<=0||s.display==='none'||s.visibility!=='visible'||Number.parseFloat(s.opacity||'1')<=0||document.elementFromPoint(x,y)!==e)return blocked('field_not_actionable');if(e.value!==expected)return blocked('stale_value');try{e.focus({preventScroll:true});if(document.activeElement!==e)return blocked('focus_failed');const proto=input?HTMLInputElement.prototype:HTMLTextAreaElement.prototype;const setter=Object.getOwnPropertyDescriptor(proto,'value')?.set;if(!setter)return blocked('native_setter_unavailable');setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));e.dispatchEvent(new Event('change',{bubbles:true}));const matches=e.value===value;return {dispatch_may_have_occurred:true,verified:matches,matches_requested_value:matches,length:e.value.length,reason_code:matches?null:'readback_mismatch'}}catch(_){return {dispatch_may_have_occurred:true,verified:false,matches_requested_value:false,reason_code:'fill_delivery_uncertain'}}}";
        let result;
        try {
          dispatchAttempted = true;
          result = await chrome.debugger.sendCommand(child, "Runtime.callFunctionOn", { objectId, functionDeclaration, arguments: [{ value: expected }, { value }], returnByValue: true, awaitPromise: false });
        } catch (error) {
          respond({ type: "result", id: request.id, result: { dispatch_acknowledged: false, dispatch_may_have_occurred: dispatchAttempted, verified: false, matches_requested_value: false, reason_code: "frame_fill_delivery_uncertain", reason: String(error) } });
          return;
        }
        const evidence = result.result?.value;
        if (!evidence || typeof evidence.verified !== "boolean") throw new Error("Frame fill returned no structured readback.");
        respond({ type: "result", id: request.id, result: { dispatch_acknowledged: true, ...evidence, frame_id: frameId, backend_node_id: backendNodeId } });
      } catch (error) {
        respond({ type: "result", id: request.id, result: { dispatch_acknowledged: false, dispatch_may_have_occurred: dispatchAttempted,
          verified: false, matches_requested_value: false, reason_code: dispatchAttempted ? "frame_fill_delivery_uncertain" : "frame_fill_preflight_rejected",
          reason: String(error) } });
      } finally {
        await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (request.method === "Controlla.frameAccessibility") {
      const frameId = request.params?.frame_id;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256) throw new Error("frame_id must be a bounded nonempty string.");
      const route = { tabId };
      await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      try {
        const sessionId = await waitForFrameSession(tabId, frameId, 1500);
        if (!sessionId) throw new Error("No attached OOPIF session matched the requested frame.");
        const result = await chrome.debugger.sendCommand({ tabId, sessionId }, "Accessibility.getFullAXTree", {});
        respond({ type: "result", id: request.id, result: { ...result, source_frame_id: frameId, routing: "oopif_child_session" } });
      } finally {
        await chrome.debugger.sendCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (!SHARED_COMMAND_METHODS.has(request.method)) throw new Error("Command is outside the shared observe/input allowlist.");
    const requestBytes = JSON.stringify({ method: request.method, params: request.params || {} }).length;
    traceSharedCommand("dispatch", request, tabId, startedAt, { requestBytes }); const result = await chrome.debugger.sendCommand({ tabId }, request.method, request.params || {}); traceSharedCommand("settled", request, tabId, startedAt, { outcome: "result", requestBytes, responseBytes: (() => { try { return JSON.stringify(result).length; } catch { return 0; } })() }); respond({ type: "result", id: request.id, result });
  } catch (error) { traceSharedCommand("settled", request, tabId, startedAt, { outcome: "error", errorClass: safeErrorClass(error) }); respond({ type: "result", id: request.id, error: String(error) }); }
}
function validateNavigationUrl(value) { if (typeof value !== "string" || value.length > 2048) throw new Error("URL must be a bounded HTTP(S) URL."); const url = new URL(value); if (!(url.protocol === "http:" || url.protocol === "https:") || !url.hostname || url.username || url.password) throw new Error("URL must be HTTP(S) and cannot contain credentials."); return url.href; }
async function nativePageIdentity(tabId) {
  for (let attempt = 0; attempt < 26; attempt++) {
    const tab = await chrome.tabs.get(tabId);
    const frame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 });
    if (frame?.documentId && frame.url === tab.url) return { url: tab.url, document_id: frame.documentId };
    if (attempt < 25) await new Promise(resolve => setTimeout(resolve, 10));
  }
  throw new Error("Navigation did not expose a stable document identity within 250 ms.");
}
async function waitForTabDOMContentLoaded(tabId, afterDocumentId) {
  const currentDocumentReady = async () => {
    const frame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
    return !!frame?.documentId && frame.documentId !== afterDocumentId && domReadyDocuments.get(tabId) === frame.documentId;
  };
  if (await currentDocumentReady()) return;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => finish(new Error("Tab did not reach DOMContentLoaded within 30 seconds.")), 30000);
    const finish = error => { clearTimeout(timer); chrome.webNavigation.onDOMContentLoaded.removeListener(listener); error ? reject(error) : resolve(); };
    const listener = details => {
      if (details.tabId !== tabId || details.frameId !== 0 || details.documentId === afterDocumentId) return;
      domReadyDocuments.set(tabId, details.documentId);
      void currentDocumentReady().then(ready => { if (ready) finish(); }, finish);
    };
    chrome.webNavigation.onDOMContentLoaded.addListener(listener);
    void currentDocumentReady().then(ready => { if (ready) finish(); }, finish);
  });
}
function waitForTabDocumentCommitted(tabId, afterDocumentId) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => finish(new Error("Tab did not commit its main document within 30 seconds.")), 30000);
    const finish = (error, details) => { clearTimeout(timer); chrome.webNavigation.onCommitted.removeListener(listener); error ? reject(error) : resolve(details); };
    const listener = details => { if (details.tabId === tabId && details.frameId === 0 && details.documentId && details.documentId !== afterDocumentId) finish(null, details); };
    chrome.webNavigation.onCommitted.addListener(listener);
  });
}
function nativeLeaseForGeneration(generation) { for (const lease of nativeLeases.values()) if (lease.generation === generation) return lease; return undefined; }
function addNativeTab(tabId, generation) { const lease = nativeLeaseForGeneration(generation); if (!lease) throw new Error("Native browser lease has ended."); lease.tabs.add(tabId); nativeTabLeases.set(tabId, lease.id); nativeAttachedTabs.add(tabId); }
function routeActive(native, generation) { return native ? nativeLeaseGenerations.has(generation) && !!nativePort : manualGeneration === generation && !!socket && socket.readyState === WebSocket.OPEN; }
async function pendingNavigationMatchesDocument(tabId, documentId) {
  const pending = pendingMainFrameNavigations.get(tabId);
  if (!pending) return true;
  if (pending.documentId === documentId) return true;
  if (!pending.ambiguous || typeof documentId !== "string" || !documentId) return false;
  // Event callbacks cannot identify which overlapping same-URL sequence
  // committed. Permit recovery only when Chrome's live tab/frame inventory
  // reports this exact document, URL, and complete state twice in succession.
  const readCurrent = async () => {
    const [tab, frame] = await Promise.all([
      chrome.tabs.get(tabId).catch(() => undefined),
      chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined),
    ]);
    return tab?.status === "complete" && frame?.documentId === documentId &&
      frame.url === tab.url && frame.url === pending.url;
  };
  if (!await readCurrent()) return false;
  await Promise.resolve();
  const stableCurrent = await readCurrent();
  return stableCurrent && pendingMainFrameNavigations.get(tabId) === pending;
}
async function clearFailedNavigationWhenStable(tabId, failedNavigation) {
  const [tab, frame] = await Promise.all([chrome.tabs.get(tabId).catch(() => undefined), chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined)]);
  if (!failedNavigation.ambiguous && pendingMainFrameNavigations.get(tabId) === failedNavigation && tab?.status === "complete" && frame?.documentId && frame.url === tab.url) pendingMainFrameNavigations.delete(tabId);
}
async function attachNewTab(tabId, url, native, generation, navigate = true, waitUntil = "dom_content_loaded") {
  if (navigate) {
    const before = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
    const afterDocumentId = before?.documentId;
    const ready = waitUntil === "document_committed" ? waitForTabDocumentCommitted(tabId, afterDocumentId) : waitForTabDOMContentLoaded(tabId, afterDocumentId);
    await chrome.tabs.update(tabId, { url });
    await ready;
  }
  else if ((await chrome.tabs.get(tabId)).url !== url) throw new Error("Recovery tab no longer matches the requested destination.");
  // Navigation already awaited the requested readiness event above. Only
  // recovery/reattach paths (navigate=false) need this additional readiness
  // check before pairing the existing tab.
  if (!navigate && waitUntil !== "document_committed") await waitForTabDOMContentLoaded(tabId); if (!routeActive(native, generation)) throw new Error("Shared session ended during tab navigation.");
  const identity = await nativePageIdentity(tabId); await attachDebugger(tabId); const attachedIdentity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!routeActive(native, generation) || !await pendingNavigationMatchesDocument(tabId, attachedIdentity?.document_id) || attachedIdentity?.url !== identity.url || attachedIdentity?.document_id !== identity.document_id) { await detachDebugger(tabId); throw new Error("Tab changed during attachment; retry from a fresh inventory."); }
  pairedTabUrls.set(tabId, attachedIdentity.url); pairedTabDocumentIds.set(tabId, attachedIdentity.document_id); if (native) addNativeTab(tabId, generation); else manualAttachedTabs.add(tabId); return attachedIdentity;
}
async function returnExistingOpen(receipt, url, native, generation) {
  if ((receipt.requested_url || receipt.url) !== url) throw new Error("Idempotency key is already bound to a different URL.");
  const tabId = Number(receipt.tab_id);
  const identity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!identity || identity.url !== receipt.url || identity.document_id !== receipt.document_id) throw new Error(`Open receipt ${receipt.tab_id} is stale; its tab changed or closed, so no replacement tab was opened.`);
  const attached = native ? nativeAttachedTabs : manualAttachedTabs;
  if (!attached.has(tabId)) {
    await attachDebugger(tabId);
    const after = await nativePageIdentity(tabId).catch(() => undefined);
    if (!routeActive(native, generation) || after?.url !== receipt.url || after?.document_id !== receipt.document_id) { await detachDebugger(tabId); throw new Error(`Open receipt ${receipt.tab_id} changed while reconnecting; no replacement tab was opened.`); }
    pairedTabUrls.set(tabId, after.url); pairedTabDocumentIds.set(tabId, after.document_id); attached.add(tabId);
  }
  return { tab_id: receipt.tab_id, url: receipt.url, document_id: receipt.document_id, idempotent_reuse: true, created_by_controlla: true };
}
async function createControllaTab(url, native, generation, tab, markerUrl, reused = false, waitUntil = "dom_content_loaded", active = true) {
  tab ||= await chrome.tabs.create({ url: markerUrl, active });
  let identity, attachedIdentity;
  try {
    const current = await chrome.tabs.get(tab.id);
    attachedIdentity = await attachNewTab(tab.id, url, native, generation, current.url !== url, waitUntil);
    identity = await nativePageIdentity(tab.id);
  }
  catch (error) {
    if (attachedIdentity && routeActive(native, generation) && pairedTabDocumentIds.get(tab.id) === attachedIdentity.document_id && (native ? nativeLeaseForGeneration(generation)?.tabs.has(tab.id) : manualAttachedTabs.has(tab.id))) { pairedTabUrls.delete(tab.id); pairedTabDocumentIds.delete(tab.id); if (native) removeNativeTab(tab.id); else manualAttachedTabs.delete(tab.id); await detachDebugger(tab.id); }
    const failure = new Error("Tab " + tab.id + " remains open after the failed open; rediscover and pair it before use. " + String(error));
    failure.controllaOrphanTabId = String(tab.id);
    throw failure;
  }
  return { tab_id: String(tab.id), ...identity, idempotent_reuse: reused, created_by_controlla: true };
}
async function openControllaTab(url, clientKey, native, generation, waitUntil = "dom_content_loaded", active = true) {
  if (clientKey === undefined) throw new Error("idempotency_key is required so an open retry cannot create another tab.");
  if (typeof clientKey !== "string" || clientKey.length < 8 || clientKey.length > 256 || /[\u0000-\u001f\u007f]/.test(clientKey)) throw new Error("idempotency_key must be 8..256 bounded printable characters.");
  const pending = openTabInflight.get(clientKey);
  if (pending) {
    if (pending.url !== url) throw new Error("Idempotency key is already bound to a different URL.");
    if (pending.active !== active) throw new Error("Idempotency key is already bound to a different activation mode.");
    return pending.promise;
  }
  const withReceiptLock = async operation => {
    const result = openTabQueue.then(operation);
    openTabQueue = result.catch(() => {});
    return result;
  };
  const operation = (async () => {
    const reservation = await withReceiptLock(async () => {
    const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
    const receipts = saved[OPEN_RECEIPTS_KEY] || (await chrome.storage.session.get(OPEN_RECEIPTS_KEY))[OPEN_RECEIPTS_KEY] || {};
    pruneOpenReceipts(receipts);
    if (!receipts[clientKey] && Object.keys(receipts).length >= OPEN_RECEIPT_MAX_COUNT) throw new Error("Open receipt capacity is full; old active or recoverable opens must be resolved before creating another tab.");
    const markerUrl = `${chrome.runtime.getURL("open-recovery.html")}#${encodeURIComponent(clientKey)}`;
    let receipt = receipts[clientKey];
    if (receipt?.status === "failed") throw new Error(`Open attempt left Controlla-created tab ${receipt.tab_id} for recovery; rediscover and pair that tab before retrying.`);
    if (receipt?.status === "complete" || receipt?.document_id) {
      if ((receipt.active ?? true) !== active) throw new Error("Idempotency key is already bound to a different activation mode.");
      if ((receipt.requested_url || receipt.url) !== url) throw new Error("Idempotency key is already bound to a different URL.");
      await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
      return { existing: receipt };
    }
    let tab;
    if (receipt?.tab_id) tab = await chrome.tabs.get(Number(receipt.tab_id)).catch(() => undefined);
    if (!tab) tab = (await chrome.tabs.query({})).find(candidate => candidate.url === markerUrl);
    if (receipt?.status === "creating" && !tab) throw new Error("Open attempt was interrupted and its recovery tab is no longer available; no duplicate tab was opened.");
    if (tab && receipt?.url !== undefined && receipt.url !== url) throw new Error("Idempotency key is already bound to a different URL.");
    if (tab && ![markerUrl, url].includes(tab.url)) throw new Error("Open recovery tab changed before completion; no replacement tab was opened.");
    const reused = !!receipt || !!tab;
    if (receipt && (receipt.active ?? true) !== active) throw new Error("Idempotency key is already bound to a different activation mode.");
    if (!tab) tab = await chrome.tabs.create({ url: markerUrl, active });
    receipt = { status: "creating", tab_id: String(tab.id), url, requested_url: url, marker_url: markerUrl, active, created_ms: receipt?.created_ms || Date.now() };
    receipts[clientKey] = receipt;
    pruneOpenReceipts(receipts);
    await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
    return { tab, markerUrl, reused, created_ms: receipt.created_ms };
    });
    if (reservation.existing) return returnExistingOpen(reservation.existing, url, native, generation);
    let result;
    try { result = await createControllaTab(url, native, generation, reservation.tab, reservation.markerUrl, reservation.reused, waitUntil, active); }
    catch (error) {
      if (error?.controllaOrphanTabId) {
        await withReceiptLock(async () => {
          const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
          const receipts = saved[OPEN_RECEIPTS_KEY] || {};
          receipts[clientKey] = { status: "failed", tab_id: error.controllaOrphanTabId, url, requested_url: url, active, created_ms: reservation.created_ms };
          pruneOpenReceipts(receipts);
          await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
        });
      }
      throw error;
    }
    await withReceiptLock(async () => {
      const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
      const receipts = saved[OPEN_RECEIPTS_KEY] || {};
      const current = receipts[clientKey];
      if (current?.tab_id !== result.tab_id) throw new Error("Open receipt changed before completion; no second tab was opened.");
      receipts[clientKey] = { status: "complete", tab_id: result.tab_id, url: result.url, requested_url: url, document_id: result.document_id, active, created_ms: reservation.created_ms };
      pruneOpenReceipts(receipts);
      await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
    });
    return result;
  })();
  const inflight = { url, active, promise: operation };
  openTabInflight.set(clientKey, inflight);
  try { return await operation; } finally { if (openTabInflight.get(clientKey) === inflight) openTabInflight.delete(clientKey); }
}
function pruneOpenReceipts(receipts, now = Date.now()) {
  for (const [key, receipt] of Object.entries(receipts)) {
    if (receipt?.status === "complete" && Number.isSafeInteger(receipt.created_ms) && now - receipt.created_ms > OPEN_RECEIPT_MAX_AGE_MS) delete receipts[key];
  }
  const completed = Object.entries(receipts).filter(([, receipt]) => receipt?.status === "complete")
    .sort((a, b) => (a[1].created_ms || 0) - (b[1].created_ms || 0));
  while (Object.keys(receipts).length > OPEN_RECEIPT_MAX_COUNT && completed.length) delete receipts[completed.shift()[0]];
}
async function attachUnrestrictedNativeTab(tabId, leaseId) {
  if (!Number.isSafeInteger(tabId) || tabId < 0) throw new Error("Invalid Chrome tab ID.");
  const lease = nativeLeases.get(leaseId);
  if (!lease?.unrestricted) throw new Error("Native lease does not allow unrestricted tab attachment.");
  if (nativeTabLeases.has(tabId) && nativeTabLeases.get(tabId) !== leaseId) throw new Error("Tab is already leased by another Controlla session.");
  const tab = await chrome.tabs.get(tabId); const frame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 });
  if (tab.status !== "complete" || !frame?.documentId) throw new Error("Chrome tab is not ready.");
  await attachDebugger(tabId);
  if (nativeLeases.get(leaseId) !== lease) { await detachDebugger(tabId); throw new Error("Native lease ended during attachment."); }
  lease.tabs.add(tabId); nativeTabLeases.set(tabId, leaseId); nativeAttachedTabs.add(tabId); pairedTabUrls.set(tabId, tab.url); pairedTabDocumentIds.set(tabId, frame.documentId);
}
async function navigatePairedTab(tabId, url, native, generation) {
  clearFileChooserState(tabId, "Target navigated before the file chooser opened.");
  const before = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
  if (!before?.documentId) throw new Error("Cannot navigate a paired tab without a current document identity.");
  const ready = waitForTabDOMContentLoaded(tabId, before.documentId);
  (native ? nativeAttachedTabs : manualAttachedTabs).delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); await detachDebugger(tabId); await chrome.tabs.update(tabId, { url }); await ready;
  if (!routeActive(native, generation)) throw new Error("Shared session ended during tab navigation.");
  const identity = await nativePageIdentity(tabId); await attachDebugger(tabId); const attachedIdentity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!routeActive(native, generation) || !await pendingNavigationMatchesDocument(tabId, attachedIdentity?.document_id) || attachedIdentity?.url !== identity.url || attachedIdentity?.document_id !== identity.document_id) { await detachDebugger(tabId); throw new Error("Tab changed during attachment; retry from a fresh inventory."); }
  pairedTabUrls.set(tabId, attachedIdentity.url); pairedTabDocumentIds.set(tabId, attachedIdentity.document_id); if (native) addNativeTab(tabId, generation); else manualAttachedTabs.add(tabId);
}
function removeNativeTab(tabId) { const leaseId = nativeTabLeases.get(tabId); if (leaseId) nativeLeases.get(leaseId)?.tabs.delete(tabId); nativeTabLeases.delete(tabId); nativeAttachedTabs.delete(tabId); }
async function detachNativeLease(leaseId) { const lease = nativeLeases.get(leaseId); if (!lease) return; nativeLeases.delete(leaseId); nativeLeaseGenerations.delete(lease.generation); const tabs = [...lease.tabs]; for (const tabId of tabs) { if (nativeTabLeases.get(tabId) !== leaseId) continue; nativeTabLeases.delete(tabId); nativeAttachedTabs.delete(tabId); clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); } await Promise.all(tabs.filter(tabId => !nativeTabLeases.has(tabId)).map(detachDebugger)); }
async function detachPairedTab(tabId) { clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); manualAttachedTabs.delete(tabId); removeNativeTab(tabId); await detachDebugger(tabId); }
async function detachManual(generation) { if (generation !== manualGeneration) return; const toDetach = [...manualAttachedTabs]; manualAttachedTabs = new Set(); for (const tabId of toDetach) { clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); } await Promise.all(toDetach.map(detachDebugger)); }
async function detachNative(generation) { if (generation !== nativeGeneration) return; for (const leaseId of [...nativeLeases.keys()]) await detachNativeLease(leaseId); }
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (["secure-input-ready", "secure-input-submit", "secure-input-cancel"].includes(message?.type)) {
    const nonce = message?.nonce;
    const pending = typeof nonce === "string" ? pendingSecureInputs.get(nonce) : undefined;
    const expectedUrl = chrome.runtime.getURL("secure-input.html");
    if (!pending || typeof _sender?.url !== "string" || _sender.url.split("#", 1)[0] !== expectedUrl) { respond({ ok: false }); return; }
    if (message.type === "secure-input-ready") { respond({ ok: true, origin: pending.origin, label: pending.label }); return; }
    clearTimeout(pending.timer); pendingSecureInputs.delete(nonce);
    if (message.type === "secure-input-cancel") pending.reject(new Error("Secure password entry was cancelled."));
    else if (typeof message.password === "string" && message.password.length > 0 && message.password.length <= 4096) {
      const password = message.password;
      message.password = "";
      pending.resolve({ value: password });
    }
    else pending.reject(new Error("Secure password value is empty or exceeds the allowed length."));
    respond({ ok: true }); return;
  }
  if (message?.type === "pair") { pair(message.endpoint, message.token, message.tabIds).then(() => respond({ ok: true })).catch(error => respond({ ok: false, error: String(error) })); return true; }
  if (message?.type === "status") respond({ connected: socket?.readyState === WebSocket.OPEN, attached: [...manualAttachedTabs] });
  if (message?.type === "release") { const current = socket; const generation = ++manualGeneration; socket = undefined; current?.close(); detachManual(generation).then(() => respond({ ok: true })); return true; }
  if (message?.type === "native-status") respond({ connected: nativeReady, attached: [...nativeAttachedTabs], error: nativeError, retry_exhausted: reconnectExhausted, transport: "native" });
});
async function pair(endpoint, token, selectedTabIds) {
  if (!Array.isArray(selectedTabIds) || selectedTabIds.length === 0 || !token || !endpoint) throw new Error("Choose tabs and provide the local pairing endpoint and token.");
  const tabs = [...new Set(selectedTabIds.map(Number))]; if (tabs.some(id => !Number.isInteger(id) || id < 0)) throw new Error("Invalid tab selection."); const endpointUrl = new URL(endpoint);
  if (endpointUrl.protocol !== "ws:" || endpointUrl.hostname !== "127.0.0.1" || endpointUrl.username || endpointUrl.password || endpointUrl.search || endpointUrl.hash) throw new Error("The pairing endpoint must use ws://127.0.0.1.");
  if (tabs.some(tabId => nativeAttachedTabs.has(tabId))) throw new Error("A selected tab is already paired through Native Messaging."); const reservation = reservePairingTabs(tabs); return pairManual(endpointUrl, token, tabs).finally(() => releasePairingTabs(tabs, reservation));
}
async function pairManual(endpointUrl, token, tabs) {
  const generation = ++manualGeneration; const previousSocket = socket; socket = undefined; previousSocket?.close(); await detachManual(generation);
  try {
    for (const tabId of tabs) {
      const navigationGeneration = tabNavigationGenerations.get(tabId) || 0; const beforeAttach = await chrome.tabs.get(tabId).catch(() => null); const beforeFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (!beforeAttach || beforeAttach.status !== "complete" || !beforeFrame?.documentId || navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0)) throw new Error("Tab " + tabId + " is navigating or its document identity is unavailable; wait for it to finish and pair again.");
      pairedTabUrls.set(tabId, beforeAttach.url); pairedTabDocumentIds.set(tabId, beforeFrame.documentId); await attachDebugger(tabId); if (generation !== manualGeneration) { await detachDebugger(tabId); throw new Error("Pairing was replaced."); }
      manualAttachedTabs.add(tabId); const attached = await chrome.tabs.get(tabId).catch(() => null); const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0) || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url || attachedFrame?.documentId !== beforeFrame.documentId) throw new Error("Tab " + tabId + " navigated during pairing; pair again.");
    }
  } catch (error) { await detachManual(generation); for (const tabId of tabs) { pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); } throw error; }
  const next = new WebSocket(endpointUrl.href); socket = next;
  try {
    await new Promise((resolve, reject) => { next.onopen = resolve; next.onerror = () => reject(new Error("Cannot connect to the loopback provider.")); next.onclose = () => reject(new Error("Loopback pairing was closed.")); }); if (generation !== manualGeneration) throw new Error("Pairing was replaced.");
    const extensionVersion = chrome.runtime.getManifest().version; next.send(JSON.stringify({ type: "hello", token, extension_version: extensionVersion, document_identity: true, batch_execution: true, batch_deadline: true, targets: [...manualAttachedTabs].map(String).sort() }));
    await new Promise((resolve, reject) => { const timer = setTimeout(() => reject(new Error("Provider did not confirm pairing.")), 5000); next.onmessage = event => { const reply = JSON.parse(event.data); if (reply.type === "ready" && reply.server_version === extensionVersion) { clearTimeout(timer); resolve(); } else { clearTimeout(timer); reject(new Error(reply.error || "Pairing rejected.")); } }; });
  } catch (error) { next.close(); if (socket === next) socket = undefined; await detachManual(generation); throw error; }
  if (generation !== manualGeneration) { next.close(); return; }
  next.onmessage = async event => {
    if (generation !== manualGeneration || socket !== next) return; const request = JSON.parse(event.data);
    const authorize = tabId => generation === manualGeneration && socket === next && manualAttachedTabs.has(tabId); const reply = response => { if (generation === manualGeneration && socket === next) next.send(JSON.stringify(response)); };
    if (request.type === "command") await dispatchCommand(request, authorize, reply); else if (request.type === "batch") await dispatchBatch(request, authorize, reply);
  };
  next.onclose = () => { if (generation !== manualGeneration) return; socket = undefined; void detachManual(generation); };
  next.onerror = () => { if (generation !== manualGeneration) return; socket = undefined; void detachManual(generation); };
}
chrome.debugger.onDetach.addListener(({ tabId }, reason) => {
  if (!Number.isInteger(tabId)) return; const committed = committedNavigationTabs.get(tabId);
  if (reason === "target_closed" && (navigatingTabs.has(tabId) || committed) && pairedTabDocumentIds.has(tabId)) {
    debuggerDetachedDuringNavigation.add(tabId);
    if (committed && !navigatingTabs.has(tabId)) void reattachAfterNavigation(tabId, committed.native, committed.generation, committed.url, committed.documentId).then(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) debuggerDetachedDuringNavigation.delete(tabId); }).catch(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) void detachPairedTab(tabId); });
    return;
  }
  manualAttachedTabs.delete(tabId); removeNativeTab(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); committedNavigationTabs.delete(tabId);
});
async function reattachAfterNavigation(tabId, native, generation, expectedUrl, expectedDocumentId) {
  await waitForTabDOMContentLoaded(tabId); const navigationGeneration = tabNavigationGenerations.get(tabId); const current = () => navigationGeneration === tabNavigationGenerations.get(tabId) && pairedTabUrls.get(tabId) === expectedUrl && pairedTabDocumentIds.get(tabId) === expectedDocumentId;
  if (!current()) return; if (!routeActive(native, generation) || !(native ? nativeAttachedTabs : manualAttachedTabs).has(tabId)) throw new Error("Shared tab session ended during navigation.");
  let attaching = navigationDebuggerAttaches.get(tabId); if (!attaching) { attaching = attachDebugger(tabId); navigationDebuggerAttaches.set(tabId, attaching); }
  try { await attaching; } finally { if (navigationDebuggerAttaches.get(tabId) === attaching) navigationDebuggerAttaches.delete(tabId); }
  const identity = await nativePageIdentity(tabId); if (!current()) return; if (!routeActive(native, generation) || identity.url !== expectedUrl || identity.document_id !== expectedDocumentId) { await detachDebugger(tabId); return; }
  pairedTabUrls.set(tabId, identity.url); pairedTabDocumentIds.set(tabId, identity.document_id);
}
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === "loading") { clearFileChooserState(tabId, "Target navigated before the file chooser opened."); tabNavigationGenerations.set(tabId, (tabNavigationGenerations.get(tabId) || 0) + 1); if (pairedTabDocumentIds.has(tabId)) navigatingTabs.add(tabId); }
  if (changeInfo.status === "complete") { const pending = pendingMainFrameNavigations.get(tabId); if (pending?.failed) void clearFailedNavigationWhenStable(tabId, pending); if (!debuggerDetachedDuringNavigation.has(tabId)) navigatingTabs.delete(tabId); }
  const pairedUrl = pairedTabUrls.get(tabId); if (pairedUrl === undefined || !changeInfo.url || changeInfo.url === "about:blank" || navigatingTabs.has(tabId)) return;
  const changed = (() => { try { const before = new URL(pairedUrl), after = new URL(changeInfo.url); return before.origin !== after.origin || before.pathname !== after.pathname || before.search !== after.search; } catch { return changeInfo.url !== pairedUrl; } })(); if (changed) void detachPairedTab(tabId);
});
chrome.webNavigation.onBeforeNavigate.addListener(details => {
  if (details.frameId !== 0) return;
  const previous = pendingMainFrameNavigations.get(details.tabId);
  const ambiguous = Boolean(previous && !previous.failed && previous.url === details.url);
  pendingMainFrameNavigations.set(details.tabId, {
    sequence: (previous?.sequence || 0) + 1,
    startedAt: details.timeStamp,
    url: details.url,
    documentId: undefined,
    ambiguous,
  });
  tabNavigationGenerations.set(details.tabId, (tabNavigationGenerations.get(details.tabId) || 0) + 1); if (pairedTabDocumentIds.has(details.tabId)) navigatingTabs.add(details.tabId);
});
chrome.webNavigation.onCommitted.addListener(details => {
  if (details.frameId !== 0) return; domReadyDocuments.delete(details.tabId); tabNavigationGenerations.set(details.tabId, (tabNavigationGenerations.get(details.tabId) || 0) + 1);
  const pending = pendingMainFrameNavigations.get(details.tabId);
  // Event timestamps identify when each callback fired, not which navigation
  // sequence emitted it. If same-URL sequences overlap, fail closed instead
  // of letting a delayed older commit claim the newer pending navigation.
  const isExpectedUrl = pending?.url === details.url;
  if (pending && isExpectedUrl && !pending.ambiguous && Number.isFinite(details.timeStamp) && details.timeStamp >= pending.startedAt) {
    pendingMainFrameNavigations.set(details.tabId, { ...pending, documentId: details.documentId });
  }
  if (!pairedTabDocumentIds.has(details.tabId)) return;
  let url; try { url = validateNavigationUrl(details.url); } catch { void detachPairedTab(details.tabId); return; } if (typeof details.documentId !== "string" || !details.documentId) { void detachPairedTab(details.tabId); return; }
  pairedTabUrls.set(details.tabId, url); pairedTabDocumentIds.set(details.tabId, details.documentId); const native = nativeAttachedTabs.has(details.tabId); const generation = native ? nativeLeases.get(nativeTabLeases.get(details.tabId))?.generation : manualGeneration;
  committedNavigationTabs.set(details.tabId, { native, generation, url, documentId: details.documentId }); setTimeout(() => { if (committedNavigationTabs.get(details.tabId)?.documentId === details.documentId) committedNavigationTabs.delete(details.tabId); }, 3000);
  if (!debuggerDetachedDuringNavigation.has(details.tabId)) { navigatingTabs.delete(details.tabId); return; }
  void reattachAfterNavigation(details.tabId, native, generation, url, details.documentId).then(() => { if (pairedTabDocumentIds.get(details.tabId) === details.documentId) { debuggerDetachedDuringNavigation.delete(details.tabId); navigatingTabs.delete(details.tabId); } }).catch(() => { if (pairedTabDocumentIds.get(details.tabId) === details.documentId) void detachPairedTab(details.tabId); });
});
chrome.webNavigation.onDOMContentLoaded.addListener(details => {
  if (details.frameId !== 0 || typeof details.documentId !== "string") return;
  domReadyDocuments.set(details.tabId, details.documentId);
  if (pendingMainFrameNavigations.get(details.tabId)?.documentId === details.documentId) pendingMainFrameNavigations.delete(details.tabId);
});
chrome.webNavigation.onErrorOccurred.addListener(details => {
  if (details.frameId !== 0) return;
  const pending = pendingMainFrameNavigations.get(details.tabId);
  const isCurrentNavigationUrl = pending?.url === details.url;
  if (pending && isCurrentNavigationUrl && !pending.ambiguous &&
      (!Number.isFinite(details.timeStamp) || details.timeStamp >= pending.startedAt)) {
    const failed = { ...pending, failed: true };
    pendingMainFrameNavigations.set(details.tabId, failed);
    void clearFailedNavigationWhenStable(details.tabId, failed);
  }
  if (navigatingTabs.has(details.tabId)) void detachPairedTab(details.tabId);
});
chrome.tabs.onReplaced.addListener((_addedTabId, removedTabId) => {
  pendingMainFrameNavigations.delete(removedTabId); tabNavigationGenerations.set(removedTabId, (tabNavigationGenerations.get(removedTabId) || 0) + 1); if (!pairedTabUrls.has(removedTabId)) return;
  navigatingTabs.delete(removedTabId); debuggerDetachedDuringNavigation.delete(removedTabId); pairedTabUrls.delete(removedTabId); pairedTabDocumentIds.delete(removedTabId); manualAttachedTabs.delete(removedTabId); nativeAttachedTabs.delete(removedTabId); void detachDebugger(removedTabId);
});
chrome.tabs.onRemoved.addListener(tabId => { pendingMainFrameNavigations.delete(tabId); tabNavigationGenerations.delete(tabId); domReadyDocuments.delete(tabId); });
void reconnectStateReady.then(() => { if (!reconnectExhausted) connectNative(); });
