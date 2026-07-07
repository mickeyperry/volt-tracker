/* Relays GET requests from the VOLT page (via content.js) to the local Everything
   HTTP server. Hard-coded to loopback — this extension can never reach anything else. */
'use strict';
const BASE = 'http://127.0.0.1:8666';

const api = typeof browser !== 'undefined' ? browser : chrome;

api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (!msg || msg.type !== 'es-fetch' || typeof msg.path !== 'string' || !msg.path.startsWith('/')) {
    sendResponse({ ok: false, error: 'bad request' });
    return;
  }
  (async () => {
    try {
      const r = await fetch(BASE + msg.path, { method: 'GET' });
      const buf = await r.arrayBuffer();
      let bin = '';
      const u = new Uint8Array(buf), CH = 0x8000;
      for (let i = 0; i < u.length; i += CH) bin += String.fromCharCode.apply(null, u.subarray(i, i + CH));
      sendResponse({ ok: r.ok, status: r.status, b64: btoa(bin) });
    } catch (e) {
      sendResponse({ ok: false, error: String(e) });
    }
  })();
  return true; /* async sendResponse */
});
