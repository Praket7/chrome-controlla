function(q, index) {
  if (q.kind === 'navigation') return false;
  const es = q.selector ? [...document.querySelectorAll(q.selector)] : [this?.nodes?.[index]].filter(Boolean);
  if (es.length !== 1) return false;
  const e = es[0], s = getComputedStyle(e), b = e.getBoundingClientRect();
  const visible = b.width>0 && b.height>0 && s.visibility==='visible' && s.display!=='none';
  const trustedText = e.hasAttribute('data-requires-trusted') && (e instanceof HTMLTextAreaElement || e instanceof HTMLInputElement && ['text','search','email','url','tel'].includes(e.type));
  const trusted = e.closest('[data-requires-trusted]');
  if (!e.isConnected || e.querySelector('[data-masked],[data-requires-trusted],input[type=password],input[type=file],input[type=hidden]') || e.closest('[data-masked],[hidden],[inert]') || trusted && (trusted!==e || !trustedText) || (e instanceof HTMLInputElement && ['password','hidden','file'].includes(e.type))) return false;
  if (q.kind === 'focused') return visible && document.activeElement === e;
  if (q.kind === 'visible') return visible;
  if (q.kind === 'text') return visible && (e.innerText || '').replace(/\s+/g,' ').trim() === q.text.replace(/\s+/g,' ').trim();
  if (q.kind === 'expanded') return visible && e.getAttribute('aria-expanded') === 'true';
  if (q.kind === 'selected') return visible && (e.getAttribute('aria-selected') === 'true' || e.getAttribute('aria-checked') === 'true');
  return false;
}
