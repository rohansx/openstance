// Starts `openstance serve` on a fresh home with the demo dataset imported.
// No network: OPENSTANCE_OFFLINE=1 and no embedding model. The enrichment
// backend is `treg` so playbooks return a real (local, estimated) quote; with
// OFFLINE set nothing can leave the machine even if a test clicked Spend.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';

const repo = resolve(import.meta.dirname, '../..');

export function binary(): string {
  const candidates = [
    process.env.OPENSTANCE_BIN,
    join(repo, 'target/debug/openstance'),
    join(repo, 'target/release/openstance'),
  ].filter((p): p is string => !!p);
  const bin = candidates.find((p) => existsSync(p));
  if (!bin) throw new Error(`no openstance binary; set OPENSTANCE_BIN (tried ${candidates.join(', ')})`);
  return bin;
}

export const NOW = '2026-10-15';

export default async function globalSetup() {
  const bin = binary();
  const home = mkdtempSync(join(tmpdir(), 'openstance-e2e-'));
  const env = {
    ...process.env,
    OPENSTANCE_HOME: home,
    OPENSTANCE_OFFLINE: '1',
    OPENSTANCE__MODEL__EMBEDDER: 'none',
    OPENSTANCE__ENRICH__BACKEND: 'treg',
    // above the $0.76 profile estimate, so the Spend button can appear
    OPENSTANCE__ENRICH__MAX_USD_PER_RUN: '2',
  };
  execFileSync(bin, ['init'], { env, stdio: 'pipe' });
  const demo = ['takeout.mbox', 'Connections.csv', 'contacts.vcf'].map((f) => join(repo, 'fixtures/demo', f));
  execFileSync(bin, ['--now', NOW, 'import', ...demo], { env, stdio: 'pipe' });

  const server = spawn(bin, ['--now', NOW, 'serve', '--port', '0', '--json'], { env, stdio: ['ignore', 'pipe', 'inherit'] });
  const line = await new Promise<string>((ok, fail) => {
    const rl = createInterface({ input: server.stdout! });
    rl.once('line', ok);
    server.once('exit', (code) => fail(new Error(`openstance serve exited early (${code})`)));
    setTimeout(() => fail(new Error('openstance serve did not print its ready line')), 20_000);
  });
  const ready = JSON.parse(line) as { api: string; ui: string; token_file: string };
  if (!ready.ui) throw new Error(`serve started without the UI: ${line}`);
  const token = new URL(ready.ui).hash.replace(/^#token=/, '');

  process.env.E2E_BASE = ready.api;
  process.env.E2E_TOKEN = token;
  process.env.E2E_HOME = home;
  process.env.E2E_BIN = bin;

  return async () => {
    server.kill('SIGTERM');
    await new Promise((r) => server.once('exit', r));
    rmSync(home, { recursive: true, force: true });
  };
}
