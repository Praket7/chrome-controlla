#!/usr/bin/env node
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, rename } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash, randomBytes } from 'node:crypto';
import { validAcceptance } from '../../adapters/phase11-verifier-state.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const statePath = resolve(process.env.PHASE11_STATE_FILE ?? resolve(root, '.phase11-state.json'));
const port = Number(process.env.PHASE11_PORT ?? process.argv.find(v => v.startsWith('--port='))?.slice(7) ?? 0);
const tasks = new Map();
const token = process.env.PHASE11_ORACLE_TOKEN;
const identifier = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,128}$/.test(value);
const privileged = req => Boolean(token && req.headers.authorization === `Bearer ${token}`);
async function body(req, limit=65536) { const chunks=[]; let size=0; for await(const chunk of req){size+=chunk.length;if(size>limit)throw new Error('body too large');chunks.push(chunk);}return Buffer.concat(chunks); }
let queue=Promise.resolve();
function serialized(fn){ const next=queue.then(fn);queue=next.catch(()=>{});return next; }

const classes = new Map([
  ['forms', ['registration-form','rich-editor-1000-char-fastkeys','slow-media-early-form']],
  ['extraction', ['structured-extraction','50k-node-targeted-control','newest-ranking-50-candidates','on-demand-frame-escalation']],
  ['multitab', ['two-tab-compare','eight-tab-pressure']],
  ['spa', ['spa-document-change','stale-after-navigation']],
  ['visual_canvas', ['canvas-targeted-probe','seven-slide-capture-manifest']],
  ['virtual_list', ['virtual-list-find','virtual-list-1000-rows']],
  ['upload', ['guarded-upload','artifact-register-4mb','artifact-register-10mb']],
  ['repeated_workflow', ['repeat-qualified-flow','five-field-single-workflow']],
  ['interference', ['human-edit-race','react-rerender-target']],
  ['long_workflow', ['bounded-long-flow']],
  ['concurrent_tabs', ['eight-tab-pressure']],
  ['cross_origin_oopif', ['cross-site-oopif-target']],
  ['download_upload', ['download-register-upload']],
]);

function taskState(id) { return tasks.get(id) ?? { task_id: id, events: [], completed: false, reset_at: Date.now() }; }
async function persist() { await mkdir(dirname(statePath), { recursive: true }); await writeFile(statePath+'.tmp', JSON.stringify(Object.fromEntries(tasks), null, 2), {mode:0o600}); await rename(statePath+'.tmp',statePath); }
async function load() { try { const raw = JSON.parse(await readFile(statePath, 'utf8')); for (const [k,v] of Object.entries(raw)) tasks.set(k,v); } catch {} }
function json(res, status, value) { const body = JSON.stringify(value); res.writeHead(status, {'content-type':'application/json','content-length':Buffer.byteLength(body)}); res.end(body); }
function page(taskId, fixture, scenario, extractionNonce) {
  const title = `Controlla Phase 11 · ${taskId}`;
  const rows = Array.from({length: fixture === 'records-table' && scenario?.includes('50k') ? 50000 : 50}, (_, i) => `<tr><td>record-${i + 1}</td><td>${i % 2 ? 'older' : 'current'}</td>${taskId === 'structured-extraction' && i === 41 ? `<td>${extractionNonce}</td>` : ''}</tr>`).join('');
  const action = `Complete ${taskId}`;
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title><style>body{font:16px system-ui;margin:32px}#scroll{height:220px;overflow:auto;border:1px solid #888}canvas{border:1px solid #888;width:500px;height:140px}label,button,input{margin:8px;padding:8px}iframe{width:400px;height:100px}</style></head><body>
<h1>${title}</h1><p data-testid="fixture-description">Controlled benchmark fixture for ${fixture}.</p>
<form id="registration-form"><label>Name <input id="name" name="name"></label><label>Notes <textarea id="notes"></textarea></label><button type="submit">Save form</button></form>
<div id="records"><table><tbody>${rows}</tbody></table><button id="extract">Read requested record</button></div>
<div id="tabs"><a href="/fixture/${fixture}?task=${encodeURIComponent(taskId)}&tab=one">Tab one</a> <a href="/fixture/${fixture}?task=${encodeURIComponent(taskId)}&tab=two">Tab two</a></div>
<div id="spa"><button id="route">Navigate SPA route</button><output id="route-output"></output></div>
<canvas id="canvas" width="500" height="140"></canvas><button id="canvas-action">Probe canvas target</button>
<div id="scroll">${Array.from({length:1000},(_,i)=>`<div class="row">virtual-row-${i+1}</div>`).join('')}</div>
<input id="upload" type="file"><button id="upload-action">Register upload</button>
<div id="editor" contenteditable="true">Editable benchmark content</div>
<iframe title="child frame" src="/frame?task=${encodeURIComponent(taskId)}"></iframe>
<button id="complete" data-task="${taskId}">${action}</button><output id="status" aria-live="polite"></output>
<script>
const task=${JSON.stringify(taskId)}; const run=${JSON.stringify('RUN_PLACEHOLDER')}; const fixture=${JSON.stringify(fixture)};
async function save(path,data){const r=await fetch(path+'?task='+encodeURIComponent(task)+'&run='+encodeURIComponent(run),{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(data)});document.querySelector('#status').textContent=r.ok?'Saved':'Save failed';}
async function event(kind, extra={}){document.querySelector('#status').textContent=kind; await fetch('/__event',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({task_id:task,kind,fixture,...extra})});}
document.querySelector('#registration-form').addEventListener('submit',e=>{e.preventDefault(); save('/accept/form',{name:document.querySelector('#name').value,notes:document.querySelector('#notes').value});});
document.querySelector('#extract').onclick=()=>event('extracted',{count:1});
document.querySelector('#route').onclick=()=>{history.pushState({},'',location.pathname+'?task='+encodeURIComponent(task)+'&route=next'); document.querySelector('#route-output').textContent='route=next'; event('spa_navigated');};
document.querySelector('#canvas-action').onclick=()=>event('canvas_probed');
document.querySelector('#upload-action').onclick=async()=>{const f=document.querySelector('#upload').files[0];if(!f)return;const r=await fetch('/accept/upload?task='+encodeURIComponent(task)+'&run='+encodeURIComponent(run)+'&filename='+encodeURIComponent(f.name),{method:'POST',body:f});document.querySelector('#status').textContent=r.ok?'Upload saved':'Upload failed';};
document.querySelector('#complete').onclick=()=>event('completed',{completed:true});
window.addEventListener('scroll',()=>event('scrolled',{scrollY:scrollY}));
</script></body></html>`;
}

await load();
const server = createServer(async (req,res) => {
 try {
  const url = new URL(req.url, `http://${req.headers.host}`);
  if (req.method === 'GET' && url.pathname === '/__health') return json(res,200,{ok:true,service:'realistic-fixture'});
  if (url.pathname === '/__state' || url.pathname === '/__reset') {
    if (!privileged(req)) return json(res,403,{ok:false});
    if (req.method==='POST' && url.pathname==='/__reset') {
      const input=JSON.parse((await body(req)).toString());
      if(!identifier(input.task_id)||!identifier(input.run_id)||(input.acceptance!==undefined&&!validAcceptance(input.acceptance))) return json(res,400,{ok:false});
      await serialized(async()=>{if(!tasks.has(input.task_id)&&tasks.size>=256)tasks.delete(tasks.keys().next().value);const state={task_id:input.task_id,run_id:input.run_id,acceptance:input.acceptance,events:[],persisted:true,...(input.task_id==='structured-extraction'?{extraction_nonce:randomBytes(16).toString('hex')}:{})};tasks.set(input.task_id,state);try{await persist();}catch(error){tasks.delete(input.task_id);throw error;}});
      return json(res,200,{ok:true,task_id:input.task_id,run_id:input.run_id});
    }
    if(req.method==='GET' && url.pathname==='/__state') {
      await queue; const state=tasks.get(url.searchParams.get('task_id'));
      return json(res,200,state?.run_id===url.searchParams.get('run_id')?state:null);
    }
    return json(res,405,{ok:false});
  }
  if(req.method==='POST' && url.pathname==='/__event') { await body(req);return json(res,202,{ok:true,diagnostic_only:true}); }
  if(req.method==='POST' && ['/accept/form','/accept/upload'].includes(url.pathname)) {
    const bytes=await body(req,url.pathname.endsWith('/form')?32768:12*1024*1024); const id=url.searchParams.get('task'); const run=url.searchParams.get('run');
    return await serialized(async()=>{
      const current=tasks.get(id);if(!current || current.run_id!==run)return json(res,409,{ok:false});
      const kind=url.pathname.endsWith('/form')?'form':'upload';
      const value=kind==='form'?{kind,...JSON.parse(bytes.toString())}:{kind,filename:url.searchParams.get('filename'),size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};
      if(value.kind!==kind || !validAcceptance(value))return json(res,400,{ok:false});
      const next={...current,[kind]:value,persisted:true};tasks.set(id,next);
      try{await persist();}catch(error){tasks.set(id,{...current,persisted:false});return json(res,500,{ok:false});}
      return json(res,200,{ok:true});
    });
  }
  if (req.method === 'GET' && url.pathname === '/frame') { const id=url.searchParams.get('task');if(!identifier(id))return json(res,400,{ok:false});return res.end(`<html><body><p>Independent child frame for ${id}</p></body></html>`); }
  if (req.method === 'GET' && (url.pathname.startsWith('/fixture/') || ['registration-form','records-table','paired-tabs','spa-router','canvas-widget','virtual-list','file-input','workflow-suite','slide-deck'].includes(url.pathname.replace(/^\//,'')))) { const fixture=url.pathname.replace(/^\/|\/\$/g,'').replace(/^fixture\//,''); const taskId=url.searchParams.get('task') ?? fixture; if(!identifier(taskId)||!identifier(fixture))return json(res,400,{ok:false}); const current=taskState(taskId); return res.end(page(taskId, fixture, url.searchParams.get('scenario') ?? '', current.extraction_nonce ?? '').replace(JSON.stringify('RUN_PLACEHOLDER'),JSON.stringify(current.run_id??''))); }
  res.writeHead(404); res.end('not found');
 } catch { if(!res.headersSent)json(res,400,{ok:false,error:'invalid or oversized request'});else res.end(); }
});
server.listen(port,'127.0.0.1',()=>{const address=server.address(); console.log(JSON.stringify({ok:true,base_url:`http://127.0.0.1:${address.port}`}));});
