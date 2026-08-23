/* Plugin parts in a WAV render.

   A render is an OfflineAudioContext of VOLT's own graph and a plugin is not in it, so plugin
   instruments came out silent — the whole point of this path. The engine renders the part and
   the browser drops it in on the right channel; the questions worth asking are all about the
   join, and none of them need a real plugin to answer:

     · are the notes handed over the ones the TRACKER actually played — with swing, with Txx,
       with the note-offs the voice model implies — or a second-guess at them?
     · does the returned audio arrive on the channel, and go through its strip like everything
       else, rather than being pasted onto the master?
     · when the engine cannot do it, does the render still finish and SAY so?

   The stub answers eng_render_vst with a WAV it builds itself, so the test can tell the plugin's
   contribution apart from everything else by frequency and by level. */
const{suite,open,INSTALL_RENDER}=require('./lib');

/* A TAURI stub that renders "plugins" as a steady tone, and remembers every job it was given. */
const PRE=`
  window.__jobs=[];
  window.__TAURI__={core:{invoke:async(cmd,args)=>{
    if(cmd==='eng_render_vst'){
      window.__jobs.push(args.job);
      if(window.__renderFail)throw new Error('the plugin did not load');
      const sr=args.job.sr,n=Math.round(args.job.dur*sr),bytes=44+n*4;
      const b=new ArrayBuffer(bytes),d=new DataView(b);
      const W=(o,s)=>{for(let i=0;i<s.length;i++)d.setUint8(o+i,s.charCodeAt(i))};
      W(0,'RIFF');d.setUint32(4,bytes-8,true);W(8,'WAVEfmt ');d.setUint32(16,16,true);
      d.setUint16(20,1,true);d.setUint16(22,2,true);d.setUint32(24,sr,true);
      d.setUint32(28,sr*4,true);d.setUint16(32,4,true);d.setUint16(34,16,true);
      W(36,'data');d.setUint32(40,n*4,true);
      /* a steady half-scale 1 kHz tone for the whole render: present everywhere, so "did it
         arrive" and "did the strip touch it" are both readable off the output */
      for(let i=0;i<n;i++){
        const v=Math.round(Math.sin(2*Math.PI*1000*i/sr)*16000);
        d.setInt16(44+i*4,v,true);d.setInt16(46+i*4,v,true);
      }
      return b;
    }
    if(cmd==='vst_patch_flush')return true;
    return null;
  }}};
`;

/* peak of the rendered WAV the app produced (INSTALL_RENDER catches the blob) */
const PEAK=async page=>page.evaluate(async()=>{
  window.__wavBlob=null;
  await renderWav();
  if(!window.__wavBlob)return-1;
  const dv=new DataView(await window.__wavBlob.arrayBuffer());
  let off=12,data=null,bits=16,ch=2;
  while(off<dv.byteLength-8){
    const id=String.fromCharCode(dv.getUint8(off),dv.getUint8(off+1),dv.getUint8(off+2),dv.getUint8(off+3));
    const sz=dv.getUint32(off+4,true);
    if(id==='fmt '){ch=dv.getUint16(off+10,true);bits=dv.getUint16(off+22,true)}
    if(id==='data'){data={off:off+8,sz};break}
    off+=8+sz+(sz&1);
  }
  if(!data)return-1;
  let pk=0;
  for(let i=data.off;i<data.off+data.sz-1;i+=2)pk=Math.max(pk,Math.abs(dv.getInt16(i,true)/32768));
  return +pk.toFixed(4);
});

module.exports=async function(){
  const S=suite('plugin render');
  const{browser,page,errs}=await open({pre:PRE+';('+INSTALL_RENDER.toString()+')()'});
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;song.swing=0;
      song.instruments=[mkInst('sub','Plug')];
      song.instruments[0].vst={path:'C:/VST3/Vital.vst3',index:0,name:'Vital'};
      /* 16 rows, so the render's 8-row pre-roll covers only the back half and the notes below
         are played exactly once. (The pre-roll is real and the plugin plays it too — that is the
         point of it, warming the FX state — it is just trimmed off the file afterwards.) */
      const pat=song.patterns[curPat];pat.rows=16;
      pat.data.forEach(trk=>{for(let r=0;r<16;r++)trk[r]=null});
      pat.data[1][0]={n:48,i:0,v:64};        /* channel 2, so "which channel" is a real question */
      pat.data[1][2]={n:50,i:0,v:64};        /* an OFF sub-beat — this is the one swing moves */
      pat.data[1][4]={n:52,i:0,v:64};
      song.order=[curPat];
    });

    S.head('the part arrives, and it arrives on the right channel');
    const pk=await PEAK(page);
    S.ok('the render is not silent',pk>.01,'peak '+pk);
    const jobs=await page.evaluate(()=>window.__jobs.map(j=>({slot:j.slot,path:j.path,n:j.notes.length,
      bpm:j.bpm,start:j.start,
      on:j.notes.filter(x=>x.on).map(x=>+x.when.toFixed(4)),off:j.notes.filter(x=>!x.on).length})));
    S.ck('one job, for the one plugin channel',jobs.length,1);
    S.ck('it names the plugin the instrument holds',jobs[0]&&jobs[0].path,'C:/VST3/Vital.vst3');
    /* the fresh instance is an undriven probe unless the job drives it — without these, every
       tempo-synced LFO/gate/delay inside the plugin rendered at the context default (120,
       stopped): the WAV came out "phasing" against the song and live playback did not */
    S.ck('the job carries the song\u2019s tempo',jobs[0]&&jobs[0].bpm,120);
    /* 16 rows at 125 ms, pre-roll = 8 rows + the .05 head: beat 0 of the song sits at 1.05 s */
    S.ok('and where beat 0 sits on the render\u2019s timeline',
      jobs[0]&&Math.abs(jobs[0].start-1.05)<.002,'start '+(jobs[0]&&jobs[0].start));
    S.ck('all three notes crossed',jobs[0]&&jobs[0].on.length,3);
    S.ck('and each one is ended',jobs[0]&&jobs[0].off,3);
    /* 120 bpm, 4 rows/beat = 125 ms a row; rows 0 and 4 are four rows apart */
    const gap=jobs[0]?+(jobs[0].on[2]-jobs[0].on[0]).toFixed(3):0;
    S.ok('row 0 to row 4 is exactly four rows',Math.abs(gap-.5)<.002,gap+' s (want 0.500)');

    S.head('it goes through the channel strip, not around it');
    const quiet=await page.evaluate(async()=>{song.trackset[1].vol=0;applyMix();return 1})
      .then(()=>PEAK(page));
    S.ok('channel 2 at zero silences the plugin part too',quiet<.002,'peak '+quiet+' (was '+pk+')');
    await page.evaluate(()=>{song.trackset[1].vol=1;applyMix()});

    S.head('the notes are the TRACKER\u2019s, not a second guess at them');
    const swung=await page.evaluate(async()=>{
      const straight=window.__jobs[0].notes.filter(x=>x.on).map(x=>x.when);
      song.swing=.6;song.swingRes=8;                 /* 60% — stored 0..1; the OFF sub-beats move late */
      window.__jobs=[];
      await renderWav();
      const on=window.__jobs[0].notes.filter(x=>x.on).map(x=>x.when);
      song.swing=0;
      return{onbeat:+(on[0]-straight[0]).toFixed(4),off:+(on[1]-straight[1]).toFixed(4)};
    });
    /* 120 bpm, 4 rows/beat: a row is 125 ms, the swing unit is 2 rows, so 60% moves the off
       sub-beat by .6 x .5 x 2 x 125 ms = 75 ms — the same arithmetic the synths get */
    S.ok('the swung note moves for the plugin too, by exactly the same amount',
      Math.abs(swung.off-.075)<.002,'row 2 is '+(swung.off*1000).toFixed(1)+' ms later (want 75.0)');
    S.ok('and the on-beat does not',Math.abs(swung.onbeat)<.001,
      'row 0 moved '+(swung.onbeat*1000).toFixed(1)+' ms');

    S.head('when the engine cannot, the render still finishes and says so');
    const failed=await page.evaluate(async()=>{window.__renderFail=1;return 1}).then(()=>PEAK(page));
    const said=await page.evaluate(()=>document.getElementById('status').textContent);
    S.ok('a WAV still comes out',failed>=0,'peak '+failed);
    S.ok('and the failure is named, not swallowed',/FAILED/.test(said),said.slice(0,120));
    await page.evaluate(()=>{window.__renderFail=0});

    S.head('the same plugin on two channels is two parts, not one');
    /* Live, a plugin slot is ONE instance and both channels' notes merge into it. A render can do
       better: each channel gets its own instance with only its own notes, so each part lands on
       the strip it belongs to instead of both arriving on one. */
    const two=await page.evaluate(async()=>{
      const pat=song.patterns[curPat];
      pat.data[3][0]={n:60,i:0,v:64};       /* same instrument, different channel */
      window.__jobs=[];
      await renderWav();
      pat.data[3][0]=null;
      return window.__jobs.map(j=>({slot:j.slot,n:j.notes.filter(x=>x.on).length})).sort((a,b)=>b.n-a.n);
    });
    S.ck('two jobs',two.length,2);
    S.ck('the second channel carries only its own note',two[1]&&two[1].n,1);
    S.ck('and the first still has all three',two[0]&&two[0].n,3);

    S.head('a browser tab is unaffected');
    const web=await page.evaluate(()=>{
      /* renderMissing is the warning that runs before a render; with no TAURI there is nothing
         to warn about, because there are no plugins to be missing */
      return renderMissing().length;
    });
    S.ck('nothing to warn about when plugins exist and instruments render',web,0);

    S.ok('no console errors',errs.length===0,errs.slice(0,3).join(' | '));
  }finally{await browser.close()}
  return S;
};
