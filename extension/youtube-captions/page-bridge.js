// Runs in the page's own JavaScript world (manifest: "world": "MAIN"), because the list of caption tracks is only
// reachable from there. It does one thing: when the content script asks, it reads that list from YouTube's player
// and posts back plain strings. It fetches nothing, changes nothing on the page and keeps nothing.
//
// What it relies on is not a public API and can change without notice (docs/YOUTUBE_CAPTIONS.md lists it):
//   document.getElementById('movie_player').getPlayerResponse()
//     .videoDetails.videoId, .videoDetails.title
//     .captions.playerCaptionsTracklistRenderer.captionTracks[] → baseUrl, languageCode, kind ("asr" = auto-generated)
// When any of it is missing the answer says so, and the content script sends nothing.
(() => {
  'use strict';
  const TAG = 'maru-captions';

  function read() {
    try {
      const player = document.getElementById('movie_player');
      const response = player && typeof player.getPlayerResponse === 'function' ? player.getPlayerResponse() : null;
      if (!response || typeof response !== 'object') return { found: false };
      const details = response.videoDetails;
      const list = response.captions?.playerCaptionsTracklistRenderer?.captionTracks;
      const text = (value) => (typeof value === 'string' ? value : null);
      return {
        found: true,
        videoId: text(details?.videoId),
        title: text(details?.title),
        // A video without captions has no `captions` at all: that is an empty list, not a failure.
        tracks: (Array.isArray(list) ? list : []).map((t) => ({ baseUrl: text(t?.baseUrl), languageCode: text(t?.languageCode), kind: text(t?.kind), name: text(t?.name?.simpleText) ?? text(t?.name?.runs?.[0]?.text) })),
      };
    } catch {
      return { found: false };
    }
  }

  window.addEventListener('message', (event) => {
    if (event.source !== window || event.origin !== location.origin) return;
    const data = event.data;
    if (!data || data.source !== TAG || data.type !== 'tracks?') return;
    window.postMessage({ source: TAG, type: 'tracks', nonce: data.nonce, ...read() }, location.origin);
  });
})();
