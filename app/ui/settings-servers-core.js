// Saved connections stage the same settings as manual entry; routing stays in the engine.
export function serverPaths(section) {
  const prefix=section==='live'?'continuous.translation.snapshot_':'llm.';
  return {endpoint:`${prefix}endpoint`,modelName:`${prefix}model_name`,provider:`${prefix}provider`,apiKey:`${prefix}api_key`};
}
export function serverAuthFields(section) {
  const paths=serverPaths(section);
  return [
    {path:paths.provider,label:'Server type',type:'select',auth:true,options:[{value:'openai',label:'OpenAI-compatible (Ollama, LM Studio, NAS)'},{value:'azure',label:'Azure OpenAI'},{value:'gemini',label:'Google Gemini'}],help:'Azure: the address is the resource URL (https://NAME.openai.azure.com) and the model is your deployment name. Gemini: the address is https://generativelanguage.googleapis.com/v1beta/openai and the model is, for example, gemini-2.5-flash.'},
    {path:paths.apiKey,label:'Translation API key',type:'password',nullable:true,auth:true,help:'Leave blank for servers without authentication. Azure needs its resource API key; Gemini an API key from Google AI Studio. Stored in config.toml on this PC.'},
  ];
}
function comparable(fields) {
  const provider=fields.provider??'openai';
  let endpoint=String(fields.endpoint??'').trim().replace(/\/$/,'').replace(/\/v1$/,'');
  if(provider==='azure'&&!endpoint.endsWith('/openai'))endpoint+='/openai';
  return JSON.stringify([endpoint,String(fields.modelName??'').trim(),provider,fields.apiKey||null]);
}
export function selectedServer(servers,fields,preferredName=null) {
  const matches=(servers??[]).filter(server=>comparable({endpoint:server.endpoint,modelName:server.model_name,provider:server.provider,apiKey:server.api_key})===comparable(fields));
  return matches.find(server=>server.name===preferredName)??matches[0]??null;
}
export function serverEdits(section,server) {
  const paths=serverPaths(section);
  return {[paths.endpoint]:server.endpoint,[paths.modelName]:server.model_name,[paths.provider]:server.provider??'openai',[paths.apiKey]:server.api_key||null};
}
/** Server types that refuse requests without an API key. */
export const needsKey = provider => provider==='azure'||provider==='gemini';

/** A pasted Azure deployment URL, split into the resource address and the deployment name. */
export function azureParts(endpoint) {
  const match=/^(https:\/\/[^/]+)\/openai\/deployments\/([^/?#]+)/.exec(String(endpoint??'').trim());
  return match ? {endpoint:match[1],modelName:decodeURIComponent(match[2])} : null;
}

/** The list without the named connection; the active address, model and key are left as they are. */
export function removeServer(servers,name) {
  return (servers??[]).filter(server=>server.name!==name);
}

export function saveServer(servers,name,fields) {
  name=String(name??'').trim();
  if(!name)throw Error('Give this connection a name, such as NAS or Azure.');
  if(/^https?:\/\/|\.(com|net|org|io|ai)\b/i.test(name))throw Error('That is an address. Put it in the server address field above, choose the server type and key, then give the connection a short name such as Azure.');
  const azure=fields.provider==='azure'&&azureParts(fields.endpoint);
  if(azure)throw Error(`For Azure, use ${azure.endpoint} as the address and ${azure.modelName} as the deployment name.`);
  if(!/^https?:\/\//.test(fields.endpoint)||!fields.modelName)throw Error('Enter a server URL and model first. Use Detect server for a bare IP.');
  if(fields.provider==='azure'&&!fields.apiKey)throw Error('Enter the Azure resource API key first.');
  if(fields.provider==='gemini'&&!fields.apiKey)throw Error('Enter the Gemini API key first.');
  const next={name,endpoint:fields.endpoint,model_name:fields.modelName,provider:fields.provider??'openai',api_key:fields.apiKey||null};
  const list=[...(servers??[])],index=list.findIndex(server=>server.name.toLowerCase()===name.toLowerCase());
  if(index<0)list.push(next);else list[index]=next;
  return list;
}
