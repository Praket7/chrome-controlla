let socket;
const NATIVE_HOST = "chrome_controlla_bridge";
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 60000;
const MAX_RECONNECT_ATTEMPTS = 8;
const RECONNECT_ALARM = "controlla-native-reconnect";
const RECONNECT_STORAGE_KEY = "nativeReconnectAttempts";
const NATIVE_LEASE_JOURNAL_KEY = "controllaNativeLeaseJournalV1";
const OPEN_RECEIPTS_KEY = "openReceipts";
const ACTIVATION_RECEIPTS_KEY = "activationReceipts";
const OPEN_RECEIPT_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const OPEN_RECEIPT_MAX_COUNT = 256;
const MAX_BATCH_ACTIONS = 64;
const MAX_NATIVE_MESSAGE_BYTES = 900 * 1024;
const MAX_DOWNLOAD_OBSERVERS_PER_TAB = 4;
const MAX_DOWNLOAD_OBSERVERS = 64;
function uploadInputPolicyFunction() { return "function(names){const e=this;if(!e||e.nodeName!=='INPUT'||e.type!=='file')return {ok:false,reason:'target_not_file_input'};const accept=(e.getAttribute('accept')||'').split(',').map(x=>x.trim().toLowerCase()).filter(Boolean);const mime=n=>({txt:'text/plain',csv:'text/csv',json:'application/json',pdf:'application/pdf',png:'image/png',jpg:'image/jpeg',jpeg:'image/jpeg',gif:'image/gif',webp:'image/webp',svg:'image/svg+xml',mp3:'audio/mpeg',mp4:'video/mp4',html:'text/html',xml:'application/xml'})[String(n).split('.').pop().toLowerCase()];if(names.length>1&&!e.multiple)return {ok:false,reason:'multiple_files_not_allowed'};for(const name of names){const lower=String(name).toLowerCase(),ext='.'+lower.split('.').pop(),type=mime(name);if(accept.length&&!accept.some(a=>a.startsWith('.')?lower.endsWith(a):a.endsWith('/*')?!!type&&type.startsWith(a.slice(0,-1)):a.includes('/')?a===type:false))return {ok:false,reason:'accept_filter_mismatch',filename:name};}return {ok:true,accept,multiple:!!e.multiple}}"; }
const DOWNLOAD_OBSERVER_TTL_MS = 60000;
const CONTROLLA_EXTENSION_BUILD_ASSETS = ["background.js", "manifest.json", "open-recovery.html", "popup.html", "popup.js", "secure-input.css", "secure-input.html", "secure-input.js"];
const CONTROLLA_EXTENSION_BUILD_ID_PROMISE = (async () => {
  const runtimeFallback = `controlla-chrome-extension:${chrome.runtime.getManifest().version}:runtime`;
  const compute = (async () => {
  const assets = await Promise.all(CONTROLLA_EXTENSION_BUILD_ASSETS.map(async name => {
    const response = await fetch(chrome.runtime.getURL(name));
    if (!response.ok) throw new Error(`Cannot verify installed Controlla extension asset ${name}.`);
    return { name, bytes: new Uint8Array(await response.arrayBuffer()) };
  }));
  const encoder = new TextEncoder();
  const parts = assets.flatMap(({ name, bytes }) => [encoder.encode(name), new Uint8Array([0]), bytes, new Uint8Array([0])]);
  const totalBytes = parts.reduce((sum, part) => sum + part.length, 0);
  const input = new Uint8Array(totalBytes);
  let offset = 0;
  for (const part of parts) { input.set(part, offset); offset += part.length; }
  const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", input))]
    .map(byte => byte.toString(16).padStart(2, "0")).join("");
    return `controlla-chrome-extension:${chrome.runtime.getManifest().version}:${digest}`;
  })();
  return await Promise.race([
    compute,
    new Promise(resolve => setTimeout(() => resolve(runtimeFallback), 750)),
  ]);
})();
const SCREENSHOT_CHUNK_CHARS = 512 * 1024;
const SHARED_COMMAND_METHODS = new Set(["Page.getFrameTree", "Page.getLayoutMetrics", "Page.captureScreenshot", "Runtime.evaluate", "Runtime.callFunctionOn", "Runtime.releaseObject",
  "Input.dispatchMouseEvent", "Input.dispatchDragEvent", "Input.dispatchTouchEvent", "Input.dispatchKeyEvent", "Input.imeSetComposition", "Input.insertText", "DOM.getDocument", "DOM.querySelector", "DOM.enable", "DOM.describeNode", "DOM.resolveNode", "DOM.setFileInputFiles", "Page.enable", "Page.setInterceptFileChooserDialog", "Accessibility.getPartialAXTree", "Accessibility.getFullAXTree", "Controlla.waitForFileChooser", "Controlla.downloadLifecycle", "Controlla.scrollToTarget"]);
async function sendCdpCommand(target, method, params, timeoutMs = 30000) {
  const boundedTimeoutMs = Number.isFinite(timeoutMs) ? Math.max(1, Math.min(60000, timeoutMs)) : 30000;
  let timer;
  try {
    return await Promise.race([
      chrome.debugger.sendCommand(target, method, params),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(Object.assign(new Error(`CDP command ${method} exceeded its ${boundedTimeoutMs}ms deadline.`), {
          reason_code: "cdp_command_timeout", dispatch_may_have_occurred: true,
        })), boundedTimeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
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
let nativeRecoveryPending = false;
async function persistNativeLeaseJournal() {
  const leases = [...nativeLeases.values()].map(lease => ({
    lease_id: lease.id,
    tabs: [...lease.tabs],
    unrestricted: lease.unrestricted === true,
    generation: lease.generation,
    saved_at_ms: Date.now(),
  }));
  const saved = await chrome.storage.local.get(NATIVE_LEASE_JOURNAL_KEY).catch(() => ({}));
  const previous = saved[NATIVE_LEASE_JOURNAL_KEY];
  if (nativeRecoveryPending && previous?.status === "recovery_required") return;
  const journal = { schema_version: 1, sequence: (Number.isSafeInteger(previous?.sequence) ? previous.sequence : 0) + 1,
    status: nativeRecoveryPending ? "recovery_required" : leases.length ? "active" : "clear", leases };
  await chrome.storage.local.set({ [NATIVE_LEASE_JOURNAL_KEY]: journal }).catch(() => {});
}
async function readNativeLeaseJournal() {
  const saved = await chrome.storage.local.get(NATIVE_LEASE_JOURNAL_KEY).catch(() => ({}));
  const journal = saved[NATIVE_LEASE_JOURNAL_KEY];
  return journal?.schema_version === 1 && Array.isArray(journal.leases) ? journal : { schema_version: 1, sequence: 0, status: "clear", leases: [] };
}
const pairedTabUrls = new Map();
const pairedTabDocumentIds = new Map();
const pairedTabTopologyGenerations = new Map();
const pairedTabsInTransit = new Set();
const tabLifecycleGenerations = new Map();
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
const downloadObservers = new Map();
const recentlyCreatedTabs = new Map();
let fileChooserEventSequence = 0;
const frameSessions = new Map();
const frameSessionWaiters = new Map();
const pendingSecureInputs = new Map();
const openTabInflight = new Map();
const cancelledScreenshotRequests = new Set();
let openTabQueue = Promise.resolve();
let activationEpoch = 0;
let focusEpoch = 0;
chrome.tabs.onActivated.addListener(() => { activationEpoch++; });
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (Object.prototype.hasOwnProperty.call(changeInfo || {}, "frozen")) {
    tabLifecycleGenerations.set(tabId, (tabLifecycleGenerations.get(tabId) || 0) + 1);
  }
});
chrome.tabs.onCreated.addListener(tab => { if (Number.isSafeInteger(tab.id)) { recentlyCreatedTabs.set(tab.id, { opener_tab_id: tab.openerTabId, window_id: tab.windowId, created_at_ms: Date.now() }); for (const [id, value] of recentlyCreatedTabs) if (value.created_at_ms < Date.now() - 60000) recentlyCreatedTabs.delete(id); } });
function bumpPairedTabTopology(tabId) {
  if (!pairedTabTopologyGenerations.has(tabId)) return;
  pairedTabTopologyGenerations.set(tabId, (pairedTabTopologyGenerations.get(tabId) || 0) + 1);
}
function pairedTabTopologyStable(tabId, expectedGeneration) {
  return !pairedTabsInTransit.has(tabId) && pairedTabTopologyGenerations.get(tabId) === expectedGeneration;
}
function bindPairedTabTopology(tabId) {
  if (!pairedTabTopologyGenerations.has(tabId)) pairedTabTopologyGenerations.set(tabId, 0);
  pairedTabsInTransit.delete(tabId);
}
function clearPairedTabTopology(tabId) {
  pairedTabTopologyGenerations.delete(tabId);
  pairedTabsInTransit.delete(tabId);
}
chrome.tabs.onDetached.addListener(tabId => {
  if (!pairedTabTopologyGenerations.has(tabId)) return;
  bumpPairedTabTopology(tabId);
  pairedTabsInTransit.add(tabId);
});
chrome.tabs.onAttached.addListener(tabId => {
  if (!pairedTabTopologyGenerations.has(tabId)) return;
  bumpPairedTabTopology(tabId);
  pairedTabsInTransit.delete(tabId);
});
chrome.tabs.onMoved.addListener(tabId => bumpPairedTabTopology(tabId));
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
function validDownloadUrl(value) {
  if (typeof value !== "string" || !value || value.length > 2048 || /\s/.test(value)) return false;
  try {
    const url = new URL(value);
    if (url.username || url.password) return false;
    if (url.protocol === "http:" || url.protocol === "https:") return !!url.hostname;
    if (url.protocol === "blob:") {
      const inner = new URL(url.pathname);
      return (inner.protocol === "http:" || inner.protocol === "https:") && !!inner.hostname;
    }
    return false;
  } catch { return false; }
}
async function downloadUrlHash(value) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}
function safeDownloadFilename(value) {
  if (typeof value !== "string" || !value) return null;
  const name = value.split(/[\\/]/).at(-1).slice(0, 255);
  return name || null;
}
function clearDownloadObserver(observerId, outcome) {
  const observer = downloadObservers.get(observerId);
  if (!observer) return false;
  clearTimeout(observer.expiryTimer);
  downloadObservers.delete(observerId);
  if (observer.waiter) {
    clearTimeout(observer.waiter.timer);
    const waiter = observer.waiter;
    observer.waiter = undefined;
    waiter.resolve(outcome || { status: "interrupted", reason_code: "observer_cleared" });
  }
  return true;
}
function clearDownloadLifecycleState(tabId, reason = "target_detached") {
  for (const [observerId, observer] of downloadObservers) {
    if (observer.tabId === tabId) {
      clearDownloadObserver(observerId, { status: "interrupted", reason_code: reason, filename: observer.filename || null,
        bytes_received: observer.bytesReceived || 0, url_identity: observer.urlIdentity || null });
    }
  }
}
function refreshDownloadObserverExpiry(observer) {
  clearTimeout(observer.expiryTimer);
  observer.expiryTimer = setTimeout(() => {
    clearDownloadObserver(observer.id, { status: "timed_out", reason_code: "observer_expired",
      filename: observer.filename || null, bytes_received: observer.bytesReceived || 0,
      url_identity: observer.urlIdentity || null, download_cancelled: false });
  }, DOWNLOAD_OBSERVER_TTL_MS);
}
function nextDownloadLifecycleEvent(observer, timeoutMs) {
  if (observer.events.length) return Promise.resolve(observer.events.shift());
  if (observer.waiter) return Promise.reject(new Error("A download lifecycle wait is already active for this observer."));
  return new Promise(resolve => {
    const waiter = { resolve, timer: setTimeout(() => {
      if (observer.waiter === waiter) observer.waiter = undefined;
      resolve({ status: "timed_out", reason_code: "download_wait_expired",
        filename: observer.filename || null, bytes_received: observer.bytesReceived || 0,
        url_identity: observer.urlIdentity || null, download_cancelled: false });
    }, timeoutMs) };
    observer.waiter = waiter;
  });
}
function publishDownloadEvent(observer, item, status, reasonCode) {
  const publish = async () => {
    if (downloadObservers.get(observer.id) !== observer || pairedTabDocumentIds.get(observer.tabId) !== observer.documentId) return;
    const filename = safeDownloadFilename(item.filename) || observer.filename || null;
    const bytesReceived = Number.isSafeInteger(item.bytesReceived) && item.bytesReceived >= 0 ? item.bytesReceived : observer.bytesReceived || 0;
    const totalBytes = Number.isSafeInteger(item.totalBytes) && item.totalBytes >= 0 ? item.totalBytes : null;
    const sourceUrl = typeof item.url === "string" ? item.url : "";
    const finalUrl = typeof item.finalUrl === "string" ? item.finalUrl : "";
    const urlIdentity = {
      matches_expected: sourceUrl === observer.expectedUrl,
      url_sha256: validDownloadUrl(sourceUrl) ? await downloadUrlHash(sourceUrl) : null,
      final_url_sha256: validDownloadUrl(finalUrl) ? await downloadUrlHash(finalUrl) : null,
    };
    // Hashing is asynchronous. Navigation, cancellation, or detachment can
    // retire this observer while it is in flight; never resurrect it.
    if (downloadObservers.get(observer.id) !== observer || pairedTabDocumentIds.get(observer.tabId) !== observer.documentId) return;
    observer.downloadId = item.id;
    observer.filename = filename;
    observer.bytesReceived = bytesReceived;
    observer.totalBytes = totalBytes;
    observer.urlIdentity = urlIdentity;
    const event = { status, filename, bytes_received: bytesReceived,
      total_bytes: totalBytes, url_identity: urlIdentity,
      // Internal acquisition binding. The MCP runtime consumes these only
      // after a completed event and strips them from the caller-facing receipt.
      ...(status === "completed" && Number.isSafeInteger(item.id) ? { download_id: item.id } : {}),
      ...(status === "completed" && typeof item.filename === "string" ? { file_path: item.filename } : {}),
      ...(reasonCode ? { reason_code: reasonCode } : {}) };
    observer.events.push(event);
    if (observer.events.length > 3) observer.events.shift();
    refreshDownloadObserverExpiry(observer);
    if (observer.waiter) {
      const waiter = observer.waiter;
      observer.waiter = undefined;
      clearTimeout(waiter.timer);
      waiter.resolve(observer.events.shift());
    }
  };
  observer.publishQueue = (observer.publishQueue || Promise.resolve()).then(publish, publish);
  return observer.publishQueue;
}
chrome.downloads.onCreated.addListener(item => {
  if (!Number.isSafeInteger(item?.tabId) || !Number.isSafeInteger(item?.id) || typeof item.url !== "string") return;
  for (const observer of downloadObservers.values()) {
    if (observer.tabId !== item.tabId || observer.documentId !== pairedTabDocumentIds.get(item.tabId) || item.url !== observer.expectedUrl) continue;
    observer.item = item;
    void publishDownloadEvent(observer, item, item.state === "complete" ? "completed" : item.state === "interrupted" ? "interrupted" : "started",
      item.state === "interrupted" ? "download_interrupted" : undefined);
  }
});
chrome.downloads.onChanged.addListener(delta => {
  if (!Number.isSafeInteger(delta?.id)) return;
  for (const observer of downloadObservers.values()) {
    if (observer.downloadId !== delta.id) continue;
    const previous = observer.item || {};
    const item = { ...previous, id: delta.id };
    for (const field of ["state", "filename", "bytesReceived", "totalBytes", "url", "finalUrl"]) {
      if (delta[field] && Object.hasOwn(delta[field], "current")) item[field] = delta[field].current;
    }
    observer.item = item;
    if (item.state === "complete" || item.state === "interrupted") {
      void publishDownloadEvent(observer, item, item.state === "complete" ? "completed" : "interrupted",
        item.state === "interrupted" ? (delta.error?.current || "download_interrupted") : undefined);
    } else {
      observer.filename = safeDownloadFilename(item.filename) || observer.filename || null;
      observer.bytesReceived = Number.isSafeInteger(item.bytesReceived) && item.bytesReceived >= 0 ? item.bytesReceived : observer.bytesReceived || 0;
    }
  }
});
async function frameActionablePoint(route, objectId) {
  const result = await sendCdpCommand(route, "Runtime.callFunctionOn", { objectId, functionDeclaration: frameActionableGuardFunction(), returnByValue: true });
  const point = result.result?.value;
  if (!(point?.x >= 0) || !(point?.y >= 0)) throw new Error("Frame control is disabled, outside the viewport, or obscured.");
  return point;
}
function frameActionableGuardFunction() {
  return `function __controllaFrameActionable(){
    const e=this,d=e?.ownerDocument,v=d?.defaultView,r=e?.getBoundingClientRect();
    if(!e||!d||!v||!r||!e.isConnected||!r.width||!r.height||r.left<0||r.top<0||r.right>v.innerWidth||r.bottom>v.innerHeight)return false;
    let root=e.getRootNode(),depth=0;
    while(root!==d&&root?.host&&depth<8){root=root.host.getRootNode();depth++}
    if(root!==d)return false;
    for(let n=e;n;n=n.parentElement||n.getRootNode()?.host){
      const s=v.getComputedStyle(n);
      if(n.disabled||n.getAttribute('aria-disabled')==='true'||n.hasAttribute('inert')||n.hasAttribute('hidden')||s.display==='none'||s.visibility!=='visible'||s.pointerEvents==='none'||Number.parseFloat(s.opacity||'1')<=0)return false
    }
    const x=r.left+r.width/2,y=r.top+r.height/2;
    let h=d.elementFromPoint(x,y);
    for(let i=0;h&&i<8;i++){
      if(h===e||e.contains(h))return {x,y,viewport:{width:v.innerWidth,height:v.innerHeight}};
      const shadow=h.shadowRoot;
      if(!shadow)break;
      h=shadow.elementFromPoint?.(x,y)
    }
    return false
  }`;
}
async function rootPointForFrame(tabId, frameId, objectId, x, y, childViewport) {
  const root = { tabId }, tree = await sendCdpCommand(root, "Page.getFrameTree", {});
  const path = (node, ancestors = []) => node?.frame?.id === frameId ? [...ancestors, node.frame] : (node?.childFrames || []).map(child => path(child, [...ancestors, node.frame])).find(Boolean);
  const chain = path(tree.frameTree, []);
  if (!chain?.length || chain.length < 2 || chain.length > 9) throw new Error("Frame ancestry is unavailable or too deep.");
  if (chain.some(frame => !frame?.id || !frame?.loaderId)) throw new Error("Frame or loader identity is unavailable in the ancestry.");
  const initialSessions = chain.slice(0, -1).map(frame => frameSessions.get(tabId)?.get(frame.id) || null);
  const owners = [];
  let viewport = childViewport;
  if (!(viewport?.width > 0) || !(viewport?.height > 0)) {
    const measured = await sendCdpCommand({ tabId, sessionId: frameSessions.get(tabId)?.get(frameId) }, "Runtime.callFunctionOn", { objectId, functionDeclaration: "function(){return {width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight}}", returnByValue: true });
    viewport = measured.result?.value;
  }
  // Each child owner belongs to its immediate parent document. Route that
  // lookup through the parent's attached OOPIF session when one exists, then
  // compose the resulting affine mapping into the next ancestor viewport.
  for (let index = chain.length - 1; index > 0; index--) {
    const childFrame = chain[index], parentFrame = chain[index - 1];
    const parentSession = frameSessions.get(tabId)?.get(parentFrame.id);
    const parentRoute = parentSession ? { tabId, sessionId: parentSession } : root;
    const owner = await sendCdpCommand(parentRoute, "DOM.getFrameOwner", { frameId: childFrame.id });
    const ownerId = owner?.backendNodeId;
    if (!Number.isSafeInteger(ownerId) || ownerId <= 0) throw new Error("Frame owner identity is unavailable in its parent session.");
    const box = await sendCdpCommand(parentRoute, "DOM.getBoxModel", { backendNodeId: ownerId });
    const q = box.model?.content;
    if (!Array.isArray(q) || q.length !== 8 || !q.every(Number.isFinite) || !(viewport?.width > 0) || !(viewport?.height > 0)) throw new Error("Frame owner geometry is unavailable.");
    if (Math.abs(q[0] + q[4] - q[2] - q[6]) > 1 || Math.abs(q[1] + q[5] - q[3] - q[7]) > 1) throw new Error("Perspective or non-affine frame transforms are unsupported.");
    const u = x / viewport.width, v = y / viewport.height;
    x = q[0] + u * (q[2] - q[0]) + v * (q[6] - q[0]);
    y = q[1] + u * (q[3] - q[1]) + v * (q[7] - q[1]);
    const resolved = await sendCdpCommand(parentRoute, "DOM.resolveNode", { backendNodeId: ownerId });
    if (!resolved.object?.objectId) throw new Error("Frame owner could not be resolved in its parent session.");
    try {
      const hit = await sendCdpCommand(parentRoute, "Runtime.callFunctionOn", {
        objectId: resolved.object.objectId,
        functionDeclaration: "function(x,y){const s=getComputedStyle(this);return this.isConnected&&s.display!=='none'&&s.visibility==='visible'&&s.pointerEvents!=='none'&&document.elementFromPoint(x,y)===this}",
        arguments: [{ value: x }, { value: y }], returnByValue: true,
      });
      if (hit?.result?.value !== true) throw new Error("The transformed frame point is obscured or outside its owner.");
      const size = await sendCdpCommand(parentRoute, "Runtime.callFunctionOn", { objectId: resolved.object.objectId, functionDeclaration: "function(){return {width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight}}", returnByValue: true });
      const parentViewport = size.result?.value;
      if (!(parentViewport?.width > 0) || !(parentViewport?.height > 0)) throw new Error("Parent frame viewport is unavailable.");
      owners.push({ child: childFrame.id, childLoader: childFrame.loaderId, parent: parentFrame.id, parentLoader: parentFrame.loaderId, parentSession: parentSession || null, ownerId, quad: q });
      viewport = parentViewport;
    } finally {
      await sendCdpCommand(parentRoute, "Runtime.releaseObject", { objectId: resolved.object.objectId }).catch(() => {});
    }
  }
  const latestTree = await sendCdpCommand(root, "Page.getFrameTree", {});
  const latestChain = path(latestTree.frameTree, []);
  if (!latestChain || latestChain.length !== chain.length || latestChain.some((frame, index) => frame.id !== chain[index].id || frame.loaderId !== chain[index].loaderId)) throw new Error("Frame ancestry or loader changed during owner mapping.");
  if (chain.slice(0, -1).some((frame, index) => (frameSessions.get(tabId)?.get(frame.id) || null) !== initialSessions[index])) throw new Error("Frame owner session changed during owner mapping.");
  return { x, y, owner_signature: owners };
}
async function directFramePoint(tabId, frameId, loaderId, x, y, childViewport) {
  const route = { tabId }, tree = await sendCdpCommand(route, "Page.getFrameTree", {});
  const root = tree?.frameTree, matches = (root?.childFrames || []).filter(child => child?.frame?.id === frameId);
  if (matches.length !== 1 || matches[0].frame.loaderId !== loaderId) throw new Error("Only a fresh direct child frame can be mapped safely.");
  const owner = await sendCdpCommand(route, "DOM.getFrameOwner", { frameId });
  const ownerNodeId = owner?.backendNodeId;
  if (!Number.isSafeInteger(ownerNodeId) || ownerNodeId <= 0) throw new Error("Frame owner identity is unavailable.");
  const box = await sendCdpCommand(route, "DOM.getBoxModel", { backendNodeId: ownerNodeId });
  const q = box?.model?.content;
  if (!Array.isArray(q) || q.length !== 8 || !q.every(Number.isFinite) || !(childViewport?.width > 0) || !(childViewport?.height > 0)) throw new Error("Frame transform geometry is unavailable.");
  // A parallelogram is the evidence that one affine transform maps this
  // child viewport into the root viewport. Perspective transforms fail closed.
  if (Math.abs(q[0] + q[4] - q[2] - q[6]) > 1 || Math.abs(q[1] + q[5] - q[3] - q[7]) > 1) throw new Error("Perspective or non-affine frame transforms are unsupported.");
  const rootPoint = {
    x: q[0] + x / childViewport.width * (q[2] - q[0]) + y / childViewport.height * (q[6] - q[0]),
    y: q[1] + x / childViewport.width * (q[3] - q[1]) + y / childViewport.height * (q[7] - q[1]),
  };
  const resolved = await sendCdpCommand(route, "DOM.resolveNode", { backendNodeId: ownerNodeId });
  if (!resolved?.object?.objectId) throw new Error("Frame owner could not be resolved for hit testing.");
  const hit = await sendCdpCommand(route, "Runtime.callFunctionOn", {
    objectId: resolved.object.objectId,
    functionDeclaration: "function(x,y){const s=getComputedStyle(this);return this.isConnected&&s.display!=='none'&&s.visibility==='visible'&&s.pointerEvents!=='none'&&document.elementFromPoint(x,y)===this}",
    arguments: [{ value: rootPoint.x }, { value: rootPoint.y }], returnByValue: true,
  });
  await sendCdpCommand(route, "Runtime.releaseObject", { objectId: resolved.object.objectId }).catch(() => {});
  if (hit?.result?.value !== true) throw new Error("The transformed frame point is obscured or outside its owner.");
  return rootPoint;
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
  await sendCdpCommand({ tabId }, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
  try {
    const inventoryGeneration = pairedTabTopologyGenerations.get(tabId) || 0;
    const sessions = await Promise.all(frames.map(async child => [child, frameSessions.get(tabId)?.get(child.id) || await waitForFrameSession(tabId, child.id, 400)]));
    if (!pairedTabTopologyStable(tabId, inventoryGeneration)) throw new Error("Frame inventory target topology changed while resolving child sessions.");
    const routes = [{ frameId: root.id, loaderId: root.loaderId, target: { tabId } }, ...sessions.filter(([, sessionId]) => sessionId).map(([child, sessionId]) => ({ frameId: child.id, loaderId: child.loaderId, target: { tabId, sessionId } }))];
    const inputs = [];
    const documents = await Promise.all(routes.map(route => sendCdpCommand(route.target, "DOM.getDocument", { depth: -1, pierce: true })));
    if (!pairedTabTopologyStable(tabId, inventoryGeneration)) throw new Error("Frame inventory target topology changed during parallel document reads.");
    for (let index = 0; index < routes.length; index++) {
      const route = routes[index];
      inputs.push(...fileInputNodes(documents[index].root).map(backendNodeId => ({ frame_id: route.frameId, frame_loader_id: route.loaderId, backend_node_id: backendNodeId })));
      if (inputs.length > 256) throw new Error("Too many file inputs for bounded upload discovery.");
    }
    return inputs;
  } finally {
    await sendCdpCommand({ tabId }, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
  }
}
chrome.debugger.onEvent.addListener((source, method, params) => {
  if (Number.isSafeInteger(source?.tabId)) {
    const tabId = source.tabId;
    captureUploadNetworkEvent(tabId, source, method, params || {});
    if (method === "Target.attachedToTarget" && params?.targetInfo?.type === "iframe" && typeof params.sessionId === "string") {
      recordFrameSession(tabId, params.targetInfo.targetId, params.sessionId);
      void sendCdpCommand({ tabId, sessionId: params.sessionId }, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] }).catch(() => {});
    } else if (method === "Page.frameNavigated" && typeof source.sessionId === "string" && typeof params?.frame?.id === "string") {
      recordFrameSession(tabId, params.frame.id, source.sessionId);
    } else if (method === "Target.detachedFromTarget") {
      const frames = frameSessions.get(tabId);
      if (frames) for (const [frameId, sessionId] of frames) if (sessionId === params?.sessionId) frames.delete(frameId);
      bumpPairedTabTopology(tabId);
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
  void sendCdpCommand({ tabId }, "Page.getFrameTree", {}).then(tree => {
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
function sendNative(port, generation, message) {
  if (nativePort !== port || nativePortGeneration !== generation) return false;
  try {
    const encoded = JSON.stringify(message);
    if (new TextEncoder().encode(encoded).byteLength > MAX_NATIVE_MESSAGE_BYTES) {
      port.postMessage({ type: message?.type === "tabs" ? "tabs_error" : (message?.type || "result"), id: message?.id, request_id: message?.request_id, lease_id: message?.lease_id,
        error: "AX or command response exceeded the safe native-message size; result was not forwarded.", reason_code: "native_message_too_large" });
      return false;
    }
    port.postMessage(message); return true;
  } catch { return false; }
}
function boundedNativeApi(promise, fallback, timeoutMs = 1500) {
  return Promise.race([
    Promise.resolve(promise).catch(() => fallback),
    new Promise(resolve => setTimeout(() => resolve(fallback), timeoutMs)),
  ]);
}
async function sendScreenshotResponse(port, generation, response) {
  const data = response?.result?.data;
  if (typeof data !== "string" || data.length <= SCREENSHOT_CHUNK_CHARS) return sendNative(port, generation, response);
  const chunks = Array.from({ length: Math.ceil(data.length / SCREENSHOT_CHUNK_CHARS) }, (_, index) =>
    data.slice(index * SCREENSHOT_CHUNK_CHARS, (index + 1) * SCREENSHOT_CHUNK_CHARS));
  if (chunks.length > 16 || data.length > 8 * 1024 * 1024) {
    return sendNative(port, generation, { type: "result", id: response.id, lease_id: response.lease_id,
      error: "Screenshot exceeded the bounded chunk-transfer limit.", reason_code: "screenshot_transfer_too_large" });
  }
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(data));
  const base64_sha256 = [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
  const visualEvidence = response?.result?.visual_evidence;
  const transfer_id = crypto.randomUUID();
  const requestKey = `${response.lease_id || ""}:${response.id}`;
  if (cancelledScreenshotRequests.delete(requestKey)) {
    sendNative(port, generation, { type: "result_chunk_abort", id: response.id, lease_id: response.lease_id, transfer_id });
    return false;
  }
  for (let index = 0; index < chunks.length; index++) {
    if (cancelledScreenshotRequests.delete(requestKey)) {
      sendNative(port, generation, { type: "result_chunk_abort", id: response.id, lease_id: response.lease_id, transfer_id });
      return false;
    }
    const chunk = { type: "result_chunk", id: response.id, lease_id: response.lease_id,
      transfer_id, index, total_chunks: chunks.length, total_chars: data.length, base64_sha256,
      ...(visualEvidence ? { visual_evidence: visualEvidence } : {}), chunk: chunks[index] };
    if (!sendNative(port, generation, chunk)) {
      sendNative(port, generation, { type: "result_chunk_abort", id: response.id, lease_id: response.lease_id, transfer_id });
      return false;
    }
  }
  return true;
}
function decodeScreenshotPng(data) {
  if (typeof data !== "string" || data.length > 8 * 1024 * 1024 || !data.startsWith("iVBOR")) throw new Error("Screenshot is empty or oversized.");
  let binary;
  try { binary = atob(data); } catch { throw new Error("Screenshot payload is not valid base64."); }
  if (binary.length < 24 || binary.charCodeAt(0) !== 0x89 || binary.slice(1, 4) !== "PNG" || binary.slice(8, 12) !== "\x00\x00\x00\x0d" || binary.slice(12, 16) !== "IHDR") throw new Error("Screenshot payload is not a PNG image.");
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(16, false), height = view.getUint32(20, false);
  if (!width || width > 4096 || !height || height > 4096 || width * height > 16_777_216) throw new Error("Screenshot dimensions exceeded the supported bounds.");
  return { bytes, width, height };
}
function resolveNativeLease(leaseId, tabId) { if (leaseId) return nativeLeases.get(leaseId); const owner = nativeTabLeases.get(tabId); if (owner) return nativeLeases.get(owner); return nativeLeases.size === 1 ? nativeLeases.values().next().value : undefined; }
function safeNativeError(value) { if (typeof value !== "string" || !value) return undefined; return value.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 256); }
function safeErrorClass(error) { try { const name = error?.name; return typeof name === "string" && /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(name) ? name : "Error"; } catch { return "Error"; } }
function traceSharedCommand(phase, request, tabId, startedAt, details = {}) {
  if ((!SHARED_COMMAND_METHODS.has(request.method) && request.method !== "Controlla.fastType" && request.method !== "Controlla.humanTypeCharacter") || !Number.isSafeInteger(tabId)) return;
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
      const response = await sendCdpCommand({ tabId }, "Runtime.callFunctionOn", { objectId, functionDeclaration: FAST_TYPE_GUARD, arguments: [{ value: index }, { value }, { value: prepare }, { value: normalizeParagraphs }, { value: true }], returnByValue: true });
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
    try { await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", keyDown); }
    catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "key_down_delivery_unknown", error: safeNativeError(String(error)) }; }
    if (enter) progress += character;
    const afterDown = await guard(progress);
    if (afterDown.ok !== true) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "focus_or_value_changed_after_keydown", guard: afterDown };
    if (!enter) {
      keyEvents++;
      try { await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", { type: "char", key, text: character, unmodifiedText: character }); }
      catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "character_delivery_unknown", error: safeNativeError(String(error)) }; }
      progress += character;
    }
    const afterInput = enter ? afterDown : await guard(progress);
    if (afterInput.ok !== true) return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "focus_or_value_changed_after_character", guard: afterInput };
    keyEvents++;
    try { await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", { type: "keyUp", key, ...(enter ? { code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36 } : {}) }); }
    catch (error) { return { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "key_up_delivery_unknown", error: safeNativeError(String(error)) }; }
    completedChars++;
  }
  const final = await guard(progress);
  return final.ok === true
    ? { status: "verified", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress }
    : { status: "unknown", completed_chars: completedChars, key_events: keyEvents, cdp_commands: keyEvents + guardChecks, readback: progress, reason: "final_editor_guard_failed", guard: final };
}
async function humanTypeCharacter(tabId, params, isAuthorized) {
  const { object_id: objectId, index, expected, character, normalize_paragraphs: normalizeParagraphs } = params || {};
  const scalar = typeof character === "string" && [...character].length === 1 ? character.codePointAt(0) : -1;
  if (typeof objectId !== "string" || objectId.length < 1 || objectId.length > 1024 || !Number.isSafeInteger(index) || index < 0 || typeof expected !== "string" || expected.length > 16384 || scalar < 0 || scalar >= 0xd800 && scalar <= 0xdfff || typeof normalizeParagraphs !== "boolean" || character === "\r" || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(character)) throw new Error("human_type_character requires one bounded Unicode scalar (or normalized newline) and a retained editor.");
  const deadline = performance.now() + Math.min(60000, Math.max(1000, Number.isSafeInteger(params.deadline_ms) ? params.deadline_ms : 30000));
  const guard = async value => {
    if (!isAuthorized(tabId) || performance.now() >= deadline) return { ok: false, reason: "deadline_or_session_changed" };
    try {
      const response = await sendCdpCommand({ tabId }, "Runtime.callFunctionOn", { objectId, functionDeclaration: FAST_TYPE_GUARD, arguments: [{ value: index }, { value }, { value: false }, { value: normalizeParagraphs }, { value: true }], returnByValue: true });
      return response?.result?.value || { ok: false, reason: "guard_response_missing" };
    } catch { return { ok: false, reason: "guard_read_failed" }; }
  };
  let keyDownAttempted = false, keyEvents = 0, cdpCommands = 0;
  let result;
  const enter = character === "\n", key = enter ? "Enter" : character;
  const keyUp = enter ? { type: "keyUp", key, code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36 } : { type: "keyUp", key };
  let progress = expected;
  try {
    if (!isAuthorized(tabId) || performance.now() >= deadline) return { status: "not_dispatched", completed_chars: 0, key_events: 0, cdp_commands: 0, readback: expected, reason: "deadline_or_session_changed" };
    const before = await guard(expected); cdpCommands++;
    if (before.ok !== true) return { status: "not_dispatched", completed_chars: 0, key_events: 0, cdp_commands: cdpCommands, readback: before.value, reason: before.reason || "editor_guard_rejected" };
    if (scalar > 0x7f) {
      if (!isAuthorized(tabId) || performance.now() >= deadline) return { status: "not_dispatched", completed_chars: 0, key_events: 0, cdp_commands: cdpCommands, readback: expected, reason: "deadline_or_session_changed" };
      await sendCdpCommand({ tabId }, "Input.insertText", { text: character }); cdpCommands++;
      progress += character;
      const afterInsert = await guard(progress); cdpCommands++;
      result = afterInsert.ok === true
        ? { status: "verified", completed_chars: 1, key_events: 0, dispatches: 1, cdp_commands: cdpCommands, readback: progress, input_method: "insertText" }
        : { status: "unknown", completed_chars: 0, key_events: 0, dispatches: 1, cdp_commands: cdpCommands, readback: afterInsert.value ?? progress, reason: "focus_or_value_changed_after_unicode_insert", guard: afterInsert, input_method: "insertText" };
      return result;
    }
    const down = enter ? { type: "keyDown", key, code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36, text: "\r", unmodifiedText: "\r" } : { type: "keyDown", key };
    keyDownAttempted = true;
    await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", down); keyEvents++; cdpCommands++;
    if (enter) progress += character;
    const afterDown = await guard(progress); cdpCommands++;
    if (afterDown.ok !== true) result = { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: afterDown.value ?? progress, reason: "focus_or_value_changed_after_keydown", guard: afterDown };
    else {
      if (!enter) {
        await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", { type: "char", key, text: character, unmodifiedText: character }); keyEvents++; cdpCommands++;
        progress += character;
      }
      const afterCharacter = enter ? afterDown : await guard(progress);
      if (!enter) cdpCommands++;
      result = afterCharacter.ok === true
        ? { status: "verified", completed_chars: 1, key_events: keyEvents, cdp_commands: cdpCommands, readback: progress }
        : { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: afterCharacter.value ?? progress, reason: "focus_or_value_changed_after_character", guard: afterCharacter };
    }
  } catch (error) {
    result = { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: progress, reason: "key_event_delivery_unknown", error: safeNativeError(String(error)) };
  } finally {
    if (keyDownAttempted) {
      // Only release to the retained editor while it is still the focused target.
      // CDP keyUp is tab-scoped and would otherwise reach a newly focused control.
      const focused = await guard(progress); cdpCommands++;
      if (focused.ok === true) {
        try { await sendCdpCommand({ tabId }, "Input.dispatchKeyEvent", keyUp); keyEvents++; cdpCommands++; }
        catch (error) { result = { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: progress, reason: "key_up_cleanup_delivery_unknown", error: safeNativeError(String(error)) }; }
      } else {
        result = { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: progress, reason: "focus_changed_keyup_suppressed_cleanup_unconfirmed", guard: focused };
      }
    }
  }
  return result || { status: "unknown", completed_chars: 0, key_events: keyEvents, cdp_commands: cdpCommands, readback: progress, reason: "character_transaction_incomplete" };
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
      const generation = ++nativeGeneration; nativeRecoveryPending = nativeLeases.size > 0;
      void queueNativeMutation(async () => { await persistNativeLeaseJournal(); await detachNative(generation); }); scheduleNativeReconnect();
    });
  } catch { scheduleNativeReconnect(); }
}
async function handleNativeMessage(port, portGeneration, message) {
  if (port !== nativePort || portGeneration !== nativePortGeneration || !message || typeof message.type !== "string") return;
  if (message.type === "ready") {
    try {
      const extensionBuildId = await CONTROLLA_EXTENSION_BUILD_ID_PROMISE;
      if (port !== nativePort || portGeneration !== nativePortGeneration) return;
      nativeReady = true; nativeError = undefined; reconnectAttempts = 0; reconnectExhausted = false;
      void chrome.storage.session.remove(RECONNECT_STORAGE_KEY).catch(() => {});
      sendNative(port, portGeneration, { type: "capabilities", protocol_version: 2, extension_build_id: extensionBuildId, multi_session_leases: true });
      const journal = await readNativeLeaseJournal();
      if (port !== nativePort || portGeneration !== nativePortGeneration) return;
      if (journal.status === "recovery_required" && journal.leases.length) {
        sendNative(port, portGeneration, { type: "recovery_status", schema_version: journal.schema_version,
          sequence: journal.sequence, status: journal.status, leases: journal.leases });
      }
    } catch (error) {
      nativeReady = false;
      nativeError = `Cannot verify the installed Controlla extension build: ${safeNativeError(error?.message || error)}`;
      void chrome.storage.local.set({ controllaLastBuildHandshakeError: nativeError }).catch(() => {});
    }
    return;
  }
  if (message.type === "recovery_ack") {
    const journal = await readNativeLeaseJournal();
    if (journal.status === "recovery_required" && message.recovery_complete === true && Number.isSafeInteger(message.sequence) && message.sequence === journal.sequence) {
      nativeRecoveryPending = false;
      await chrome.storage.local.remove(NATIVE_LEASE_JOURNAL_KEY).catch(() => {});
    }
    return;
  }
  if (message.type === "list_tabs") {
    try {
      const offset = Number.isSafeInteger(message.offset) && message.offset >= 0 && message.offset <= 1000000 ? message.offset : 0;
      const pageSize = Number.isSafeInteger(message.page_size) ? Math.min(100, Math.max(1, message.page_size)) : 100;
      const tabs = await boundedNativeApi(chrome.tabs.query({}), [], 2000);
      const focusedWindowTabs = await boundedNativeApi(chrome.tabs.query({ active: true, lastFocusedWindow: true }), [], 1000);
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
      const listed = await Promise.all(prioritized.slice(offset, offset + pageSize).map(async tab => {
        const firstFrame = await boundedNativeApi(chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }), undefined);
        const current = await boundedNativeApi(chrome.tabs.get(tab.id), undefined);
        const lastFrame = await boundedNativeApi(chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }), undefined);
        const stableDocument = current?.status === "complete" && /^https?:\/\//i.test(current.url || "") && firstFrame?.documentId && firstFrame.documentId === lastFrame?.documentId && lastFrame.url === current.url;
        return { id: tab.id, window_id: current?.windowId, group_id: Number.isSafeInteger(current?.groupId) && current.groupId >= 0 ? current.groupId : null, last_accessed: Number.isFinite(current?.lastAccessed) ? current.lastAccessed : undefined, title: String(current?.title || tab.title || "").slice(0, 256), url: String(current?.url || tab.url).slice(0, 2048), active_for_open: current?.id === lastFocusedActive, ...(stableDocument ? { document_id: lastFrame.documentId } : {}) };
      }));
      const [windows, groups] = await Promise.all([
        chrome.windows.getAll({ windowTypes: ["normal", "popup"] }).catch(() => []),
        chrome.tabGroups?.query ? chrome.tabGroups.query({}).catch(() => []) : Promise.resolve([])
      ]);
      const boundedWindows = windows.filter(win => Number.isSafeInteger(win.id));
      const boundedGroups = groups.filter(group => Number.isSafeInteger(group.id));
      const windowLimit = 100, groupLimit = 100;
      sendNative(port, portGeneration, {
        type: "tabs", request_id: message.request_id, offset, total_count: prioritized.length,
        next_offset: offset + listed.length < prioritized.length ? offset + listed.length : null,
        last_focused_window_id: lastFocusedWindowId,
        truncated: offset + listed.length < prioritized.length || boundedWindows.length > windowLimit || boundedGroups.length > groupLimit,
        windows_total: boundedWindows.length, windows_truncated: boundedWindows.length > windowLimit,
        groups_total: boundedGroups.length, groups_truncated: boundedGroups.length > groupLimit,
        windows: boundedWindows.slice(0, windowLimit).map(win => ({ id: win.id, focused: !!win.focused, state: win.state, type: win.type, incognito: !!win.incognito })),
        groups: boundedGroups.slice(0, groupLimit).map(group => ({ id: group.id, window_id: group.windowId, title: String(group.title || "").slice(0, 256), color: group.color, collapsed: !!group.collapsed })),
        tabs: listed
      });
    } catch (error) { sendNative(port, portGeneration, { type: "tabs_error", request_id: message.request_id, error: String(error).slice(0, 512) }); }
    return;
  }
  if (message.type === "pair") { await queueNativeMutation(() => pairNative(port, portGeneration, message)); return; }
  if (message.type === "release") { await queueNativeMutation(() => message.lease_id ? detachNativeLease(message.lease_id) : detachNative(++nativeGeneration)); return; }
  if (message.type === "cancel_screenshot") {
    if (Number.isSafeInteger(message.id) && typeof message.lease_id === "string" && message.lease_id) {
      cancelledScreenshotRequests.add(`${message.lease_id}:${message.id}`);
      while (cancelledScreenshotRequests.size > 128) cancelledScreenshotRequests.delete(cancelledScreenshotRequests.values().next().value);
    }
    return;
  }
  if (message.type === "command" || message.type === "batch") {
    const commandGeneration = nativeGeneration; const leaseId = message.lease_id; const startedAt = Date.now();
    const lease = resolveNativeLease(leaseId, Number(message.target_id));
    const authorize = tabId => nativeGeneration === commandGeneration && (lease ? lease.tabs.has(tabId) || lease.unrestricted : nativeAttachedTabs.has(tabId));
    const reply = response => {
      const envelope = { ...response, ...(leaseId ? { lease_id: leaseId } : {}) };
      const screenshot = message.method === "Page.captureScreenshot" && response.type === "result";
      const sent = nativeGeneration === commandGeneration && (screenshot || sendNative(port, portGeneration, envelope));
      if (sent && screenshot) void sendScreenshotResponse(port, portGeneration, envelope).then(ok => {
        if (!ok) sendNative(port, portGeneration, { type: "result", id: response.id, lease_id: leaseId, error: "Screenshot chunk transfer was interrupted.", reason_code: "screenshot_transfer_interrupted" });
      }).catch(() => sendNative(port, portGeneration, { type: "result", id: response.id, lease_id: leaseId, error: "Screenshot chunk transfer failed.", reason_code: "screenshot_transfer_failed" }));
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
      pairedTabUrls.set(tabId, beforeAttach.url); pairedTabDocumentIds.set(tabId, beforeFrame.documentId); bindPairedTabTopology(tabId); await attachDebugger(tabId);
      if (port !== nativePort || nativeLeases.get(leaseId) !== lease) { await detachDebugger(tabId); throw new Error("Pairing was released during attachment."); }
      lease.tabs.add(tabId); nativeTabLeases.set(tabId, leaseId); nativeAttachedTabs.add(tabId);
      const attached = await chrome.tabs.get(tabId).catch(() => null); const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0) || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url || attachedFrame?.documentId !== beforeFrame.documentId) throw new Error("Tab " + tabId + " navigated during pairing; rediscover tabs and pair again.");
    }
    const extensionBuildId = await CONTROLLA_EXTENSION_BUILD_ID_PROMISE;
    // A fresh, identity-checked pair is the owner-side recovery completion
    // point. Until then the reconnect journal must remain durable.
    nativeRecoveryPending = false;
    await persistNativeLeaseJournal();
    sendNative(port, portGeneration, { type: "paired", request_id: message.request_id, lease_id: leaseId, targets: [...lease.tabs].map(String).sort(), extension_version: chrome.runtime.getManifest().version, extension_build_id: extensionBuildId, protocol_version: 2, document_identity: true, batch_execution: true, batch_deadline: true });
  } catch (error) {
    if (attemptGeneration !== undefined) await detachNativeLease(typeof message.lease_id === "string" && message.lease_id ? message.lease_id : message.request_id);
    if (reservation) for (const tabId of requested || []) { if (manualAttachedTabs.has(tabId)) continue; pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); }
    sendNative(port, portGeneration, { type: "pair_error", request_id: message.request_id, error: String(error) });
  } finally { if (reservation) releasePairingTabs(requested, reservation); }
}
async function dispatchBatch(request, isAuthorized, respond, nativeRoute = false) {
  const tabId = Number(request.target_id); const actions = request.actions; const startedAt = performance.now();
  const lease = nativeRoute ? nativeLeases.get(request.lease_id) : undefined;
  if (nativeRoute && lease?.unrestricted && !lease.tabs.has(tabId)) { try { await attachUnrestrictedNativeTab(tabId, lease.id); } catch (error) { respond({ type: "batch_result", id: request.id, error: String(error) }); return; } }
  const topologyGeneration = pairedTabTopologyGenerations.get(tabId);
  const topologyStable = () => topologyGeneration !== undefined && pairedTabTopologyStable(tabId, topologyGeneration);
  if (!Number.isSafeInteger(tabId) || !isAuthorized(tabId)) { respond({ type: "batch_result", id: request.id, error: "Target is not attached." }); return; }
  if (!topologyStable()) { respond({ type: "batch_result", id: request.id, result: { status: "not_dispatched", reason_code: "tab_topology_unstable", dispatch_may_have_occurred: false }, error: "Paired tab is moving between windows; retry after its topology settles." }); return; }
  if (!Array.isArray(actions) || actions.length === 0 || actions.length > MAX_BATCH_ACTIONS) { respond({ type: "batch_result", id: request.id, error: "Batch requires 1..64 actions." }); return; }
  if (!Number.isSafeInteger(request.deadline_ms) || request.deadline_ms < 1 || request.deadline_ms > 60000) { respond({ type: "batch_result", id: request.id, error: "Batch deadline must be within 1..60000ms." }); return; }
  const receipts = [];
  const acknowledgedKeyEvent = () => receipts.some(receipt => receipt.method === "Input.dispatchKeyEvent" && !receipt.error);
  for (let index = 0; index < actions.length; index++) {
    if (!topologyStable()) { respond({ type: "batch_result", id: request.id, result: { status: receipts.length ? "unknown" : "not_dispatched", reason_code: "tab_topology_changed", receipts, completed: index, stopped_before: index, dispatch_may_have_occurred: acknowledgedKeyEvent() || receipts.length > 0, host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Paired tab topology changed; remaining batch actions were fenced." }); return; }
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
    if (!topologyStable()) { respond({ type: "batch_result", id: request.id, result: { status: "unknown", reason_code: "tab_topology_changed", receipts, completed: index + 1, stopped_before: index + 1, dispatch_may_have_occurred: true, host_round_trips: 1, in_browser_actions: actions.length, elapsed_ms: performance.now() - startedAt }, error: "Paired tab topology changed; remaining batch actions were fenced." }); return; }
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
    operation.networkOutcomes?.set(params.requestId, "pending");
  } else if (method === "Network.responseReceived") {
    request = operation.networkRequests.get(params.requestId);
    if (!request || !(params.type === "XHR" || params.type === "Fetch")) return;
    operation.networkOutcomes?.set(params.requestId, "response");
    operation.networkEvents.push({ ...request, status: params.response?.status, status_text: params.response?.statusText, mime_type: params.response?.mimeType });
  } else if (method === "Network.loadingFailed") {
    request = operation.networkRequests.get(params.requestId);
    if (!request) return;
    operation.networkOutcomes?.set(params.requestId, "failed");
    operation.networkEvents.push({ ...request, error: params.errorText, blocked_reason: params.blockedReason, cors_error: params.corsErrorStatus?.corsError });
  } else return;
  if (operation.networkEvents.length > 24) operation.networkEvents.shift();
  operation.networkChanged?.(operation.networkEvents.at(-1));
}
function waitForUploadNetwork(operation, maxWaitMs = 10000) {
  return new Promise(resolve => {
    const started = Date.now();
    let quietTimer;
    const finish = reason => {
      clearTimeout(maxTimer);
      clearTimeout(quietTimer);
      operation.networkChanged = undefined;
      const pendingRequests = [...(operation.networkOutcomes || new Map()).values()].filter(value => value === "pending").length;
      resolve({ status: pendingRequests ? "stalled" : "observed", reason: pendingRequests ? reason || "upload_response_not_observed" : undefined,
        pending_requests: pendingRequests, observation_bound_ms: maxWaitMs, observation_window_ms: Math.max(0, Date.now() - started) });
    };
    const maxTimer = setTimeout(() => finish("upload_observation_timeout"), maxWaitMs);
    operation.networkChanged = event => {
      if (event?.status != null || event?.error) return finish();
      clearTimeout(quietTimer);
      quietTimer = setTimeout(() => finish("upload_response_not_observed"), 1200);
    };
    if (operation.networkEvents.some(event => event.status != null || event.error)) finish();
    else quietTimer = setTimeout(() => finish("upload_response_not_observed"), operation.networkRequests.size ? 1200 : 2500);
  });
}
async function dispatchCommand(request, isAuthorized, respond, nativeMessage = false) {
  const tabId = Number(request.target_id); const startedAt = Date.now();
  const nativeLease = nativeMessage ? resolveNativeLease(request.lease_id, tabId) : undefined;
  if (nativeMessage && nativeLease?.unrestricted && !nativeLease.tabs.has(tabId)) { try { await attachUnrestrictedNativeTab(tabId, nativeLease.id); } catch (error) { respond({ type: "result", id: request.id, error: String(error) }); return; } }
  const leaseAuthorized = isAuthorized;
  const topologyGeneration = pairedTabTopologyGenerations.get(tabId);
  const topologyStable = () => pairedTabTopologyStable(tabId, topologyGeneration);
  isAuthorized = candidateTabId => leaseAuthorized(candidateTabId) && candidateTabId === tabId && topologyStable();
  const sendResponse = respond;
  let commandMayHaveDispatched = false;
  respond = message => {
    if (commandMayHaveDispatched && !topologyStable() && (message.result || message.reason_code === "tab_topology_changed")) {
      sendResponse({ type: "result", id: request.id, result: {
        status: "unknown", reason_code: "tab_topology_changed", dispatch_may_have_occurred: true,
        reason: "Paired tab topology changed during the command; inspect current page state before retrying."
      } });
      return;
    }
    sendResponse(message);
  };
  if (!Number.isSafeInteger(tabId) || !leaseAuthorized(tabId)) { respond({ type: "result", id: request.id, error: "Target is not attached." }); return; }
  if (pairedTabsInTransit.has(tabId) || topologyGeneration === undefined) {
    respond({ type: "result", id: request.id, result: { status: "not_dispatched", reason_code: "tab_topology_unstable", dispatch_may_have_occurred: false,
      reason: "Paired tab is moving between windows; retry after its topology settles." } });
    return;
  }
  if (!isAuthorized(tabId)) { respond({ type: "result", id: request.id, error: "Target is no longer attached or its topology is unstable." }); return; }
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
    if (!topologyStable()) {
      respond({ type: "result", id: request.id, result: { status: "not_dispatched", reason_code: "tab_topology_changed", dispatch_may_have_occurred: false,
        reason: "Paired tab topology changed during preflight; retry after reconciling its window." } });
      return;
    }
    commandMayHaveDispatched = true;
    if (request.method === "Controlla.openTab") {
      const url = validateNavigationUrl(request.params?.url); const native = nativeRoute; const waitUntil = request.params?.wait_until ?? "dom_content_loaded";
      const active = request.params?.active ?? true;
      const createBackgroundWindow = request.params?.create_background_window === true;
      const targetWindowId = request.params?.target_window_id;
      const placement = Object.fromEntries(["left", "top", "width", "height"].filter(key => Number.isSafeInteger(request.params?.[`window_${key}`])).map(key => [key, request.params[`window_${key}`]]));
      if (waitUntil !== "document_committed" && waitUntil !== "dom_content_loaded") throw new Error("wait_until must be document_committed or dom_content_loaded.");
      if (typeof active !== "boolean") throw new Error("active must be a boolean.");
      if (createBackgroundWindow && active) throw new Error("A Controlla-owned automation window must be requested in background mode.");
      if (targetWindowId != null && (!Number.isSafeInteger(targetWindowId) || targetWindowId <= 0 || createBackgroundWindow)) throw new Error("target_window_id must identify an existing Controlla automation window and cannot be combined with create_background_window.");
      if (Object.values(placement).some(value => Math.abs(value) > 32768) || [placement.width, placement.height].some(value => value !== undefined && (value < 200 || value > 8192))) throw new Error("Background window placement is outside the supported bounds.");
      if (!createBackgroundWindow && Object.keys(placement).length) throw new Error("Window placement requires create_background_window.");
      const result = await openControllaTab(url, request.params?.idempotency_key, native, routeGeneration, waitUntil, active, createBackgroundWindow, placement, targetWindowId);
      respond({ type: "result", id: request.id, result }); return;
    }
    if (request.method === "Controlla.closeOwnedTab") {
      const ownedTabId = Number(request.params?.owned_tab_id);
      if (!Number.isSafeInteger(ownedTabId) || ownedTabId <= 0 || String(ownedTabId) !== String(request.params?.owned_tab_id)) throw new Error("owned_tab_id must be a decimal Chrome tab ID.");
      const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
      const receipts = saved[OPEN_RECEIPTS_KEY] || {};
      const receiptEntry = Object.entries(receipts).find(([, receipt]) => receipt?.tab_id === String(ownedTabId));
      if (!receiptEntry) throw new Error("This tab has no Controlla-owned open receipt; user-owned tabs cannot be closed by this operation.");
      const [operationKey, receipt] = receiptEntry;
      if (receipt.status === "closed") {
        const stillOpen = await chrome.tabs.get(ownedTabId).then(() => true).catch(() => false);
        if (stillOpen) throw new Error("Close was previously recorded but Chrome still reports the tab; inspect its lifecycle before retrying.");
        respond({ type: "result", id: request.id, result: { status: "closed", tab_id: String(ownedTabId), window_id: receipt.window_id, document_id: receipt.document_id, topology_verified: true, idempotent_reuse: true } }); return;
      }
      if (receipt.status !== "complete" || !receipt.document_id) throw new Error("The Controlla open receipt is incomplete; refusing to close an unverified target.");
      const identity = await nativePageIdentity(ownedTabId).catch(() => undefined);
      if (!identity || identity.document_id !== receipt.document_id || identity.url !== receipt.url) throw new Error("The created tab's document changed after open; refusing to close the changed target.");
      const tab = await chrome.tabs.get(ownedTabId);
      await chrome.tabs.remove(ownedTabId);
      const remainsOpen = await chrome.tabs.get(ownedTabId).then(() => true).catch(() => false);
      if (remainsOpen) throw new Error("Chrome did not confirm that the Controlla-owned tab was removed.");
      receipts[operationKey] = { ...receipt, status: "closed", closed_ms: Date.now() };
      await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
      respond({ type: "result", id: request.id, result: { status: "closed", tab_id: String(ownedTabId), window_id: tab.windowId, document_id: identity.document_id, topology_verified: true, idempotent_reuse: false } }); return;
    }
    if (request.method === "Controlla.activateOwnedTab") {
      const ownedTabId = Number(request.params?.owned_tab_id);
      const idempotencyKey = request.params?.idempotency_key;
      if (!Number.isSafeInteger(ownedTabId) || ownedTabId <= 0 || String(ownedTabId) !== String(request.params?.owned_tab_id)) throw new Error("owned_tab_id must be a decimal Chrome tab ID.");
      if (typeof idempotencyKey !== "string" || !/^[!-~]{1,128}$/.test(idempotencyKey)) throw new Error("idempotency_key must contain 1–128 printable ASCII characters.");
      const saved = await chrome.storage.local.get([OPEN_RECEIPTS_KEY, ACTIVATION_RECEIPTS_KEY]);
      const receipts = saved[OPEN_RECEIPTS_KEY] || {};
      const receiptEntry = Object.entries(receipts).find(([, receipt]) => receipt?.tab_id === String(ownedTabId) && receipt.status === "complete");
      if (!receiptEntry?.[1]?.document_id) throw new Error("This tab has no completed Controlla-owned open receipt; user-owned tabs cannot be activated by this operation.");
      const [openKey, receipt] = receiptEntry;
      const identity = await nativePageIdentity(ownedTabId).catch(() => undefined);
      if (!identity || identity.document_id !== receipt.document_id || identity.url !== receipt.url) throw new Error("The owned tab document changed; refusing to activate a stale target.");
      const activationReceipts = saved[ACTIVATION_RECEIPTS_KEY] || {};
      const prior = activationReceipts[idempotencyKey];
      if (prior && (prior.tab_id !== String(ownedTabId) || prior.document_id !== identity.document_id || prior.open_key !== openKey)) throw new Error("idempotency_key is already bound to a different activation target.");
      const before = await chrome.tabs.get(ownedTabId);
      if (!before.active || !(await chrome.windows.get(before.windowId)).focused) {
        await chrome.windows.update(before.windowId, { focused: true });
        await chrome.tabs.update(ownedTabId, { active: true });
      }
      const [after, afterIdentity, window] = await Promise.all([
        chrome.tabs.get(ownedTabId), nativePageIdentity(ownedTabId).catch(() => undefined), chrome.windows.get(before.windowId)
      ]);
      if (!after.active || !window.focused || after.windowId !== before.windowId || !afterIdentity || afterIdentity.document_id !== identity.document_id || afterIdentity.url !== identity.url) {
        throw new Error("Chrome did not verify the exact owned tab, unchanged document, active tab, and focused window.");
      }
      activationReceipts[idempotencyKey] = { tab_id: String(ownedTabId), document_id: identity.document_id, open_key: openKey, activated_ms: Date.now() };
      const bounded = Object.entries(activationReceipts).slice(-256);
      await chrome.storage.local.set({ [ACTIVATION_RECEIPTS_KEY]: Object.fromEntries(bounded) });
      respond({ type: "result", id: request.id, result: { status: "activated", tab_id: String(ownedTabId), window_id: after.windowId,
        document_id: afterIdentity.document_id, url: afterIdentity.url, tab_active: true, window_focused: true,
        topology_verified: true, idempotent_reuse: !!prior } }); return;
    }
    if (request.method === "Controlla.moveOwnedTab") {
      const ownedTabId = Number(request.params?.owned_tab_id);
      const targetWindowId = request.params?.target_window_id;
      if (!Number.isSafeInteger(ownedTabId) || ownedTabId <= 0 || String(ownedTabId) !== String(request.params?.owned_tab_id)) throw new Error("owned_tab_id must be a decimal Chrome tab ID.");
      if (!Number.isSafeInteger(targetWindowId) || targetWindowId <= 0) throw new Error("target_window_id must be a positive Chrome window ID.");
      const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
      const receipts = saved[OPEN_RECEIPTS_KEY] || {};
      const receipt = Object.values(receipts).find(value => value?.tab_id === String(ownedTabId) && value.status === "complete");
      if (!receipt?.document_id) throw new Error("This tab has no completed Controlla-owned open receipt; user-owned tabs cannot be moved by this operation.");
      const targetWindow = await assertOwnedAutomationWindow(targetWindowId);
      const before = await chrome.tabs.get(ownedTabId);
      const identity = await nativePageIdentity(ownedTabId).catch(() => undefined);
      if (!identity || identity.document_id !== receipt.document_id || identity.url !== receipt.url) throw new Error("The owned tab document changed; refusing to move a stale target.");
      if (before.active || (await chrome.windows.get(before.windowId)).focused) throw new Error("Move requires an inactive owned tab in an unfocused source window.");
      await chrome.tabs.move(ownedTabId, { windowId: targetWindowId, index: -1 });
      const [after, afterIdentity, finalWindow] = await Promise.all([
        chrome.tabs.get(ownedTabId), nativePageIdentity(ownedTabId).catch(() => undefined), chrome.windows.get(targetWindowId)
      ]);
      if (after.windowId !== targetWindowId || !afterIdentity || afterIdentity.document_id !== identity.document_id || afterIdentity.url !== identity.url || finalWindow.focused || targetWindow.focused) {
        throw new Error("Chrome did not verify the exact moved tab, unchanged document, and unfocused destination window.");
      }
      respond({ type: "result", id: request.id, result: { status: "moved", tab_id: String(ownedTabId), source_window_id: before.windowId,
        window_id: after.windowId, document_id: afterIdentity.document_id, url: afterIdentity.url, tab_active: !!after.active,
        window_focused: !!finalWindow.focused, topology_verified: true } }); return;
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
      }).slice(0, 8).map(tab => {
        const event = recentlyCreatedTabs.get(tab.id);
        return { tab_id: String(tab.id), url: String(tab.url).slice(0, 2048), window_id: event?.window_id,
          created_at_ms: event?.created_at_ms, opener_tab_id: String(event?.opener_tab_id ?? ""),
          opener_match: event?.opener_tab_id === tabId };
      });
      respond({ type: "result", id: request.id, result: { tabs: created } }); return;
    }
    if (request.method === "Controlla.fastType") {
      const result = await fastTypeProgram(tabId, request.params, isAuthorized);
      traceSharedCommand("settled", request, tabId, startedAt, { outcome: result.status, completedChars: result.completed_chars, keyEvents: result.key_events, cdpCommands: result.cdp_commands });
      respond({ type: "result", id: request.id, result }); return;
    }
    if (request.method === "Controlla.humanTypeCharacter") {
      const result = await humanTypeCharacter(tabId, request.params, isAuthorized);
      traceSharedCommand("settled", request, tabId, startedAt, { outcome: result.status, completedChars: result.completed_chars, keyEvents: result.key_events, cdpCommands: result.cdp_commands });
      respond({ type: "result", id: request.id, result }); return;
    }
    if (request.method === "Controlla.navigate") { const url = validateNavigationUrl(request.params?.url); await navigatePairedTab(tabId, url, nativeRoute, routeGeneration); respond({ type: "result", id: request.id, result: { tab_id: String(tabId), ...(await nativePageIdentity(tabId)) } }); return; }
    if (request.method === "Controlla.presentationState") {
      const tab = await chrome.tabs.get(tabId);
      const win = await chrome.windows.get(tab.windowId);
      let renderer = {};
      let rendererFailure = "";
      try {
        const observed = await sendCdpCommand({ tabId }, "Runtime.evaluate", {
          expression: "/*__controllaPresentationState*/({visibility_state:document.visibilityState,document_hidden:document.hidden,document_has_focus:document.hasFocus(),viewport:{width:innerWidth,height:innerHeight,device_scale_factor:devicePixelRatio},discarded:document.wasDiscarded===true,prerendering:document.prerendering===true})",
          returnByValue: true,
        });
        const value = observed?.result?.value;
        if (value && typeof value === "object") renderer = value;
      } catch (error) { rendererFailure = safeNativeError(String(error)); }
      const visibility = ["visible", "hidden", "prerender"].includes(renderer.visibility_state) ? renderer.visibility_state : "unknown";
      const viewport = renderer.viewport && Number.isFinite(renderer.viewport.width) && Number.isFinite(renderer.viewport.height)
        && Number.isFinite(renderer.viewport.device_scale_factor) && renderer.viewport.width > 0 && renderer.viewport.height > 0
        ? { width: renderer.viewport.width, height: renderer.viewport.height, device_scale_factor: renderer.viewport.device_scale_factor }
        : undefined;
      const crashed = /(?:target|renderer).{0,24}(?:crash|gone)|(?:crash|gone).{0,24}(?:target|renderer)/i.test(rendererFailure);
      const lifecycle = tab.discarded || renderer.discarded ? "discarded" : tab.status === "loading" ? "loading" : crashed ? "crashed" : tab.frozen === true ? "frozen" : tab.frozen === false ? "active" : "unknown";
      respond({ type: "result", id: request.id, result: {
        tab_active: !!tab.active, window_focused: !!win.focused, window_id: tab.windowId,
        window_state: ["normal", "minimized", "maximized", "fullscreen"].includes(win.state) ? win.state : "unknown",
        visibility_state: visibility, document_hidden: typeof renderer.document_hidden === "boolean" ? renderer.document_hidden : undefined,
        document_has_focus: typeof renderer.document_has_focus === "boolean" ? renderer.document_has_focus : undefined,
        lifecycle, renderer_unavailable: !!rendererFailure, prerendering: renderer.prerendering === true, viewport,
        activation_epoch: activationEpoch, focus_epoch: focusEpoch,
      } }); return;
    }
    if (request.method === "Controlla.pointerClick") {
      const params = request.params || {}, objectId = params.object_id, index = params.index, outcome = params.outcome;
      const clickKind = params.click_kind || "click";
      const guard = params.function_declaration, expectedViewport = params.viewport;
      if (typeof objectId !== "string" || !objectId || objectId.length > 512 || !Number.isSafeInteger(index) || index < 0 ||
          typeof guard !== "string" || !guard.startsWith("function(index,") || guard.length > 12000 ||
          !["click", "double_click", "context_click"].includes(clickKind) ||
          (expectedViewport != null && (typeof expectedViewport !== "string" || expectedViewport.length > 256))) throw new Error("pointer click requires a bounded guarded target and supported click kind.");
      const target = await chrome.tabs.get(tabId), window = await chrome.windows.get(target.windowId);
      const wasActive = !!target.active;
      if (!wasActive) {
        respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,dispatch_may_have_occurred:false,temporary_activation:false,
          reason_code:"activation_forbidden",reason:"Pointer input on an inactive user-owned tab would activate it. Use an explicitly Controlla-owned automation window or select the tab in the foreground."} });
        return;
      }
      const previous = wasActive ? target : (await chrome.tabs.query({ active:true, windowId:target.windowId }))[0];
      if (!previous?.id) { respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,reason_code:"active_tab_unavailable",reason:"Could not identify the tab to restore after pointer input."} }); return; }
      const documentId = pairedTabDocumentIds.get(tabId), generation = routeGeneration, activationEpochBefore = activationEpoch;
      let activated = false, restoredPreviousTab = false, pressAttempted = false, acknowledged = false, finalChecked, activePress;
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
        const checked = await sendCdpCommand({tabId}, "Runtime.callFunctionOn", {objectId,
          functionDeclaration:guard, arguments:[{value:index},{value:outcome}], returnByValue:true});
        finalChecked = checked?.result?.value;
        if (!finalChecked?.ok || (!activated && expectedViewport != null && finalChecked.viewport !== expectedViewport) || !Number.isFinite(finalChecked.x) || !Number.isFinite(finalChecked.y)) {
          respond({ type:"result", id:request.id, result:{status:"not_dispatched",dispatch_acknowledged:false,reason_code:"final_target_revalidation_failed",reason:finalChecked?.reason || "Target or viewport changed before pointer input."} }); return;
        }
        const point = {x:finalChecked.x,y:finalChecked.y};
        await sendCdpCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseMoved",...point});
        const button = clickKind === "context_click" ? "right" : "left";
        const clicks = clickKind === "double_click" ? 2 : 1;
        for (let clickCount = 1; clickCount <= clicks; clickCount++) {
          if (clickCount === 2) {
            const currentTarget = await chrome.tabs.get(tabId);
            if (!isAuthorized(tabId) || (nativeRoute && resolveNativeLease(request.lease_id, tabId)?.generation !== generation) ||
                pairedTabDocumentIds.get(tabId) !== documentId || !currentTarget.active) {
              respond({ type:"result", id:request.id, result:{status:"unknown",dispatch_acknowledged:false,dispatch_may_have_occurred:true,
                completed_clicks:1,reason_code:"double_click_target_changed",reason:"The first click was sent, but target identity or active-tab state changed before the second click. Inspect before retrying."} }); return;
            }
            const secondCheck = await sendCdpCommand({tabId}, "Runtime.callFunctionOn", {objectId,
              functionDeclaration:guard, arguments:[{value:index},{value:outcome}], returnByValue:true});
            const secondTarget = secondCheck?.result?.value;
            if (!secondTarget?.ok || secondTarget.viewport !== finalChecked.viewport ||
                secondTarget.x !== point.x || secondTarget.y !== point.y) {
              respond({ type:"result", id:request.id, result:{status:"unknown",dispatch_acknowledged:false,dispatch_may_have_occurred:true,
                completed_clicks:1,reason_code:"double_click_target_changed",reason:"The first click was sent, but the same target, viewport, or hit point could not be revalidated before the second click. Inspect before retrying."} }); return;
            }
          }
          activePress = {button, clickCount, ...point};
          pressAttempted = true;
          await sendCdpCommand({tabId}, "Input.dispatchMouseEvent", {type:"mousePressed",...activePress});
          await sendCdpCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseReleased",...activePress});
          activePress = undefined;
        }
        acknowledged = true;
      } catch (error) {
        if (activePress) await sendCdpCommand({tabId}, "Input.dispatchMouseEvent", {type:"mouseReleased",...activePress}).catch(() => {});
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
    if (request.method === "Controlla.pointerSwipe") {
      const params=request.params||{},objectId=params.object_id,index=params.index,guard=params.guard_function;
      const sx=params.start_x,sy=params.start_y,ex=params.end_x,ey=params.end_y,viewport=params.viewport,timeoutMs=params.timeout_ms??3000;
      const expectedActivationEpoch=params.expected_activation_epoch??undefined,expectedFocusEpoch=params.expected_focus_epoch??undefined;
      if(typeof objectId!=="string"||!objectId||objectId.length>512||!Number.isSafeInteger(index)||index<0||
          typeof guard!=="string"||!guard.startsWith("function(index,sx,sy,ex,ey,expectedViewport,phase)")||guard.length>12000||
          ![sx,sy,ex,ey].every(value=>Number.isFinite(value)&&value>=0&&value<=10000)||
          Math.hypot(ex-sx,ey-sy)<20||Math.hypot(ex-sx,ey-sy)>1200||typeof viewport!=="string"||viewport.length>256||
          !Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("pointer swipe requires bounded coordinates and a guarded fresh target.");
      const target=await chrome.tabs.get(tabId),deadline=performance.now()+timeoutMs;
      if(!target.active){respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"inactive_target",dispatch_may_have_occurred:false,reason:"Touch input requires an active target tab; no activation was attempted."}});return;}
      const documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      let touchStartAttempted=false,dispatchAcknowledged=false,touchReleaseAcknowledged=false,before;
      const guardCall=async phase=>{
        const result=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,
          arguments:[{value:index},{value:sx},{value:sy},{value:ex},{value:ey},{value:viewport},{value:phase}],returnByValue:true});
        return result?.result?.value;
      };
      const stateValid=value=>value?.ok===true&&value.viewport===viewport&&
        value.start?.x===sx&&value.start?.y===sy&&value.end?.x===ex&&value.end?.y===ey&&
        [value.rect?.left,value.rect?.top,value.rect?.width,value.rect?.height].every(Number.isFinite);
      try{
        before=await guardCall("before");
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)||!stateValid(before)){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:before?.reason||"swipe_preflight_refused",dispatch_may_have_occurred:false}});return;
        }
        const final=await guardCall("before");
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)||!stateValid(final)||
            JSON.stringify(final.rect)!==JSON.stringify(before.rect)||JSON.stringify(final.scroll)!==JSON.stringify(before.scroll)){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"swipe_target_changed_before_dispatch",dispatch_may_have_occurred:false}});return;
        }
        touchStartAttempted=true;
        await sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchStart",touchPoints:[{id:1,x:sx,y:sy,radiusX:1,radiusY:1,force:1}]});
        await sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchMove",touchPoints:[{id:1,x:ex,y:ey,radiusX:1,radiusY:1,force:1}]});
        dispatchAcknowledged=true;
      }catch(error){
        respond({type:"result",id:request.id,result:{status:touchStartAttempted?"unknown":"not_dispatched",reason_code:touchStartAttempted?"touch_delivery_unknown":"swipe_preflight_failed",dispatch_acknowledged:false,dispatch_may_have_occurred:touchStartAttempted,reason:safeNativeError(String(error))}});return;
      }finally{
        if(touchStartAttempted){
          touchReleaseAcknowledged=await Promise.race([sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]}).then(()=>true).catch(()=>false),new Promise(resolve=>setTimeout(()=>resolve(false),1000))]);
        }
      }
      try{
        if(!touchReleaseAcknowledged){respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"touch_release_unconfirmed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true}});return;}
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"swipe_identity_changed_after_dispatch",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true}});return;
        }
        const after=await guardCall("after");
        if(!stateValid(after)){
          respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"swipe_readback_unavailable",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true}});return;
        }
        const meaningful=JSON.stringify(after.rect)!==JSON.stringify(before.rect)||JSON.stringify(after.scroll)!==JSON.stringify(before.scroll);
        respond({type:"result",id:request.id,result:{status:meaningful?"verified":"inconclusive",reason_code:meaningful?undefined:"swipe_movement_not_observed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true,meaningful_movement_observed:meaningful,before:{rect:before.rect,scroll:before.scroll},after:{rect:after.rect,scroll:after.scroll}}});return;
      }catch(error){
        respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"swipe_readback_unknown",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true,reason:safeNativeError(String(error))}});return;
      }
    }
    if (request.method === "Controlla.pointerMultiTouch") {
      const params=request.params||{},objectId=params.object_id,index=params.index,guard=params.guard_function;
      const cx=params.center_x,cy=params.center_y,startDistance=params.start_distance,endDistance=params.end_distance,viewport=params.viewport,timeoutMs=params.timeout_ms??3000;
      const expectedActivationEpoch=params.expected_activation_epoch??undefined,expectedFocusEpoch=params.expected_focus_epoch??undefined;
      if(typeof objectId!=="string"||!objectId||objectId.length>512||!Number.isSafeInteger(index)||index<0||
          typeof guard!=="string"||!guard.startsWith("function(index,cx,cy,startDistance,endDistance,expectedViewport)")||guard.length>12000||
          ![cx,cy,startDistance,endDistance].every(value=>Number.isFinite(value)&&value>=0&&value<=10000)||
          startDistance<40||startDistance>600||endDistance<20||endDistance>800||endDistance/startDistance<0.5||endDistance/startDistance>2||
          typeof viewport!=="string"||viewport.length>256||!Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("multi-touch pinch requires bounded two-finger geometry and a guarded fresh root target.");
      const target=await chrome.tabs.get(tabId),deadline=performance.now()+timeoutMs;
      if(!target.active){respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"inactive_target",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;}
      const documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      let touchStartAttempted=false,dispatchAcknowledged=false,releaseAcknowledged=false;
      const guardCall=async()=>{
        const result=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,
          arguments:[{value:index},{value:cx},{value:cy},{value:startDistance},{value:endDistance},{value:viewport}],returnByValue:true});
        return result?.result?.value;
      };
      const valid=value=>value?.ok===true&&value.viewport===viewport&&[value.rect?.left,value.rect?.top,value.rect?.width,value.rect?.height].every(Number.isFinite);
      const points=distance=>[{id:1,x:cx-distance/2,y:cy,radiusX:1,radiusY:1,force:1},{id:2,x:cx+distance/2,y:cy,radiusX:1,radiusY:1,force:1}];
      try {
        const before=await guardCall();
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)||!valid(before)) {
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:before?.reason||"pinch_preflight_refused",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        const final=await guardCall();
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)||!valid(final)||JSON.stringify(final.rect)!==JSON.stringify(before.rect)) {
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"pinch_target_changed_before_dispatch",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        touchStartAttempted=true;
        await sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchStart",touchPoints:points(startDistance)});
        await sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchMove",touchPoints:points(endDistance)});
        dispatchAcknowledged=true;
      } catch(error) {
        respond({type:"result",id:request.id,result:{status:touchStartAttempted?"unknown":"not_dispatched",reason_code:touchStartAttempted?"touch_delivery_unknown":"pinch_preflight_failed",dispatch_acknowledged:false,dispatch_may_have_occurred:touchStartAttempted,reason:safeNativeError(String(error))}});return;
      } finally {
        if(touchStartAttempted) releaseAcknowledged=await Promise.race([sendCdpCommand({tabId},"Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]}).then(()=>true).catch(()=>false),new Promise(resolve=>setTimeout(()=>resolve(false),1000))]);
      }
      if(!releaseAcknowledged){respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"touch_release_unconfirmed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true}});return;}
      if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
          (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
          (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)) {
        respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"pinch_identity_changed_after_dispatch",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true}});return;
      }
      respond({type:"result",id:request.id,result:{status:dispatchAcknowledged?"dispatched":"unknown",reason_code:dispatchAcknowledged?undefined:"pinch_delivery_unknown",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true,gesture:"two_finger_pinch",start_distance:startDistance,end_distance:endDistance}});return;
    }
    if (request.method === "Controlla.pointerScroll") {
      const params=request.params||{},objectId=params.object_id,index=params.index,guard=params.guard_function;
      const dx=params.delta_x,dy=params.delta_y,expectedViewport=params.viewport,timeoutMs=params.timeout_ms??3000;
      const expectedActivationEpoch=params.expected_activation_epoch??undefined,expectedFocusEpoch=params.expected_focus_epoch??undefined;
      if(typeof objectId!=="string"||!objectId||objectId.length>512||!Number.isSafeInteger(index)||index<0||
          typeof guard!=="string"||!guard.startsWith("function(index,dx,dy,expectedViewport)")||guard.length>12000||
          !Number.isSafeInteger(dx)||!Number.isSafeInteger(dy)||((dx===0)===(dy===0))||Math.abs(dx)>1200||Math.abs(dy)>1200||
          typeof expectedViewport!=="string"||expectedViewport.length>256||!Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("pointer scroll requires bounded one-axis deltas and a guarded fresh target.");
      const target=await chrome.tabs.get(tabId),deadline=performance.now()+timeoutMs;
      if(!target.active){respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"inactive_target",dispatch_acknowledged:false,dispatch_may_have_occurred:false,reason:"Wheel input requires the target tab to be active; no tab activation was attempted."}});return;}
      const documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      let descriptorId,finalDescriptorId,containerId,dispatchAttempted=false,dispatchAcknowledged=false;
      try{
        const guarded=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,
          arguments:[{value:index},{value:dx},{value:dy},{value:expectedViewport}],returnByValue:false});
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"guard_deadline_or_identity_changed",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        const remote=guarded?.result;
        descriptorId=remote?.objectId;
        if(!descriptorId||remote.type!=="object")throw new Error("scroll guard omitted its receipt object.");
        const properties=await sendCdpCommand({tabId},"Runtime.getProperties",{objectId:descriptorId,ownProperties:true,generatePreview:false});
        const fields=new Map((properties?.result||[]).map(property=>[property.name,property.value]));
        const scalar=name=>fields.get(name)?.value;
        containerId=fields.get("container")?.objectId;
        if(scalar("ok")!==true||scalar("viewport")!==expectedViewport||!Number.isFinite(scalar("x"))||!Number.isFinite(scalar("y"))||!containerId){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:scalar("reason")||"scroll_target_refused",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        const x=scalar("x"),y=scalar("y"),before={left:scalar("left"),top:scalar("top")};
        if(!Number.isFinite(before.left)||!Number.isFinite(before.top))throw new Error("scroll guard omitted initial container offsets.");
        const beforeRead=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId:containerId,
          functionDeclaration:"function(){return {connected:this.isConnected,left:this.scrollLeft,top:this.scrollTop}}",returnByValue:true});
        const beforeValue=beforeRead?.result?.value;
        if(!beforeValue?.connected||beforeValue.left!==before.left||beforeValue.top!==before.top){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"scroll_container_changed",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        const finalGuard=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,
          arguments:[{value:index},{value:dx},{value:dy},{value:expectedViewport},{objectId:containerId}],returnByValue:false});
        finalDescriptorId=finalGuard?.result?.objectId;
        if(!finalDescriptorId||finalGuard?.result?.type!=="object")throw new Error("final scroll guard did not retain its receipt.");
        const finalProperties=await sendCdpCommand({tabId},"Runtime.getProperties",{objectId:finalDescriptorId,ownProperties:true,generatePreview:false});
        const finalFields=new Map((finalProperties?.result||[]).map(property=>[property.name,property.value]));
        const finalScalar=name=>finalFields.get(name)?.value;
        if(finalScalar("ok")!==true||finalScalar("viewport")!==expectedViewport||finalScalar("x")!==x||finalScalar("y")!==y){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:finalScalar("reason")||"scroll_target_changed_before_dispatch",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"guard_deadline_or_identity_changed",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        dispatchAttempted=true;
        await sendCdpCommand({tabId},"Input.dispatchMouseEvent",{type:"mouseMoved",button:"none",x,y});
        await sendCdpCommand({tabId},"Input.dispatchMouseEvent",{type:"mouseWheel",x,y,deltaX:dx,deltaY:dy});
        dispatchAcknowledged=true;
        const afterRead=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId:containerId,
          functionDeclaration:"function(){return {connected:this.isConnected,left:this.scrollLeft,top:this.scrollTop}}",returnByValue:true});
        const after=afterRead?.result?.value;
        if(!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"target_identity_changed_after_scroll",dispatch_acknowledged:true,dispatch_may_have_occurred:true}});return;
        }
        if(!after?.connected||!Number.isFinite(after.left)||!Number.isFinite(after.top)){
          respond({type:"result",id:request.id,result:{status:"inconclusive",reason_code:"scroll_container_readback_unavailable",dispatch_acknowledged:true,dispatch_may_have_occurred:true,before,after:null}});return;
        }
        const receipt={same_container:true,container_connected:true,before,after:{left:after.left,top:after.top},delta_x:dx,delta_y:dy};
        const moved=dx? (dx<0?after.left<before.left:after.left>before.left):(dy<0?after.top<before.top:after.top>before.top);
        respond({type:"result",id:request.id,result:{status:moved?"verified":"inconclusive",reason_code:moved?undefined:"scroll_offset_unchanged",dispatch_acknowledged:true,dispatch_may_have_occurred:true,...receipt}});return;
      }catch(error){
        respond({type:"result",id:request.id,result:{status:dispatchAttempted?"unknown":"not_dispatched",reason_code:dispatchAttempted?"wheel_delivery_unknown":"scroll_preflight_failed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:dispatchAttempted,reason:safeNativeError(String(error))}});return;
      }finally{
        if(containerId)await sendCdpCommand({tabId},"Runtime.releaseObject",{objectId:containerId}).catch(()=>{});
        if(descriptorId)await sendCdpCommand({tabId},"Runtime.releaseObject",{objectId:descriptorId}).catch(()=>{});
        if(finalDescriptorId)await sendCdpCommand({tabId},"Runtime.releaseObject",{objectId:finalDescriptorId}).catch(()=>{});
      }
    }
    if (request.method === "Controlla.scrollToTarget") {
      const params=request.params||{},objectId=params.object_id,index=params.index,guard=params.guard_function;
      const timeoutMs=params.timeout_ms??3000,expectedActivationEpoch=params.expected_activation_epoch,expectedFocusEpoch=params.expected_focus_epoch;
      if(typeof objectId!=="string"||!objectId||objectId.length>512||!Number.isSafeInteger(index)||index<0||
          typeof guard!=="string"||!guard.startsWith("function(index,scrollRequest)")||guard.length>16000||
          params.scroll_request!=="scroll-to-target-v1"||!Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("scroll-to-target requires a bounded semantic guard and current target.");
      const deadline=performance.now()+timeoutMs;
      const documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      let evaluationAttempted=false,evaluationAcknowledged=false;
      try{
        if(!documentId||!isAuthorized(tabId)||(nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"target_identity_changed_before_scroll",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        evaluationAttempted=true;
        const response=await sendCdpCommand({tabId},"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,
          arguments:[{value:index},{value:"scroll-to-target-v1"}],returnByValue:true});
        evaluationAcknowledged=true;
        const receipt=response?.result?.value;
        if(!receipt||typeof receipt!=="object")throw new Error("scroll-to-target guard returned no outcome evidence.");
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch)){
          respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"target_identity_changed_after_scroll",dispatch_acknowledged:evaluationAcknowledged,dispatch_may_have_occurred:receipt.dispatch_may_have_occurred!==false,postcondition:receipt}});return;
        }
        if(receipt.phase==="before"&&receipt.dispatch_may_have_occurred===false){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:receipt.reason||"scroll_target_refused",dispatch_acknowledged:false,dispatch_may_have_occurred:false,postcondition:receipt}});return;
        }
        if(receipt.ok===true&&receipt.target_visible===true&&receipt.center_hit===true){
          respond({type:"result",id:request.id,result:{status:"verified",reason_code:undefined,dispatch_acknowledged:true,dispatch_may_have_occurred:true,...receipt}});return;
        }
        respond({type:"result",id:request.id,result:{status:"inconclusive",reason_code:receipt.reason||"target_visibility_not_verified",dispatch_acknowledged:true,dispatch_may_have_occurred:true,postcondition:receipt}});return;
      }catch(error){
        respond({type:"result",id:request.id,result:{status:evaluationAttempted?"unknown":"not_dispatched",reason_code:evaluationAttempted?"scroll_to_target_delivery_unknown":"scroll_to_target_preflight_failed",dispatch_acknowledged:evaluationAcknowledged,dispatch_may_have_occurred:evaluationAttempted,reason:safeNativeError(String(error))}});return;
      }
    }
    if (request.method === "Controlla.frameScroll") {
      const p=request.params||{},frameId=p.frame_id,loaderId=p.loader_id,backendNodeId=p.backend_node_id;
      const dx=p.delta_x,dy=p.delta_y,timeoutMs=p.timeout_ms??3000;
      const expectedActivationEpoch=p.expected_activation_epoch,expectedFocusEpoch=p.expected_focus_epoch;
      if(typeof frameId!=="string"||!frameId||frameId.length>256||typeof loaderId!=="string"||!loaderId||loaderId.length>256||
          !Number.isSafeInteger(backendNodeId)||backendNodeId<=0||!Number.isSafeInteger(dx)||!Number.isSafeInteger(dy)||
          ((dx===0)===(dy===0))||Math.abs(dx)>1200||Math.abs(dy)>1200||!Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("frame scroll requires bounded frame identity, one-axis wheel deltas, and timeout.");
      const root={tabId},deadline=performance.now()+timeoutMs,documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      let containerId,descriptorId,finalDescriptorId,frameRoute={tabId},wheelAttempted=false,wheelAcknowledged=false;
      const currentFrame=async()=>{
        const tree=await sendCdpCommand(root,"Page.getFrameTree",{});
        const visit=(node,ancestors=[])=>node?.frame?.id===frameId?{frame:node.frame,depth:ancestors.length,chain:[...ancestors,node.frame]}:(node?.childFrames||[]).map(child=>visit(child,[...ancestors,node.frame])).find(Boolean);
        return visit(tree.frameTree,[]);
      };
      const validFrameChain=info=>!!info?.chain&&info.depth>=1&&info.depth<=8&&info.chain.every(frame=>frame?.id&&frame?.loaderId);
      const identityStillOwned=async()=>{
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||
            (expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch))return false;
        const tab=await chrome.tabs.get(tabId);
        return tab.active===true;
      };
      try{
        if(!documentId||!(await identityStillOwned())){
          respond({type:"result",id:request.id,result:{status:"not_dispatched",reason_code:"inactive_target_or_identity_changed",dispatch_acknowledged:false,dispatch_may_have_occurred:false}});return;
        }
        await sendCdpCommand(root,"Target.setAutoAttach",{autoAttach:true,waitForDebuggerOnStart:false,flatten:true,filter:[{type:"iframe",exclude:false}]});
        try{
          const info=await currentFrame();
          if(!info||info.frame.loaderId!==loaderId||!validFrameChain(info))throw new Error("Frame ancestry is stale, too deep, or unavailable.");
          const expectedChain=info.chain.map(frame=>({id:frame.id,loaderId:frame.loaderId}));
          const frameSessionId=await waitForFrameSession(tabId,frameId,400);
          frameRoute=frameSessionId?{tabId,sessionId:frameSessionId}:root;
          const ax=await sendCdpCommand(frameRoute,"Accessibility.getFullAXTree",frameSessionId?{}:{frameId});
          if(!ax.nodes?.some(node=>node.backendDOMNodeId===backendNodeId))throw new Error("Frame scroll reference is stale or unavailable in this same-process frame.");
          const resolved=await sendCdpCommand(frameRoute,"DOM.resolveNode",{backendNodeId});
          const objectId=resolved?.object?.objectId;
          if(!objectId)throw new Error("Frame scroll target could not be resolved.");
          const guard="function(dx,dy,expectedViewport,expectedContainer){const e=this,d=e?.ownerDocument,w=d?.defaultView;if(!e||!e.isConnected||!d||!w)return {ok:false,reason:'stale_reference'};const b=e.getBoundingClientRect(),s=w.getComputedStyle(e),vw=w.innerWidth,vh=w.innerHeight;if(s.display==='none'||s.visibility==='hidden'||Number.parseFloat(s.opacity||'1')<=0||b.width<=0||b.height<=0)return {ok:false,reason:'not_visible'};const x=Math.max(0,Math.min(vw-1,b.left+b.width/2)),y=Math.max(0,Math.min(vh-1,b.top+b.height/2)),hit=d.elementFromPoint(x,y);if(!hit||!(hit===e||e.contains(hit))||hit.getRootNode()!==d||hit instanceof HTMLIFrameElement)return {ok:false,reason:'obscured_or_nested_document'};const viewport=`${vw}x${vh}@${w.devicePixelRatio||1}`;if(expectedViewport&&viewport!==expectedViewport)return {ok:false,reason:'viewport_changed'};const axis=dy?'y':'x',scrollable=n=>{if(!(n instanceof Element))return false;const cs=w.getComputedStyle(n),overflow=axis==='y'?cs.overflowY:cs.overflowX;return ['auto','scroll','overlay'].includes(overflow)&&(axis==='y'?n.scrollHeight-n.clientHeight:n.scrollWidth-n.clientWidth)>1};let container=null;for(let n=hit;n&&n instanceof Element;n=n.parentElement){if(scrollable(n)){container=n;break}}if(!container&&scrollable(d.scrollingElement))container=d.scrollingElement;if(!container)return {ok:false,reason:'no_scrollable_ancestor'};if(expectedContainer&&container!==expectedContainer)return {ok:false,reason:'scroll_container_changed'};const left=container.scrollLeft,top=container.scrollTop,maxLeft=Math.max(0,container.scrollWidth-container.clientWidth),maxTop=Math.max(0,container.scrollHeight-container.clientHeight),pos=dy?top:left,max=dy?maxTop:maxLeft;if(dy<0?pos<=1:dy>0?pos>=max-1:dx<0?pos<=1:pos>=max-1)return {ok:false,reason:'nearest_scroll_container_at_boundary'};return {ok:true,x,y,width:vw,height:vh,viewport,container,left,top}}";
          const readGuard=async(expectedContainer)=>{
            const result=await sendCdpCommand(frameRoute,"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,arguments:[{value:dx},{value:dy},{value:expectedContainer?.viewport??null},...(expectedContainer?[{objectId:expectedContainer.containerId}]:[])],returnByValue:false});
            const id=result?.result?.objectId;if(!id||result.result.type!=="object")throw new Error("Frame scroll guard returned no retained state.");
            const props=await sendCdpCommand(frameRoute,"Runtime.getProperties",{objectId:id,ownProperties:true,generatePreview:false});
            const fields=new Map((props?.result||[]).map(item=>[item.name,item.value])),scalar=name=>fields.get(name)?.value;
            return {id,fields,scalar,containerId:fields.get("container")?.objectId};
          };
          const first=await readGuard();descriptorId=first.id;containerId=first.containerId;
          const state=first.scalar;
          if(state("ok")!==true||!containerId||!Number.isFinite(state("x"))||!Number.isFinite(state("y"))||!Number.isFinite(state("width"))||!Number.isFinite(state("height")))throw new Error(state("reason")||`Frame scroll target was refused (ok=${state("ok")}, container=${!!containerId}, point=${state("x")},${state("y")}, viewport=${state("width")}x${state("height")}).`);
          const viewport=state("viewport"),before={left:state("left"),top:state("top")};
          const beforeRead=await sendCdpCommand(frameRoute,"Runtime.callFunctionOn",{objectId:containerId,functionDeclaration:"function(){return {connected:this.isConnected,left:this.scrollLeft,top:this.scrollTop}}",returnByValue:true});
          if(!beforeRead.result?.value?.connected||beforeRead.result.value.left!==before.left||beforeRead.result.value.top!==before.top)throw new Error("Frame scroll container changed before dispatch.");
          const initialPoint=await rootPointForFrame(tabId,frameId,objectId,state("x"),state("y"),{width:state("width"),height:state("height")});
          const final=await readGuard({viewport,containerId});finalDescriptorId=final.id;
          if(final.scalar("ok")!==true||final.scalar("x")!==state("x")||final.scalar("y")!==state("y")||final.containerId!==containerId)throw new Error("Frame scroll target or nearest container changed before dispatch.");
          let finalInfo=await currentFrame();
          const sameChain=info=>info?.chain?.length===expectedChain.length&&info.chain.every((frame,index)=>frame.id===expectedChain[index].id&&frame.loaderId===expectedChain[index].loaderId);
          if(!sameChain(finalInfo)||frameSessions.get(tabId)?.get(frameId)!==(frameSessionId||undefined))throw new Error("Frame ancestry or owner session changed before wheel dispatch.");
          finalInfo=await currentFrame();
          if(!sameChain(finalInfo))throw new Error("Frame or loader identity changed during final scroll preflight.");
          const finalPoint=await rootPointForFrame(tabId,frameId,objectId,final.scalar("x"),final.scalar("y"),{width:final.scalar("width"),height:final.scalar("height")});
          const dispatchInfo=await currentFrame();
          if(Math.abs(finalPoint.x-initialPoint.x)>0.5||Math.abs(finalPoint.y-initialPoint.y)>0.5||!sameChain(dispatchInfo)||frameSessions.get(tabId)?.get(frameId)!==(frameSessionId||undefined)||!(await identityStillOwned()))throw new Error("Frame transform, ancestry, document, or activation changed before wheel dispatch.");
          await sendCdpCommand(root,"Input.dispatchMouseEvent",{type:"mouseMoved",button:"none",...finalPoint});
          wheelAttempted=true;
          await sendCdpCommand(root,"Input.dispatchMouseEvent",{type:"mouseWheel",...finalPoint,deltaX:dx,deltaY:dy});
          wheelAcknowledged=true;
          const afterRead=await sendCdpCommand(frameRoute,"Runtime.callFunctionOn",{objectId:containerId,functionDeclaration:"function(){return {connected:this.isConnected,left:this.scrollLeft,top:this.scrollTop}}",returnByValue:true});
          const after=afterRead.result?.value;
          if(!(await identityStillOwned())||(await currentFrame())?.frame?.loaderId!==loaderId){respond({type:"result",id:request.id,result:{status:"unknown",reason_code:"frame_scroll_identity_changed_after_dispatch",dispatch_acknowledged:true,dispatch_may_have_occurred:true}});return;}
          if(!after?.connected||!Number.isFinite(after.left)||!Number.isFinite(after.top)){respond({type:"result",id:request.id,result:{status:"inconclusive",reason_code:"frame_scroll_readback_unavailable",dispatch_acknowledged:true,dispatch_may_have_occurred:true,before,after:null}});return;}
          const moved=dx?(dx<0?after.left<before.left:after.left>before.left):(dy<0?after.top<before.top:after.top>before.top);
          respond({type:"result",id:request.id,result:{status:moved?"verified":"inconclusive",reason_code:moved?undefined:"scroll_offset_unchanged",dispatch_acknowledged:true,dispatch_may_have_occurred:true,frame_id:frameId,frame_loader_id:loaderId,root_point:finalPoint,same_container:true,before,after:{left:after.left,top:after.top},delta_x:dx,delta_y:dy}});return;
        }finally{
          if(containerId)await sendCdpCommand(frameRoute||root,"Runtime.releaseObject",{objectId:containerId}).catch(()=>{});
          if(descriptorId)await sendCdpCommand(frameRoute||root,"Runtime.releaseObject",{objectId:descriptorId}).catch(()=>{});
          if(finalDescriptorId)await sendCdpCommand(frameRoute||root,"Runtime.releaseObject",{objectId:finalDescriptorId}).catch(()=>{});
          await sendCdpCommand(root,"Target.setAutoAttach",{autoAttach:false,waitForDebuggerOnStart:false,flatten:true}).catch(()=>{});
        }
      }catch(error){
        respond({type:"result",id:request.id,result:{status:wheelAttempted?"unknown":"not_dispatched",reason_code:wheelAttempted?"wheel_delivery_unknown":"frame_scroll_preflight_rejected",dispatch_acknowledged:wheelAcknowledged,dispatch_may_have_occurred:wheelAttempted,reason:safeNativeError(String(error))}});return;
      }
    }
    if (request.method === "Controlla.securePasswordFill") {
      const frameId = request.params?.frame_id, loaderId = request.params?.loader_id, backendNodeId = request.params?.backend_node_id;
      const label = typeof request.params?.label === "string" ? request.params.label.slice(0, 120) : "Password";
      const timeoutMs = request.params?.timeout_ms ?? 20000;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256 || typeof loaderId !== "string" || !loaderId || loaderId.length > 256 || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0 || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 22000) throw new Error("secure password fill requires bounded frame identity and timeout.");
      const target = await chrome.tabs.get(tabId), window = await chrome.windows.get(target.windowId);
      if (!target.active || !window.focused) {
        respond({ type: "result", id: request.id, result: { status: "not_dispatched", dispatch_may_have_occurred: false, reason_code: "secure_input_requires_active_target", reason: "Secure password entry requires the target tab to be active in the focused Chrome window.", observed: { target_active: !!target.active, window_focused: !!window.focused } } }); return;
      }
      const root = { tabId }, frameTree = await sendCdpCommand(root, "Page.getFrameTree", {});
      const findFrame = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(findFrame).find(Boolean);
      const frame = findFrame(frameTree.frameTree);
      if (!frame || frame.loaderId !== loaderId) throw new Error("Password field frame changed since its fresh snapshot.");
      const originForFrame = candidate => {
        try {
          const parsed = new URL(candidate?.securityOrigin);
          return /^https?:$/.test(parsed.protocol) && parsed.origin !== "null" ? parsed.origin : undefined;
        } catch { return undefined; }
      };
      const origin = originForFrame(frame);
      if (!origin) throw new Error("Password field has no trustworthy HTTP(S) frame origin.");
      await sendCdpCommand(root, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      let promptTab, promptActivationEpoch;
      const capturePromptActivation = info => {
        if (promptTab?.id === info.tabId && promptActivationEpoch === undefined) promptActivationEpoch = activationEpoch;
      };
      chrome.tabs.onActivated.addListener(capturePromptActivation);
      const ownsPromptActivation = async () => {
        if (!promptTab?.id || promptActivationEpoch === undefined) return false;
        const observedEpoch = activationEpoch;
        if (observedEpoch !== promptActivationEpoch) return false;
        const active = await chrome.tabs.query({ active: true, windowId: target.windowId });
        return activationEpoch === observedEpoch && active[0]?.id === promptTab.id;
      };
      const route = () => { const sessionId = frameSessions.get(tabId)?.get(frameId); return sessionId ? { tabId, sessionId } : root; };
      const checkPassword = async () => {
        if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || !isAuthorized(tabId)) throw new Error("Target document or Controlla lease changed during secure entry.");
        const current = await sendCdpCommand(root, "Page.getFrameTree", {});
        const currentFrame = findFrame(current.frameTree);
        if (currentFrame?.loaderId !== loaderId || originForFrame(currentFrame) !== origin) throw new Error("Password field origin or frame identity changed during secure entry.");
        const resolved = await sendCdpCommand(route(), "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Password field is no longer available.");
        const result = await sendCdpCommand(route(), "Runtime.callFunctionOn", { objectId,
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
          promptActivationEpoch ??= activationEpoch;
          submitted = await input;
          if (!(await ownsPromptActivation())) throw new Error("Active tab changed during secure entry; password was not dispatched.");
        }
        if (!submitted || typeof submitted.value !== "string" || submitted.value.length < 1 || submitted.value.length > 4096) throw new Error("Secure password entry was cancelled or exceeded its bound.");
        let password = submitted.value;
        let passwordDispatchStarted = false;
        try {
          const latest = await checkPassword();
          if (latest.objectId !== field.objectId) throw new Error("Password field identity changed during secure entry.");
          if (hasChatPassword) {
            const currentTarget = await chrome.tabs.get(tabId), currentWindow = await chrome.windows.get(currentTarget.windowId);
            if (!currentTarget.active || !currentWindow.focused) throw new Error("Target lost foreground ownership before password dispatch.");
          } else if (!(await ownsPromptActivation())) {
            throw new Error("Active tab changed during password field revalidation; password was not dispatched.");
          }
          passwordDispatchStarted = true;
          const result = await sendCdpCommand(latest.route, "Runtime.callFunctionOn", { objectId: latest.objectId,
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
        chrome.tabs.onActivated.removeListener(capturePromptActivation);
        await sendCdpCommand(root, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
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
    if (request.method === "Controlla.downloadLifecycle") {
      const params = request.params || {};
      const action = params.action;
      if (action === "arm") {
        if ((params.expected_document_id !== undefined && params.expected_document_id !== expectedDocumentId) || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("Download observer requires the current paired document identity.");
        if (!validDownloadUrl(params.expected_url)) throw new Error("Download observer requires an exact credential-free HTTP(S) or HTTP(S)-blob URL.");
        const perTab = [...downloadObservers.values()].filter(observer => observer.tabId === tabId);
        if (perTab.length >= MAX_DOWNLOAD_OBSERVERS_PER_TAB || downloadObservers.size >= MAX_DOWNLOAD_OBSERVERS) throw new Error("Download observer capacity is full.");
        if (perTab.some(observer => observer.expectedUrl === params.expected_url)) throw new Error("A download observer for this URL is already active on the tab.");
        const observerId = crypto.randomUUID();
        const observer = { id: observerId, tabId, documentId: expectedDocumentId, expectedUrl: params.expected_url,
          downloadId: undefined, filename: null, bytesReceived: 0, totalBytes: null, urlIdentity: null,
          events: [], waiter: undefined, item: undefined, expiryTimer: undefined };
        downloadObservers.set(observerId, observer);
        refreshDownloadObserverExpiry(observer);
        respond({ type: "result", id: request.id, result: { status: "armed", observer_id: observerId, document_id: expectedDocumentId } }); return;
      }
      if (typeof params.observer_id !== "string" || params.observer_id.length > 64) throw new Error("Download lifecycle action requires a bounded observer_id.");
      const observer = downloadObservers.get(params.observer_id);
      if (!observer || observer.tabId !== tabId || observer.documentId !== expectedDocumentId || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("Download observer is stale or belongs to another target document.");
      if (action === "cancel") {
        let cancelAcknowledged = false;
        if (Number.isSafeInteger(observer.downloadId)) {
          try { await chrome.downloads.cancel(observer.downloadId); cancelAcknowledged = true; } catch {}
        }
        clearDownloadObserver(observer.id, { status: "interrupted", reason_code: "download_observer_cancelled",
          filename: observer.filename, bytes_received: observer.bytesReceived, url_identity: observer.urlIdentity });
        respond({ type: "result", id: request.id, result: { status: "observer_cancelled", download_cancel_acknowledged: cancelAcknowledged } }); return;
      }
      if (action !== "wait") throw new Error("Download lifecycle action must be arm, wait, or cancel.");
      const timeout = params.timeout_ms;
      if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30000) throw new Error("Download lifecycle timeout_ms must be 1..30000.");
      const event = await nextDownloadLifecycleEvent(observer, timeout);
      if (!isAuthorized(tabId) || !routeActive(nativeRoute, routeGeneration) || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) {
        clearDownloadObserver(observer.id, { status: "interrupted", reason_code: "target_identity_changed" });
        throw new Error("Download lifecycle event belongs to a detached or stale target.");
      }
      if (event.status === "timed_out" || event.status === "completed" || event.status === "interrupted") clearDownloadObserver(observer.id);
      else refreshDownloadObserverExpiry(observer);
      respond({ type: "result", id: request.id, result: event }); return;
    }
    if (request.method === "Controlla.checkIdentityMarker") {
      const { selector, expected } = request.params || {};
      if (typeof selector !== "string" || !selector || selector.length > 512 || typeof expected !== "string" || !expected || expected.length > 512) throw new Error("Identity marker requires bounded selector and expected text.");
      const direct = await sendCdpCommand({ tabId }, "Runtime.evaluate", { expression: `(()=>{const norm=s=>String(s||'').replace(/\\s+/g,' ').trim(),visible=[];for(const e of document.querySelectorAll(${JSON.stringify(selector)})){if(!e.isConnected||e.getClientRects().length===0||e.getAttribute('aria-selected')==='true'||e.getAttribute('aria-checked')==='true'||e.closest('[role="menu"],[role="listbox"],[role="option"],[role="menuitem"],[role="dialog"],[aria-modal="true"]'))continue;let shown=true;for(let n=e;n;n=n.parentElement){const s=getComputedStyle(n);if(s.display==='none'||s.visibility!=='visible'||Number.parseFloat(s.opacity||'1')<=0||n.hasAttribute('hidden')||n.hasAttribute('inert')){shown=false;break}}if(shown)visible.push(e);if(visible.length===2)break}const e=visible[0];const labelled=(e?.getAttribute('aria-labelledby')||'').split(/\\s+/).filter(Boolean).map(id=>document.getElementById(id)?.innerText||'').join(' ').trim();const name=labelled||e?.getAttribute('aria-label')||e?.innerText||e?.getAttribute('alt')||Array.from(e?.labels||[]).map(label=>label.innerText).join(' ')||e?.getAttribute('title')||e?.getAttribute('content')||'';return visible.length===1&&norm(name)===norm(${JSON.stringify(expected)})})()`, returnByValue: true });
      if (direct.result?.value === true) {
        const currentFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
        if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || currentFrame?.documentId !== expectedDocumentId) throw new Error("Identity marker document changed during the root DOM check.");
        respond({ type: "result", id: request.id, result: { matches: true, match_count: 1, route: "root_dom" } }); return;
      }
      respond({ type: "result", id: request.id, result: { matches: false, match_count: 0, route: "none" } });
      return;
    }
    if (request.method === "Controlla.fileInputSnapshot") {
      const tree = await sendCdpCommand({ tabId }, "Page.getFrameTree", {});
      const inputs = await fileInputsInFrames(tabId, tree.frameTree);
      const current = await sendCdpCommand({ tabId }, "Page.getFrameTree", {});
      if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || current.frameTree?.frame?.id !== tree.frameTree?.frame?.id || current.frameTree?.frame?.loaderId !== tree.frameTree?.frame?.loaderId) throw new Error("Upload target document changed during file-input baseline discovery.");
      respond({ type: "result", id: request.id, result: { inputs, document_id: expectedDocumentId } }); return;
    }
    if (request.method === "Controlla.findFileInput") {
      const baseline = request.params?.baseline;
      if (!Array.isArray(baseline) || baseline.length > 256 || baseline.some(item => typeof item?.frame_id !== "string" || typeof item?.frame_loader_id !== "string" || !Number.isSafeInteger(item?.backend_node_id))) throw new Error("Lazy-input fallback requires a bounded pre-trigger file-input baseline.");
      const tree = await sendCdpCommand({ tabId }, "Page.getFrameTree", {});
      const all = await fileInputsInFrames(tabId, tree.frameTree);
      const existed = new Set(baseline.map(item => `${item.frame_id}:${item.frame_loader_id}:${item.backend_node_id}`));
      const candidates = all.filter(item => !existed.has(`${item.frame_id}:${item.frame_loader_id}:${item.backend_node_id}`));
      if (candidates.length === 0) throw new Error("No new file input appeared after the upload trigger.");
      if (candidates.length !== 1) throw new Error("Multiple new file inputs appeared after the upload trigger; refusing ambiguous selection.");
      const selected = candidates[0];
      const current = await sendCdpCommand({ tabId }, "Page.getFrameTree", {});
      const currentFrame = (() => { const find = node => node?.frame?.id === selected.frame_id ? node.frame : (node?.childFrames || []).map(find).find(Boolean); return find(current.frameTree); })();
      if (pairedTabDocumentIds.get(tabId) !== expectedDocumentId || current.frameTree?.frame?.id !== tree.frameTree?.frame?.id || current.frameTree?.frame?.loaderId !== tree.frameTree?.frame?.loaderId || currentFrame?.loaderId !== selected.frame_loader_id) throw new Error("Upload target document changed during lazy file-input discovery.");
      respond({ type: "result", id: request.id, result: { ...selected, document_id: expectedDocumentId } }); return;
    }
    if (request.method === "Controlla.setFrameFileInputFiles") {
      if (!nativeRoute || !nativeLease?.tabs.has(tabId)) throw new Error("File selection requires an exact native lease for the target tab.");
      const { frame_id: frameId, frame_loader_id: loaderId, backend_node_id: backendNodeId } = request.params || {};
      const filePaths = request.params?.file_paths ?? [request.params?.file_path];
      if (typeof frameId !== "string" || !frameId || typeof loaderId !== "string" || !loaderId || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0 || !Array.isArray(filePaths) || filePaths.length < 1 || filePaths.length > 5 || filePaths.some(path => typeof path !== "string" || !path || path.length > 4096)) throw new Error("Frame upload target or bounded staged paths are invalid.");
      const readTree = () => sendCdpCommand({ tabId }, "Page.getFrameTree", {});
      const find = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(find).find(Boolean);
      if (find((await readTree()).frameTree)?.loaderId !== loaderId) throw new Error("Upload frame navigated after the chooser opened.");
      const childSessionId = frameSessions.get(tabId)?.get(frameId);
      const route = childSessionId ? { tabId, sessionId: childSessionId } : { tabId };
      const resolved = await sendCdpCommand(route, "DOM.resolveNode", { backendNodeId });
      if (!resolved.object?.objectId) throw new Error("Chooser input could not be resolved for accept-filter validation.");
      const policy = await sendCdpCommand(route, "Runtime.callFunctionOn", { objectId: resolved.object.objectId, functionDeclaration: uploadInputPolicyFunction(), arguments: [{ value: filePaths.map(path => path.split(/[\\/]/).at(-1)) }], returnByValue: true });
      if (policy.result?.value?.ok !== true) throw new Error(`File selection refused before dispatch: ${policy.result?.value?.reason || "accept_filter_unverified"}.`);
      await sendCdpCommand(route, "DOM.setFileInputFiles", { files: filePaths, backendNodeId });
      const objectId = resolved.object?.objectId;
      if (!objectId) throw new Error("Chooser input could not be resolved for file readback.");
      const readback = await sendCdpCommand(route, "Runtime.callFunctionOn", { objectId, functionDeclaration: "function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}))}", returnByValue: true });
      if (find((await readTree()).frameTree)?.loaderId !== loaderId) throw new Error("Upload frame navigated during file selection.");
      respond({ type: "result", id: request.id, result: { frame_id: frameId, frame_loader_id: loaderId, files: readback.result?.value, input_policy: policy.result.value } }); return;
    }
    if (request.method === "Controlla.frameSelectFile") {
      if (!nativeRoute || !nativeLease?.tabs.has(tabId)) throw new Error("File selection requires an exact native lease for the target tab.");
      const { frame_id: frameId, frame_loader_id: loaderId, trigger_backend_node_id: triggerId } = request.params || {};
      const filePaths = request.params?.file_paths ?? [request.params?.file_path];
      if (typeof frameId !== "string" || !frameId || typeof loaderId !== "string" || !loaderId || !Number.isSafeInteger(triggerId) || triggerId <= 0 || !Array.isArray(filePaths) || filePaths.length < 1 || filePaths.length > 5 || filePaths.some(path => typeof path !== "string" || !path || path.length > 4096)) throw new Error("Frame upload trigger or bounded staged paths are invalid.");
      const root = { tabId };
      const readFrame = async () => {
        const tree = await sendCdpCommand(root, "Page.getFrameTree", {});
        const find = node => node?.frame?.id === frameId ? node.frame : (node?.childFrames || []).map(find).find(Boolean);
        return find(tree.frameTree);
      };
      if ((await readFrame())?.loaderId !== loaderId) throw new Error("Upload frame changed before selection.");
      if (fileChooserOperations.has(tabId) || fileChooserWaiters.has(tabId)) throw new Error("Another file chooser operation is active for this tab.");
      const operation = { frameId, pressing: false, conflicted: false, networkRequests: new Map(), networkOutcomes: new Map(), networkEvents: [] };
      fileChooserOperations.set(tabId, operation);
      let route, interceptionEnabled = false, networkEnabled = false, pressAttempted = false, dispatched = false, selectionResult;
      try {
        await sendCdpCommand(root, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
        const sessionId = frameSessions.get(tabId)?.get(frameId) || await waitForFrameSession(tabId, frameId, 400);
        route = sessionId ? { tabId, sessionId } : root;
        operation.networkSessionId = sessionId || null;
        await sendCdpCommand(route, "Network.enable", {});
        networkEnabled = true;
        const ax = await sendCdpCommand(route, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const trigger = ax.nodes?.find(node => node.backendDOMNodeId === triggerId);
        if (!trigger || !["button", "link", "option", "menuitem"].includes(trigger.role?.value)) throw new Error("Frame upload trigger is stale or is not actionable.");
        const resolvedTrigger = await sendCdpCommand(route, "DOM.resolveNode", { backendNodeId: triggerId });
        if (!resolvedTrigger.object?.objectId) throw new Error("Frame upload trigger could not be resolved.");
        const localPoint = await frameActionablePoint(route, resolvedTrigger.object.objectId);
        const box = await sendCdpCommand(route, "DOM.getBoxModel", { backendNodeId: triggerId });
        const quad = box.model?.content;
        if (!Array.isArray(quad) || quad.length !== 8 || (await readFrame())?.loaderId !== loaderId) throw new Error("Frame upload trigger changed before dispatch.");
        clearFileChooserState(tabId);
        await sendCdpCommand(route, "Page.enable", {});
        await sendCdpCommand(route, "Page.setInterceptFileChooserDialog", { enabled: true });
        interceptionEnabled = true;
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4, y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
        ({ x, y } = await rootPointForFrame(tabId, frameId, resolvedTrigger.object.objectId, localPoint.x, localPoint.y, localPoint.viewport));
        if ((await readFrame())?.loaderId !== loaderId) throw new Error("Upload frame changed before dispatch.");
        if (operation.conflicted) throw new Error("Another file chooser opened before the upload trigger was pressed.");
        // shortcut: CDP does not identify a chooser's initiating click; add causal page instrumentation if same-frame human interaction must be distinguished during dispatch.
        operation.eventCursor = fileChooserEventSequence;
        operation.pressing = true;
        await sendCdpCommand(root, "Input.dispatchMouseEvent", { type: "mouseMoved", button: "none", x, y });
        pressAttempted = true;
        await sendCdpCommand(root, "Input.dispatchMouseEvent", { type: "mousePressed", button: "left", clickCount: 1, x, y });
        await sendCdpCommand(root, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y });
        dispatched = true;
        const chooser = await waitForFileChooser(tabId, expectedDocumentId, 4000, operation.eventCursor);
        if (operation.conflicted || chooser.frame_id !== frameId || chooser.frame_loader_id !== loaderId || (await readFrame())?.loaderId !== loaderId || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("File chooser belongs to another frame, document, or overlapping operation.");
        const chooserNode = await sendCdpCommand(route, "DOM.resolveNode", { backendNodeId: chooser.backend_node_id });
        if (!chooserNode.object?.objectId) throw new Error("Chooser input could not be resolved for accept-filter validation.");
        const policy = await sendCdpCommand(route, "Runtime.callFunctionOn", { objectId: chooserNode.object.objectId, functionDeclaration: uploadInputPolicyFunction(), arguments: [{ value: filePaths.map(path => path.split(/[\\/]/).at(-1)) }], returnByValue: true });
        if (policy.result?.value?.ok !== true) throw new Error(`File selection refused before dispatch: ${policy.result?.value?.reason || "accept_filter_unverified"}.`);
        await sendCdpCommand(route, "DOM.setFileInputFiles", { files: filePaths, backendNodeId: chooser.backend_node_id });
        const readback = await sendCdpCommand(route, "Runtime.callFunctionOn", { objectId: chooserNode.object.objectId, functionDeclaration: "function(){return Array.from(this.files||[]).map(f=>({name:f.name,size:f.size}))}", returnByValue: true });
        if ((await readFrame())?.loaderId !== loaderId || pairedTabDocumentIds.get(tabId) !== expectedDocumentId) throw new Error("Upload frame or root document changed during file selection.");
        if (!Array.isArray(readback.result?.value) || readback.result.value.length !== filePaths.length || readback.result.value.some(file => !file?.name)) throw new Error("File input did not retain the complete selected file set for readback.");
        selectionResult = { status: "selected", files: readback.result?.value, input_policy: policy.result.value, frame_id: frameId, frame_loader_id: loaderId, document_id: expectedDocumentId, dispatch_acknowledged: true };
        if ([...operation.networkOutcomes.values()].some(value => value === "pending")) {
          const networkObservation = await waitForUploadNetwork(operation, 1500);
          if (networkObservation.status === "stalled") {
            selectionResult = { ...selectionResult, status: "unknown", reason_code: "upload_network_stalled", network_observation: networkObservation };
          } else selectionResult.network_observation = networkObservation;
        }
      } catch (error) {
        selectionResult = { status: pressAttempted ? "unknown" : "not_dispatched", dispatch_acknowledged: dispatched, reason: String(error) };
      } finally {
        if (networkEnabled && selectionResult?.reason?.includes("File input did not retain")) {
          selectionResult.network_observation = await waitForUploadNetwork(operation);
        }
        if (networkEnabled) await sendCdpCommand(route, "Network.disable", {}).catch(() => {});
        if (selectionResult) selectionResult.network_events = operation.networkEvents;
        if (fileChooserOperations.get(tabId) === operation) fileChooserOperations.delete(tabId);
        if (interceptionEnabled) await sendCdpCommand(route, "Page.setInterceptFileChooserDialog", { enabled: false }).catch(() => {});
        await sendCdpCommand(root, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
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
        const tree = await sendCdpCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.loaderId : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      const frameUrl = async () => {
        const tree = await sendCdpCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.url : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      try {
        if (await frameLoader() !== loaderId) throw new Error("Frame document changed since the accessibility snapshot.");
        let sessionId = frameSessions.get(tabId)?.get(frameId);
        if (!sessionId) sessionId = await waitForFrameSession(tabId, frameId, 400);
        const child = sessionId ? { tabId, sessionId } : route;
        const beforeUrl = await frameUrl();
        const ax = await sendCdpCommand(child, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const node = ax.nodes?.find(item => item.backendDOMNodeId === backendNodeId);
        if (!node || !["button", "link", "checkbox", "radio", "option", "menuitem", "tab", "switch"].includes(node.role?.value)) throw new Error("Frame control is stale or is not an actionable control.");
        const box = await sendCdpCommand(child, "DOM.getBoxModel", { backendNodeId });
        const quad = box.model?.content;
        if (!Array.isArray(quad) || quad.length !== 8 || (sessionId && frameSessions.get(tabId)?.get(frameId) !== sessionId) || await frameLoader() !== loaderId) throw new Error("Frame control changed before dispatch.");
        const resolved = await sendCdpCommand(child, "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Frame click target could not be resolved for outcome verification.");
        const localPoint = await frameActionablePoint(child, objectId);
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4, y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
        const initialMapping = await rootPointForFrame(tabId, frameId, objectId, localPoint.x, localPoint.y, localPoint.viewport);
        ({ x, y } = initialMapping);
        if (await frameLoader() !== loaderId) throw new Error("Frame control changed before dispatch.");
        const dispatchPoint = await rootPointForFrame(tabId, frameId, objectId, localPoint.x, localPoint.y, localPoint.viewport);
        if (Math.abs(dispatchPoint.x - x) > 0.5 || Math.abs(dispatchPoint.y - y) > 0.5 || JSON.stringify(dispatchPoint.owner_signature) !== JSON.stringify(initialMapping.owner_signature) || await frameLoader() !== loaderId) throw new Error("Frame owner geometry changed before dispatch.");
        let pressAttempted = false;
        try {
          if (inputMode === "mouse") await sendCdpCommand(route, "Input.dispatchMouseEvent", { type: "mouseMoved", button: "none", x, y });
          pressAttempted = true;
          if (inputMode === "touch") {
            await sendCdpCommand(route, "Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x, y, radiusX: 1, radiusY: 1, force: 1 }] });
            await sendCdpCommand(route, "Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
          } else {
            await sendCdpCommand(route, "Input.dispatchMouseEvent", { type: "mousePressed", button: "left", clickCount: 1, x, y });
            await sendCdpCommand(route, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y });
          }
        } catch (error) {
          if (pressAttempted && inputMode === "touch") await sendCdpCommand(route, "Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] }).catch(() => {});
          if (pressAttempted && inputMode === "mouse") await sendCdpCommand(route, "Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x, y }).catch(() => {});
          respond({ type: "result", id: request.id, result: { frame_id: frameId, backend_node_id: backendNodeId, dispatch_acknowledged: false, dispatch_may_have_occurred: pressAttempted, verified: false, reason: String(error) } });
          return;
        }
        const verifyOutcome = async () => {
          if (outcome.kind === "navigation") {
            const currentUrl = await frameUrl();
            return outcome.url == null ? currentUrl !== beforeUrl : new URL(currentUrl).href === new URL(outcome.url).href;
          }
          const result = await sendCdpCommand(child, "Runtime.callFunctionOn", {
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
        await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (request.method === "Controlla.frameSwipe") {
      const p=request.params||{},frameId=p.frame_id,loaderId=p.loader_id,backendNodeId=p.backend_node_id;
      const sx=p.start_x,sy=p.start_y,ex=p.end_x,ey=p.end_y,timeoutMs=p.timeout_ms??3000;
      const expectedActivationEpoch=p.expected_activation_epoch,expectedFocusEpoch=p.expected_focus_epoch;
      if(typeof frameId!=="string"||!frameId||frameId.length>256||typeof loaderId!=="string"||!loaderId||loaderId.length>256||
          !Number.isSafeInteger(backendNodeId)||backendNodeId<=0||![sx,sy,ex,ey].every(value=>Number.isFinite(value)&&value>=0&&value<=10000)||
          Math.hypot(ex-sx,ey-sy)<20||Math.hypot(ex-sx,ey-sy)>1200||!Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("frame swipe requires bounded frame identity, child-viewport coordinates, and timeout.");
      const root={tabId},deadline=performance.now()+timeoutMs,documentId=pairedTabDocumentIds.get(tabId),generation=routeGeneration;
      const currentInfo=async()=>{
        const tree=await sendCdpCommand(root,"Page.getFrameTree",{});
        const visit=(node,ancestors=[])=>node?.frame?.id===frameId?{frame:node.frame,chain:[...ancestors,node.frame]}:(node?.childFrames||[]).map(child=>visit(child,[...ancestors,node.frame])).find(Boolean);
        return visit(tree.frameTree);
      };
      const validChain=info=>!!info?.chain&&info.chain.length>=2&&info.chain.length<=9&&info.chain.every(frame=>frame?.id&&frame?.loaderId);
      const safeOrigins=info=>validChain(info)&&info.chain.every(frame=>{
        if(typeof frame.securityOrigin!=="string"||frame.securityOrigin==="null")return false;
        try { const u=new URL(frame.url); return ["http:","https:"].includes(u.protocol)&&u.origin===frame.securityOrigin; } catch { return false; }
      });
      const sameChain=(a,b)=>validChain(a)&&validChain(b)&&a.chain.length===b.chain.length&&a.chain.every((frame,index)=>frame.id===b.chain[index].id&&frame.loaderId===b.chain[index].loaderId);
      const identityStillOwned=async()=>{
        if(performance.now()>=deadline||!isAuthorized(tabId)||pairedTabDocumentIds.get(tabId)!==documentId||
            (nativeRoute&&resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&focusEpoch!==expectedFocusEpoch))return false;
        return (await chrome.tabs.get(tabId)).active===true;
      };
      let sessionId,child,objectId,touchStartAttempted=false,dispatchAcknowledged=false,releaseAcknowledged=false,receipt;
      try {
        await sendCdpCommand(root,"Target.setAutoAttach",{autoAttach:true,waitForDebuggerOnStart:false,flatten:true,filter:[{type:"iframe",exclude:false}]});
        const initialInfo=await currentInfo();
        if(!safeOrigins(initialInfo)||initialInfo.frame.loaderId!==loaderId)throw new Error("Frame identity or non-opaque HTTP(S) origin could not be proven.");
        sessionId=frameSessions.get(tabId)?.get(frameId)||await waitForFrameSession(tabId,frameId,400);
        child=sessionId?{tabId,sessionId}:root;
        const ax=await sendCdpCommand(child,"Accessibility.getFullAXTree",sessionId?{}:{frameId});
        if(!ax.nodes?.some(node=>node.backendDOMNodeId===backendNodeId))throw new Error("Swipe target is absent from the selected frame accessibility tree.");
        const resolved=await sendCdpCommand(child,"DOM.resolveNode",{backendNodeId});
        objectId=resolved.object?.objectId;
        if(!objectId)throw new Error("Swipe target could not be resolved in its child frame.");
        const guard="function(sx,sy,ex,ey,expectedViewport,phase){const e=this,d=e?.ownerDocument,w=d?.defaultView;if(!e||!d||!w||!e.isConnected)return {ok:false,reason:'target_stale'};const b=e.getBoundingClientRect(),vw=w.innerWidth,vh=w.innerHeight,viewport=JSON.stringify([vw,vh,w.devicePixelRatio||1]),point=(x,y)=>Number.isFinite(x)&&x>=0&&y>=0&&x<vw&&y<vh,style=w.getComputedStyle(e);if(expectedViewport&&expectedViewport!==viewport)return {ok:false,reason:'frame_viewport_changed'};if(b.width<=0||b.height<=0||style.display==='none'||style.visibility!=='visible'||style.pointerEvents==='none'||Number.parseFloat(style.opacity||'1')<=0)return {ok:false,reason:'target_not_interactable'};let scroll=null;for(let n=e;n&&n instanceof Element;n=n.parentElement){const s=w.getComputedStyle(n);if((['auto','scroll','overlay'].includes(s.overflowY)&&n.scrollHeight>n.clientHeight+1)||(['auto','scroll','overlay'].includes(s.overflowX)&&n.scrollWidth>n.clientWidth+1)){scroll={left:n.scrollLeft,top:n.scrollTop};break}}if(phase==='before'){const startInside=sx>=b.left&&sx<=b.right&&sy>=b.top&&sy<=b.bottom,hit=point(sx,sy)?d.elementFromPoint(sx,sy):null;if(!point(sx,sy)||!point(ex,ey)||!startInside||!(hit===e||e.contains(hit)))return {ok:false,reason:'swipe_target_or_endpoint_not_interactable'}}return {ok:true,viewport,rect:{left:b.left,top:b.top,width:b.width,height:b.height},scroll};}";
        const guardCall=async(phase,expectedViewport=null)=>{
          const result=await sendCdpCommand(child,"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,arguments:[{value:sx},{value:sy},{value:ex},{value:ey},{value:expectedViewport},{value:phase}],returnByValue:true});
          return result?.result?.value;
        };
        const before=await guardCall("before");
        if(before?.ok!==true)throw new Error(before?.reason||"Frame swipe target preflight refused.");
        const initialStart=await rootPointForFrame(tabId,frameId,objectId,sx,sy,{width:JSON.parse(before.viewport)[0],height:JSON.parse(before.viewport)[1]});
        const initialEnd=await rootPointForFrame(tabId,frameId,objectId,ex,ey,{width:JSON.parse(before.viewport)[0],height:JSON.parse(before.viewport)[1]});
        if(JSON.stringify(initialStart.owner_signature)!==JSON.stringify(initialEnd.owner_signature))throw new Error("Frame owner transform differed across swipe endpoints.");
        const finalGuard=await guardCall("before",before.viewport),finalInfo=await currentInfo();
        if(finalGuard?.ok!==true||JSON.stringify(finalGuard.rect)!==JSON.stringify(before.rect)||!sameChain(initialInfo,finalInfo)||
            frameSessions.get(tabId)?.get(frameId)!==(sessionId||undefined))throw new Error("Frame target, loader, or session changed during swipe preflight.");
        const finalStart=await rootPointForFrame(tabId,frameId,objectId,sx,sy,{width:JSON.parse(finalGuard.viewport)[0],height:JSON.parse(finalGuard.viewport)[1]});
        const finalEnd=await rootPointForFrame(tabId,frameId,objectId,ex,ey,{width:JSON.parse(finalGuard.viewport)[0],height:JSON.parse(finalGuard.viewport)[1]});
        const dispatchInfo=await currentInfo();
        const startChanged=Math.hypot(finalStart.x-initialStart.x,finalStart.y-initialStart.y)>0.5,endChanged=Math.hypot(finalEnd.x-initialEnd.x,finalEnd.y-initialEnd.y)>0.5;
        const ownerChanged=JSON.stringify(finalStart.owner_signature)!==JSON.stringify(initialStart.owner_signature)||JSON.stringify(finalEnd.owner_signature)!==JSON.stringify(initialEnd.owner_signature);
        const chainChanged=!sameChain(initialInfo,dispatchInfo),originChanged=!safeOrigins(dispatchInfo),identityChanged=!(await identityStillOwned());
        if(startChanged||endChanged||ownerChanged||chainChanged||originChanged||identityChanged)throw new Error(`Frame transform or document identity changed before trusted touch input (start=${startChanged}, end=${endChanged}, owner=${ownerChanged}, chain=${chainChanged}, origin=${originChanged}, identity=${identityChanged}).`);
        touchStartAttempted=true;
        await sendCdpCommand(root,"Input.dispatchTouchEvent",{type:"touchStart",touchPoints:[{id:1,x:finalStart.x,y:finalStart.y,radiusX:1,radiusY:1,force:1}]});
        await sendCdpCommand(root,"Input.dispatchTouchEvent",{type:"touchMove",touchPoints:[{id:1,x:finalEnd.x,y:finalEnd.y,radiusX:1,radiusY:1,force:1}]});
        dispatchAcknowledged=true;
        if(!sameChain(initialInfo,await currentInfo())||frameSessions.get(tabId)?.get(frameId)!==(sessionId||undefined)||!(await identityStillOwned()))throw new Error("Frame identity changed after swipe dispatch.");
        const after=await guardCall("after",before.viewport);
        if(after?.ok!==true)throw new Error("Frame swipe readback could not revalidate the target.");
        const moved=JSON.stringify(after.rect)!==JSON.stringify(before.rect)||JSON.stringify(after.scroll)!==JSON.stringify(before.scroll);
        receipt={status:moved?"verified":"inconclusive",reason_code:moved?undefined:"swipe_movement_not_observed",dispatch_acknowledged:true,dispatch_may_have_occurred:true,frame_id:frameId,frame_loader_id:loaderId,coordinate_space:"child_frame_css_viewport",root_start:finalStart,root_end:finalEnd,before:{rect:before.rect,scroll:before.scroll},after:{rect:after.rect,scroll:after.scroll}};
      } catch(error) {
        receipt={status:touchStartAttempted?"unknown":"not_dispatched",reason_code:touchStartAttempted?"frame_swipe_delivery_unknown":"frame_swipe_preflight_refused",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:touchStartAttempted,reason:safeNativeError(String(error))};
      } finally {
        if(touchStartAttempted)releaseAcknowledged=await Promise.race([sendCdpCommand(root,"Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]}).then(()=>true).catch(()=>false),new Promise(resolve=>setTimeout(()=>resolve(false),1000))]);
        if(objectId&&child)await sendCdpCommand(child,"Runtime.releaseObject",{objectId}).catch(()=>{});
        await sendCdpCommand(root,"Target.setAutoAttach",{autoAttach:false,waitForDebuggerOnStart:false,flatten:true}).catch(()=>{});
      }
      if(touchStartAttempted&&!releaseAcknowledged)receipt={status:"unknown",reason_code:"touch_release_unconfirmed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true};
      respond({type:"result",id:request.id,result:receipt||{status:"unknown",reason_code:"frame_swipe_receipt_missing",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:touchStartAttempted}});
      return;
    }
    if (request.method === "Controlla.frameFill") {
      const frameId = request.params?.frame_id, loaderId = request.params?.loader_id, backendNodeId = request.params?.backend_node_id;
      const expected = request.params?.expected_value, value = request.params?.value, timeoutMs = request.params?.timeout_ms;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256 || typeof loaderId !== "string" || !loaderId || loaderId.length > 256 || !Number.isSafeInteger(backendNodeId) || backendNodeId <= 0) throw new Error("frame fill requires bounded frame, loader, and backend node identities.");
      if (typeof expected !== "string" || typeof value !== "string" || expected.length > 16384 || value.length > 16384 || !Number.isSafeInteger(timeoutMs) || timeoutMs < 250 || timeoutMs > 10000) throw new Error("frame fill values or timeout exceed their bounds.");
      const route = { tabId };
      const frameLoader = async () => {
        const tree = await sendCdpCommand(route, "Page.getFrameTree", {});
        const visit = node => node?.frame?.id === frameId ? node.frame.loaderId : (node?.childFrames || []).map(visit).find(Boolean);
        return visit(tree.frameTree);
      };
      await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      let dispatchAttempted = false;
      try {
        if (await frameLoader() !== loaderId) throw new Error("Frame document changed since the accessibility snapshot.");
        let sessionId = frameSessions.get(tabId)?.get(frameId);
        if (!sessionId) sessionId = await waitForFrameSession(tabId, frameId, 400);
        const child = sessionId ? { tabId, sessionId } : route;
        const ax = await sendCdpCommand(child, "Accessibility.getFullAXTree", sessionId ? {} : { frameId });
        const node = ax.nodes?.find(item => item.backendDOMNodeId === backendNodeId);
        if (!node || !["textbox", "searchbox", "combobox"].includes(node.role?.value)) throw new Error("Frame field is stale or is not an editable text control.");
        const resolved = await sendCdpCommand(child, "DOM.resolveNode", { backendNodeId });
        const objectId = resolved.object?.objectId;
        if (!objectId) throw new Error("Frame field could not be resolved for guarded fill.");
        if ((sessionId && frameSessions.get(tabId)?.get(frameId) !== sessionId) || await frameLoader() !== loaderId) throw new Error("Frame document changed before fill dispatch.");
        const functionDeclaration = "function(expected,value){const e=this;const blocked=r=>({dispatch_may_have_occurred:false,verified:false,matches_requested_value:false,reason_code:r});if(!e||!e.isConnected)return blocked('target_stale');const input=e instanceof HTMLInputElement,area=e instanceof HTMLTextAreaElement;if(!input&&!area)return blocked('unsupported_control');if(input&&['password','file','hidden','checkbox','radio','submit','button','reset'].includes((e.type||'').toLowerCase()))return blocked('protected_or_nontext_control');if(e.disabled||e.readOnly||e.getAttribute('aria-disabled')==='true'||e.getAttribute('aria-readonly')==='true'||e.hasAttribute('data-masked')||e.hasAttribute('data-requires-trusted'))return blocked('field_blocked');const r=e.getBoundingClientRect(),s=getComputedStyle(e),x=Math.min(innerWidth-1,Math.max(0,r.left+r.width/2)),y=Math.min(innerHeight-1,Math.max(0,r.top+r.height/2));if(r.width<=0||r.height<=0||s.display==='none'||s.visibility!=='visible'||Number.parseFloat(s.opacity||'1')<=0||document.elementFromPoint(x,y)!==e)return blocked('field_not_actionable');if(e.value!==expected)return blocked('stale_value');try{e.focus({preventScroll:true});if(document.activeElement!==e)return blocked('focus_failed');const proto=input?HTMLInputElement.prototype:HTMLTextAreaElement.prototype;const setter=Object.getOwnPropertyDescriptor(proto,'value')?.set;if(!setter)return blocked('native_setter_unavailable');setter.call(e,value);e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:value}));e.dispatchEvent(new Event('change',{bubbles:true}));const matches=e.value===value;return {dispatch_may_have_occurred:true,verified:matches,matches_requested_value:matches,length:e.value.length,reason_code:matches?null:'readback_mismatch'}}catch(_){return {dispatch_may_have_occurred:true,verified:false,matches_requested_value:false,reason_code:'fill_delivery_uncertain'}}}";
        let result;
        try {
          dispatchAttempted = true;
          result = await sendCdpCommand(child, "Runtime.callFunctionOn", { objectId, functionDeclaration, arguments: [{ value: expected }, { value }], returnByValue: true, awaitPromise: false });
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
        await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (request.method === "Controlla.frameAccessibility") {
      const frameId = request.params?.frame_id;
      if (typeof frameId !== "string" || !frameId || frameId.length > 256) throw new Error("frame_id must be a bounded nonempty string.");
      const route = { tabId };
      await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true, filter: [{ type: "iframe", exclude: false }] });
      try {
        const sessionId = await waitForFrameSession(tabId, frameId, 1500);
        if (!sessionId) throw new Error("No attached OOPIF session matched the requested frame.");
        const result = await sendCdpCommand({ tabId, sessionId }, "Accessibility.getFullAXTree", {});
        respond({ type: "result", id: request.id, result: { ...result, source_frame_id: frameId, routing: "oopif_child_session" } });
      } finally {
        await sendCdpCommand(route, "Target.setAutoAttach", { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
      }
      return;
    }
    if (!SHARED_COMMAND_METHODS.has(request.method)) throw new Error("Command is outside the shared observe/input allowlist.");
    if (request.method === "Page.captureScreenshot") {
      const p = request.params || {}, clip = p.clip;
      if (p.format !== "png" || typeof p.captureBeyondViewport !== "boolean" || p.fromSurface !== true ||
          !clip || ![clip.x, clip.y, clip.width, clip.height, clip.scale].every(Number.isFinite) ||
          clip.x < 0 || clip.y < 0 || clip.width <= 0 || clip.height <= 0 || clip.width > 4096 || clip.height > 4096 ||
          clip.scale <= 0 || clip.scale > 2 || clip.width * clip.height * clip.scale * clip.scale > 16_777_216) {
        throw new Error("Shared screenshot requires a bounded PNG crop (maximum 4096x4096 CSS pixels and 16 megapixels).");
      }
    }
    const screenshotDocumentId = request.method === "Page.captureScreenshot" ? pairedTabDocumentIds.get(tabId) : undefined;
    const screenshotStartedAt = request.method === "Page.captureScreenshot" ? Date.now() : undefined;
    const screenshotLifecycleGeneration = request.method === "Page.captureScreenshot" ? (tabLifecycleGenerations.get(tabId) || 0) : undefined;
    const screenshotTabBefore = request.method === "Page.captureScreenshot" ? await chrome.tabs.get(tabId) : undefined;
    const screenshotLifecycleBefore = request.method === "Page.captureScreenshot" ? screenshotTargetLifecycle(screenshotTabBefore) : undefined;
    if (request.method === "Page.captureScreenshot" && screenshotLifecycleBefore !== "active") {
      const reasonCode = screenshotLifecycleBefore === "frozen" ? "target_frozen" : screenshotLifecycleBefore === "discarded" ? "target_discarded" : screenshotLifecycleBefore === "loading" ? "target_not_ready" : "target_lifecycle_unknown";
      throw Object.assign(new Error(`Screenshot target lifecycle is ${screenshotLifecycleBefore}; capture requires an observed active, unfrozen renderer.`), { reason_code: reasonCode });
    }
    const screenshotFrameBefore = request.method === "Page.captureScreenshot"
      ? await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined)
      : undefined;
    if (request.method === "Page.captureScreenshot" && (!screenshotDocumentId || screenshotFrameBefore?.documentId !== screenshotDocumentId)) {
      throw Object.assign(new Error("Screenshot target document is not fresh and stable before capture."), { reason_code: "screenshot_document_changed" });
    }
    if (request.method === "Page.captureScreenshot" && (tabLifecycleGenerations.get(tabId) || 0) !== screenshotLifecycleGeneration) {
      throw Object.assign(new Error("Screenshot target lifecycle changed before capture dispatch."), { reason_code: "target_lifecycle_changed" });
    }
    let renderFrame;
    if (request.method === "Page.captureScreenshot") {
      renderFrame = await observeFreshRendererFrame(tabId);
      const frame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      const target = await chrome.tabs.get(tabId);
      if (frame?.documentId !== screenshotDocumentId || screenshotTargetLifecycle(target) !== "active" ||
          (tabLifecycleGenerations.get(tabId) || 0) !== screenshotLifecycleGeneration) {
        throw Object.assign(new Error("Target document or lifecycle changed while waiting for a fresh renderer frame."), { reason_code: "screenshot_render_frame_stale" });
      }
    }
    const requestBytes = JSON.stringify({ method: request.method, params: request.params || {} }).length;
    if (!topologyStable()) {
      commandMayHaveDispatched = false;
      respond({ type: "result", id: request.id, result: { status: "not_dispatched", reason_code: "tab_topology_changed", dispatch_may_have_occurred: false,
        reason: "Paired tab topology changed before CDP dispatch; retry after reconciling its window." } });
      return;
    }
    traceSharedCommand("dispatch", request, tabId, startedAt, { requestBytes }); const result = await sendCdpCommand({ tabId }, request.method, request.params || {}, request.deadline_ms);
    if (!topologyStable()) throw Object.assign(new Error("Paired tab topology changed during CDP dispatch."), { reason_code: "tab_topology_changed" });
    if (screenshotDocumentId && pairedTabDocumentIds.get(tabId) !== screenshotDocumentId) {
      throw Object.assign(new Error("Target document changed during screenshot capture; image bytes were discarded."), { reason_code: "screenshot_document_changed" });
    }
    const screenshotFrameAfter = request.method === "Page.captureScreenshot"
      ? await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined)
      : undefined;
    if (request.method === "Page.captureScreenshot" && screenshotFrameAfter?.documentId !== screenshotDocumentId) {
      throw Object.assign(new Error("Screenshot document changed during compositor capture; image bytes were discarded."), { reason_code: "screenshot_document_changed" });
    }
    if (request.method === "Page.captureScreenshot") decodeScreenshotPng(result?.data);
    if (request.method === "Page.captureScreenshot") {
      const image = decodeScreenshotPng(result.data);
      const dataHash = await crypto.subtle.digest("SHA-256", image.bytes);
      const imageSha256 = [...new Uint8Array(dataHash)].map(byte => byte.toString(16).padStart(2, "0")).join("");
      const target = await chrome.tabs.get(tabId);
      const lifecycleAfter = screenshotTargetLifecycle(target);
      if (lifecycleAfter !== "active" || (tabLifecycleGenerations.get(tabId) || 0) !== screenshotLifecycleGeneration) {
        const reasonCode = lifecycleAfter === "frozen" ? "target_frozen" : lifecycleAfter === "discarded" ? "target_discarded" : lifecycleAfter === "loading" ? "target_not_ready" : lifecycleAfter === "unknown" ? "target_lifecycle_unknown" : "target_lifecycle_changed";
        throw Object.assign(new Error("Screenshot target lifecycle changed during capture; image bytes were discarded."), { reason_code: reasonCode });
      }
      const clip = request.params.clip;
      result.visual_evidence = {
        target_id: String(tabId), document_id: screenshotDocumentId,
        url: String(target.url || "").slice(0, 2048), frame_url: String(screenshotFrameAfter.url || "").slice(0, 2048), captured_at_ms: Date.now(),
        capture_started_at_ms: screenshotStartedAt, visual_revision: imageSha256, image_sha256: imageSha256,
        render_revision: `${screenshotDocumentId}:${renderFrame.timestamp_ms.toFixed(3)}`,
        render_frame_timestamp_ms: renderFrame.timestamp_ms,
        render_frame_provenance: "two_animation_frames_before_compositor_capture",
        target_lifecycle: lifecycleAfter,
        dimensions: { width: image.width, height: image.height },
      };
    }
    traceSharedCommand("settled", request, tabId, startedAt, { outcome: "result", requestBytes, responseBytes: (() => { try { return JSON.stringify(result).length; } catch { return 0; } })() }); respond({ type: "result", id: request.id, result });
  } catch (error) { traceSharedCommand("settled", request, tabId, startedAt, { outcome: "error", errorClass: safeErrorClass(error) }); respond({ type: "result", id: request.id, error: String(error), ...(error?.reason_code ? { reason_code: error.reason_code } : {}) }); }
}
async function observeFreshRendererFrame(tabId) {
  const expression = "new Promise(resolve=>{let settled=false;const timer=setTimeout(()=>{if(!settled){settled=true;resolve({ready:false})}},1000);requestAnimationFrame(()=>requestAnimationFrame(timestamp=>{if(!settled){settled=true;clearTimeout(timer);resolve({ready:true,timestamp_ms:timestamp})}}))})";
  let result, deadlineTimer;
  try {
    result = await Promise.race([
      sendCdpCommand({ tabId }, "Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }),
      new Promise((_, reject) => { deadlineTimer = setTimeout(() => reject(Object.assign(new Error("Timed out waiting for a fresh renderer animation frame."), { reason_code: "screenshot_render_frame_timeout" })), 1500); }),
    ]);
  } catch (error) {
    throw Object.assign(new Error(error?.reason_code ? String(error.message) : "Could not observe a fresh renderer animation frame before screenshot capture."), {
      reason_code: error?.reason_code || "screenshot_render_frame_unavailable",
    });
  } finally {
    clearTimeout(deadlineTimer);
  }
  const frame = result?.result?.value;
  if (frame?.ready !== true || !Number.isFinite(frame.timestamp_ms) || frame.timestamp_ms < 0) {
    throw Object.assign(new Error("Renderer did not produce two animation frames before screenshot capture."), { reason_code: "screenshot_render_frame_unavailable" });
  }
  return frame;
}
function screenshotTargetLifecycle(tab) {
  if (!tab) return "unknown";
  if (tab.discarded) return "discarded";
  if (tab.status !== "complete") return "loading";
  if (tab.frozen === true) return "frozen";
  if (tab.frozen === false) return "active";
  return "unknown";
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
  pairedTabUrls.set(tabId, attachedIdentity.url); pairedTabDocumentIds.set(tabId, attachedIdentity.document_id); bindPairedTabTopology(tabId); if (native) addNativeTab(tabId, generation); else manualAttachedTabs.add(tabId); return attachedIdentity;
}
async function returnExistingOpen(receipt, url, native, generation) {
  if ((receipt.requested_url || receipt.url) !== url) throw new Error("Idempotency key is already bound to a different URL.");
  const tabId = Number(receipt.tab_id);
  if (Number.isSafeInteger(receipt.target_window_id)) {
    const window = await assertOwnedAutomationWindow(receipt.target_window_id);
    const existingTab = await chrome.tabs.get(tabId).catch(() => undefined);
    if (!existingTab || existingTab.windowId !== receipt.target_window_id || window.focused) throw new Error("The selected automation window changed or is focused; no replacement tab was opened.");
  }
  const identity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!identity || identity.url !== receipt.url || identity.document_id !== receipt.document_id) throw new Error(`Open receipt ${receipt.tab_id} is stale; its tab changed or closed, so no replacement tab was opened.`);
  const attached = native ? nativeAttachedTabs : manualAttachedTabs;
  if (!attached.has(tabId)) {
    await attachDebugger(tabId);
    const after = await nativePageIdentity(tabId).catch(() => undefined);
    if (!routeActive(native, generation) || after?.url !== receipt.url || after?.document_id !== receipt.document_id) { await detachDebugger(tabId); throw new Error(`Open receipt ${receipt.tab_id} changed while reconnecting; no replacement tab was opened.`); }
    pairedTabUrls.set(tabId, after.url); pairedTabDocumentIds.set(tabId, after.document_id); bindPairedTabTopology(tabId); attached.add(tabId);
  }
  const tab = await chrome.tabs.get(tabId), window = await chrome.windows.get(tab.windowId);
  return { tab_id: receipt.tab_id, window_id: receipt.window_id ?? tab.windowId, url: receipt.url, document_id: receipt.document_id, idempotent_reuse: true, created_by_controlla: true, created_background_window: receipt.create_background_window === true, placement: receipt.placement || {}, tab_active: !!tab.active, window_focused: !!window.focused };
}
async function assertOwnedAutomationWindow(windowId) {
  const window = await chrome.windows.get(windowId, { populate: true }).catch(() => undefined);
  if (!window || window.focused !== false) throw new Error("Selected automation window is unavailable or focused; refusing to change user presentation.");
  const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
  const receipts = saved[OPEN_RECEIPTS_KEY] || {};
  const owned = Object.values(receipts).some(receipt => receipt?.status === "complete"
    && receipt.create_background_window === true && receipt.window_id === windowId);
  if (!owned) throw new Error("Selected window has no completed Controlla automation-window ownership receipt.");
  return window;
}
async function createControllaTab(url, native, generation, tab, markerUrl, reused = false, waitUntil = "dom_content_loaded", active = true, createBackgroundWindow = false, placement = {}) {
  tab ||= await chrome.tabs.create({ url: markerUrl, active });
  let identity, attachedIdentity;
  try {
    const current = await chrome.tabs.get(tab.id);
    attachedIdentity = await attachNewTab(tab.id, url, native, generation, current.url !== url, waitUntil);
    identity = await nativePageIdentity(tab.id);
  }
  catch (error) {
    if (attachedIdentity && routeActive(native, generation) && pairedTabDocumentIds.get(tab.id) === attachedIdentity.document_id && (native ? nativeLeaseForGeneration(generation)?.tabs.has(tab.id) : manualAttachedTabs.has(tab.id))) { pairedTabUrls.delete(tab.id); pairedTabDocumentIds.delete(tab.id); clearPairedTabTopology(tab.id); if (native) removeNativeTab(tab.id); else manualAttachedTabs.delete(tab.id); await detachDebugger(tab.id); }
    const failure = new Error("Tab " + tab.id + " remains open after the failed open; rediscover and pair it before use. " + String(error));
    failure.controllaOrphanTabId = String(tab.id);
    throw failure;
  }
  const [currentTab, currentWindow] = await Promise.all([chrome.tabs.get(tab.id), chrome.windows.get(tab.windowId)]);
  return { tab_id: String(tab.id), window_id: tab.windowId, ...identity, idempotent_reuse: reused, created_by_controlla: true,
    created_background_window: createBackgroundWindow, placement, tab_active: !!currentTab.active, window_focused: !!currentWindow.focused };
}
async function openControllaTab(url, clientKey, native, generation, waitUntil = "dom_content_loaded", active = true, createBackgroundWindow = false, placement = {}, targetWindowId) {
  if (clientKey === undefined) throw new Error("idempotency_key is required so an open retry cannot create another tab.");
  if (typeof clientKey !== "string" || clientKey.length < 8 || clientKey.length > 256 || /[\u0000-\u001f\u007f]/.test(clientKey)) throw new Error("idempotency_key must be 8..256 bounded printable characters.");
  const pending = openTabInflight.get(clientKey);
  if (pending) {
    if (pending.url !== url) throw new Error("Idempotency key is already bound to a different URL.");
    if (pending.active !== active) throw new Error("Idempotency key is already bound to a different activation mode.");
    if (pending.createBackgroundWindow !== createBackgroundWindow || JSON.stringify(pending.placement) !== JSON.stringify(placement)) throw new Error("Idempotency key is already bound to a different target/window policy.");
    if (pending.targetWindowId !== targetWindowId) throw new Error("Idempotency key is already bound to a different automation window.");
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
    if (receipt?.status === "closed") throw new Error("This idempotency key belongs to a tab Controlla already closed; use a new key to create another tab.");
    if (receipt?.status === "complete" || receipt?.document_id) {
      if ((receipt.active ?? true) !== active) throw new Error("Idempotency key is already bound to a different activation mode.");
      if ((receipt.create_background_window === true) !== createBackgroundWindow || JSON.stringify(receipt.placement || {}) !== JSON.stringify(placement)) throw new Error("Idempotency key is already bound to a different target/window policy.");
      if (receipt.target_window_id !== targetWindowId) throw new Error("Idempotency key is already bound to a different automation window.");
      if (Number.isSafeInteger(targetWindowId)) await assertOwnedAutomationWindow(targetWindowId);
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
    if (receipt && ((receipt.create_background_window === true) !== createBackgroundWindow || JSON.stringify(receipt.placement || {}) !== JSON.stringify(placement))) throw new Error("Idempotency key is already bound to a different target/window policy.");
    if (receipt && receipt.target_window_id !== targetWindowId) throw new Error("Idempotency key is already bound to a different automation window.");
    if (Number.isSafeInteger(targetWindowId)) await assertOwnedAutomationWindow(targetWindowId);
    let createdWindow;
    if (!tab && createBackgroundWindow) {
      createdWindow = await chrome.windows.create({ url: markerUrl, focused: false, type: "normal", ...placement });
      tab = createdWindow?.tabs?.find(candidate => Number.isSafeInteger(candidate.id));
      if (!tab || !Number.isSafeInteger(createdWindow?.id)) throw new Error("Chrome did not return the new unfocused Controlla window and tab.");
    } else if (!tab && Number.isSafeInteger(targetWindowId)) tab = await chrome.tabs.create({ windowId: targetWindowId, url: markerUrl, active });
    else if (!tab) tab = await chrome.tabs.create({ url: markerUrl, active });
    receipt = { status: "creating", tab_id: String(tab.id), window_id: createdWindow?.id ?? tab.windowId, target_window_id: targetWindowId, url, requested_url: url, marker_url: markerUrl, active, create_background_window: createBackgroundWindow, placement, created_ms: receipt?.created_ms || Date.now() };
    receipts[clientKey] = receipt;
    pruneOpenReceipts(receipts);
    await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
    return { tab, markerUrl, reused, created_ms: receipt.created_ms, window_id: receipt.window_id, target_window_id: targetWindowId, create_background_window: createBackgroundWindow, placement };
    });
    if (reservation.existing) return returnExistingOpen(reservation.existing, url, native, generation);
    let result;
    try { result = await createControllaTab(url, native, generation, reservation.tab, reservation.markerUrl, reservation.reused, waitUntil, active, createBackgroundWindow, placement); }
    catch (error) {
      if (error?.controllaOrphanTabId) {
        await withReceiptLock(async () => {
          const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
          const receipts = saved[OPEN_RECEIPTS_KEY] || {};
          receipts[clientKey] = { status: "failed", tab_id: error.controllaOrphanTabId, window_id: reservation.window_id, target_window_id: targetWindowId, url, requested_url: url, active, create_background_window: createBackgroundWindow, placement, created_ms: reservation.created_ms };
          pruneOpenReceipts(receipts);
          await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
        });
      }
      throw error;
    }
    if (Number.isSafeInteger(targetWindowId)) {
      const selectedWindow = await assertOwnedAutomationWindow(targetWindowId);
      if (result.window_id !== targetWindowId || selectedWindow.focused) throw new Error("Created tab moved or the selected automation window was focused; inspect its topology before retrying.");
    }
    await withReceiptLock(async () => {
      const saved = await chrome.storage.local.get(OPEN_RECEIPTS_KEY);
      const receipts = saved[OPEN_RECEIPTS_KEY] || {};
      const current = receipts[clientKey];
      if (current?.tab_id !== result.tab_id) throw new Error("Open receipt changed before completion; no second tab was opened.");
      receipts[clientKey] = { status: "complete", tab_id: result.tab_id, window_id: result.window_id ?? reservation.window_id, target_window_id: targetWindowId, url: result.url, requested_url: url, document_id: result.document_id, active, create_background_window: createBackgroundWindow, placement, created_ms: reservation.created_ms };
      pruneOpenReceipts(receipts);
      await chrome.storage.local.set({ [OPEN_RECEIPTS_KEY]: receipts });
    });
    return result;
  })();
  const inflight = { url, active, createBackgroundWindow, placement, targetWindowId, promise: operation };
  openTabInflight.set(clientKey, inflight);
  try { return await operation; } finally { if (openTabInflight.get(clientKey) === inflight) openTabInflight.delete(clientKey); }
}
function pruneOpenReceipts(receipts, now = Date.now()) {
  for (const [key, receipt] of Object.entries(receipts)) {
    if (["complete", "closed"].includes(receipt?.status) && Number.isSafeInteger(receipt.created_ms) && now - receipt.created_ms > OPEN_RECEIPT_MAX_AGE_MS) delete receipts[key];
  }
  const completed = Object.entries(receipts).filter(([, receipt]) => ["complete", "closed"].includes(receipt?.status))
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
  lease.tabs.add(tabId); nativeTabLeases.set(tabId, leaseId); nativeAttachedTabs.add(tabId); pairedTabUrls.set(tabId, tab.url); pairedTabDocumentIds.set(tabId, frame.documentId); bindPairedTabTopology(tabId);
}
async function navigatePairedTab(tabId, url, native, generation) {
  clearDownloadLifecycleState(tabId, "tab_navigated");
  clearFileChooserState(tabId, "Target navigated before the file chooser opened.");
  const before = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
  if (!before?.documentId) throw new Error("Cannot navigate a paired tab without a current document identity.");
  const ready = waitForTabDOMContentLoaded(tabId, before.documentId);
  (native ? nativeAttachedTabs : manualAttachedTabs).delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); await detachDebugger(tabId); await chrome.tabs.update(tabId, { url }); await ready;
  if (!routeActive(native, generation)) throw new Error("Shared session ended during tab navigation.");
  const identity = await nativePageIdentity(tabId); await attachDebugger(tabId); const attachedIdentity = await nativePageIdentity(tabId).catch(() => undefined);
  if (!routeActive(native, generation) || !await pendingNavigationMatchesDocument(tabId, attachedIdentity?.document_id) || attachedIdentity?.url !== identity.url || attachedIdentity?.document_id !== identity.document_id) { await detachDebugger(tabId); throw new Error("Tab changed during attachment; retry from a fresh inventory."); }
  pairedTabUrls.set(tabId, attachedIdentity.url); pairedTabDocumentIds.set(tabId, attachedIdentity.document_id); bindPairedTabTopology(tabId); if (native) addNativeTab(tabId, generation); else manualAttachedTabs.add(tabId);
}
function removeNativeTab(tabId) { const leaseId = nativeTabLeases.get(tabId); if (leaseId) nativeLeases.get(leaseId)?.tabs.delete(tabId); nativeTabLeases.delete(tabId); nativeAttachedTabs.delete(tabId); }
async function detachNativeLease(leaseId) { const lease = nativeLeases.get(leaseId); if (!lease) return; nativeLeases.delete(leaseId); nativeLeaseGenerations.delete(lease.generation); const tabs = [...lease.tabs]; for (const tabId of tabs) { if (nativeTabLeases.get(tabId) !== leaseId) continue; nativeTabLeases.delete(tabId); nativeAttachedTabs.delete(tabId); clearDownloadLifecycleState(tabId, "native_lease_ended"); clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); } await Promise.all(tabs.filter(tabId => !nativeTabLeases.has(tabId)).map(detachDebugger)); await persistNativeLeaseJournal(); }
async function detachPairedTab(tabId) { clearDownloadLifecycleState(tabId); clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); manualAttachedTabs.delete(tabId); removeNativeTab(tabId); await detachDebugger(tabId); }
async function detachManual(generation) { if (generation !== manualGeneration) return; const toDetach = [...manualAttachedTabs]; manualAttachedTabs = new Set(); for (const tabId of toDetach) { clearDownloadLifecycleState(tabId, "manual_bridge_ended"); clearFileChooserState(tabId); clearFrameSessions(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); } await Promise.all(toDetach.map(detachDebugger)); }
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
  if (message?.type === "native-status") { readNativeLeaseJournal().then(journal => respond({ connected: nativeReady, attached: [...nativeAttachedTabs], error: nativeError, retry_exhausted: reconnectExhausted, recovery_journal: journal, transport: "native" })).catch(() => respond({ connected: nativeReady, attached: [...nativeAttachedTabs], error: nativeError, retry_exhausted: reconnectExhausted, recovery_journal: { schema_version: 1, sequence: 0, status: "unavailable", leases: [] }, transport: "native" })); return true; }
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
      pairedTabUrls.set(tabId, beforeAttach.url); pairedTabDocumentIds.set(tabId, beforeFrame.documentId); bindPairedTabTopology(tabId); await attachDebugger(tabId); if (generation !== manualGeneration) { await detachDebugger(tabId); throw new Error("Pairing was replaced."); }
      manualAttachedTabs.add(tabId); const attached = await chrome.tabs.get(tabId).catch(() => null); const attachedFrame = await chrome.webNavigation.getFrame({ tabId, frameId: 0 }).catch(() => undefined);
      if (navigationGeneration !== (tabNavigationGenerations.get(tabId) || 0) || !attached || attached.status !== "complete" || attached.url !== beforeAttach.url || attachedFrame?.documentId !== beforeFrame.documentId) throw new Error("Tab " + tabId + " navigated during pairing; pair again.");
    }
  } catch (error) { await detachManual(generation); for (const tabId of tabs) { pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); } throw error; }
  const next = new WebSocket(endpointUrl.href); socket = next;
  try {
    await new Promise((resolve, reject) => { next.onopen = resolve; next.onerror = () => reject(new Error("Cannot connect to the loopback provider.")); next.onclose = () => reject(new Error("Loopback pairing was closed.")); }); if (generation !== manualGeneration) throw new Error("Pairing was replaced.");
    const extensionVersion = chrome.runtime.getManifest().version;
    const extensionBuildId = await CONTROLLA_EXTENSION_BUILD_ID_PROMISE;
    next.send(JSON.stringify({ type: "hello", token, extension_version: extensionVersion, extension_build_id: extensionBuildId, document_identity: true, batch_execution: true, batch_deadline: true, targets: [...manualAttachedTabs].map(String).sort() }));
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
    clearDownloadLifecycleState(tabId, "target_detached_during_navigation");
    debuggerDetachedDuringNavigation.add(tabId);
    if (committed && !navigatingTabs.has(tabId)) void reattachAfterNavigation(tabId, committed.native, committed.generation, committed.url, committed.documentId).then(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) debuggerDetachedDuringNavigation.delete(tabId); }).catch(() => { if (pairedTabDocumentIds.get(tabId) === committed.documentId) void detachPairedTab(tabId); });
    return;
  }
  clearDownloadLifecycleState(tabId, "debugger_detached"); manualAttachedTabs.delete(tabId); removeNativeTab(tabId); pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); committedNavigationTabs.delete(tabId);
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
  if (changeInfo.status === "loading") { clearDownloadLifecycleState(tabId, "tab_navigated"); clearFileChooserState(tabId, "Target navigated before the file chooser opened."); tabNavigationGenerations.set(tabId, (tabNavigationGenerations.get(tabId) || 0) + 1); if (pairedTabDocumentIds.has(tabId)) navigatingTabs.add(tabId); }
  if (changeInfo.status === "complete") { const pending = pendingMainFrameNavigations.get(tabId); if (pending?.failed) void clearFailedNavigationWhenStable(tabId, pending); if (!debuggerDetachedDuringNavigation.has(tabId)) navigatingTabs.delete(tabId); }
  const pairedUrl = pairedTabUrls.get(tabId); if (pairedUrl === undefined || !changeInfo.url || changeInfo.url === "about:blank" || navigatingTabs.has(tabId)) return;
  const changed = (() => { try { const before = new URL(pairedUrl), after = new URL(changeInfo.url); return before.origin !== after.origin || before.pathname !== after.pathname || before.search !== after.search; } catch { return changeInfo.url !== pairedUrl; } })(); if (changed) void detachPairedTab(tabId);
});
chrome.webNavigation.onBeforeNavigate.addListener(details => {
  if (details.frameId !== 0) return;
  clearDownloadLifecycleState(details.tabId, "tab_navigated");
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
  recentlyCreatedTabs.delete(removedTabId);
  clearDownloadLifecycleState(removedTabId, "tab_replaced");
  pendingMainFrameNavigations.delete(removedTabId); tabNavigationGenerations.set(removedTabId, (tabNavigationGenerations.get(removedTabId) || 0) + 1); if (!pairedTabUrls.has(removedTabId)) return;
  navigatingTabs.delete(removedTabId); debuggerDetachedDuringNavigation.delete(removedTabId); pairedTabUrls.delete(removedTabId); pairedTabDocumentIds.delete(removedTabId); clearPairedTabTopology(removedTabId); manualAttachedTabs.delete(removedTabId); nativeAttachedTabs.delete(removedTabId); void detachDebugger(removedTabId);
});
chrome.tabs.onRemoved.addListener(tabId => {
  recentlyCreatedTabs.delete(tabId);
  clearDownloadLifecycleState(tabId, "tab_closed");
  pendingMainFrameNavigations.delete(tabId); tabNavigationGenerations.delete(tabId); domReadyDocuments.delete(tabId);
  clearFrameSessions(tabId); clearFileChooserState(tabId, "Target tab was closed before the file chooser opened.");
  fileChooserEvents.delete(tabId); fileChooserOperations.delete(tabId); navigationDebuggerAttaches.delete(tabId);
  pairedTabUrls.delete(tabId); pairedTabDocumentIds.delete(tabId); clearPairedTabTopology(tabId); navigatingTabs.delete(tabId); debuggerDetachedDuringNavigation.delete(tabId); committedNavigationTabs.delete(tabId);
});
void reconnectStateReady.then(() => { if (!reconnectExhausted) connectNative(); });
