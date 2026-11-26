// Pure rules for finding a model server from what was typed as its address (`settings-discovery.js` draws them).

/** The two fields a found server fills in. The connection editor maps them onto its own form. */
export function connectionPaths() {
  return { endpoint: 'llm.endpoint', modelName: 'llm.model_name' };
}

/** A bare IP or name (no port, no path) has to be looked for; a full address is used as it is. */
export function needsServerDetection(input) {
  const raw = String(input ?? '').trim();
  if (!raw) return false;
  try {
    const url = new URL(raw.includes('://') ? raw : `http://${raw}`);
    const rest = raw.includes('://') ? raw.split('://')[1] : raw;
    const authority = rest.split('/')[0];
    const port = authority.startsWith('[') ? authority.includes(']:') : authority.includes(':');
    const path = rest.includes('/') ? rest.slice(rest.indexOf('/') + 1).replaceAll('/', '') : '';
    return url.protocol === 'http:' && !port && !path;
  } catch {
    return false;
  }
}

export function discoveryText(progress) {
  if (!progress) return '';
  const found = progress.servers.length;
  if (progress.cancelled) return `Search cancelled · ${progress.checked}/${progress.total} ports checked · ${found} server(s) found`;
  if (!progress.done) return `Checking ports ${progress.checked}/${progress.total} · ${found} server(s) found…`;
  return found ? `${found} model server(s) found. Choose the server and its model below.` : 'No model server found. Check the address and the firewall, or type the port yourself.';
}
