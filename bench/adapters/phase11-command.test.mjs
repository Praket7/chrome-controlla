import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadAdapterContract, validateAdapterResult } from '../runners/phase11-runner.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const adapterPath = resolve(root, 'bench/adapters/phase11-command.mjs');

test('unsupported adapter task classes report missing evidence and leave safety counters unknown', async () => {
  const input = { task_id: 'eight-tab-pressure', task_class: 'concurrent_tabs', mode: 'cold_browser', execution_mode: 'headless', concurrency: 1 };
  const output = await invoke(input);
  assert.match(output.runner_error, /not an independent task acceptance oracle/);
  for (const field of ['focus_disruptions', 'clipboard_disruptions', 'human_interventions', 'severe_wrong_target_events', 'authority_violations', 'silent_stale_mutations']) {
    assert.equal(output[field], null, `${field} is uninstrumented and must remain missing`);
    assert.ok(output.metric_evidence[field].missing_reason);
  }
  assert.equal(output.success, false);
  assert.equal(output.independently_verified_success, false);
  assert.doesNotThrow(() => validateAdapterResult(output, awaitContract, {
    adapter: 'playwright-mcp', task_id: input.task_id, mode: input.mode,
    execution_mode: input.execution_mode, concurrency: input.concurrency,
  }));
});

const awaitContract = await loadAdapterContract();

function invoke(input) {
  return new Promise((resolveOutput, reject) => {
    const child = spawn(process.execPath, [adapterPath, 'playwright-mcp'], {
      cwd: root, env: { ...process.env, PHASE11_FIXTURE_BASE_URL: 'http://127.0.0.1:1' },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', code => {
      if (code !== 0) return reject(new Error(stderr || `adapter exited ${code}`));
      try { resolveOutput(JSON.parse(stdout)); } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify(input));
  });
}

test('generic extraction is unqualified without independent fixture acceptance', async () => {
  const output = await invoke({ task_id:'50k-node-targeted-control', task_class:'extraction', mode:'cold_browser', execution_mode:'headless', concurrency:1 });
  assert.equal(output.success,false);
  assert.equal(output.independently_verified_success,false);
  assert.match(output.runner_error,/not an independent task acceptance oracle/);
});

test('extraction actor requires an exact bound observed row', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-extraction-'));
  try {
    const executable=join(dir,'codex');
    const input={task_id:'structured-extraction',task_class:'extraction',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/records-table?task=structured-extraction',acceptance:{kind:'extraction',row:'record-42',status:'older'},deadline_ms:60000};
    const nonce='a'.repeat(32), answer=`record-42|older|${nonce}`;
    for (const scenario of ['valid','wrong_url','no_observation','wrong_final']) {
      const url=scenario==='wrong_url'?'http://127.0.0.1:1234/other':input.fixture_url;
      const events=[...(scenario==='no_observation'?[]:[{type:'item.completed',item:{type:'mcp_tool_call',server:'chrome-controlla',tool:'browser',status:'completed',arguments:{action:'task',task:{kind:'research',selector:'#records tbody tr:nth-child(42)',urls:[input.fixture_url]}},result:{structured_content:{status:'completed',receipts:[{kind:'extract',url,snippet:`record-42 older ${nonce}`}]}}}}]),{type:'item.completed',item:{type:'agent_message',text:scenario==='wrong_final'?'record-42|older':answer}}];
      await writeFile(executable, `#!/usr/bin/env node\nfor(const e of ${JSON.stringify(events)}) console.log(JSON.stringify(e));\n`,{mode:0o755});
      const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'controlla'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_CONTROLLA_BINARY:'/controlla'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
      assert.equal(output.success,scenario==='valid');
      assert.equal(output.independently_verified_success,false);
    }
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('cold adapter distinguishes host exit from deadline', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-host-failure-'));
  try {
    const executable = join(dir, 'codex');
    const input = {task_id:'form-guarded-fill', task_class:'forms', mode:'cold_browser', execution_mode:'headless', concurrency:1, fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill', acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'}, deadline_ms:3000};
    for (const [script, expected] of [['process.exit(7);', 'Codex host exited with code 7'], ['setInterval(() => {}, 1000);', 'Codex host timed out']]) {
      await writeFile(executable, `#!/usr/bin/env node\n${script}\n`, {mode:0o755});
      const output = await new Promise((resolveOutput, reject) => {
        const child = spawn(process.execPath, [adapterPath, 'playwright-mcp'], {cwd:root, env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome'}, stdio:['pipe','pipe','pipe']});
        let stdout='', stderr=''; child.stdout.on('data', chunk => stdout += chunk); child.stderr.on('data', chunk => stderr += chunk);
        child.once('error', reject); child.once('close', code => code === 0 ? resolveOutput(JSON.parse(stdout)) : reject(new Error(stderr))); child.stdin.end(JSON.stringify(input));
      });
      assert.equal(output.runner_error, expected);
    }
  } finally { await rm(dir, {recursive:true, force:true}); }
});

test('cold form adapter runs the configured Codex host without oracle secrets', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'phase11-host-test-'));
  try {
    const executable = join(dir, 'codex');
    await writeFile(executable, `#!/usr/bin/env node\nif (process.env.PHASE11_ORACLE_TOKEN || process.env.PHASE11_STATE_FILE || process.env.PRIVATE_SENTINEL || !['shell_tool','unified_exec','apps','plugins','remote_plugin','--approve-for-me','mcp_servers.playwright-mcp.startup_timeout_sec=120'].every(x=>process.argv.includes(x))) process.exit(9);\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_navigate',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`, { mode: 0o755 });
    const input = { task_id:'form-guarded-fill', task_class:'forms', mode:'cold_browser', execution_mode:'headless', concurrency:1, fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill', acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'}, deadline_ms:60000 };
    const output = await new Promise((resolveOutput, reject) => {
      const child = spawn(process.execPath, [adapterPath, 'playwright-mcp'], {cwd:root, env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_ORACLE_TOKEN:'secret',PHASE11_STATE_FILE:'secret',PRIVATE_SENTINEL:'secret'}, stdio:['pipe','pipe','pipe']});
      let stdout='', stderr=''; child.stdout.on('data',chunk=>stdout+=chunk); child.stderr.on('data',chunk=>stderr+=chunk);
      child.once('error',reject); child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr))); child.stdin.end(JSON.stringify(input));
    });
    assert.equal(output.success, true);
    assert.equal(output.independently_verified_success, false);
    assert.equal(output.mcp_calls, 1);
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('cold form adapter refuses non-loopback fixture URLs before spawning', async () => {
  const input = {task_id:'form-guarded-fill', task_class:'forms', mode:'cold_browser', execution_mode:'headless', concurrency:1, fixture_url:'https://example.com/registration-form?task=form-guarded-fill', acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'}};
  const code = await new Promise((resolveCode, reject) => {
    const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234'},stdio:['pipe','ignore','ignore']});
    child.once('error',reject); child.once('close',resolveCode); child.stdin.end(JSON.stringify(input));
  });
  assert.notEqual(code,0);
});

test('cold upload adapter launches a client without claiming server acceptance', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-upload-test-'));
  try {
    const executable=join(dir,'codex');
    await writeFile(executable, `#!/usr/bin/env node\nif (!process.argv.at(-1).includes('Call browser_snapshot') || !process.argv.at(-1).includes('browser_wait_for') || !process.argv.at(-1).includes('browser_close')) process.exit(9);\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_file_upload',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`, {mode:0o755});
    const input={task_id:'guarded-upload',task_class:'upload',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/file-input?task=guarded-upload',acceptance:{kind:'upload',filename:'benchmark.txt',size:20,sha256:'e6cd2f3dd6fbc01ec2f9b8f1578535ab325cd7a884a14d1170cb50dc744a0231'},deadline_ms:60000};
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,true);
    assert.equal(output.independently_verified_success,false);
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('Playwright actor can launch the verified pinned local CLI directly', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-playwright-cli-test-'));
  try {
    const cli=join(dir,'cli.js'), executable=join(dir,'codex');
    await writeFile(join(dir,'package.json'),JSON.stringify({name:'@playwright/mcp',version:'0.0.83'}));
    await writeFile(cli,'');
    await writeFile(executable, `#!/usr/bin/env node\nif (!process.argv.some(x=>x.includes(${JSON.stringify(cli)}))) process.exit(9);\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_navigate',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`,{mode:0o755});
    const input={task_id:'form-guarded-fill',task_class:'forms',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill',acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'},deadline_ms:60000};
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_PLAYWRIGHT_MCP_CLI:cli},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,true);
    await writeFile(join(dir,'package.json'),JSON.stringify({name:'@playwright/mcp',version:'0.0.84'}));
    const code=await new Promise((resolveCode,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_PLAYWRIGHT_MCP_CLI:cli},stdio:['pipe','ignore','ignore']});child.once('error',reject);child.once('close',resolveCode);child.stdin.end(JSON.stringify(input));});
    assert.notEqual(code,0);
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('an incomplete browser call cannot claim actor success', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-incomplete-test-'));
  try {
    const executable=join(dir,'codex');
    await writeFile(executable, `#!/usr/bin/env node\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_navigate',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'FAILED'}}));\n`, {mode:0o755});
    const input={task_id:'form-guarded-fill',task_class:'forms',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill',acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'},deadline_ms:60000};
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,false);
    assert.equal(output.runner_error,'Codex host did not report task completion');
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('malformed client output cannot claim actor success', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-malformed-test-'));
  try {
    const executable=join(dir,'codex');
    await writeFile(executable, `#!/usr/bin/env node\nconsole.log('not-json');\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_navigate',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`,{mode:0o755});
    const input={task_id:'form-guarded-fill',task_class:'forms',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill',acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'},deadline_ms:60000};
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,false);
    assert.equal(output.runner_error,'Codex host returned invalid or oversized event output');
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('not-dispatched browser calls remain unqualified and identify the tool', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-failed-call-test-'));
  try {
    const executable=join(dir,'codex');
    const input={task_id:'form-guarded-fill',task_class:'forms',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill',acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'},deadline_ms:60000};
    for (const result of [{structured_content:{status:'not_dispatched'}},{structuredContent:{status:'completed_unverified'}},{content:[{type:'text',text:'{"status":"unknown"}'}]},{isError:true},{status:'incomplete'},{structured_content:{status:'not_dispatched',reason_code:'unsafe_page'}},{structured_content:{status:'not_dispatched',reason_code:'sensitive text'}}]) {
    await writeFile(executable, `#!/usr/bin/env node\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'playwright-mcp',tool:'browser_close',status:'completed',result:${JSON.stringify(result)}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`,{mode:0o755});
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'playwright-mcp'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,false);
    assert.equal(output.runner_error,`Codex host MCP call failed: browser_close${result.structured_content?.reason_code === 'unsafe_page' ? ' (unsafe_page)' : ''}`);
    }
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('Controlla form actor directs the client to the private headless route', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-controlla-route-test-'));
  try {
    const executable=join(dir,'codex');
    await writeFile(executable, `#!/usr/bin/env node\nconst prompt=process.argv.at(-1); if (!prompt.includes('action=launch and mode=headless') || !prompt.includes('Browser action=open is only for shared tabs') || !prompt.includes('browser action=task')) process.exit(9);\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'mcp_tool_call',server:'chrome-controlla',tool:'browser',status:'completed',result:{content:[{type:'text',text:'ok'}]}}}));\nconsole.log(JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'DONE'}}));\n`,{mode:0o755});
    const input={task_id:'form-guarded-fill',task_class:'forms',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/registration-form?task=form-guarded-fill',acceptance:{kind:'form',name:'Benchmark User',notes:'Persisted benchmark form'},deadline_ms:60000};
    const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'controlla'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_CONTROLLA_BINARY:'/controlla'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
    assert.equal(output.success,true);
    assert.equal(output.independently_verified_success,false);
  } finally { await rm(dir,{recursive:true,force:true}); }
});

test('Controlla upload selection requires a later verified application click', async () => {
  const dir=await mkdtemp(join(tmpdir(),'phase11-selection-test-'));
  try {
    const executable=join(dir,'codex');
    const input={task_id:'guarded-upload',task_class:'upload',mode:'cold_browser',execution_mode:'headless',concurrency:1,fixture_url:'http://127.0.0.1:1234/file-input?task=guarded-upload',acceptance:{kind:'upload',filename:'benchmark.txt',size:20,sha256:'e6cd2f3dd6fbc01ec2f9b8f1578535ab325cd7a884a14d1170cb50dc744a0231'},deadline_ms:60000};
    const selected={type:'item.completed',item:{type:'mcp_tool_call',server:'chrome-controlla',tool:'browser',status:'completed',arguments:{session_id:'s',chrome_tab_id:'t',task:{kind:'upload'}},result:{structured_content:{status:'incomplete',receipts:[{status:'selected',chrome_selected_file:true,files:{files:[input.acceptance]}}]}}}};
    const verified={type:'item.completed',item:{type:'mcp_tool_call',server:'chrome-controlla',tool:'act',status:'completed',arguments:{session_id:'s',chrome_tab_id:'t',action:'click',outcome:{kind:'text',selector:'#status',text:'Upload saved'}},result:{structured_content:{status:'verified'}}}};
    for (const scenario of ['no_selection','missing','wrong_tab','duplicate','verified']) {
      const followup = scenario === 'wrong_tab' ? {...verified,item:{...verified.item,arguments:{...verified.item.arguments,chrome_tab_id:'other'}}} : verified;
      const events=[...(scenario === 'no_selection'?[]:[selected]),...(scenario === 'duplicate'?[selected]:[]),...(scenario === 'missing'?[]:[followup]),{type:'item.completed',item:{type:'agent_message',text:'DONE'}}];
      await writeFile(executable, `#!/usr/bin/env node\nfor(const event of ${JSON.stringify(events)}) console.log(JSON.stringify(event));\n`,{mode:0o755});
      const output=await new Promise((resolveOutput,reject)=>{const child=spawn(process.execPath,[adapterPath,'controlla'],{cwd:root,env:{...process.env,PHASE11_FIXTURE_BASE_URL:'http://127.0.0.1:1234',PHASE11_CODEX_PATH:executable,PHASE11_CHROME_PATH:'/chrome',PHASE11_CONTROLLA_BINARY:'/controlla'},stdio:['pipe','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',chunk=>stdout+=chunk);child.stderr.on('data',chunk=>stderr+=chunk);child.once('error',reject);child.once('close',code=>code===0?resolveOutput(JSON.parse(stdout)):reject(new Error(stderr)));child.stdin.end(JSON.stringify(input));});
      assert.equal(output.success,scenario === 'verified');
      assert.equal(output.independently_verified_success,false);
    }
  } finally { await rm(dir,{recursive:true,force:true}); }
});
