// Shared pointer/gesture planning. Geometry is always resolved immediately
// before trusted input; the model never has to locate scroll containers.

export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function resolveClickablePoint({ rect, viewport, rootPoint }) {
  if (!rect || !viewport) return null;
  const left = Math.max(rect.left, 0);
  const top = Math.max(rect.top, 0);
  const right = Math.min(rect.right, viewport.width);
  const bottom = Math.min(rect.bottom, viewport.height);
  if (!(right > left && bottom > top)) return null;
  const x = left + (right - left) / 2;
  const y = top + (bottom - top) / 2;
  return rootPoint ? { x: rootPoint.x + x, y: rootPoint.y + y, localX: x, localY: y } : { x, y, localX: x, localY: y };
}

export function resolveScrollPlan({ rect, viewport, containerRect = null, margin = 24 }) {
  if (!rect || !viewport) return { needed: false, reason: "missing_geometry" };
  const bounds = containerRect ?? { left: 0, top: 0, right: viewport.width, bottom: viewport.height };
  const visible = rect.top >= bounds.top + margin && rect.left >= bounds.left + margin
    && rect.bottom <= bounds.bottom - margin && rect.right <= bounds.right - margin;
  if (visible) return { needed: false };
  const dx = rect.right > bounds.right - margin ? rect.right - (bounds.right - margin)
    : rect.left < bounds.left + margin ? rect.left - (bounds.left + margin) : 0;
  const dy = rect.bottom > bounds.bottom - margin ? rect.bottom - (bounds.bottom - margin)
    : rect.top < bounds.top + margin ? rect.top - (bounds.top + margin) : 0;
  return { needed: dx !== 0 || dy !== 0, dx, dy };
}

export function interpolateDragPath(from, to, { steps = 8, maxSteps = 24 } = {}) {
  if (!from || !to) return [];
  const count = clamp(steps, 2, maxSteps);
  const path = [];
  for (let index = 1; index <= count; index += 1) {
    const fraction = index / count;
    path.push({
      x: from.x + (to.x - from.x) * fraction,
      y: from.y + (to.y - from.y) * fraction,
    });
  }
  return path;
}

export function clickBackendLadder({ semanticEquivalent, trustedRequired, frameLocal, unique }) {
  const ladder = [];
  if (semanticEquivalent && !trustedRequired) ladder.push("semantic_activation");
  ladder.push(frameLocal ? "frame_cdp_mouse" : "cdp_mouse");
  if (!unique) ladder.push("visual_grounded_coordinate");
  if (semanticEquivalent) ladder.push("keyboard_activation");
  return ladder;
}

export function touchPath(from, to, { steps = 6, maxSteps = 16 } = {}) {
  return interpolateDragPath(from, to, { steps, maxSteps });
}

// One trusted-input engine for root and child-frame routes. Callers own target
// identity validation and receipts; this module owns event vocabulary,
// ordering, interpolation, and release cleanup.
export class PointerEngine {
  constructor({ send } = {}) {
    if (typeof send !== "function") throw new Error("send is required");
    this.send = send;
  }

  async move(target, { x, y }, { button } = {}) {
    const event = { type: "mouseMoved", x, y };
    if (button) event.button = button;
    return this.send(target, "Input.dispatchMouseEvent", event);
  }

  async press(target, { x, y, button = "left", clickCount = 1 }) {
    return this.send(target, "Input.dispatchMouseEvent", { type: "mousePressed", button, clickCount, x, y });
  }

  async release(target, { x, y, button = "left", clickCount = 1 }) {
    return this.send(target, "Input.dispatchMouseEvent", { type: "mouseReleased", button, clickCount, x, y });
  }

  async click(target, point, { button = "left", clickCount = 1, move = true } = {}) {
    if (move) await this.move(target, point);
    await this.press(target, { ...point, button, clickCount });
    await this.release(target, { ...point, button, clickCount });
  }

  async wheel(target, point, { deltaX = 0, deltaY = 0 } = {}) {
    await this.move(target, point, { button: "none" });
    return this.send(target, "Input.dispatchMouseEvent", { type: "mouseWheel", x: point.x, y: point.y, deltaX, deltaY });
  }

  async touchStart(target, point) {
    return this.send(target, "Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ id: 1, x: point.x, y: point.y, radiusX: 1, radiusY: 1, force: 1 }],
    });
  }

  async touchMove(target, point) {
    return this.send(target, "Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ id: 1, x: point.x, y: point.y, radiusX: 1, radiusY: 1, force: 1 }],
    });
  }

  async touchEnd(target) {
    return this.send(target, "Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  }

  // Two-finger gestures need both contact points in one trusted CDP event.
  // Single-point helpers stay intact for ordinary taps and swipes.
  async touchMulti(target, type, points) {
    if (!["touchStart", "touchMove", "touchEnd"].includes(type)) throw new Error("invalid touch phase");
    if (!Array.isArray(points) || points.length < 1 || points.length > 10) throw new Error("touch point count is out of bounds");
    const touchPoints = points.map((point, index) => {
      if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)) throw new Error("touch point geometry is invalid");
      return {
        id: Number.isSafeInteger(point.id) ? point.id : index + 1,
        x: point.x, y: point.y,
        radiusX: 1, radiusY: 1, force: 1,
      };
    });
    return this.send(target, "Input.dispatchTouchEvent", { type, touchPoints });
  }

  async touch(target, type, touchPoints) {
    return this.send(target, "Input.dispatchTouchEvent", { type, touchPoints });
  }

  async swipe(target, from, to, { steps = 6 } = {}) {
    await this.touchStart(target, from);
    for (const point of touchPath(from, to, { steps })) await this.touchMove(target, point);
    await this.touchEnd(target);
  }

  async drag(target, from, to, { steps = 8, button = "left" } = {}) {
    await this.move(target, from);
    await this.press(target, { ...from, button, clickCount: 1 });
    for (const point of interpolateDragPath(from, to, { steps })) await this.move(target, point);
    await this.release(target, { ...to, button, clickCount: 1 });
  }
}
