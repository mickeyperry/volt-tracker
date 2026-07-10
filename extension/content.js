/* Bridge between the VOLT page and the extension background.
   Only ever injected on the origins listed in manifest.json. */
'use strict';
(() => {
  const api = typeof browser !== 'undefined' ? browser : chrome;
  const ORIGIN = window.location.origin && window.location.origin !== 'null' ? window.location.origin : '*';
  const announce = () => window.postMessage({ src: 'volt-es-bridge', type: 'ready' }, ORIGIN);
  announce();
  window.addEventListener('message', e => {
    if (e.source !== window) return;
    const d = e.data;
    if (!d || d.src !== 'volt-page') return;
    if (d.type === 'ping') { announce(); return; }
    if (d.type !== 'req' || typeof d.path !== 'string') return;
    api.runtime.sendMessage({ type: 'es-fetch', path: d.path }, res => {
      window.postMessage({
        src: 'volt-es-bridge', type: 'res', id: d.id,
        res: res || { ok: false, error: (api.runtime.lastError && api.runtime.lastError.message) || 'no response' }
      }, ORIGIN);
    });
  });
})();
