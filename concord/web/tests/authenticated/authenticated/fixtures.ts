import { readFileSync } from 'node:fs';
import { createConnection } from 'node:net';
import WebSocket from 'ws';

export const sessions = JSON.parse(readFileSync(process.env.CONCORD_AUTH_SESSIONS_FILE!, 'utf8')) as Record<'alice' | 'alice_revoke' | 'bob' | 'bob_irc' | 'helper_bot' | 'helper_bot_token_id' | 'limited_helper_bot' | 'wrong_bot' | 'historical_non_uuid_message_id' | 'historical_padded_message_id' | 'historical_long_message_id', string>;

export async function ircClient() {
  const socket = createConnection({ host: '127.0.0.1', port: Number(process.env.CONCORD_IRC_PORT) });
  const lines: string[] = [];
  const waiters: Array<{ predicate: (line: string) => boolean; resolve: (line: string) => void }> = [];
  let buffered = '';
  socket.setEncoding('utf8');
  socket.on('data', (chunk) => {
    buffered += chunk;
    const complete = buffered.split(/\r?\n/);
    buffered = complete.pop() ?? '';
    for (const line of complete) {
      lines.push(line);
      const index = waiters.findIndex((waiter) => waiter.predicate(line));
      if (index >= 0) waiters.splice(index, 1)[0].resolve(line);
    }
  });
  await new Promise<void>((resolve, reject) => {
    socket.once('connect', resolve);
    socket.once('error', reject);
  });
  const waitFor = (predicate: (line: string) => boolean) => {
    const existing = lines.find(predicate);
    if (existing) return Promise.resolve(existing);
    return Promise.race([
      new Promise<string>((resolve) => waiters.push({ predicate, resolve })),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`IRC line timed out; received: ${lines.join(' | ')}`)), 5_000)),
    ]);
  };
  const send = (line: string) => socket.write(`${line}\r\n`);
  return { socket, send, waitFor };
}

export async function registerIrc(client: Awaited<ReturnType<typeof ircClient>>, tagged = false) {
  if (tagged) {
    client.send('CAP LS 302');
    await client.waitFor((line) => line.includes(' CAP * LS :') && line.includes('server-time') && line.includes('message-tags'));
    client.send('CAP REQ :server-time message-tags');
    await client.waitFor((line) => line.includes(' CAP * ACK :server-time message-tags'));
    client.send('CAP END');
  }
  client.send(`PASS ${sessions.bob_irc}`);
  client.send('NICK bob');
  client.send('USER bob 0 * :Bob');
  const welcome = await client.waitFor((line) => line.includes(' 001 '));
  const nick = welcome.split(' ')[2];
  if (!nick) throw new Error(`IRC welcome returned no nickname: ${welcome}`);
  client.send('LIST');
  const listed = await client.waitFor((line) => line.includes(` 322 ${nick} `));
  const channel = listed.split(' ')[3];
  if (!channel?.startsWith('#')) throw new Error(`IRC LIST returned no channel: ${listed}`);
  return channel;
}

export function decodeIrcTagValue(value: string) {
  let decoded = '';
  for (let index = 0;index < value.length;index += 1) {
    if (value[index] !== '\\' || index + 1 >= value.length) {
      decoded += value[index];
      continue;
    }
    index += 1;
    decoded += ({ ':': ';', s: ' ', '\\': '\\', r: '\r', n: '\n' } as Record<string, string>)[value[index]] ?? value[index];
  }
  return decoded;
}

export function ircTags(line: string) {
  if (!line.startsWith('@')) return {};
  const separator = line.indexOf(' ');
  return Object.fromEntries(line.slice(1, separator).split(';').map((tag) => {
    const equals = tag.indexOf('=');
    return equals < 0
      ? [tag, '']
      : [tag.slice(0, equals), decodeIrcTagValue(tag.slice(equals + 1))];
  }));
}

export function captureSocketDiagnostics(page: import('@playwright/test').Page, label: string, pageErrors: string[]) {
  page.on('console', (message) => console.log(`[${label}:console:${message.type()}] ${message.text()}`));
  page.on('pageerror', (error) => {
    pageErrors.push(error.message);
    console.log(`[${label}:pageerror] ${error.message}`);
  });
  page.on('websocket', (socket) => {
    socket.on('framereceived', (event) => console.log(`[${label}:ws:received] ${String(event.payload).slice(0, 500)}`));
    socket.on('framesent', (event) => console.log(`[${label}:ws:sent] ${String(event.payload).slice(0, 500)}`));
    socket.on('socketerror', (error) => console.log(`[${label}:ws:error] ${error}`));
    socket.on('close', () => console.log(`[${label}:ws:close]`));
  });
}

export async function openGeneral(page: import('@playwright/test').Page) {
  await page.getByTitle('Browser fixture').click();
  await page.getByRole('button', { name: 'general' }).click();
}

export async function attachRawSocket(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const state = window as typeof window & { rawSocket?: WebSocket; rawFrames?: unknown[] };
    state.rawFrames = [];
    state.rawSocket = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/ws`);
    state.rawSocket.addEventListener('message', (event) => {
      try { state.rawFrames!.push(JSON.parse(String(event.data))); } catch { /* ignore non-JSON diagnostics */ }
    });
  });
  await page.waitForFunction(() => (window as typeof window & { rawSocket?: WebSocket }).rawSocket?.readyState === WebSocket.OPEN);
}

export async function rawSend(page: import('@playwright/test').Page, message: unknown) {
  await page.evaluate((payload) => {
    (window as typeof window & { rawSocket?: WebSocket }).rawSocket!.send(JSON.stringify(payload));
  }, message);
}

export async function rawFramesFromPage(page: import('@playwright/test').Page) {
  return page.evaluate(() => (window as typeof window & { rawFrames?: unknown[] }).rawFrames ?? []);
}

export async function botSocket(baseURL: string, token: string) {
  const frames: unknown[] = [];
  const socket = new WebSocket(baseURL.replace(/^http/, 'ws') + '/ws', {
    headers: { Authorization: `Bearer ${token}`, Origin: baseURL },
  });
  socket.on('message', (data) => {
    try { frames.push(JSON.parse(data.toString())); } catch { /* ignore non-JSON diagnostics */ }
  });
  let resolveClosed!: () => void;
  const closed = new Promise<void>((resolve) => { resolveClosed = resolve; });
  socket.once('close', resolveClosed);
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
    socket.once('unexpected-response', (_request, response) => reject(new Error(`WebSocket rejected with ${response.statusCode}`)));
  });
  return { socket, frames, closed, send: (message: unknown) => socket.send(JSON.stringify(message)) };
}

export const isErrorFrame = (frame: unknown) => ['error', 'command_error'].includes((frame as { type?: string }).type ?? '');
