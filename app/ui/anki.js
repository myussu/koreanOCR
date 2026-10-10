// The "Add to Anki" popover's rules: choosing a deck (pick or type a new one), tags, and what to say
// about the result. No DOM here, so it is tested.

/** A deck name as Anki wants it: parts joined by `::`, none empty or padded (mirrors the app's `clean_deck`). */
export const cleanDeck = (s) =>
  String(s ?? '')
    .split('::')
    .map((p) => p.trim())
    .filter(Boolean)
    .join('::');

const sameDeck = (a, b) => a.toLowerCase() === b.toLowerCase();

/**
 * The rows under the deck box. Empty box: the suggestions (best first). Typing: decks that match,
 * those starting with what was typed first, then those with a part that does, then any that contain it;
 * and if the typed name is not an existing deck, a row that creates it.
 * @returns {{ deck: string, kind: 'suggest' | 'existing' | 'new', reason?: string, exists: boolean }[]}
 */
export function deckChoices(query, decks, suggestions, max = 7) {
  const q = cleanDeck(query);
  if (!q) return suggestions.slice(0, max).map((s) => ({ deck: s.deck, kind: 'suggest', reason: s.reason, exists: s.exists }));
  const lower = q.toLowerCase();
  const rank = (d) => {
    const l = d.toLowerCase();
    if (l === lower) return 0;
    if (l.startsWith(lower)) return 1;
    if (l.split('::').some((part) => part.startsWith(lower))) return 2;
    return l.includes(lower) ? 3 : null;
  };
  const found = decks
    .map((d) => [rank(d), d])
    .filter(([r]) => r !== null)
    .sort((a, b) => a[0] - b[0] || a[1].length - b[1].length || a[1].localeCompare(b[1]))
    .slice(0, max)
    .map(([, d]) => ({ deck: d, kind: 'existing', exists: true }));
  if (decks.some((d) => sameDeck(d, q))) return found;
  const create = { deck: q, kind: 'new', reason: 'create this deck', exists: false };
  return found.length ? [...found, create] : [create];
}

/** A tag as Anki keeps it: no spaces or quotes (mirrors the app's `tag_of`). */
export function tagOf(raw) {
  const t = String(raw ?? '')
    .trim()
    .replace(/[\s"',]/g, '_');
  return t.replace(/^[_:]+|[_:]+$/g, '');
}

/** Add what was typed (several tags may be separated by commas or spaces) without repeating one. */
export function addTags(tags, typed) {
  const out = [...tags];
  for (const piece of String(typed ?? '').split(/[,\s]+/)) {
    const t = tagOf(piece);
    if (t && !out.some((x) => x.toLowerCase() === t.toLowerCase())) out.push(t);
  }
  return out;
}

export const removeTag = (tags, tag) => tags.filter((t) => t !== tag);

/** How long Anki says you hold a card, in words. */
export function intervalText(days) {
  if (!days || days < 1) return 'still being learned';
  if (days < 30) return `held for ${Math.round(days)} day${Math.round(days) === 1 ? '' : 's'}`;
  if (days < 365) return `held for ${Math.round(days / 30)} month${Math.round(days / 30) === 1 ? '' : 's'}`;
  return `held for ${(days / 365).toFixed(1)} years`;
}

/** What to tell the user once a word has gone in. */
export function addedText(reply) {
  if (reply.result !== 'added') return '';
  const cards = reply.cards === 1 ? '1 card' : `${reply.cards} cards`;
  const reverse = reply.reverse_deck ? `; the English → Korean one is in ${reply.reverse_deck}` : '';
  return `✓ Added to ${reply.deck} (${cards}${reverse})${reply.sync ? ' · syncing with AnkiWeb' : ''}`;
}

/** Which reverse setting to send: the configured one unless the box says otherwise. */
export function reverseChoice(wanted, configured) {
  if (!wanted) return 'off';
  return configured === 'off' ? 'subdeck' : null;
}
