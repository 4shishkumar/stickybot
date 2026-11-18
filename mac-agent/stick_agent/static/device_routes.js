// Device selection belongs to the browser view. Capture a path before starting an operation:
// changing the view must never retarget its follow-up requests or event stream.
const DEVICE_PATHS = /^\/api\/(?:status|config|events|pause|pet|mcp|hosts|stick|sounds|firmware|faces|ir|ask|history|new-chat|permission|wireless-mic|clips\/play|voice\/test|update\/firmware)(?:\/|\?|$)/;

export function devicePath(path, id) {
  if (!id || !DEVICE_PATHS.test(path) || path.startsWith('/api/devices/')) return path;
  return `/api/devices/${encodeURIComponent(id)}${path}`;
}

export function hostTitle(host, hosts = []) {
  if (host.display) return host.display;
  const label = host.label || host.name || 'Mac';
  const duplicate = hosts.filter(h => (h.label || h.name || 'Mac') === label).length > 1;
  return duplicate ? `${label} · ${String(host.id || '').slice(-4)}` : label;
}

export function validHostLabel(value) {
  if (/[\u0000-\u001f\u007f]/u.test(value)) return 'Use a name without control characters.';
  if (/[^\x20-\x7e]/u.test(value)) return 'Use English letters, numbers or punctuation that the stick can display.';
  if ([...value].length > 12) return 'Use at most 12 characters for the short name.';
  return '';
}
