import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { buildDashboards } from './build.mjs';
import { config, client, provision, rollback, validateQueries } from './provision.mjs';

async function fixture(t) {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'o2-test-'));
  const state = { docs: new Map(), requests: [], failCreate: false, conflict: false, redirect: false, queryError: false, badFields: false };
  let seq = 0;
  const wrap = entry => ({ version: entry.d.version, [`v${entry.d.version}`]: entry.d, hash: String(entry.rev) });
  state.add = d => { const id = d.dashboardId || String(++seq); state.docs.set(id, { d: { ...d, dashboardId: id }, rev: 1 }); return id; };
  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost'); let raw = '';
    for await (const chunk of req) raw += chunk;
    const body = raw ? JSON.parse(raw) : undefined;
    state.requests.push({ method: req.method, path: url.pathname, query: url.searchParams, body });
    const send = (status, obj) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(obj)); };
    if (state.redirect) { res.writeHead(302, { Location: '/stolen' }); res.end('Bearer SECRET'); return; }
    if (url.pathname.endsWith('/_search')) {
      const hits = state.badFields ? [{}] : [Object.fromEntries([...body.query.sql.matchAll(/ AS ([a-z_]+)/gi)].map(m => [m[1], 1]))];
      // Include all known raw output fields; query validator also checks aliases.
      if (!state.badFields) Object.assign(hits[0], Object.fromEntries(['_timestamp','trace_id','span_id','gen_ai_conversation_id','gen_ai_agent_name','workspace_project_name','workspace_path','agent_hook_event','agent_tool_archetype','span_status','duration','stage','outcome','queue','reason','result','gen_ai_provider_name','bucket','host_name','service_instance_id','quota_group'].map(k => [k, 1])));
      send(200, { hits, function_error: state.queryError ? 'SECRET raw telemetry' : '' }); return;
    }
    const id = url.pathname.split('/')[4];
    if (req.method === 'GET' && !id) { send(200, { dashboards: [...state.docs.values()].map(wrap) }); return; }
    if (req.method === 'POST') {
      const id = state.add(body);
      if (state.failCreate) { state.failCreate = false; send(503, { message: 'SECRET' }); return; }
      send(200, wrap(state.docs.get(id))); return;
    }
    const entry = state.docs.get(id);
    if (!entry) { send(404, {}); return; }
    if (req.method === 'GET') { send(200, wrap(entry)); return; }
    if (req.method === 'PUT') {
      if (state.conflict || url.searchParams.get('hash') !== String(entry.rev)) { send(409, { error: 'SECRET' }); return; }
      entry.d = { ...body, owner: 'server-owner', created: 'server-time', serverDefault: null };
      entry.d.tabs[0].panels.forEach(p => { p.config.serverDefault = false; });
      entry.rev++;
      send(200, wrap(entry));
      // Mutate only after serializing the PUT result, before verification GET.
      if (state.afterPutEdit) {
        entry.d.unmanagedNote = 'intervening writer';
        if (state.afterPutEdit === 'revision') entry.rev++;
      }
      return;
    }
    send(405, {});
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); await fs.rm(dir, { recursive: true }); });
  const c = { ...config({ OPENOBSERVE_URL: `http://127.0.0.1:${server.address().port}`, OPENOBSERVE_ORG: 'default', OPENOBSERVE_AUTH: 'Bearer SECRET' }), stateDir: path.join(dir, 'state') };
  return { c, state };
}
const writes = state => state.requests.filter(r => ['POST','PUT','DELETE'].includes(r.method));

test('preview performs zero server mutations and writes no local state', async t => {
  const { c, state } = await fixture(t);
  assert.equal((await provision(c)).created.length, 5);
  assert.equal(writes(state).length, 0);
  await assert.rejects(fs.stat(c.stateDir));
});

test('two passes resolve all links; normalized server rerun has zero writes; unmanaged preserved', async t => {
  const { c, state } = await fixture(t);
  const foreign = { version: 8, title: 'Private dashboard', description: 'Other owner', dashboardId: 'foreign' };
  state.add(foreign);
  const result = await provision(c, { apply: true });
  assert.equal(result.created.length, 5); assert.equal(result.updated.length, 5);
  assert.equal(result.backups.length, 5);
  assert.deepEqual(state.docs.get('foreign').d, foreign);
  for (const [id, { d }] of state.docs) if (id !== 'foreign') {
    assert.ok(!JSON.stringify(d).includes('dashboard_id:'));
    for (const p of d.tabs[0].panels) for (const link of p.config.drilldown) assert.ok(state.docs.has(link.data.dashboard));
  }
  const before = writes(state).length;
  assert.equal((await provision(c, { apply: true })).unchanged.length, 5);
  assert.equal(writes(state).length, before);
});

test('lost create response reconciles without creating a duplicate', async t => {
  const { c, state } = await fixture(t); state.failCreate = true;
  await assert.rejects(provision(c, { apply: true }), /HTTP 503/);
  assert.equal(state.docs.size, 1);
  await provision(c, { apply: true });
  assert.equal(state.docs.size, 5);
  assert.equal(state.requests.filter(r => r.method === 'POST').length, 5);
});

test('unmanaged title, duplicate marker, prefix marker and unsupported owned version fail before writes', async t => {
  const { c, state } = await fixture(t);
  const d = Object.values(buildDashboards())[0];
  for (const candidates of [
    [{ ...d, description: 'unmanaged' }],
    [d, { ...d, title: 'renamed' }],
    [{ ...d, description: d.description.replace('fleet-portfolio', 'fleet-portfolio-extra') }],
    [{ ...d, version: 8 }],
    [{ ...d, description: 'prefix ' + d.description }],
  ]) {
    state.docs.clear(); candidates.forEach(state.add);
    await assert.rejects(provision(c, { apply: true }));
    assert.equal(writes(state).length, 0);
  }
});

test('PUT conflict retains immutable backup and prevents overwrite', async t => {
  const { c, state } = await fixture(t); state.conflict = true;
  await assert.rejects(provision(c, { apply: true }), /HTTP 409/);
  const files = await fs.readdir(c.stateDir);
  assert.equal(files.filter(f => f.endsWith('.backup.json')).length, 1);
  assert.equal(files.filter(f => f.endsWith('.receipt.json')).length, 0);
  const put = state.requests.find(r => r.method === 'PUT');
  assert.equal(put.query.get('hash'), '1');
  assert.equal([...state.docs.values()][0].rev, 1);
});

for (const mode of ['revision', 'content-only']) test(`intervening unmanaged ${mode} edit after PUT cannot authorize rollback`, async t => {
  const { c, state } = await fixture(t);
  state.afterPutEdit = mode;
  await assert.rejects(provision(c, { apply: true }), /Concurrent changes after PUT/);
  const files = await fs.readdir(c.stateDir);
  const backups = files.filter(f => f.endsWith('.backup.json'));
  assert.equal(backups.length, 1);
  assert.equal(files.filter(f => f.endsWith('.receipt.json')).length, 0);
  const saved = JSON.parse(await fs.readFile(path.join(c.stateDir, backups[0]), 'utf8'));
  const entry = state.docs.get(saved.id);
  assert.equal(entry.d.unmanagedNote, 'intervening writer');
  assert.equal(entry.rev, mode === 'revision' ? 3 : 2);
  const before = writes(state).length;
  await assert.rejects(rollback(c, backups[0].replace('.backup.json', ''), { apply: true }), /Complete backup and receipt required/);
  assert.equal(writes(state).length, before);
  assert.equal(entry.d.unmanagedNote, 'intervening writer');
});

test('rollback verifies target, receipt and concurrent changes, previews safely and never deletes', async t => {
  const { c, state } = await fixture(t);
  const { backups } = await provision(c, { apply: true });
  const token = backups[0];
  const before = writes(state).length;
  await rollback(c, token);
  assert.equal(writes(state).length, before);
  await assert.rejects(rollback({ ...c, org: 'other' }, token), /target mismatch/);
  await assert.rejects(rollback({ ...c, url: 'https://other.invalid' }, token), /target mismatch/);
  await assert.rejects(rollback(c, '../bad'), /token/);
  const saved = JSON.parse(await fs.readFile(path.join(c.stateDir, token + '.backup.json')));
  const entry = state.docs.get(saved.id); entry.rev++;
  await assert.rejects(rollback(c, token, { apply: true }), /Concurrent/);
  entry.rev--;
  entry.d.title = 'Concurrent title';
  await assert.rejects(rollback(c, token, { apply: true }), /Concurrent/);
  entry.d.title = saved.desired.title;
  const result = await rollback(c, token, { apply: true });
  assert.equal(result.backups.length, 1);
  assert.equal(state.docs.get(saved.id).d.description, saved.before.description);
  await assert.rejects(rollback(c, token, { apply: true }), /Concurrent/);
  assert.equal(state.requests.filter(r => r.method === 'DELETE').length, 0);
});

test('auth and bodies never appear in errors, redirects refused without following', async t => {
  const { c, state } = await fixture(t); state.redirect = true;
  await assert.rejects(provision(c), error => !error.message.includes('SECRET') && error.message.includes('302'));
  assert.equal(state.requests.length, 1);
  assert.ok(!state.requests.some(r => r.path === '/stolen'));
  state.redirect = false;
  await provision(c, { apply: true });
  for (const file of await fs.readdir(c.stateDir)) assert.ok(!(await fs.readFile(path.join(c.stateDir, file), 'utf8')).includes('SECRET'));
});

test('configuration rejects insecure origins and embedded secrets; RBAC fails before requests', async t => {
  const env = { OPENOBSERVE_URL: 'https://example.com', OPENOBSERVE_ORG: 'default', OPENOBSERVE_AUTH: 'Basic abc' };
  for (const url of ['http://example.com','https://user:pass@example.com','https://example.com/path','https://example.com?secret=x']) assert.throws(() => config({ ...env, OPENOBSERVE_URL: url }));
  assert.equal(config(env).url, 'https://example.com');
  const { c, state } = await fixture(t);
  await assert.rejects(provision(c, { apply: true, requireRbac: true }), /RBAC/);
  assert.equal(state.requests.length, 0);
});

test('rollback rejects missing receipt, changed identity and forged backup ownership', async t => {
  const { c, state } = await fixture(t);
  const { backups } = await provision(c, { apply: true });
  const token = backups[0];
  const file = path.join(c.stateDir, `${token}.backup.json`);
  const receiptFile = path.join(c.stateDir, `${token}.receipt.json`);
  const receipt = await fs.readFile(receiptFile);
  const backup = JSON.parse(await fs.readFile(file));
  const count = writes(state).length;
  await fs.unlink(receiptFile);
  await assert.rejects(rollback(c, token, { apply: true }), /Complete backup/);
  await fs.writeFile(receiptFile, receipt);
  const entry = state.docs.get(backup.id);
  entry.d.description = 'foreign';
  await assert.rejects(rollback(c, token, { apply: true }), /Ownership/);
  entry.d.description = backup.desired.description;
  backup.before.description = 'foreign';
  await fs.writeFile(file, JSON.stringify(backup));
  await assert.rejects(rollback(c, token, { apply: true }), /Ownership/);
  assert.equal(writes(state).length, count);
});

test('local apply lock blocks writers before any requests', async t => {
  const { c, state } = await fixture(t);
  await fs.mkdir(c.stateDir);
  await fs.writeFile(path.join(c.stateDir, 'apply.lock'), '');
  await assert.rejects(provision(c, { apply: true }), /lock exists/);
  assert.equal(state.requests.length, 0);
});

test('timeout includes response body consumption and hides transport errors', async t => {
  const server = http.createServer((req, res) => { res.writeHead(200); res.write('{'); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  await assert.rejects(client({ url: `http://127.0.0.1:${server.address().port}`, auth: 'Bearer SECRET', timeout: 30 })('GET', '/'), /transport failed or timed out/);
});

test('query validation covers all SQL, quotes variables, validates fields and suppresses telemetry', async t => {
  const { c, state } = await fixture(t);
  const variables = { workspace_project_name: "O'Brien", gen_ai_agent_name: '*', gen_ai_conversation_id: '' };
  const result = await validateQueries(c, variables, 1000000, 2000000);
  assert.equal(result.checked, 21); assert.equal(result.failed, 0);
  assert.ok(state.requests.some(r => r.body.query.sql.includes("O''Brien")));
  assert.ok(state.requests.every(r => r.query.get('validate') === 'true'));
  assert.ok(!JSON.stringify(result).includes("O'Brien"));
  state.queryError = true;
  assert.equal((await validateQueries(c, variables, 1000000, 2000000)).failed, 21);
  state.queryError = false; state.badFields = true;
  assert.equal((await validateQueries(c, variables, 1000000, 2000000)).failed, 21);
  await assert.rejects(validateQueries(c, {}, 1, 2), /variables/);
});
