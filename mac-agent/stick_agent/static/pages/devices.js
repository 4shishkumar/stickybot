// All devices known to this Mac, and the host labels seen on each stick.
import {h, clear, card, field, get, post, api, report, ask, store, deviceId, selectDevice} from '../lib.js';
import {hostTitle, validHostLabel} from '../device_routes.js';

export const title = 'Devices';
export const subtitle = 'Choose which stick to view. Each stick chooses which Mac receives its input.';
let root = null, generation = 0;
export function actions() { return [h('button', {class: 'btn', onclick: () => load()}, 'Refresh')]; }
export function render(el) { root = el; load(); }
export function leave() { root = null; generation++; }

async function load() {
  const run = ++generation;
  const data = await get('/api/devices');
  if (!root || run !== generation) return;
  clear(root);
  if (data.error) {
    root.appendChild(card('Devices', h('p', null, data.missing
      ? 'This app manages one stick at a time. Use the Stick page, or update StickyBot.app for multiple devices.' : data.error),
    h('a', {class: 'btn', href: data.missing ? '#stick' : '#devices'}, data.missing ? 'Open Stick' : 'Devices')));
    return;
  }
  store.multiDevice = true;
  root.appendChild(hostCard(data.host || {}));
  const devices = data.devices || [];
  for (const d of devices) root.appendChild(deviceCard(d, data.default_id));
  const scanResults = h('div', {class: 'list'});
  const scan = h('button', {class: 'btn primary', onclick: async () => {
    scan.disabled = true;
    const r = await get('/api/scan');
    scan.disabled = false;
    if (!root || run !== generation) return;
    clear(scanResults);
    if (r.error) { report(r); return; }
    const found = Array.isArray(r) ? r : r.devices || r.sticks || [];
    const fresh = found.filter(f => !devices.some(d => (d.address || d.target?.address) === f.address));
    if (!fresh.length) scanResults.appendChild(h('p', {class: 'dim'}, 'No new devices found. Turn on the stick and keep it nearby.'));
    for (const f of fresh) scanResults.appendChild(h('div', {class: 'item'},
      h('div', {class: 'what'}, h('b', null, f.name || 'StickyBot'), h('span', {class: 'sub'}, f.address)),
      h('button', {class: 'btn', onclick: async () => {
        if (report(await post('/api/devices', {address: f.address, name: f.name || 'StickyBot'}), 'Device added')) load();
      }}, 'Add device')));
  }}, 'Find nearby devices');
  root.appendChild(card('Add a device', scan, scanResults));
  const policy = await get('/api/devices/primary/api/config');
  if (root && run === generation && !policy.error) root.appendChild(policyCard(policy, devices));
}

function hostCard(host) {
  const input = h('input', {id: 'hostShortName', value: host.custom_label || '', placeholder: host.label || 'Automatic', autocomplete: 'off'});
  const saved = h('div', {class: 'small dim', role: 'status'}, host.label ? `Shown on sticks as ${host.label}` : '');
  const save = h('button', {class: 'btn primary', onclick: async () => {
    const error = validHostLabel(input.value);
    if (error) { saved.textContent = error; return; }
    save.disabled = true;
    const r = await api('/api/host', {method: 'PUT', body: JSON.stringify({label: input.value})});
    save.disabled = false;
    if (report(r, 'Mac name updated')) {
      saved.textContent = `Shown on sticks as ${r.label || r.host?.label || input.value || 'automatic name'}`;
    }
  }}, 'Save name');
  return card('This Mac', h('div', {class: 'kv'}, h('span', null, 'Computer name'), h('span', null, host.name || 'Mac')),
    field('Short name on sticks', input, 'Up to 12 English letters, numbers or punctuation. Leave empty to use an abbreviation of the computer name.', 'hostShortName'),
    h('div', {class: 'row'}, save, saved));
}

function deviceCard(d, defaultId) {
  const base = `/api/devices/${encodeURIComponent(d.id)}`;
  const status = d.status || {};
  const state = d.state || status.device || status.stick_state || {};
  const hostsState = d.hosts || status.hosts || {};
  const hosts = Array.isArray(hostsState) ? hostsState : hostsState.hosts || [];
  const active = hosts.find(host => host.active || host.id === hostsState.active);
  const name = d.name || d.target?.name || status.stick?.name || 'StickyBot';
  const selected = d.id === deviceId();
  const panel = card(name,
    h('div', {class: 'kv'},
      h('span', null, 'Model'), h('span', null, d.board || state.board || 'Identifying…'),
      h('span', null, 'Connection'), h('span', null, d.paused ? 'Paused' : status.link || d.link || 'Searching'),
      h('span', null, 'Battery'), h('span', null, Number.isFinite(state.battery) ? `${state.battery}%` : '—'),
      h('span', null, 'Input goes to'), h('span', null, active ? hostTitle(active, hosts) : 'Selected on stick')),
    h('div', {class: 'btns'},
      h('button', {class: 'btn' + (selected ? ' primary' : ''), onclick: () => { selectDevice(d.id); location.hash = '#stick'; }}, selected ? 'View selected stick' : 'View stick'),
      h('button', {class: 'btn', disabled: d.id === defaultId, onclick: async () => {
        if (report(await post(`${base}/default`), 'Default device updated')) load();
      }}, d.id === defaultId ? 'Default device' : 'Make default'),
      h('button', {class: 'btn', onclick: async () => {
        if (report(await post(`${base}/pause`, {paused: !d.paused}), d.paused ? 'Resumed' : 'Paused')) load();
      }}, d.paused ? 'Resume' : 'Pause'),
      h('button', {class: 'btn danger', onclick: async () => {
        if (!await ask(`Remove ${name} from this app? Its Bluetooth pairings and settings stay on the stick.`)) return;
        if (report(await api(base, {method: 'DELETE'}), 'Device removed')) {
          if (deviceId() === d.id) selectDevice(null);
          load();
        }
      }}, 'Remove')));
  if (hosts.length) panel.appendChild(h('div', {class: 'list'}, hosts.map(host => h('div', {class: 'item'},
    h('div', {class: 'what'}, h('b', null, hostTitle(host, hosts)), h('span', {class: 'sub'},
      `${host.name || ''}${host.active ? ' · Active input target' : ''} · ${host.online ? 'Connected' : 'Offline'}`)),
    h('button', {class: 'btn', disabled: !host.online || host.active, onclick: async () => {
      report(await post(`${base}/api/hosts/select`, {id: host.id}), 'Confirm the Mac change on the stick.');
    }}, host.active ? 'Active' : 'Request control')))));
  if (state.multiHost || hostsState.multiHost) panel.appendChild(h('button', {class: 'btn', onclick: async () => {
    const shared = hostsState.shared || state.shared;
    if (!shared && !await ask('Enable multiple Macs on this stick? Macs saved in other slots may need to pair again with this shared identity. Existing slot pairings are kept.')) return;
    const r = await post(`${base}/api/stick/proto`, {line: `@hosts shared=${shared ? 0 : 1}`});
    if (report(r, shared ? 'Legacy mode selected' : 'Shared mode selected')) load();
  }}, (hostsState.shared || state.shared) ? 'Use legacy Mac slots' : 'Enable multiple Macs'));
  return panel;
}

// Delivery policy belongs to the Mac; the primary config route addresses it explicitly.
function policyCard(config, devices) {
  const choices = {};
  const fields = [['notification_devices', 'Notifications'], ['approval_devices', 'Approvals']].map(([key, name]) => {
    const value = config[key];
    const mode = h('select', {'aria-label': name + ' destinations'},
      h('option', {value: 'default', selected: !Array.isArray(value)}, 'Default device'),
      h('option', {value: 'selected', selected: Array.isArray(value) && value.length > 0}, 'Selected devices'),
      h('option', {value: 'none', selected: Array.isArray(value) && !value.length}, 'No devices'));
    const boxes = devices.map(d => ({id: d.id, input: h('input', {type: 'checkbox', checked: Array.isArray(value) && value.includes(d.id)}), name: d.name || 'StickyBot'}));
    const list = h('div', {class: 'btns'}, boxes.map(b => h('label', {class: 'switch'}, b.input, h('span', null, b.name))));
    const update = () => { list.hidden = mode.value !== 'selected'; };
    mode.addEventListener('change', update); update();
    choices[key] = () => mode.value === 'default' ? null : mode.value === 'none' ? [] : boxes.filter(b => b.input.checked).map(b => b.id);
    return field(name, h('div', null, mode, list));
  });
  const save = h('button', {class: 'btn primary', onclick: async () => {
    save.disabled = true;
    const body = Object.fromEntries(Object.entries(choices).map(([key, read]) => [key, read()]));
    const result = await api('/api/devices/primary/api/config', {method: 'PUT', body: JSON.stringify(body)});
    save.disabled = false;
    report(result, 'Delivery preferences saved');
  }}, 'Save delivery preferences');
  return card('Where this Mac sends updates',
    h('p', {class: 'dim'}, 'Choose where Mac notifications and approval prompts appear. The first approval answer clears the other displays.'), fields, save);
}
