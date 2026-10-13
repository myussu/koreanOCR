/** A label keeps a usable translation even if a later model refinement fails. */
export function labelStatus(data) {
  if (data.label?.trim()) return { kind: 'ready', text: data.label, hint: '' };
  if (data.source === 'pending') return { kind: 'loading', text: '•••', hint: 'Waiting for translation…' };
  return unavailable();
}

function unavailable() {
  return { kind: 'unavailable', text: '?', hint: 'No translation returned. Use Refresh labels to try again.' };
}

/** Completion only changes labels still waiting for an answer. */
export function finishLabelStatus(status, error = null) {
  if (status.kind !== 'loading') return status;
  return error
    ? { kind: 'failed', text: '!', hint: `Translation failed: ${error}` }
    : unavailable();
}

/** Scans, joins and corrections share an ID, so finish only the listed items. */
export function finishLabels(items, event) {
  const changes = [];
  for (const id of event.items) {
    const it = items.get(id);
    if (!it) continue;
    const status = finishLabelStatus(it.status, event.error);
    if (status !== it.status) changes.push([id, status]);
  }
  return changes;
}
