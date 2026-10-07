#!/usr/bin/env node
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const binaryDefault = '/ABS/PATH/TO/controlla';
const stateDefault = '/ABS/PATH/TO/controlla-state';
const shellQuote = (value) => `'${value.replaceAll("'", `'"'"'`)}'`;

export const configs = {
  freebuff: {
    label: 'Freebuff / Codebuff CLI',
    schemaVersion: 'freebuff-codebuff-2026-10-06-v1',
    format: 'json',
    make: (binary, state) => ({
      mcpServers: {
        'chrome-controlla': {
          command: binary,
          args: ['mcp'],
          env: {
            COMPTROL_CHROME_AUTO_CONNECT: '1',
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
    make: (binary, state) => ({
      $schema: 'https://opencode.ai/config.json',
      mcp: {
        'chrome-controlla': {
          type: 'local',
          command: [binary, 'mcp'],
          enabled: true,
          environment: {
            COMPTROL_CHROME_AUTO_CONNECT: '1',
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
    make: (binary, state) => ({
      $schema: 'https://opencode.ai/config.json',
      mcp: {
        servers: {
          'chrome-controlla': {
            type: 'local',
            command: [binary, 'mcp'],
            environment: {
              COMPTROL_CHROME_AUTO_CONNECT: '1',
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
      `claude mcp add --env ${shellQuote('COMPTROL_CHROME_AUTO_CONNECT=1')} --env ${shellQuote(`CONTROLLA_STATE_DIR=${path.join(state, 'claude')}`)} --transport stdio --scope user chrome-controlla -- ${shellQuote(binary)} mcp\nclaude mcp get chrome-controlla`,
  },
};

export function generate(client, binary = binaryDefault, state = stateDefault) {
  const config = configs[client];
  if (!config) throw new Error(`Unsupported local-stdio client config: ${client}`);
  if (!path.isAbsolute(binary) || !path.isAbsolute(state)) {
    throw new Error('Executable and state directory must be absolute paths');
  }
  return config.make(binary, state);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [, , client, binary = binaryDefault, state = stateDefault] = process.argv;
  if (!client || client === '--help') {
    console.log(`Usage: node integrations/generate-config.mjs <client> [absolute-controlla-path] [absolute-state-root]\nClients: ${Object.keys(configs).join(', ')}\nChatGPT has no local-stdio config; this preview has no authenticated remote endpoint.`);
    process.exit(client ? 0 : 2);
  }
  try {
    const output = generate(client, binary, state);
    console.log(typeof output === 'string' ? output : JSON.stringify(output, null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
