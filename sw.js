/* VOLT service worker — network-first so online users always get the newest
   build; the cache only answers when you're offline. */
const CACHE='volt-v1';
self.addEventListener('install',e=>self.skipWaiting());
self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));
self.addEventListener('fetch',e=>{
  if(e.request.method!=='GET')return;
  const url=new URL(e.request.url);
  if(url.origin!==self.location.origin)return; /* never touch Everything/API calls */
  e.respondWith(
    fetch(e.request).then(r=>{
      if(r.ok){const c=r.clone();caches.open(CACHE).then(x=>x.put(e.request,c))}
      return r;
    }).catch(()=>caches.match(e.request))
  );
});
