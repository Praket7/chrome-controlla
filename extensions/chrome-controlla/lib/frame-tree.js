// Event-driven frame/document/loader registry for one debugger attachment.
// `Page.getFrameTree` hydrates or recovers this graph; it is not the default
// per-action lookup once the graph is warm.

export function flattenFrameTree(tree) {
  const output = [];
  const visit = (node, parentId, depth) => {
    if (!node?.frame?.id) return;
    const frame = node.frame;
    output.push({
      frame_id: frame.id,
      loader_id: frame.loaderId,
      parent_frame_id: frame.parentId ?? parentId ?? null,
      url: frame.url,
      security_origin: frame.securityOrigin,
      depth,
    });
    for (const child of node.childFrames ?? []) visit(child, frame.id, depth + 1);
  };
  visit(tree, null, 0);
  return output;
}

export function findFrameInTree(tree, frameId) {
  if (!tree) return null;
  let found = null;
  const visit = (node, parentId) => {
    if (found || !node?.frame) return;
    if (node.frame.id === frameId) {
      found = {
        frame_id: node.frame.id,
        loader_id: node.frame.loaderId,
        parent_frame_id: node.frame.parentId ?? parentId ?? null,
        url: node.frame.url,
        security_origin: node.frame.securityOrigin,
      };
      return;
    }
    for (const child of node.childFrames ?? []) visit(child, node.frame.id);
  };
  visit(tree, null);
  return found;
}

export class FrameTreeRegistry {
  constructor({ now = () => Date.now() } = {}) {
    this.now = now;
    this.tabs = new Map();
  }

  ensureTab(tabId) {
    let state = this.tabs.get(tabId);
    if (!state) {
      state = {
        generation: 0,
        frames: new Map(),
        root_frame_id: null,
        document_id: null,
        last_synced_at_ms: 0,
        auto_attach: false,
        attached: false,
      };
      this.tabs.set(tabId, state);
    }
    return state;
  }

  hydrate(tabId, tree, sessionId = null) {
    const state = this.ensureTab(tabId);
    state.generation += 1;
    state.frames.clear();
    state.root_frame_id = tree?.frame?.id ?? null;
    for (const frame of flattenFrameTree(tree)) {
      state.frames.set(frame.frame_id, {
        ...frame,
        session_id: frame.frame_id === state.root_frame_id ? sessionId : null,
        last_seen_at_ms: this.now(),
      });
    }
    state.last_synced_at_ms = this.now();
    return state;
  }

  upsert(tabId, frame) {
    if (!frame?.frame_id) return null;
    const state = this.ensureTab(tabId);
    const previous = state.frames.get(frame.frame_id);
    const value = {
      frame_id: frame.frame_id,
      loader_id: frame.loader_id ?? previous?.loader_id ?? null,
      parent_frame_id: frame.parent_frame_id ?? previous?.parent_frame_id ?? null,
      url: frame.url ?? previous?.url,
      security_origin: frame.security_origin ?? previous?.security_origin,
      depth: frame.depth ?? previous?.depth ?? 0,
      session_id: frame.session_id ?? previous?.session_id ?? null,
      last_seen_at_ms: this.now(),
    };
    state.frames.set(value.frame_id, value);
    if (!state.root_frame_id && !value.parent_frame_id) state.root_frame_id = value.frame_id;
    return value;
  }

  remove(tabId, frameId) {
    const state = this.tabs.get(tabId);
    if (!state) return;
    state.frames.delete(frameId);
    state.generation += 1;
    if (state.root_frame_id === frameId) state.root_frame_id = null;
  }

  bindSession(tabId, frameId, sessionId) {
    const state = this.ensureTab(tabId);
    const existing = state.frames.get(frameId) ?? {
      frame_id: frameId,
      loader_id: null,
      parent_frame_id: null,
      depth: 0,
      last_seen_at_ms: this.now(),
    };
    existing.session_id = sessionId;
    existing.last_seen_at_ms = this.now();
    state.frames.set(frameId, existing);
    return existing;
  }

  frame(tabId, frameId) {
    return this.tabs.get(tabId)?.frames.get(frameId) ?? null;
  }

  sessionForFrame(tabId, frameId) {
    return this.frame(tabId, frameId)?.session_id ?? null;
  }

  loaderForFrame(tabId, frameId) {
    return this.frame(tabId, frameId)?.loader_id ?? null;
  }

  rootFrameId(tabId) {
    return this.tabs.get(tabId)?.root_frame_id ?? null;
  }

  frames(tabId) {
    return [...(this.tabs.get(tabId)?.frames.values() ?? [])];
  }

  markAutoAttach(tabId, value = true) {
    this.ensureTab(tabId).auto_attach = value === true;
  }

  hasAutoAttach(tabId) {
    return this.tabs.get(tabId)?.auto_attach === true;
  }

  invalidateFrame(tabId, frameId, reason = "unknown") {
    const state = this.tabs.get(tabId);
    if (!state) return;
    const frame = state.frames.get(frameId);
    if (frame) {
      frame.loader_id = null;
      frame.session_id = null;
      frame.invalidated_reason = reason;
      frame.last_seen_at_ms = this.now();
    }
    state.generation += 1;
  }

  invalidateTab(tabId, reason = "unknown") {
    const state = this.tabs.get(tabId);
    if (!state) return;
    state.frames.clear();
    state.generation += 1;
    state.auto_attach = false;
    state.attached = false;
    state.invalidated_reason = reason;
    state.last_synced_at_ms = this.now();
  }

  release(tabId) {
    this.tabs.delete(tabId);
  }
}
