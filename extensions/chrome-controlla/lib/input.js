// Canonical text-input transaction. One native command covers an entire text
// operation; target resolution, focus, chunking, readback, and IME composition
// happen locally. No caller-visible per-character bridge command is used.

import {
  chooseInputBackend,
  codePointLength,
  graphemeChunks,
  hashText,
  selectEditorAdapter,
} from "./editor-adapters.js";

export const MAX_TRANSACTION_TEXT = 16384;
const SUPPORTED_OPERATIONS = new Set(["replace", "append", "insert_at_caret"]);
const SUPPORTED_STRATEGIES = new Set([
  "auto",
  "native_setter",
  "insert_text",
  "key_events",
  "ime",
  "clipboard_paste",
]);

export const TRANSACTION_GUARD = `function(expectedKind,expectedValue,expectedAnchor,operation,prepare,normalize){
  const e=this;
  if(!e||!e.isConnected)return {ok:false,reason:'target_stale'};
  const rich=e instanceof HTMLElement&&!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement)&&(e.isContentEditable||e.getAttribute('role')==='textbox');
  const plain=e instanceof HTMLTextAreaElement||e instanceof HTMLInputElement&&['text','search','email','url','tel'].includes(e.type);
  const trusted=e.closest('[data-requires-trusted]');
  if(!rich&&!plain)return {ok:false,reason:'not_editable'};
  if(e.matches(':disabled,[aria-disabled="true"]')||e.readOnly||e.getAttribute('aria-readonly')==='true'||e.getAttribute('contenteditable')?.toLowerCase()==='false'||e.closest('[data-masked]')||(trusted&&trusted!==e))return {ok:false,reason:'blocked'};
  const read=()=>{const raw=rich?(e.innerText??e.textContent??'').replace(/\\r\\n/g,'\\n').replace(/\\u00a0/g,' '):e.value;return rich?(raw==='\\n'?'':normalize?raw.replace(/\\n{2,}/g,b=>'\\n'.repeat(Math.ceil(b.length/2))):raw):raw};
  let value=read();
  const expectedOk=expectedKind==='unchecked'||expectedKind==='contains'?expectedKind==='contains'?value.includes(expectedValue):true:expectedKind==='suffix'?value.endsWith(expectedValue):value===expectedValue;
  if(!expectedOk)return {ok:false,reason:'stale_value',value,length:value.length,rich};
  if(prepare){
    e.focus();
    if(rich){
      const s=e.ownerDocument.getSelection(),r=e.ownerDocument.createRange();
      if(operation==='replace')r.selectNodeContents(e);
      else {r.selectNodeContents(e);r.collapse(false)}
      s.removeAllRanges();s.addRange(r);
    }else if(operation==='replace')e.setSelectionRange(0,e.value.length);
    else if(operation==='append')e.setSelectionRange(e.value.length,e.value.length);
  }
  const focused=e.ownerDocument.activeElement===e||e.contains(e.ownerDocument.activeElement);
  let atBoundary=true,selectionStart=null,selectionEnd=null;
  if(rich){
    const s=e.ownerDocument.getSelection();
    selectionStart=s?.anchorOffset??null;selectionEnd=s?.focusOffset??null;
    if(operation==='append'){if(s?.isCollapsed&&s.anchorNode){const r=e.ownerDocument.createRange();r.selectNodeContents(e);r.setStart(s.anchorNode,s.anchorOffset);atBoundary=r.toString()===''}else atBoundary=false}
    if(operation==='replace')atBoundary=!!s&&!s.isCollapsed&&s.toString()===value;
  }else{
    selectionStart=e.selectionStart;selectionEnd=e.selectionEnd;
    if(operation==='append')atBoundary=selectionStart===selectionEnd&&selectionEnd===e.value.length;
    if(operation==='replace')atBoundary=selectionStart===0&&selectionEnd===e.value.length;
  }
  if(operation==='insert_at_caret'?false:!focused||!atBoundary)return {ok:false,reason:'focus_or_selection_changed',value,length:value.length,focused,atBoundary,rich};
  value=read();
  return {ok:true,value,length:value.length,rich,focused,selectionStart,selectionEnd,expectedAnchor:expectedAnchor??null};
}`;

const NATIVE_SETTER = `function(expected,value){
  const e=this;
  if(!e||!e.isConnected)return {ok:false,reason:'target_stale'};
  if(e.value!==expected)return {ok:false,reason:'stale_value',value:e.value};
  const d=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value');
  d?.set?.call(e,value);
  e.dispatchEvent(new Event('input',{bubbles:true}));
  e.dispatchEvent(new Event('change',{bubbles:true}));
  return {ok:e.value===value,value:e.value,length:e.value.length};
}`;

function boundedInteger(value, fallback, min, max) {
  return Number.isSafeInteger(value) ? Math.max(min, Math.min(max, value)) : fallback;
}

export function validateTypeTextTransaction(params = {}) {
  const text = params.text;
  if (typeof text !== "string" || text.length < 1 || text.length > MAX_TRANSACTION_TEXT) {
    throw new Error("typeTextTransaction requires 1..16384 characters of text");
  }
  if (typeof params.object_id !== "string" && !Number.isSafeInteger(params.backend_node_id)) {
    throw new Error("typeTextTransaction requires object_id or backend_node_id");
  }
  if (typeof params.object_id === "string" && (params.object_id.length < 1 || params.object_id.length > 1024)) {
    throw new Error("typeTextTransaction object_id is invalid");
  }
  if (Number.isSafeInteger(params.backend_node_id) && params.backend_node_id <= 0) {
    throw new Error("typeTextTransaction backend_node_id must be positive");
  }
  if (params.frame_id !== undefined || params.loader_id !== undefined) {
    if (typeof params.frame_id !== "string" || !params.frame_id || params.frame_id.length > 256) {
      throw new Error("typeTextTransaction frame_id is invalid");
    }
    if (typeof params.loader_id !== "string" || !params.loader_id || params.loader_id.length > 256) {
      throw new Error("typeTextTransaction loader_id is invalid");
    }
  }
  const operation = params.operation ?? "append";
  if (!SUPPORTED_OPERATIONS.has(operation)) throw new Error("typeTextTransaction operation is unsupported");
  const strategy = params.strategy ?? "auto";
  if (!SUPPORTED_STRATEGIES.has(strategy)) throw new Error("typeTextTransaction strategy is unsupported");
  const expected = params.expected;
  if (!expected || typeof expected !== "object" || !["exact_value", "suffix", "caret_anchor", "unchecked"].includes(expected.kind)) {
    throw new Error("typeTextTransaction requires a supported expected state");
  }
  if (["exact_value", "suffix"].includes(expected.kind) && typeof expected.value !== "string") {
    throw new Error("typeTextTransaction expected value is required");
  }
  if (expected.kind === "exact_value" && expected.value.length > MAX_TRANSACTION_TEXT) {
    throw new Error("typeTextTransaction expected value exceeds bounds");
  }
  const normalize = params.normalize_paragraphs !== false;
  const chunkChars = boundedInteger(params.chunk_chars, 256, 1, 1024);
  const verifyEvery = boundedInteger(params.verify_every_chunks, 1, 0, 64);
  const pacingMs = boundedInteger(params.pacing_ms, 0, 0, 250);
  const deadlineMs = boundedInteger(params.deadline_ms, 30000, 1000, 600000);
  return {
    tabId: Number.isSafeInteger(params.tabId) ? params.tabId : null,
    objectId: params.object_id ?? null,
    backendNodeId: Number.isSafeInteger(params.backend_node_id) ? params.backend_node_id : null,
    frameId: params.frame_id ?? null,
    loaderId: params.loader_id ?? null,
    expected,
    operation,
    text,
    strategy,
    normalize,
    chunkChars,
    verifyEvery,
    pacingMs,
    deadlineMs,
  };
}

async function resolveObjectId(tabId, transaction, ctx) {
  if (transaction.objectId) return { objectId: transaction.objectId, sessionId: null, target: { tabId } };
  let sessionId = null;
  if (transaction.frameId) {
    sessionId = ctx.sessionForFrame(tabId, transaction.frameId);
    if (!sessionId && transaction.loaderId) {
      const cachedLoader = ctx.loaderForFrame(tabId, transaction.frameId);
      if (!cachedLoader) await ctx.syncFrameTree(tabId);
      sessionId = ctx.sessionForFrame(tabId, transaction.frameId);
    }
    if (!sessionId) {
      await ctx.ensureAutoAttach(tabId);
      sessionId = await ctx.waitForFrameSession(tabId, transaction.frameId, 400);
    }
  }
  const target = sessionId ? { tabId, sessionId } : { tabId };
  const resolved = await ctx.send(target, "DOM.resolveNode", { backendNodeId: transaction.backendNodeId });
  const objectId = resolved?.object?.objectId;
  if (!objectId) throw new Error("target node could not be resolved for the text transaction");
  return { objectId, sessionId, target };
}

async function verifyFrameLoader(tabId, transaction, ctx) {
  if (!transaction.frameId || !transaction.loaderId) return true;
  let cached = ctx.loaderForFrame(tabId, transaction.frameId);
  if (!cached) {
    await ctx.syncFrameTree(tabId);
    cached = ctx.loaderForFrame(tabId, transaction.frameId);
  }
  return cached === transaction.loaderId;
}

async function guard(ctx, target, objectId, transaction, values, prepare) {
  const response = await ctx.send(target, "Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: TRANSACTION_GUARD,
    arguments: [
      { value: values.kind },
      { value: values.value },
      { value: values.anchor ?? null },
      { value: transaction.operation },
      { value: prepare },
      { value: transaction.normalize },
    ],
    returnByValue: true,
  });
  return response?.result?.value ?? { ok: false, reason: "guard_response_missing" };
}

function expectedProgress(transaction, dispatched, baseValue = "") {
  if (transaction.operation === "append") return { kind: "exact_value", value: baseValue + dispatched };
  if (transaction.operation === "replace") return { kind: "exact_value", value: dispatched };
  return { kind: "contains", value: dispatched };
}

async function delay(ms) {
  if (ms > 0) await new Promise(resolve => setTimeout(resolve, ms));
}

export async function typeTextTransaction(tabId, params, ctx) {
  const transaction = validateTypeTextTransaction({ ...params, tabId });
  const startedAt = ctx.now();
  const deadline = startedAt + transaction.deadlineMs;
  const commandStart = ctx.commandCount();
  let inputCommands = 0;
  let cdpCommands = 0;
  let dispatchMayHaveOccurred = false;
  let resolved = null;
  let prepared = false;
  let dispatched = "";
  let lastReadback = typeof transaction.expected.value === "string" ? transaction.expected.value : null;
  const finish = extra => {
    cdpCommands = ctx.commandCount() - commandStart;
    return {
      status: extra.status,
      backend: extra.backend ?? "insert_text",
      operation: transaction.operation,
      completed_chars: extra.completed_chars ?? codePointLength(dispatched),
      chunks: extra.chunks ?? 0,
      cdp_commands: cdpCommands,
      input_commands: inputCommands,
      native_host_commands: 1,
      readback_length: typeof lastReadback === "string" ? lastReadback.length : null,
      readback_hash: typeof lastReadback === "string" ? hashText(lastReadback) : null,
      dispatch_may_have_occurred: dispatchMayHaveOccurred,
      interruption: extra.interruption ?? null,
      frame_id: transaction.frameId,
      loader_id: transaction.loaderId,
      session_id: resolved?.sessionId ?? null,
      reason: extra.reason,
    };
  };

  if (!ctx.isAuthorized(tabId)) {
    return finish({ status: "not_dispatched", reason: "session_not_authorized" });
  }
  try {
    if (!(await verifyFrameLoader(tabId, transaction, ctx))) {
      return finish({ status: "not_dispatched", reason: "frame_identity_stale" });
    }
    resolved = await resolveObjectId(tabId, transaction, ctx);
    const firstValue = transaction.expected.kind === "unchecked"
      ? { kind: "unchecked", value: "" }
      : { kind: transaction.expected.kind, value: transaction.expected.value ?? "", anchor: transaction.expected.anchor ?? null };
    const before = await guard(ctx, resolved.target, resolved.objectId, transaction, firstValue, true);
    cdpCommands = ctx.commandCount() - commandStart;
    if (before.ok !== true) {
      return finish({ status: "not_dispatched", reason: before.reason ?? "editor_guard_rejected" });
    }
    prepared = true;
    lastReadback = before.value;
    const initialValue = typeof before.value === "string" ? before.value : "";
    const probe = {
      tag: before.rich ? "DIV" : "TEXTAREA",
      contenteditable: before.rich === true,
      role: before.rich ? "textbox" : null,
      masked: false,
      requiresTrustedEvents: false,
      valueLength: before.length,
      selection: { start: before.selectionStart, end: before.selectionEnd },
    };
    const { adapter } = selectEditorAdapter(probe);
    const choice = chooseInputBackend({ strategy: transaction.strategy, text: transaction.text, probe });
    const backend = choice.backend;
    if (backend === "clipboard_paste" && ctx.sensitive === true) {
      return finish({ status: "not_dispatched", backend, reason: "clipboard_paste_refused_for_sensitive_text" });
    }
    if (backend === "native_setter") {
      if (before.rich) {
        return finish({ status: "not_dispatched", backend, reason: "native_setter_requires_plain_text_control" });
      }
      const base = typeof transaction.expected.value === "string" ? transaction.expected.value : before.value;
      const finalValue = transaction.operation === "append" ? base + transaction.text : transaction.text;
      const response = await ctx.send(resolved.target, "Runtime.callFunctionOn", {
        objectId: resolved.objectId,
        functionDeclaration: NATIVE_SETTER,
        arguments: [{ value: base }, { value: finalValue }],
        returnByValue: true,
      });
      const value = response?.result?.value;
      dispatchMayHaveOccurred = true;
      inputCommands += 1;
      lastReadback = value?.value ?? null;
      if (value?.ok !== true) {
        return finish({ status: "unknown", backend, reason: value?.reason ?? "native_setter_readback_failed" });
      }
      return finish({ status: "verified", backend, completed_chars: codePointLength(transaction.text), chunks: 1 });
    }

    const chunkSize = backend === "key_events"
      ? Math.max(1, Math.min(transaction.chunkChars, 32))
      : transaction.chunkChars;
    const chunks = backend === "ime_composition"
      ? graphemeChunks(transaction.text, chunkSize)
      : backend === "key_events"
        ? graphemeChunks(transaction.text, chunkSize)
        : graphemeChunks(transaction.text, chunkSize);
    if (!chunks.length) return finish({ status: "not_dispatched", backend, reason: "empty_text" });

    for (let index = 0; index < chunks.length; index += 1) {
      if (!ctx.isAuthorized(tabId) || ctx.now() >= deadline) {
        return finish({
          status: dispatchMayHaveOccurred ? "unknown" : "not_dispatched",
          backend,
          chunks: index,
          reason: dispatchMayHaveOccurred ? "transaction_deadline_or_session_change" : "deadline_or_session_change",
          interruption: { reason: "deadline_or_session_change", phase: "before_chunk" },
        });
      }
      const chunk = chunks[index];
      if (backend === "key_events") {
        for (const character of Array.from(chunk)) {
          const enter = character === "\n";
          const key = enter ? "Enter" : character;
          const down = enter
            ? { type: "keyDown", key, code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36, text: "\r", unmodifiedText: "\r" }
            : { type: "keyDown", key };
          await ctx.send(resolved.target, "Input.dispatchKeyEvent", down);
          dispatchMayHaveOccurred = true;
          inputCommands += 1;
          if (!enter) {
            await ctx.send(resolved.target, "Input.dispatchKeyEvent", { type: "char", key, text: character, unmodifiedText: character });
            inputCommands += 1;
          }
          await ctx.send(resolved.target, "Input.dispatchKeyEvent", enter
            ? { type: "keyUp", key, code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36 }
            : { type: "keyUp", key });
          inputCommands += 1;
          dispatched += character;
          if (transaction.pacingMs > 0) await delay(transaction.pacingMs);
        }
      } else if (backend === "ime_composition") {
        const codePoints = Array.from(chunk).length;
        await ctx.send(resolved.target, "Input.imeSetComposition", {
          text: chunk,
          selectionStart: codePoints,
          selectionEnd: codePoints,
        });
        dispatchMayHaveOccurred = true;
        inputCommands += 1;
        await ctx.send(resolved.target, "Input.insertText", { text: chunk });
        inputCommands += 1;
        dispatched += chunk;
      } else {
        await ctx.send(resolved.target, "Input.insertText", { text: chunk });
        dispatchMayHaveOccurred = true;
        inputCommands += 1;
        dispatched += chunk;
      }

      const shouldVerify = transaction.verifyEvery === 0
        ? index === chunks.length - 1
        : (index + 1) % transaction.verifyEvery === 0 || index === chunks.length - 1;
      if (shouldVerify) {
        const expected = expectedProgress(transaction, dispatched, initialValue);
        const after = await guard(ctx, resolved.target, resolved.objectId, transaction, expected, false);
        if (after.ok !== true) {
          lastReadback = after.value ?? lastReadback;
          return finish({
            status: "unknown",
            backend,
            chunks: index + 1,
            reason: after.reason ?? "chunk_readback_failed",
            interruption: { reason: after.reason ?? "chunk_readback_failed", phase: "readback", chunk: index },
          });
        }
        lastReadback = after.value;
      }
    }

    const finalExpected = transaction.operation === "insert_at_caret"
      ? { kind: "contains", value: transaction.text }
      : expectedProgress(transaction, dispatched, initialValue);
    const final = await guard(ctx, resolved.target, resolved.objectId, transaction, finalExpected, false);
    if (final.ok !== true) {
      lastReadback = final.value ?? lastReadback;
      return finish({
        status: dispatchMayHaveOccurred ? "unknown" : "not_dispatched",
        backend,
        chunks: chunks.length,
        reason: final.reason ?? "final_readback_failed",
        interruption: { reason: final.reason ?? "final_readback_failed", phase: "final_readback" },
      });
    }
    lastReadback = final.value;
    return {
      ...finish({ status: "verified", backend, chunks: chunks.length }),
      adapter: adapter.id,
      adapter_qualified: adapter.qualified === true,
      completed_chars: codePointLength(transaction.text),
    };
  } catch (error) {
    return finish({
      status: dispatchMayHaveOccurred ? "unknown" : "not_dispatched",
      reason: dispatchMayHaveOccurred ? "transaction_delivery_uncertain" : "transaction_preflight_failed",
      interruption: { reason: String(error?.message ?? error).slice(0, 200), phase: prepared ? "dispatch" : "preflight" },
    });
  }
}
