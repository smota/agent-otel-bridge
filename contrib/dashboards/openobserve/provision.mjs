// Native Node >=22; no dependencies and no credential persistence.
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { buildDashboards, DASHBOARD_LEVELS, MARKER_PREFIX, formatDashboardMarker } from './build.mjs';

class SafeError extends Error {}
const fail = (message) => { throw new SafeError(message); };
const enc = encodeURIComponent;
const stable = (v) => JSON.stringify(v, (_, x) => x && !Array.isArray(x) && typeof x === 'object'
  ? Object.fromEntries(Object.keys(x).sort().map(k => [k, x[k]])) : x);
const digest = (v) => createHash('sha256').update(stable(v)).digest('hex');
export const defaultStateDir = () => path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), '.local', 'state'), 'agent-otel-bridge', 'openobserve');

export function config(env = process.env) {
  let url;
  try { url = new URL(env.OPENOBSERVE_URL); } catch { fail('Invalid OPENOBSERVE_URL'); }
  if (url.username || url.password || url.search || url.hash || url.pathname !== '/') fail('URL must be an origin without credentials, path, query or fragment');
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && ['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname))) fail('HTTPS or loopback HTTP required');
  if (!/^[A-Za-z0-9_-]+$/.test(env.OPENOBSERVE_ORG || '')) fail('Invalid OPENOBSERVE_ORG');
  if (!/^(Basic|Bearer) [^\s]+$/.test(env.OPENOBSERVE_AUTH || '')) fail('OPENOBSERVE_AUTH must be a complete Basic or Bearer header');
  return { url: url.origin, org: env.OPENOBSERVE_ORG, auth: env.OPENOBSERVE_AUTH, folder: 'default', stateDir: defaultStateDir(), timeout: 10000 };
}

export function client(c) {
  return async (method, route, body) => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), c.timeout);
    try {
      const response = await fetch(c.url + route, { method, redirect: 'manual', signal: controller.signal,
        headers: { Authorization: c.auth, 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body) });
      if (!response.ok) fail(`API request refused (HTTP ${response.status})`);
      // Bound responses and keep timeout active through body consumption.
      const chunks = []; let size = 0;
      for await (const chunk of response.body) {
        size += chunk.length;
        if (size > 16 * 1024 * 1024) { controller.abort(); fail('API response too large'); }
        chunks.push(chunk);
      }
      try { return JSON.parse(Buffer.concat(chunks).toString('utf8')); }
      catch { fail('Invalid API JSON response'); }
    } catch (error) {
      if (/^(API request refused \(HTTP \d+\)|API response too large|Invalid API JSON response)$/.test(error.message)) throw error;
      fail('API transport failed or timed out');
    } finally { clearTimeout(timer); }
  };
}

export function unwrap(w) {
  if (!w || typeof w !== 'object') fail('Invalid dashboard wrapper');
  const d = w[`v${w.version}`] || w;
  if (typeof d.title !== 'string' || typeof d.description !== 'string' || typeof d.dashboardId !== 'string') fail('Invalid dashboard definition');
  return d;
}

function owned(d, key) {
  const markers = d.description.match(/agent-otel-bridge:openobserve:[^\s|]+/g) || [];
  return markers.length === 1 && markers[0] === formatDashboardMarker(key)
    && d.description.split('|')[0].trim() === formatDashboardMarker(key);
}
function assertOwned(d, key, id) {
  if (!DASHBOARD_LEVELS.some(l => l.key === key) || !owned(d, key) || d.version !== 5 || d.dashboardId !== id) fail('Ownership, version or ID mismatch');
}

// Compare managed fields only: the server may add default fields and owner/created metadata.
function projects(actual, desired) {
  if (Array.isArray(desired)) return Array.isArray(actual) && actual.length === desired.length && desired.every((v, i) => projects(actual[i], v));
  if (desired && typeof desired === 'object') return actual && typeof actual === 'object' && Object.entries(desired).every(([k, v]) => projects(actual[k], v));
  return actual === desired;
}
function same(actual, desired) {
  const { owner: _o, created: _c, ...managed } = desired;
  return projects(actual, managed);
}
const base = c => `/api/${enc(c.org)}/dashboards`;
const route = (c, id, hash) => `${base(c)}/${enc(id)}?folder=${enc(c.folder)}${hash === undefined ? '' : `&hash=${enc(hash)}`}`;
async function snapshot(c, api, key, id) {
  const w = await api('GET', route(c, id)); const d = unwrap(w);
  assertOwned(d, key, id);
  if (typeof w.hash !== 'string' || !w.hash) fail('Missing optimistic concurrency hash');
  return { definition: d, hash: w.hash };
}
async function inventory(c, api) {
  const response = await api('GET', `${base(c)}?folder=${enc(c.folder)}`);
  if (!Array.isArray(response.dashboards)) fail('Invalid dashboard inventory');
  const all = response.dashboards.map(unwrap); const found = {};
  for (const level of DASHBOARD_LEVELS) {
    const candidates = all.filter(d => d.title === level.title || d.description.includes(formatDashboardMarker(level.key)));
    if (candidates.length > 1) fail('Duplicate ownership marker or title collision');
    if (candidates.length) {
      assertOwned(candidates[0], level.key, candidates[0].dashboardId);
      if (!candidates[0].dashboardId) fail('Missing dashboard ID');
      found[level.key] = candidates[0].dashboardId;
    }
  }
  // A package marker with an unknown key must never be adopted implicitly.
  for (const d of all) if (d.description.includes(MARKER_PREFIX) && !DASHBOARD_LEVELS.some(l => owned(d, l.key))) fail('Malformed or unknown ownership marker');
  if (new Set(Object.values(found)).size !== Object.values(found).length) fail('Duplicate dashboard IDs');
  return found;
}
async function immutable(dir, name, value) {
  await fs.mkdir(dir, { recursive: true, mode: 0o700 });
  const file = path.join(dir, name);
  const handle = await fs.open(file, 'wx', 0o600);
  try { await handle.writeFile(JSON.stringify(value, null, 2) + '\n'); await handle.sync(); }
  finally { await handle.close(); }
}
async function locked(c, fn) {
  await fs.mkdir(c.stateDir, { recursive: true, mode: 0o700 });
  let handle;
  try { handle = await fs.open(path.join(c.stateDir, 'apply.lock'), 'wx', 0o600); }
  catch { fail('Local apply lock exists; check for an active worker before removing it'); }
  try { return await fn(); }
  finally { await handle.close(); await fs.unlink(path.join(c.stateDir, 'apply.lock')); }
}
async function update(c, api, key, id, desired, current) {
  const token = randomUUID();
  const record = { version: 1, target: c.url, org: c.org, folder: c.folder, key, id,
    before: current.definition, beforeHash: current.hash, desired };
  await immutable(c.stateDir, `${token}.backup.json`, record);
  const result = await api('PUT', route(c, id, current.hash), { ...desired, owner: current.definition.owner, created: current.definition.created });
  const definition = unwrap(result);
  assertOwned(definition, key, id);
  if (typeof result.hash !== 'string' || !result.hash) fail('Missing optimistic concurrency hash; backup retained');
  if (!same(definition, desired)) fail('Server did not retain managed definition; backup retained');
  const receipt = { target: c.url, org: c.org, folder: c.folder, key, id, hash: result.hash, digest: digest(definition) };
  const after = await snapshot(c, api, key, id);
  if (after.hash !== receipt.hash || digest(after.definition) !== receipt.digest) fail('Concurrent changes after PUT prevent receipt; backup retained');
  await immutable(c.stateDir, `${token}.receipt.json`, receipt);
  return token;
}

export async function provision(c, { apply = false, requireRbac = false } = {}) {
  if (requireRbac) fail('RBAC support is not validated; enterprise edition and permission verification required');
  const run = async () => {
    const api = client(c); let ids = await inventory(c, api);
    const summary = { mode: apply ? 'apply' : 'preview', access: 'personas-only', created: [], updated: [], unchanged: [], backups: [] };
    // Pass one: create only missing identities. A lost POST response is reconciled
    // by exact marker inventory on the next run; POST is never blindly retried.
    for (const { key } of DASHBOARD_LEVELS) if (!ids[key]) {
      summary.created.push(key);
      if (apply) {
        ids = await inventory(c, api);
        if (!ids[key]) {
          await api('POST', `${base(c)}?folder=${enc(c.folder)}`, buildDashboards({ orgIdentifier: c.org, folder: c.folder })[key]);
          ids = await inventory(c, api);
          if (!ids[key]) fail('Created identity not discoverable; rerun to reconcile');
        }
      }
    }
    // Pass two: all runtime IDs exist before regenerating links.
    const desired = buildDashboards({ dashboardIds: ids, orgIdentifier: c.org, folder: c.folder });
    const pending = [];
    for (const { key } of DASHBOARD_LEVELS) if (ids[key]) {
      const current = await snapshot(c, api, key, ids[key]);
      if (same(current.definition, desired[key])) summary.unchanged.push(key);
      else pending.push({ key, current });
    }
    for (const { key, current } of pending) {
      summary.updated.push(key);
      if (apply) summary.backups.push(await update(c, api, key, ids[key], desired[key], current));
    }
    return summary;
  };
  return apply ? locked(c, run) : run();
}

export async function rollback(c, token, { apply = false } = {}) {
  if (!/^[0-9a-f-]{36}$/.test(token || '')) fail('Invalid backup token');
  const run = async () => {
    let backup, receipt;
    try {
      backup = JSON.parse(await fs.readFile(path.join(c.stateDir, `${token}.backup.json`), 'utf8'));
      receipt = JSON.parse(await fs.readFile(path.join(c.stateDir, `${token}.receipt.json`), 'utf8'));
    } catch { fail('Complete backup and receipt required for rollback'); }
    for (const record of [backup, receipt]) if (record.target !== c.url || record.org !== c.org || record.folder !== c.folder || record.key !== backup.key || record.id !== backup.id) fail('Rollback target mismatch');
    assertOwned(backup.before, backup.key, backup.id);
    const api = client(c); const ids = await inventory(c, api);
    if (ids[backup.key] !== backup.id) fail('Rollback identity mismatch');
    const current = await snapshot(c, api, backup.key, backup.id);
    if (receipt.hash !== current.hash || receipt.digest !== digest(current.definition)) fail('Concurrent changes prevent rollback');
    return { mode: apply ? 'rollback' : 'rollback-preview', key: backup.key,
      backups: apply ? [await update(c, api, backup.key, backup.id, backup.before, current)] : [] };
  };
  return apply ? locked(c, run) : run();
}

export async function validateQueries(c, variables, start, end) {
  const names = ['workspace_project_name', 'gen_ai_agent_name', 'gen_ai_conversation_id'];
  if (!variables || names.some(n => typeof variables[n] !== 'string') || Object.keys(variables).some(n => !names.includes(n))) fail('Supply exactly the three dashboard variables as strings');
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start <= 0 || end <= start || end - start > 86400e6) fail('Supply a positive microsecond range of at most 24 hours');
  const api = client(c); const results = [];
  for (const [key, d] of Object.entries(buildDashboards())) for (const tab of d.tabs) for (const panel of tab.panels) {
    if (panel.queryType !== 'sql') continue;
    for (const q of panel.queries) {
      const sql = q.query.replace(/\$([a-z_]+)/g, (_, name) => {
        if (!names.includes(name)) fail('Unknown query variable');
        return variables[name].replaceAll("'", "''");
      });
      let ok = false;
      try {
        const response = await api('POST', `/api/${enc(c.org)}/_search?type=${enc(q.fields.stream_type)}&validate=true`,
          { query: { sql, start_time: start, end_time: end, from: 0, size: 1 }, timeout: 10 });
        const errors = response.error || response.function_error || response.is_partial || response.partial || (response.code && response.code !== 200);
        const fields = [...q.fields.x, ...q.fields.y, ...q.fields.z].map(f => f.alias || f.column);
        const columns = new Set((response.columns || []).map(f => typeof f === 'string' ? f : f.name));
        // Empty results cannot establish output field presence; fail closed.
        ok = !errors && Array.isArray(response.hits) && response.hits.length > 0
          && fields.every(f => columns.has(f) || Object.hasOwn(response.hits[0], f));
      } catch { /* Safe summary only; no SQL, values, errors or telemetry. */ }
      results.push({ key, panel: panel.id, ok });
    }
  }
  return { checked: results.length, passed: results.filter(r => r.ok).length, failed: results.filter(r => !r.ok).length, results };
}

export async function main(args = process.argv.slice(2)) {
  const command = args[0] && !args[0].startsWith('--') ? args.shift() : 'provision';
  const flags = {};
  while (args.length) {
    const flag = args.shift();
    if (!['--apply', '--require-rbac', '--backup', '--variables', '--start', '--end'].includes(flag) || Object.hasOwn(flags, flag)) fail('Unknown or duplicate argument');
    flags[flag] = ['--apply', '--require-rbac'].includes(flag) ? true : args.shift();
    if (flags[flag] === undefined) fail('Missing argument');
  }
  const allowed = { provision: ['--apply', '--require-rbac'], rollback: ['--apply', '--backup', '--require-rbac'], validate: ['--variables', '--start', '--end', '--require-rbac'] };
  if (!allowed[command] || Object.keys(flags).some(f => !allowed[command].includes(f))) fail('Invalid command arguments');
  if (flags['--require-rbac']) fail('RBAC support is not validated; enterprise permission verification required');
  const c = config(); let result;
  if (command === 'provision') result = await provision(c, { apply: !!flags['--apply'] });
  else if (command === 'rollback') result = await rollback(c, flags['--backup'], { apply: !!flags['--apply'] });
  else {
    let variables;
    try { variables = JSON.parse(await fs.readFile(flags['--variables'], 'utf8')); } catch { fail('Cannot read variables JSON'); }
    result = await validateQueries(c, variables, Number(flags['--start']), Number(flags['--end']));
    if (result.failed) process.exitCode = 1;
  }
  console.log(JSON.stringify(result, null, 2));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error instanceof SafeError ? error.message : 'OpenObserve operation failed; check local state and configuration.'); process.exitCode = 1; });
}
