/* The sample store. This one guards your files, so it checks the whole round trip:
   audio in → hashed to OPFS → project JSON shrinks to a reference → reload gets the audio back.
   Plus the two things that would actually lose work: a stale hash after you edit a sample, and
   the cleanup deleting something a saved project still points at. */
const{suite,open,FILE}=require('./lib');

/* Each published build gets its own storage namespace so a bug in one can never reach a song
   saved by another. Derive the expected namespace from the file under test rather than pinning
   one, so adding a build can't quietly weaken the guarantee. */
const EXPECT_NS={'volt.html':'volt','beta.html':'voltbeta','beta2.html':'voltbeta2'}[FILE];

/* a small, valid WAV we can hand to the store and recognise coming back out */
const MAKE_WAV=`(function(seed,secs){
  const sr=8000,n=Math.floor(sr*secs),buf=new ArrayBuffer(44+n*2),dv=new DataView(buf);
  const str=(o,s)=>{for(let i=0;i<s.length;i++)dv.setUint8(o+i,s.charCodeAt(i))};
  str(0,'RIFF');dv.setUint32(4,36+n*2,true);str(8,'WAVEfmt ');dv.setUint32(16,16,true);
  dv.setUint16(20,1,true);dv.setUint16(22,1,true);dv.setUint32(24,sr,true);
  dv.setUint32(28,sr*2,true);dv.setUint16(32,2,true);dv.setUint16(34,16,true);
  str(36,'data');dv.setUint32(40,n*2,true);
  for(let i=0;i<n;i++)dv.setInt16(44+i*2,Math.round(12000*Math.sin(i*(seed+1)*0.02)),true);
  return abToB64(buf);
})`;

module.exports=async function(){
  const S=suite('sample store');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(m=>{window.mkWav=eval(m)},MAKE_WAV);

    S.head('the store comes up');
    let r=await page.evaluate(async()=>({ok:await ssInit(),ns:VNS,err:SS.lastErr}));
    S.ok('OPFS + WebCrypto available',r.ok,r.ok?'namespace /'+r.ns:r.err);
    if(!r.ok){
      S.note('no store here — the app falls back to inline samples, nothing else to check');
      return S;
    }
    if(EXPECT_NS)S.ck(FILE+' writes to its own namespace',r.ns,EXPECT_NS);
    else S.ok('unknown build '+FILE+' — namespace unchecked',false,'add it to EXPECT_NS in store.test.js');

    S.head('content addressing');
    r=await page.evaluate(async()=>{
      await ssGC();                                   /* start from a known floor */
      const a=mkWav(1,.05),b=mkWav(2,.05);
      const h1=await ssPut(a),h2=await ssPut(a),h3=await ssPut(b);
      const before=(await ssList()).length;
      const back=await ssGet(h1);
      return{h1,h2,h3,same:h1===h2,diff:h1!==h3,files:before,roundTrip:back===a,len:h1.length};
    });
    S.ok('the same audio hashes the same',r.same,r.h1);
    S.ok('  and different audio does not',r.diff);
    S.ck('storing it twice writes one file',r.files,2);
    S.ok('what comes back out is byte-identical',r.roundTrip);

    S.head('a project saves as a reference, not a payload');
    r=await page.evaluate(async()=>{
      await loadSong(KITS['mine'].make());
      /* the stock kit is all synths — give it two real samplers to store */
      for(let k=0;k<2;k++){
        const ins={name:'smp'+k,type:'sampler',params:{},sampleName:'smp'+k+'.wav',sampleB64:mkWav(k+3,.4)};
        normInst(ins);
        audio();
        try{ins.buffer=await AC.decodeAudioData(b64ToAb(ins.sampleB64))}catch(e){}
        song.instruments.push(ins);
      }
      renderInstList();
      const ins=song.instruments.find(i=>i&&(i.type==='sampler'||i.type==='sliced')&&i.sampleB64);
      if(!ins)return{skip:true};
      const fat=serialize(true).length;
      await ssSyncSong(song);
      const lean=serialize(false).length;
      const j=JSON.parse(serialize(false));
      const withSha=j.instruments.filter(i=>i.sampleSha).length;
      const withB64=j.instruments.filter(i=>i.sampleB64).length;
      return{fat,lean,withSha,withB64,shaOnIns:!!ins.sampleSha};
    });
    if(r.skip)S.note('the default kit has no samplers — skipping');
    else{
      S.ok('the instrument learned its hash',r.shaOnIns);
      S.ok('the saved project is far smaller',r.lean<r.fat/2,
        (r.fat/1024|0)+' KB with audio → '+(r.lean/1024|0)+' KB with references');
      S.ok('  and carries hashes, not audio',r.withSha>0&&r.withB64===0,r.withSha+' referenced');
    }

    S.head('reloading gets the audio back');
    r=await page.evaluate(async()=>{
      const lean=serialize(false);
      const j=JSON.parse(lean);
      const n=j.instruments.filter(i=>i.sampleSha).length;
      if(!n)return{skip:true};
      await loadSong(j);                              /* exactly what a page refresh does */
      const got=song.instruments.filter(i=>i.sampleSha&&i.sampleB64).length;
      const decoded=song.instruments.filter(i=>i.sampleSha&&i.buffer).length;
      const missing=song.instruments.filter(i=>i._missing).length;
      return{n,got,decoded,missing};
    });
    if(r.skip)S.note('nothing referenced — skipping');
    else{
      S.ck('every referenced sample is restored',r.got,r.n);
      S.ck('  and decodes to real audio',r.decoded,r.n);
      S.ck('  nothing is flagged missing',r.missing,0);
    }

    S.head('editing a sample re-hashes it (no stale references)');
    r=await page.evaluate(async()=>{
      const ins=song.instruments.find(i=>i&&i.sampleSha);
      if(!ins)return{skip:true};
      const first=ins.sampleSha;
      ins.sampleB64=mkWav(9,.05);                     /* as reverse / normalize / tempo-fit do */
      await ssSyncSong(song);
      const second=ins.sampleSha;
      const back=await ssGet(second);
      return{changed:first!==second,correct:back===ins.sampleB64};
    });
    if(r.skip)S.note('no sampler — skipping');
    else{
      S.ok('the hash follows the new audio',r.changed);
      S.ok('  and resolves to the new audio',r.correct);
    }

    S.head('cleanup only deletes what nothing points at');
    r=await page.evaluate(async()=>{
      const orphan=await ssPut(mkWav(42,.05));        /* referenced by nothing */
      const live=song.instruments.filter(i=>i&&i.sampleSha).map(i=>i.sampleSha);
      flushSave();                                    /* put the project in localStorage */
      await new Promise(x=>setTimeout(x,400));
      const res=await ssGC();
      const after=(await ssList()).map(f=>f.sha);
      return{removed:res.removed,orphanGone:after.indexOf(orphan)<0,
             liveKept:live.every(s=>after.indexOf(s)>=0),live:live.length};
    });
    S.ok('the unreferenced sample is removed',r.orphanGone,r.removed+' removed');
    S.ok('  and every sample a project uses survives',r.liveKept,r.live+' in use');

    S.head('export still contains the audio (a shared file must stand alone)');
    r=await page.evaluate(()=>{
      const j=JSON.parse(serialize(true));
      const n=song.instruments.filter(i=>i&&i.sampleB64).length;
      return{embedded:j.instruments.filter(i=>i.sampleB64).length,have:n};
    });
    S.ck('Export embeds every sample',r.embedded,r.have);

    S.head('an old project (audio inline, no hashes) still opens');
    r=await page.evaluate(async()=>{
      const j=JSON.parse(serialize(true));
      j.instruments.forEach(i=>{delete i.sampleSha});  /* pre-store format */
      await loadSong(j);
      const playable=song.instruments.filter(i=>i&&i.sampleB64).length;
      await ssSyncSong(song);                          /* migrates on the next save */
      const migrated=song.instruments.filter(i=>i&&i.sampleSha).length;
      return{playable,migrated};
    });
    S.ok('it loads with its audio',r.playable>0,r.playable+' samples');
    S.ck('  and migrates itself',r.migrated,r.playable);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
