function(index) {
  const e = this.nodes[index], saved = this.items[index];
  if (!e || !saved || !e.isConnected || e.ownerDocument !== document) return {ok:false,reason:'stale_reference'};
  if (this.raw(e) !== saved.raw_value || this.name(e) !== saved.name || (e instanceof HTMLAnchorElement && e.href !== saved.href)) return {ok:false,reason:'changed_target'};
  e.scrollIntoView({block:'nearest',inline:'nearest'});
  const t = e.type || '', s = getComputedStyle(e), b = e.getBoundingClientRect();
  const x = b.left+b.width/2, y = b.top+b.height/2, h = document.elementFromPoint(x,y);
  const unsafe = __UNSAFE__;
  if (unsafe || !this.visible(e) || e.matches(':disabled') || e.getAttribute('aria-disabled')==='true' || (e.closest('[data-masked],[data-requires-trusted]') || e.querySelector('[data-masked],[data-requires-trusted]')) || s.pointerEvents==='none' || b.left<0 || b.top<0 || b.right>innerWidth || b.bottom>innerHeight || !h || !(h===e || e.contains(h))) return {ok:false,reason:'blocked'};
  return {ok:true,x,y};
}
