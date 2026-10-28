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
