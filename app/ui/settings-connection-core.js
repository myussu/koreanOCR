// Draft fields used by each external-server check; results belong to these exact values.
export function connectionFields(section, value) {
  const prefix = section === 'live' ? 'continuous.translation.snapshot_' : 'llm.';
  return {
    endpoint: String(value(`${prefix}endpoint`) ?? '').trim(),
    modelName: String(value(`${prefix}model_name`) ?? '').trim(),
    reasoningEffort: String(value('llm.reasoning_effort') ?? '').trim(),
  };
}

export const connectionKey = fields => JSON.stringify(fields);
export const canTestConnection = fields => !!fields.endpoint && !!fields.modelName;
export function connectionResult(state, fields) {
  return state?.key === connectionKey(fields) ? state.result : null;
}

export function connectionPaths(section) {
  const prefix=section==='live' ? 'continuous.translation.snapshot_' : 'llm.';
  return {endpoint:`${prefix}endpoint`, modelName:`${prefix}model_name`};
}

export function needsServerDetection(input) {
  const raw=String(input??'').trim();
  if(!raw)return false;
  try {
    const url=new URL(raw.includes('://') ? raw : `http://${raw}`);
    const rest=raw.includes('://') ? raw.split('://')[1] : raw;
    const authority=rest.split('/')[0];
    const port=authority.startsWith('[') ? authority.includes(']:') : authority.includes(':');
    const path=rest.includes('/') ? rest.slice(rest.indexOf('/')+1).replaceAll('/','') : '';
    return url.protocol==='http:' && !port && !path;
  } catch {return false;}
}

export function discoveryText(progress) {
  if(!progress)return '';
  const found=progress.servers.length;
  if(progress.cancelled)return `Search cancelled · ${progress.checked}/${progress.total} ports checked · ${found} server(s) found`;
  if(!progress.done)return `Checking ports ${progress.checked}/${progress.total} · ${found} server(s) found…`;
  return found ? `${found} model server(s) found. Select a server and model below.` : 'No model server found. Check the address, firewall, or enter a custom port.';
}
