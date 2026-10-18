function(index, action, expected, next, key) {
  const e = this.nodes[index];
  const refuse = reason => ({ ok: false, dispatched: false, reason });
  const sensitive = () => e instanceof HTMLInputElement && !['text', 'search', 'email', 'url', 'tel', 'number', 'button', 'submit', 'reset', 'checkbox', 'radio'].includes(e.type)
    || e.closest('[data-masked],[data-requires-trusted]');
  if (!e?.isConnected || e.ownerDocument !== document) return refuse('target_replaced');
  if (sensitive()) return refuse('sensitive_or_trusted_target');
  if (e.matches(':disabled,[aria-disabled="true"],[inert]') || e.closest('[inert]')) return refuse('blocked');
  const valueOf = () => {
    if (!e.isConnected || sensitive()) return undefined;
    return e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement || e instanceof HTMLSelectElement
      ? e.value : e.isContentEditable ? e.textContent : undefined;
  };
  const before = valueOf();
  let dispatched = false;
  try {
    if (['fill', 'type', 'select'].includes(action)) {
      if (typeof next !== 'string') return refuse('value_required');
      if (e.readOnly || e.getAttribute('aria-readonly') === 'true') return refuse('not_editable');
      if (before !== (expected ?? '')) return refuse('same_field_conflict');
      if (action === 'select') {
        if (!(e instanceof HTMLSelectElement)) return refuse('not_select');
        if (![...e.options].some(o => o.value === next && !o.disabled && !o.parentElement?.disabled)) return refuse('option_missing');
        dispatched = true;
        e.value = next;
      } else {
        const proto = e instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype
          : e instanceof HTMLInputElement && ['text', 'search', 'email', 'url', 'tel', 'number'].includes(e.type) ? HTMLInputElement.prototype : null;
        if (proto) {
          const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
          if (!setter) return refuse('missing_value_setter');
          dispatched = true;
          setter.call(e, next);
        } else if (e.isContentEditable) {
          dispatched = true;
          e.textContent = next;
        } else return refuse('not_editable');
      }
      e.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: next }));
      e.dispatchEvent(new Event('change', { bubbles: true }));
      const after = valueOf();
      const verified = after !== undefined && after === next;
      return { ok: verified, verified, dispatched, before, after, transport: 'dom_value', trusted_input: false };
    }
    if (['click', 'double_click', 'context_click'].includes(action)) {
      const eventType = action === 'double_click' ? 'dblclick' : action === 'context_click' ? 'contextmenu' : 'click';
      let eventSeen = false;
      const mark = () => { eventSeen = true; };
      e.addEventListener(eventType, mark, { once: true });
      try {
        dispatched = true;
        if (action === 'context_click') e.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2, view: window }));
        else {
          e.click();
          if (action === 'double_click') {
            e.click();
            e.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true, detail: 2, view: window }));
          }
        }
      } finally { e.removeEventListener(eventType, mark); }
      return { ok: eventSeen, dispatched, event_seen: eventSeen,
        expanded_after: e.isConnected && e.getAttribute('aria-expanded') === 'true',
        selected_after: e.isConnected && (e.getAttribute('aria-selected') === 'true' || e.getAttribute('aria-checked') === 'true'),
        focused_after: document.activeElement === e, transport: 'dom_activation', trusted_input: false };
    }
    // Synthetic key events do not perform browser default actions (Enter/Tab).
    // Route these through trusted key input instead of claiming an effect.
    return refuse(action === 'press' ? 'trusted_key_route_required' : 'unsupported_background_action');
  } catch (_) {
    return { ok: false, dispatched, reason: dispatched ? 'background_effect_uncertain' : 'background_preflight_failed' };
  }
}
