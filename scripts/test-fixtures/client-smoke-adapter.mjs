import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fingerprintToolSchema } from '../client-smoke-contract.mjs';

let input = '';
for await (const chunk of process.stdin) input += chunk;
const request = JSON.parse(input);
const names = ['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow'];
const schema = names.map(name => ({ name, inputSchema: { type: 'object' } }));
const cases = request.required_capabilities.map(capability => {
  const proof = {
    startup: { tools: names, tool_schema: schema, schema_sha256: fingerprintToolSchema(schema), process_id: 1234, isolation: 'disposable' },
    persistent_session: { session_id: 'synthetic-session', session_reused: true },
    foreground: { mode: 'foreground', activated: true },
    background: { mode: 'background', source_tab_before: 'source', source_tab_after: 'source', opened_tab_id: 'new' },
    headless: { mode: 'headless', visible: false, readback: true },
    block_typing: { blocked: true, value_unchanged: true, dispatch_acknowledged: false },
    fast_keys: { mode: 'fast_keys', key_events: 2, readback: true, intentional_delay_ms: 0 },
    human_keys: { mode: 'human_keys', key_events: 2, readback: true, intentional_delay_ms: 10 },
    ime: { composition_start: true, composition_update: true, composition_end: true, readback: true },
    multi_tab: { tab_ids: ['one', 'two'] },
    user_interference: { interference_detected: true, stale_mutation_dispatched: false },
    upload_download: { direction: 'download', sha256: 'a'.repeat(64), verified: true, bytes: 1 },
    research_extract: { sources: [{ url: 'https://example.test/' }], records: 1, independent_readback: true },
    verified_mutations: { actor_id: 'fixture-actor', action_receipt: { status: 'verified' }, independent_verification: { status: 'passed', observer_id: 'fixture-independent-observer' } },
  }[capability];
  if (!proof) throw new Error(`unknown test capability: ${capability}`);
  proof.test_fixture = true;
  return { capability, status: 'passed', live: true, observer: 'synthetic-test-fixture', observed_at: new Date().toISOString(), proof };
});
const evidence = { schema_version: 1, synthetic: true, run_id: request.run_id, client: request.client,
  integration_config_sha256: request.client_adapter.config_sha256,
  client_version: 'fixture-client-1', controlla_version: 'fixture-controlla-1', cases, host_task_runs: request.host_task_runs ?? [] };
const bytes = Buffer.from(`${JSON.stringify(evidence)}\n`);
const dir = process.env.CONTROLLA_TEST_EVIDENCE_ROOT;
if (!dir) throw new Error('test evidence directory is required');
await mkdir(dir, { recursive: true });
const evidencePath = join(dir, `${request.client}-${request.run_id}.json`);
await writeFile(evidencePath, bytes, { flag: 'wx' });
process.stdout.write(JSON.stringify({ schema_version: 1, run_id: request.run_id, client: request.client, status: 'passed', live: true,
  evidence_path: evidencePath, evidence_sha256: createHash('sha256').update(bytes).digest('hex'), client_version: evidence.client_version,
  controlla_version: evidence.controlla_version, integration_config_sha256: evidence.integration_config_sha256,
  verified_capabilities: request.required_capabilities, host_task_runs: request.host_task_runs ?? [] }));
