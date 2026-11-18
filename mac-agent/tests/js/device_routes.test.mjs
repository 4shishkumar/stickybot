import test from 'node:test';
import assert from 'node:assert/strict';
import {devicePath, hostTitle, validHostLabel} from '../../stick_agent/static/device_routes.js';

test('device views scope device actions but leave enrollment and Mac services global', () => {
  for (const path of ['/api/status', '/api/stick/proto', '/api/firmware/ota', '/api/faces/jobs/j/cancel', '/api/permission/decide']) {
    assert.equal(devicePath(path, 'desk/one'), '/api/devices/desk%2Fone' + path);
  }
  for (const path of ['/api/devices', '/api/host', '/api/scan', '/api/install', '/api/update/install', '/api/devices/d/api/status']) {
    assert.equal(devicePath(path, 'd'), path);
  }
  assert.equal(devicePath('/api/status', null), '/api/status', 'legacy app remains unscoped');
});

test('captured job URLs remain bound to the originating device', () => {
  let selected = 'one';
  const jobs = devicePath('/api/firmware/jobs/j', selected);
  selected = 'two';
  assert.equal(jobs, '/api/devices/one/api/firmware/jobs/j');
  assert.equal(devicePath('/api/status', selected), '/api/devices/two/api/status');
});

test('host labels are human-readable and duplicates disambiguate without changing identity', () => {
  const hosts = [{id: 'id-1111', name: 'Test Laptop', label: 'Desk'}, {id: 'id-2222', name: 'Test Mini', label: 'Desk'}];
  assert.equal(hostTitle(hosts[0], hosts), 'Desk · 1111');
  assert.equal(hostTitle({...hosts[0], label: 'Work'}, hosts), 'Work');
  assert.equal(hostTitle({name: 'Test Mac'}), 'Test Mac');
  assert.equal(hostTitle({}), 'Mac');
});

test('short name input accepts reset, counts characters, and rejects control characters', () => {
  assert.equal(validHostLabel(''), '');
  assert.equal(validHostLabel('Work laptop'), '');
  assert.match(validHostLabel('é'.repeat(12)), /display/);
  assert.match(validHostLabel('a'.repeat(13)), /12/);
  assert.match(validHostLabel('Desk\nMac'), /control/);
  assert.match(validHostLabel('Desk\x7f'), /control/);
});

test('requests, job followups and status responses stay on their originating device', async () => {
  globalThis.window = {STICK_TOKEN: ''};
  globalThis.localStorage = {setItem() {}, getItem() {return null;}};
  const {selectDevice, deviceApi, post, refreshStatus, store} = await import('../../stick_agent/static/lib.js');
  const calls = [];
  let finishStatus;
  globalThis.fetch = async (path, opts) => {
    calls.push({path, method: opts.method});
    if (path.endsWith('/api/status')) return new Promise(resolve => {finishStatus = value => resolve(new Response(JSON.stringify(value), {headers: {'content-type':'application/json'}}));});
    return new Response('{}', {headers: {'content-type':'application/json'}});
  };
  selectDevice('one');
  const job = deviceApi();
  const pending = refreshStatus();
  selectDevice('two');
  await job.post('/api/firmware/jobs/job1/cancel');
  await post('/api/stick/proto', {line: '@state'});
  finishStatus({stick: {name: 'Old device'}});
  await pending;
  assert.deepEqual(store.status, {}, 'late status cannot repaint the new device');
  assert.deepEqual(calls.map(c => c.path), [
    '/api/devices/one/api/status', '/api/devices/one/api/firmware/jobs/job1/cancel', '/api/devices/two/api/stick/proto',
  ]);
  assert.equal(job.url('/api/faces/jobs/123'), '/api/devices/one/api/faces/jobs/123');
  selectDevice(null);
  await post('/api/stick/proto', {line: '@state'});
  assert.equal(calls.at(-1).path, '/api/stick/proto', 'legacy app continues to use existing API');
});
