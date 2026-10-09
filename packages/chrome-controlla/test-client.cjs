'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test, after } = require('node:test');

const client = path.join(__dirname, 'bin', 'controlla-client.cjs');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'controlla-client-test-'));
after(() => fs.rmSync(dir, {recursive:true,force:true}));
const fake = path.join(dir, 'fake-server.cjs');
fs.writeFileSync(fake, String.raw`
const readline=require('node:readline');
const rl=readline.createInterface({input:process.stdin});
let calls=0;
rl.on('line',line=>{const m=JSON.parse(line);let result;
if(m.method==='initialize') result={protocolVersion:'2026-07-28',capabilities:{},serverInfo:{name:'fixture',version:'1'}};
else if(m.method==='notifications/initialized') return;
else if(m.method==='tools/list'){calls++;if(calls>1)throw Error('schemas requested twice');result={tools:[{name:'sum',inputSchema:{type:'object',required:['a','b'],properties:{a:{type:'number'},b:{type:'number'}}}}]};}
else if(m.method==='tools/call') result={content:[{type:'text',text:String(m.params.arguments.a+m.params.arguments.b)}]};
else if(m.method==='shutdown'){process.exit(0);}
else result={};
if(m.id!==undefined)process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result})+'\n');
});
`);

test('client initializes once, lists cached schemas, validates locally, and routes tool calls', async () => {
  const child = spawn(process.execPath, [client, '--server', process.execPath, '--server-arg', fake], { stdio: ['pipe','pipe','pipe'] });
  let output='', error=''; child.stdout.setEncoding('utf8'); child.stdout.on('data',s=>output+=s); child.stderr.on('data',s=>error+=s);
  child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/list',params:{}})+'\n');
  child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:2,method:'tools/call',params:{name:'sum',arguments:{a:2,b:3}}})+'\n');
  child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:3,method:'tools/call',params:{name:'sum',arguments:{a:2}}})+'\n');
  child.stdin.end(); const [code]=await once(child,'close'); assert.equal(code,0,error);
  const rows=output.trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(rows.length,3);
  assert.equal(rows[0].id,1); assert.equal(rows[0].result.tools[0].name,'sum');
  assert.equal(rows[1].id,2); assert.equal(rows[1].result.content[0].text,'5');
  assert.equal(rows[2].id,3); assert.match(rows[2].error.message,/b/);
});

test('client reports server stderr without corrupting JSONL replies', async () => {
  const noisy = path.join(dir, 'noisy.cjs');
  fs.writeFileSync(noisy, "process.stderr.write('fixture diagnostic\\n');require('node:readline').createInterface({input:process.stdin}).on('line',l=>{const m=JSON.parse(l);if(m.id!==undefined)process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result:m.method==='initialize'?{protocolVersion:'2026-07-28',capabilities:{},serverInfo:{name:'fixture',version:'1'}}:{tools:[]}})+'\\n')})");
  const child=spawn(process.execPath,[client,'--server',process.execPath,'--server-arg',noisy],{stdio:['pipe','pipe','pipe']});
  let err=''; child.stderr.on('data',s=>err+=s);
  let out=''; child.stdout.setEncoding('utf8'); child.stdout.on('data',s=>out+=s); child.stdin.end(); const [code]=await once(child,'close'); assert.equal(code,0); assert.match(err,/fixture diagnostic/);
  assert.doesNotThrow(()=>out.trim().split(/\r?\n/).filter(Boolean).map(JSON.parse));
});

async function fixture(t, name, behavior, requests, timeout=1000, keepOpen=false) {
  const script=path.join(dir,name+'.cjs');
  fs.writeFileSync(script, `const rl=require('node:readline').createInterface({input:process.stdin});
const reply=(m,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result})+'\\n');
rl.on('line',line=>{const m=JSON.parse(line);
if(m.method==='initialize') reply(m,{protocolVersion:'2024-11-05',serverInfo:{name:'fixture',version:'1'},capabilities:{}});
else if(m.method==='tools/list') reply(m,{tools:[{name:'act',inputSchema:{type:'object',properties:{},additionalProperties:false}}]});
else if(m.method==='tools/call'){${behavior}}});`);
  const child=spawn(process.execPath,[client,'--server',process.execPath,'--server-arg',script,'--timeout-ms',String(timeout)],{stdio:['pipe','pipe','pipe']});
  t.after(()=>child.kill('SIGKILL'));
  let output='',error=''; child.stdout.on('data',s=>output+=s);child.stderr.on('data',s=>error+=s);
  for(const req of requests) child.stdin.write(JSON.stringify(req)+'\n');
  if(!keepOpen)child.stdin.end();
  const [code]=await once(child,'close');
  return {code,error,rows:output.trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)};
}
const action=id=>({jsonrpc:'2.0',id,method:'tools/call',params:{name:'act',arguments:{}}});

test('timeout reports unknown and never repeats an action', {timeout:5000}, async t=>{
  const count=path.join(dir,'dispatches');
  const result=await fixture(t,'timeout',`require('node:fs').appendFileSync(${JSON.stringify(count)},'1');`,[action(1)],100);
  assert.equal(result.code,0);assert.match(result.rows[0].error.message,/outcome unknown/);assert.equal(fs.readFileSync(count,'utf8'),'1');
});
test('tool errors remain tool errors and duplicated content is removed', {timeout:5000}, async t=>{
  const result=await fixture(t,'tool-error',`reply(m,{isError:true,structuredContent:{reason:'blocked'},content:[{type:'text',text:'blocked'}]});`,[42,action(2)]);
  assert.equal(result.rows[0].error.code,-32600);assert.equal(result.rows[1].result.isError,true);assert.equal(result.rows[1].result.structuredContent.reason,'blocked');assert.equal(result.rows[1].result.content,undefined);
});
test('schema change refuses queued action instead of using stale schema', {timeout:5000}, async t=>{
  const result=await fixture(t,'schema-change',`process.stdout.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/tools/list_changed'})+'\\n');reply(m,{content:[]});`,[action(1),action(2)]);
  assert.match(result.rows[1].error.message,/schemas changed/);
});
test('unexpected child exit closes client even with input still open', {timeout:5000}, async t=>{
  const result=await fixture(t,'exit',`process.exit(7);`,[action(1)],1000,true);
  assert.equal(result.code,1);assert.match(result.rows[0].error.message,/exited/);
});
test('failed startup exits even with input still open', {timeout:5000}, async t=>{
  const child=spawn(process.execPath,[client,'--server',path.join(dir,'missing-server')],{stdio:['pipe','pipe','pipe']});
  t.after(()=>child.kill('SIGKILL'));
  let error='';child.stderr.on('data',s=>error+=s);const [code]=await once(child,'close');assert.equal(code,1);assert.match(error,/initialization failed/);
});
