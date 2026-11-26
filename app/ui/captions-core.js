// What the Status page says about captions sent by the browser extension (the `captions_status` command).
// Pure, so it is tested; hub.js only draws it.

const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** `{ on, text }` for the "YouTube captions" row; `status` is null when the shell has no such command. */
export function captionsLine(status) {
  if (!status || !status.enabled) return { on: false, text: 'off (Settings → Live subtitles)' };
  if (!status.listening) return { on: false, text: `not listening: ${status.problem || 'restart Maru'}` };
  const parts = [`port ${status.port} on this PC`, `${count(status.accepted, 'line', 'lines')} taken`];
  if (status.dropped) parts.push(`${count(status.dropped, 'repeat or empty line', 'repeats or empty lines')} dropped`);
  // A refused request is nearly always the extension holding an old code.
  if (status.refused) parts.push(`${count(status.refused, 'request', 'requests')} refused (check the code in the extension)`);
  return { on: true, text: parts.join(' · ') };
}
