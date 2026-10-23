import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, mkdir, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { evaluateAcceptance } from './phase11-verifier-state.mjs';

test('privileged HTTP readback accepts persisted bytes and rejects forgery, stale runs and persistence failures',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'oracle-http-')); const statePath=join(dir,'state.json'); const token='test-privileged-oracle-token';
 const server=spawn(process.execPath,['bench/fixtures/realistic/server.mjs','--port=0'],{env:{...process.env,PHASE11_ORACLE_TOKEN:token,PHASE11_STATE_FILE:statePath},stdio:['ignore','pipe','inherit']});
 try{
 const base=await new Promise((resolve,reject)=>{let buffer='';const timer=setTimeout(()=>reject(new Error('startup timeout')),5000);server.stdout.on('data',chunk=>{buffer+=chunk; if(!buffer.includes('\n'))return;clearTimeout(timer);resolve(JSON.parse(buffer.split('\n')[0]).base_url);});server.once('error',reject);});
 const headers={'content-type':'application/json',authorization:`Bearer ${token}`};
 const post=(path,value,auth=true)=>fetch(base+path,{method:'POST',headers:auth?headers:{'content-type':'application/json'},body:JSON.stringify(value)});
 const state=async(input)=>{const response=await fetch(`${base}/__state?task_id=${input.task_id}&run_id=${input.run_id}`,{headers});assert.equal(response.status,200);return response.json();};
 const input={task_id:'form-one',run_id:'run-one',task_class:'forms',acceptance:{kind:'form',name:'Alice',notes:'Saved'}};
 assert.equal((await post('/__reset',input,false)).status,403);
 assert.equal((await post('/__reset',input)).status,200);
 await assert.doesNotReject(async()=>{
  const malformed=await fetch(`${base}/accept/form?task=form-one&run=run-one`,{method:'POST',headers:{'content-type':'application/json'},body:'{broken'});
  assert.equal(malformed.status,400);
  assert.equal((await fetch(`${base}/__health`)).status,200);
  assert.equal((await state(input)).run_id,input.run_id);
 },'malformed queued request must return 400 without terminating the server');

 assert.equal((await fetch(`${base}/__state?task_id=form-one&run_id=run-one`)).status,403);
 await post('/__event',{...input,form:input.acceptance,kind:'form_saved',completed:true},false);
 assert.equal(evaluateAcceptance(input,await state(input)).passed,false);
 assert.equal((await post('/accept/form?task=form-one&run=wrong',input.acceptance,false)).status,409);
 assert.equal((await post('/accept/form?task=form-one&run=run-one',input.acceptance,false)).status,200);
 assert.equal(evaluateAcceptance(input,await state(input)).passed,true);
 const cli=async(script,payload,privileged=true)=>new Promise((resolve,reject)=>{
  const child=spawn(process.execPath,[script],{env:{...process.env,PHASE11_FIXTURE_BASE_URL:base,PHASE11_ORACLE_TOKEN:privileged?token:''},stdio:['pipe','pipe','pipe']});
  let output='';let errors='';child.stdout.on('data',chunk=>output+=chunk);child.stderr.on('data',chunk=>errors+=chunk);child.once('error',reject);child.once('close',code=>{if(code!==0)return reject(new Error(errors));try{resolve(JSON.parse(output));}catch(error){reject(error);}});child.stdin.end(JSON.stringify(payload));
 });
 assert.equal((await cli('bench/adapters/phase11-verifier.mjs',{input})).passed,true);
 assert.equal((await cli('bench/adapters/phase11-verifier.mjs',{input},false)).passed,false);

 assert.equal(JSON.parse(await readFile(statePath,'utf8'))['form-one'].form.name,'Alice');
 const page=await (await fetch(`${base}/registration-form?task=form-one`)).text(); assert.ok(page.includes('run-one'));assert.ok(!page.includes(token));
 const newer={...input,run_id:'run-two'};assert.equal((await cli('bench/adapters/phase11-reset.mjs',newer)).ok,true);
 assert.equal(evaluateAcceptance(input,await state(input)).passed,false);
 assert.equal((await post('/accept/form?task=form-one&run=run-one',input.acceptance,false)).status,409);
 assert.equal(evaluateAcceptance(newer,await state(newer)).passed,false);
 const bytes=Buffer.from('genuine upload bytes');const upload={task_id:'upload-one',run_id:'upload-run',task_class:'upload',acceptance:{kind:'upload',filename:'sample.bin',size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')}};
 await post('/__reset',upload);
 assert.equal((await fetch(`${base}/accept/upload?task=upload-one&run=upload-run&filename=sample.bin`,{method:'POST',body:bytes})).status,200);
 assert.equal(evaluateAcceptance(upload,await state(upload)).passed,true);
 assert.equal((await post('/accept/form?task=form-one&run=run-two',{name:'a'.repeat(4097),notes:''},false)).status,400);
 await rm(statePath);await mkdir(statePath);
 assert.equal((await post('/accept/form?task=form-one&run=run-two',input.acceptance,false)).status,500);
 assert.equal(evaluateAcceptance(newer,await state(newer)).passed,false);
 }finally{server.kill();await new Promise(resolve=>server.once('close',resolve));await rm(dir,{recursive:true,force:true});}
});
