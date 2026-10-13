import { isAbsolute } from 'node:path';
import { createHash } from 'node:crypto';

const V3_TOOLS = ['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow'];

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  return value;
}

export function fingerprintToolSchema(toolSchema) {
  const sorted = [...toolSchema].sort((a, b) => a.name.localeCompare(b.name));
  return createHash('sha256').update(JSON.stringify(canonical(sorted))).digest('hex');
}

export function validateClientSmoke(clientId, result, requiredCapabilities, expectedRunId, expectedConfigHash) {
  if (!result || result.schema_version !== 1 || result.client !== clientId || result.status !== 'passed' || result.live !== true) {
    throw new Error(`${clientId} adapter must return schema_version=1, matching client, status=passed, and live=true`);
  }
  if (typeof expectedRunId !== 'string' || result.run_id !== expectedRunId) throw new Error(`${clientId} adapter result run_id does not match the current run`);
  if (expectedConfigHash && result.integration_config_sha256 !== expectedConfigHash) throw new Error(`${clientId} adapter result is not bound to the resolved integration config`);
  if (!isAbsolute(result.evidence_path) || !/^[a-f0-9]{64}$/.test(result.evidence_sha256 ?? '')) {
    throw new Error(`${clientId} adapter must return an absolute evidence_path and SHA-256`);
  }
  if (![result.client_version, result.controlla_version].every(value => typeof value === 'string' && value.trim())) {
    throw new Error(`${clientId} adapter must report client_version and controlla_version`);
  }
  const reported = new Set(Array.isArray(result.verified_capabilities) ? result.verified_capabilities : []);
  const missing = requiredCapabilities.filter(capability => !reported.has(capability));
  if (missing.length) throw new Error(`${clientId} evidence omits required capabilities: ${missing.join(', ')}`);
  return result;
}

export function canonicalV3ToolNames() {
  return [...V3_TOOLS];
}

export function validateClientEvidence(clientId, result, evidence, requiredCapabilities, runWindow, { allowSyntheticTestEvidence = false, expectedConfigHash } = {}) {
  if (!allowSyntheticTestEvidence && (evidence?.synthetic === true || evidence?.cases?.some(testCase => testCase?.proof?.test_fixture === true))) {
    throw new Error(`${clientId} synthetic fixture evidence cannot qualify a real client`);
  }
  if (!evidence || evidence.schema_version !== 1 || evidence.client !== clientId || typeof evidence.run_id !== 'string' || !evidence.run_id.trim()) {
    throw new Error(`${clientId} evidence must be versioned and bound to a non-empty run_id and matching client`);
  }
  if (!runWindow || evidence.run_id !== runWindow.runId || result.run_id !== runWindow.runId) throw new Error(`${clientId} evidence run_id does not match the current run`);
  if (expectedConfigHash && (result.integration_config_sha256 !== expectedConfigHash || evidence.integration_config_sha256 !== expectedConfigHash)) {
    throw new Error(`${clientId} evidence is not bound to the resolved integration config`);
  }
  for (const field of ['client_version', 'controlla_version']) {
    if (evidence[field] !== result[field]) throw new Error(`${clientId} evidence ${field} does not match adapter result`);
  }
  if (!Array.isArray(evidence.cases) || evidence.cases.length === 0) throw new Error(`${clientId} evidence must contain observed test cases`);

  const covered = new Set();
  for (const testCase of evidence.cases) {
    if (!testCase || typeof testCase.capability !== 'string' || !testCase.capability.trim()) throw new Error(`${clientId} evidence case omitted capability`);
    if (testCase.status !== 'passed' || testCase.live !== true) throw new Error(`${clientId} evidence case ${testCase.capability} must be live and passed`);
    const observedAt = Date.parse(testCase.observed_at);
    if (typeof testCase.observer !== 'string' || !testCase.observer.trim() || !Number.isFinite(observedAt)) {
      throw new Error(`${clientId} evidence case ${testCase.capability} must identify its observer and observation time`);
    }
    if (observedAt < runWindow.startedAtMs - 2_000 || observedAt > runWindow.finishedAtMs + 2_000) throw new Error(`${clientId} evidence case ${testCase.capability} was not observed during the current run`);
    if (!testCase.proof || typeof testCase.proof !== 'object' || Array.isArray(testCase.proof) || !Object.keys(testCase.proof).length) {
      throw new Error(`${clientId} evidence case ${testCase.capability} omitted proof data`);
    }
    validateCapabilityProof(clientId, testCase.capability, testCase.proof);
    if (testCase.capability === 'startup') {
      const tools = Array.isArray(testCase.proof.tools) ? [...testCase.proof.tools].sort() : [];
      const schema = Array.isArray(testCase.proof.tool_schema) ? testCase.proof.tool_schema : [];
      const schemaNames = schema.map(tool => tool?.name).sort();
      const schemaValid = schema.every(tool => tool && typeof tool.name === 'string' && tool.inputSchema && typeof tool.inputSchema === 'object');
      if (tools.length !== V3_TOOLS.length || tools.some((tool, index) => tool !== V3_TOOLS[index])
        || schema.length !== V3_TOOLS.length || schemaNames.some((tool, index) => tool !== V3_TOOLS[index]) || !schemaValid) {
        throw new Error(`${clientId} startup evidence must contain the exact six-tool schema and its SHA-256`);
      }
      if (!/^[a-f0-9]{64}$/.test(testCase.proof.schema_sha256 ?? '') || testCase.proof.schema_sha256 !== fingerprintToolSchema(schema)) {
        throw new Error(`${clientId} startup evidence SHA-256 does not match the tool schemas`);
      }
    }
    if (testCase.capability === 'verified_mutations') {
      const { actor_id, action_receipt, independent_verification: verification } = testCase.proof;
      if (typeof actor_id !== 'string' || !actor_id || action_receipt?.status !== 'verified'
        || verification?.status !== 'passed' || typeof verification.observer_id !== 'string'
        || !verification.observer_id || verification.observer_id === actor_id) {
        throw new Error(`${clientId} mutation evidence must include a verified receipt and an independent observer`);
      }
    }
    covered.add(testCase.capability);
  }

  const missing = requiredCapabilities.filter(capability => !covered.has(capability));
  if (missing.length) throw new Error(`${clientId} evidence omits required capabilities: ${missing.join(', ')}`);
  const reported = new Set(Array.isArray(result.verified_capabilities) ? result.verified_capabilities : []);
  if (requiredCapabilities.some(capability => !reported.has(capability))) {
    throw new Error(`${clientId} adapter claims do not match evidence coverage`);
  }
  return { ...result, verified_capabilities: [...covered].sort() };
}

function validateCapabilityProof(client, capability, proof) {
  const requireProof = (condition, message) => { if (!condition) throw new Error(`${client} ${capability} evidence ${message}`); };
  switch (capability) {
    case 'startup':
      requireProof(Number.isSafeInteger(proof.process_id) && proof.process_id > 0, 'must identify the launched client process');
      requireProof(proof.isolation === 'disposable', 'must confirm a disposable client environment');
      break;
    case 'persistent_session':
      requireProof(typeof proof.session_id === 'string' && proof.session_id.length > 0 && proof.session_reused === true, 'must prove reuse of the same live session');
      break;
    case 'foreground':
      requireProof(proof.mode === 'foreground' && proof.activated === true, 'must prove foreground activation');
      break;
    case 'background':
      requireProof(proof.mode === 'background' && typeof proof.source_tab_before === 'string'
        && proof.source_tab_before === proof.source_tab_after && typeof proof.opened_tab_id === 'string'
        && proof.opened_tab_id !== proof.source_tab_before, 'must prove a background tab opened without changing the active tab');
      break;
    case 'headless':
      requireProof(proof.mode === 'headless' && proof.visible === false && proof.readback === true, 'must prove a hidden browser run and readback');
      break;
    case 'block_typing':
      requireProof(proof.blocked === true && proof.value_unchanged === true && proof.dispatch_acknowledged !== true, 'must prove blocked typing left the value unchanged');
      break;
    case 'fast_keys':
      requireProof(proof.mode === 'fast_keys' && Number.isSafeInteger(proof.key_events) && proof.key_events > 0 && proof.readback === true && proof.intentional_delay_ms === 0, 'must prove immediate key events and readback');
      break;
    case 'human_keys':
      requireProof(proof.mode === 'human_keys' && Number.isSafeInteger(proof.key_events) && proof.key_events > 0 && proof.readback === true && Number.isFinite(proof.intentional_delay_ms) && proof.intentional_delay_ms > 0, 'must prove paced key events and readback');
      break;
    case 'ime':
      requireProof(proof.composition_start === true && proof.composition_update === true && proof.composition_end === true && proof.readback === true, 'must prove IME composition lifecycle and readback');
      break;
    case 'multi_tab':
      requireProof(Array.isArray(proof.tab_ids) && proof.tab_ids.length >= 2 && new Set(proof.tab_ids).size === proof.tab_ids.length, 'must prove actions across distinct tabs');
      break;
    case 'user_interference':
      requireProof(proof.interference_detected === true && proof.stale_mutation_dispatched !== true, 'must prove user interference stopped a stale mutation');
      break;
    case 'upload_download':
      requireProof(['upload', 'download'].includes(proof.direction) && /^[a-f0-9]{64}$/.test(proof.sha256 ?? '') && proof.verified === true, 'must prove a verified artifact transfer with SHA-256');
      if (proof.direction === 'upload') requireProof(proof.application_accepted === true, 'must prove application acceptance of the upload');
      if (proof.direction === 'download') requireProof(Number.isSafeInteger(proof.bytes) && proof.bytes > 0, 'must prove downloaded bytes');
      break;
    case 'research_extract':
      requireProof(Array.isArray(proof.sources) && proof.sources.length > 0 && proof.sources.every(source => typeof source.url === 'string' && /^https:\/\//.test(source.url))
        && Number.isSafeInteger(proof.records) && proof.records > 0 && proof.independent_readback === true, 'must prove sourced live extraction and independent readback');
      break;
    case 'verified_mutations':
      requireProof(typeof proof.actor_id === 'string' && proof.actor_id && proof.action_receipt?.status === 'verified'
        && proof.independent_verification?.status === 'passed' && typeof proof.independent_verification.observer_id === 'string'
        && proof.independent_verification.observer_id && proof.independent_verification.observer_id !== proof.actor_id,
      'must include a verified action and a distinct independent observer');
      break;
    default:
      requireProof(false, 'uses an unsupported capability name');
  }
}
