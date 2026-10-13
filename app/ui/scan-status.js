/** Keep the dot elements while other labels stream in, so their animation does not restart. */
export function paintLabelStatus(el, status, korean) {
  const text = el.firstChild;
  if (text.dataset.status !== status.kind) {
    text.replaceChildren();
    text.dataset.status = status.kind;
    if (status.kind === 'loading') {
      for (let i = 0; i < 3; i++) {
        const dot = document.createElement('span');
        dot.className = 'loading-dot';
        dot.textContent = '•';
        dot.setAttribute('aria-hidden', 'true');
        text.append(dot);
      }
    }
  }
  if (status.kind !== 'loading' && text.textContent !== status.text) text.textContent = status.text;
  el.dataset.status = status.kind;
  el.setAttribute('aria-busy', String(status.kind === 'loading'));
  el.setAttribute('aria-label', `${korean}: ${status.hint || status.text}`);
}
