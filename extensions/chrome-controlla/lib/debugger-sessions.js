// One debugger/session manager per bound tab. Auto-attach is enabled once and
// kept warm; it is only disabled on explicit release, tab closure, or debugger
// replacement. Recovery reads the frame tree but does not toggle auto-attach.

import { findFrameInTree } from "./frame-tree.js";

const IFRAME_FILTER = [{ type: "iframe", exclude: false }];

export class DebuggerSessionManager {
  constructor({ send, frames, attach, detach, now = () => Date.now() } = {}) {
    if (typeof send !== "function") throw new Error("send is required");
    if (!frames) throw new Error("frames registry is required");
    this.send = send;
    this.frames = frames;
    this.attachImpl = attach ?? (async () => {});
    this.detachImpl = detach ?? (async () => {});
    this.now = now;
    this.state = new Map();
  }

  ensureState(tabId) {
    let state = this.state.get(tabId);
    if (!state) {
      state = {
        attached: false,
        attach_generation: 0,
        auto_attach: false,
        root_session_id: null,
        last_event_at_ms: 0,
        detach_reason: null,
      };
      this.state.set(tabId, state);
    }
    return state;
  }

  async ensureAttached(tabId) {
    const state = this.ensureState(tabId);
    if (state.attached) return state;
    await this.attachImpl(tabId);
    state.attached = true;
    state.attach_generation += 1;
    state.detach_reason = null;
    this.frames.ensureTab(tabId).attached = true;
    return state;
  }

  markAttached(tabId) {
    const state = this.ensureState(tabId);
    state.attached = true;
    this.frames.ensureTab(tabId).attached = true;
  }

  async ensureAutoAttach(tabId, { sessionId = null } = {}) {
    const state = this.ensureState(tabId);
    if (!sessionId && state.auto_attach) return false;
    if (sessionId) {
      state.child_auto_attach = state.child_auto_attach ?? new Set();
      if (state.child_auto_attach.has(sessionId)) return false;
    }
    const target = sessionId ? { tabId, sessionId } : { tabId };
    await this.send(target, "Target.setAutoAttach", {
      autoAttach: true,
      waitForDebuggerOnStart: false,
      flatten: true,
      filter: IFRAME_FILTER,
    });
    if (sessionId) {
      state.child_auto_attach.add(sessionId);
    } else {
      state.auto_attach = true;
      state.attached = true;
      this.frames.markAutoAttach(tabId, true);
    }
    return true;
  }

  sessionForFrame(tabId, frameId) {
    return this.frames.sessionForFrame(tabId, frameId) ?? this.ensureState(tabId).root_session_id;
  }

  currentDocument(tabId) {
    return this.frames.ensureTab(tabId).document_id ?? null;
  }

  setCurrentDocument(tabId, documentId) {
    this.frames.ensureTab(tabId).document_id = documentId;
  }

  handleDebuggerEvent(source, method, params = {}) {
    const tabId = source?.tabId;
    if (!Number.isSafeInteger(tabId)) return;
    const state = this.ensureState(tabId);
    state.last_event_at_ms = this.now();
    const sessionId = params.sessionId ?? source?.sessionId ?? null;
    if (method === "Target.attachedToTarget") {
      const target = params.targetInfo ?? {};
      const frameId = target.targetId ?? params.frameId ?? null;
      state.attached = true;
      if (!sessionId) return;
      if (target.type === "page" && !state.root_session_id) state.root_session_id = sessionId;
      if (frameId && target.type === "iframe") {
        this.frames.bindSession(tabId, frameId, sessionId);
      }
      // Child targets receive nested OOPIF discovery exactly once.
      void this.ensureAutoAttach(tabId, { sessionId }).catch(() => {});
    } else if (method === "Target.detachedFromTarget") {
      if (sessionId && state.root_session_id === sessionId) state.root_session_id = null;
      for (const frame of this.frames.frames(tabId)) {
        if (frame.session_id === sessionId) this.frames.invalidateFrame(tabId, frame.frame_id, "target_detached");
      }
    } else if (method === "Target.targetDestroyed") {
      const frameId = params.targetId;
      if (frameId) this.frames.remove(tabId, frameId);
    } else if (method === "Page.frameAttached") {
      this.frames.upsert(tabId, {
        frame_id: params.frameId,
        parent_frame_id: params.parentFrameId ?? null,
        loader_id: null,
      });
    } else if (method === "Page.frameNavigated") {
      const frame = params.frame ?? {};
      if (frame.id) {
        this.frames.upsert(tabId, {
          frame_id: frame.id,
          loader_id: frame.loaderId,
          parent_frame_id: frame.parentId ?? null,
          url: frame.url,
          security_origin: frame.securityOrigin,
          session_id: sessionId ?? this.frames.sessionForFrame(tabId, frame.id),
        });
        if (!frame.parentId) {
          this.setCurrentDocument(tabId, frame.loaderId ?? null);
          state.root_session_id = state.root_session_id ?? sessionId ?? null;
        }
      }
    } else if (method === "Page.frameDetached") {
      this.frames.remove(tabId, params.frameId);
    } else if (method === "Runtime.executionContextCreated") {
      const aux = params.context?.auxData ?? {};
      if (aux.frameId && aux.isDefault === true) {
        this.frames.upsert(tabId, {
          frame_id: aux.frameId,
          loader_id: aux.loaderId ?? this.frames.loaderForFrame(tabId, aux.frameId),
          session_id: sessionId ?? this.frames.sessionForFrame(tabId, aux.frameId),
        });
      }
    }
  }

  async syncFrameTree(tabId, { sessionId = null } = {}) {
    const target = sessionId ? { tabId, sessionId } : { tabId };
    const response = await this.send(target, "Page.getFrameTree", {});
    const tree = response?.frameTree;
    if (!tree) return null;
    this.frames.hydrate(tabId, tree, sessionId);
    if (!sessionId) {
      this.frames.ensureTab(tabId).attached = true;
      this.ensureState(tabId).attached = true;
    }
    return tree;
  }

  async frameLoader(tabId, frameId, { refresh = false } = {}) {
    const cached = this.frames.loaderForFrame(tabId, frameId);
    if (cached && !refresh) return cached;
    const tree = await this.syncFrameTree(tabId);
    if (tree) return findFrameInTree(tree, frameId)?.loader_id ?? null;
    return this.frames.loaderForFrame(tabId, frameId);
  }

  invalidateFrame(tabId, frameId, reason = "unknown") {
    this.frames.invalidateFrame(tabId, frameId, reason);
  }

  invalidateTab(tabId, reason = "unknown") {
    this.state.delete(tabId);
    this.frames.invalidateTab(tabId, reason);
  }

  async release(tabId) {
    const state = this.state.get(tabId);
    this.state.delete(tabId);
    this.frames.release(tabId);
    if (!state) return;
    await this.detachImpl(tabId).catch(() => {});
  }
}
