import { chromium } from 'playwright';
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
const baseline = execFileSync('git', ['show', '0ef14468ee2b2597b7a486d95fda758904f429de:crates/controlla-runtime/src/shared_page/snapshot.js'], {encoding:'utf8'});
const candidate = readFileSync('crates/controlla-runtime/src/shared_page/snapshot.js', 'utf8');
const browser = await chromium.launch({headless: true, ...(process.env.CONTROLLA_CHROME ? {executablePath: process.env.CONTROLLA_CHROME} : {channel:'chrome'})});
try {
 const page = await browser.newPage();
 await page.setContent('<button>Other</button>'.repeat(1000) + '<a href="/target">Target</a>');
 const results = [];
 for (let i = 0; i < 34; i++) {
  for (const name of i % 2 ? ['candidate','baseline'] : ['baseline','candidate']) {
   const result = await page.evaluate(({source}) => {
    let reads=0; const original=Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect=function(){ reads++; return original.call(this); };
    try {
     const started=performance.now();
     const out=new Function(`return (${source})`)()({selector:'body',token:'bench',max_items:20,max_text_chars:0,max_bytes:8192,max_scan_nodes:5000,goal:{query:'Target',roles:['link'],exhaustive:true}});
     return {ms:performance.now()-started,reads,items:out.items.map(x=>x.name),complete:out.public.scope_complete,scanned:out.public.scanned_nodes};
    } finally {Element.prototype.getBoundingClientRect=original;}
   }, {source:name === 'baseline' ? baseline : candidate});
   results.push({iteration:i,warmup:i<4,name,...result});
  }
 }
 writeFileSync(process.argv[2] || '/tmp/controlla-collector-paired.json', JSON.stringify(results,null,2));
 for (const name of ['baseline','candidate']) {
  const rows=results.filter(r=>r.name===name&&!r.warmup); const times=rows.map(r=>r.ms).sort((a,b)=>a-b);
  assert.equal(rows.filter(r=>r.items.length===1&&r.items[0]==='Target'&&r.complete).length, 30);
  console.log(JSON.stringify({name,n:rows.length,p50:(times[14]+times[15])/2,p95:times[28],reads:rows[0].reads,verified:rows.filter(r=>r.items.length===1&&r.items[0]==='Target'&&r.complete).length,scanned:rows[0].scanned}));
 }
} finally {await browser.close();}
