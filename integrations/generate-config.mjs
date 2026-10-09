#!/usr/bin/env node
import { randomUUID } from 'node:crypto';
import { open, lstat, readFile, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { fileURLToPath } from 'node:url';

const binaryDefault = '/ABS/PATH/TO/controlla';
const stateDefault = '/ABS/PATH/TO/controlla-state';
const shellQuote = (value) => `'${value.replaceAll("'", `'"'"'`)}'`;

export const configs = {
  freebuff: {
    label: 'Freebuff / Codebuff CLI',
    schemaVersion: 'freebuff-codebuff-2026-10-06-v1',
    format: 'json',
    installPath: ['mcpServers'],
    make: (binary, state) => ({
      mcpServers: {
        'chrome-controlla': {
          command: binary,
          args: ['mcp'],
          env: {
            CONTROLLA_STATE_DIR: path.join(state, 'freebuff'),
          },
        },
      },
    }),
  },
  'opencode-v1': {
    label: 'OpenCode v1',
    schemaVersion: 'opencode-v1-2026-10-06-v1',
    format: 'json',
    installPath: ['mcp'],
    make: (binary, state) => ({
      $schema: 'https://opencode.ai/config.json',
      mcp: {
        'chrome-controlla': {
          type: 'local',
          command: [binary, 'mcp'],
          enabled: true,
          environment: {
            CONTROLLA_STATE_DIR: path.join(state, 'opencode-v1'),
          },
        },
      },
    }),
  },
  'opencode-v2': {
    label: 'OpenCode v2',
    schemaVersion: 'opencode-v2-2026-10-06-v1',
    format: 'json',
    installPath: ['mcp', 'servers'],
    make: (binary, state) => ({
      $schema: 'https://opencode.ai/config.json',
      mcp: {
        servers: {
          'chrome-controlla': {
            type: 'local',
            command: [binary, 'mcp'],
            environment: {
              CONTROLLA_STATE_DIR: path.join(state, 'opencode-v2'),
            },
          },
        },
      },
    }),
  },
  claude: {
    label: 'Claude Code',
    schemaVersion: 'claude-code-stdio-2026-10-06-v1',
    format: 'shell',
    make: (binary, state) =>
      `claude mcp add --env ${shellQuote(`CONTROLLA_STATE_DIR=${path.join(state, 'claude')}`)} --transport stdio --scope user chrome-controlla -- ${shellQuote(binary)} mcp\nclaude mcp get chrome-controlla`,
  },
};

function unsupportedClient(client) {
  if (client === 'chatgpt') {
    return new Error('ChatGPT Desktop uses the packaged Chrome Controlla local plugin; see docs/integrations/chatgpt-desktop.md');
  }
  return new Error(`Unsupported local-stdio client config: ${client}`);
}

export function generate(client, binary = binaryDefault, state = stateDefault) {
  const config = configs[client];
  if (!config) throw unsupportedClient(client);
  if (!path.isAbsolute(binary) || !path.isAbsolute(state)) {
    throw new Error('Executable and state directory must be absolute paths');
  }
  return config.make(binary, state);
}

export async function install(client, configPath, binary = binaryDefault, state = stateDefault) {
  const clientConfig = configs[client];
  if (!clientConfig) throw unsupportedClient(client);
  if (clientConfig.format !== 'json') {
    throw new Error(`${client} uses a client CLI command; run the generated command yourself`);
  }
  if (!path.isAbsolute(configPath)) throw new Error('Configuration file path must be absolute');

  let before = null;
  let mode = 0o600;
  try {
    const stat = await lstat(configPath);
    if (!stat.isFile()) throw new Error('Configuration target must be a regular file, not a link or directory');
    before = await readFile(configPath, 'utf8');
    mode = stat.mode & 0o777;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }

  let document = {};
  if (before !== null) {
    try {
      document = JSON.parse(before);
    } catch {
      throw new Error('Existing configuration is not valid JSON; it was left unchanged');
    }
    if (!document || typeof document !== 'object' || Array.isArray(document)) {
      throw new Error('Existing configuration must be a JSON object; it was left unchanged');
    }
  }

  let parent = document;
  for (const key of clientConfig.installPath) {
    if (parent[key] === undefined) parent[key] = {};
    if (!parent[key] || typeof parent[key] !== 'object' || Array.isArray(parent[key])) {
      throw new Error(`Configuration field ${key} must be an object; it was left unchanged`);
    }
    parent = parent[key];
  }
  let generatedParent = generate(client, binary, state);
  for (const key of clientConfig.installPath) generatedParent = generatedParent[key];
  const generatedEntry = generatedParent['chrome-controlla'];
  if (Object.hasOwn(parent, 'chrome-controlla')) {
    if (isDeepStrictEqual(parent['chrome-controlla'], generatedEntry)) {
      return { installed: false, path: configPath };
    }
    throw new Error('A different chrome-controlla entry already exists; it was left unchanged');
  }
  parent['chrome-controlla'] = generatedEntry;

  const dir = path.dirname(configPath);
  const tempPath = path.join(dir, `.${path.basename(configPath)}.controlla-${randomUUID()}.tmp`);
  let temp;
  try {
    temp = await open(tempPath, 'wx', mode);
    await temp.writeFile(`${JSON.stringify(document, null, 2)}\n`, 'utf8');
    await temp.sync();
    await temp.close();
    temp = undefined;

    const latest = await readFile(configPath, 'utf8').catch((error) => {
      if (error.code === 'ENOENT') return null;
      throw error;
    });
    if (latest !== before) throw new Error('Configuration changed during install; retry after reviewing it');
    await rename(tempPath, configPath);
    return { installed: true, path: configPath };
  } catch (error) {
    await temp?.close().catch(() => {});
    await unlink(tempPath).catch(() => {});
    throw error;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [, , ...args] = process.argv;
  const installMode = args[0] === '--install';
  const client = installMode ? args[1] : args[0];
  const configPath = installMode ? args[2] : undefined;
  const binary = (installMode ? args[3] : args[1]) ?? binaryDefault;
  const state = (installMode ? args[4] : args[2]) ?? stateDefault;
  if (!client || client === '--help') {
    console.log(`Usage: node integrations/generate-config.mjs <client> [absolute-controlla-path] [absolute-state-root]\n       node integrations/generate-config.mjs --install <client> <absolute-json-config> [absolute-controlla-path] [absolute-state-root]\nClients: ${Object.keys(configs).join(', ')}\n--install preserves other settings, atomically adds a missing JSON entry, and refuses conflicts or links. Claude Code outputs a command for you to review and run. ChatGPT Desktop uses the packaged local plugin; see docs/integrations/chatgpt-desktop.md.`);
    process.exit(client ? 0 : 2);
  }
  try {
    if (installMode && !configPath) throw new Error('--install requires an absolute JSON configuration path');
    const output = installMode
      ? await install(client, configPath, binary, state)
      : generate(client, binary, state);
    console.log(typeof output === 'string' ? output : JSON.stringify(output, null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
