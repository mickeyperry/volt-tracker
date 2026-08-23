/* Plugin FX CHAINS in a WAV render — the other half of vstrender.test.js.

   A channel whose VST FX chain lives in the engine renders in passes now: pass 1 taps the
   channel's strip at fxOut (post ported FX, pre drive — where the live engine splices the chain
   in) and sends the dry stem to `eng_render_fx`; pass 2 cuts the dry tail and plays the wet stem
   into the drive instead. The questions worth asking are about the JOIN, and none need a real
   plugin:

     · does the stem actually carry the channel's audio, at the song's tempo, with beat 0 named?
     · does the wet copy REPLACE the dry one — never sum with it? (Two copies at once is the
       phasing this path exists to prevent.)
     · does the whole mix really pass through a MASTER chain?
     · when the engine cannot, does the render still finish, ship the channel dry, and SAY so?

   The stub answers eng_render_fx with the input scaled by __fxgain, so "applied", "replaced" and
   "absent" are all readable off the output level. */
const{suite,open,INSTALL_RENDER}=require('./lib');

const PRE=`
  window.__fxjobs=[];window.__fxgain=1;window.__fxfail=0;window.__vstjobs=0;
  window.__TAURI__={core:{invoke:async(cmd,args)=>{
    if(cmd==='eng_render_vst'){
      /* a plugin INSTRUMENT: a steady half-scale 1 kHz tone, same stub as vstrender.test.js */
      window.__vstjobs++;
      window.__vstparams=args.job.params||[];
      const sr=args.job.sr,n=Math.round(args.job.dur*sr),bytes=44+n*4;
      const b=new ArrayBuffer(bytes),d=new DataView(b);
      const W=(o,s)=>{for(let i=0;i<s.length;i++)d.setUint8(o+i,s.charCodeAt(i))};
      W(0,'RIFF');d.setUint32(4,bytes-8,true);W(8,'WAVEfmt ');d.setUint32(16,16,true);
      d.setUint16(20,1,true);d.setUint16(22,2,true);d.setUint32(24,sr,true);
      d.setUint32(28,sr*4,true);d.setUint16(32,4,true);d.setUint16(34,16,true);
      W(36,'data');d.setUint32(40,n*4,true);
      for(let i=0;i<n;i++){
        const v=Math.round(Math.sin(2*Math.PI*1000*i/sr)*16000);
        d.setInt16(44+i*4,v,true);d.setInt16(46+i*4,v,true);
      }
      return b;
    }
    if(cmd==='eng_render_fx'){
      if(window.__fxfail)throw new Error('the chain did not load');
      const u8=args instanceof Uint8Array?args:new Uint8Array(args);
      const dv=new DataView(u8.buffer,u8.byteOffset,u8.byteLength);
      const jlen=dv.getUint32(0,true);
      const off=(4+jlen+3)&~3;
      const job=JSON.parse(new TextDecoder().decode(u8.subarray(4,4+jlen)));
      const f=new Float32Array(u8.buffer,u8.byteOffset+off,job.frames*2);
      let sum=0;for(let i=0;i<f.length;i++)sum+=f[i]*f[i];
      job.rms=Math.sqrt(sum/f.length);
      window.__fxjobs.push(job);
      /* the "chain": gain by __fxgain, back as a 16-bit stereo WAV */
      const n=job.frames,bytes=44+n*4,b=new ArrayBuffer(bytes),d=new DataView(b);
      const W=(o,s)=>{for(let i=0;i<s.length;i++)d.setUint8(o+i,s.charCodeAt(i))};
      W(0,'RIFF');d.setUint32(4,bytes-8,true);W(8,'WAVEfmt ');d.setUint32(16,16,true);
      d.setUint16(20,1,true);d.setUint16(22,2,true);d.setUint32(24,job.sr,true);
      d.setUint32(28,job.sr*4,true);d.setUint16(32,4,true);d.setUint16(34,16,true);
      W(36,'data');d.setUint32(40,n*4,true);
      const g=window.__fxgain;
      for(let i=0;i<n;i++){
        const l=Math.max(-1,Math.min(1,f[2*i]*g)),r=Math.max(-1,Math.min(1,f[2*i+1]*g));
        d.setInt16(44+i*4,l<0?l*0x8000:l*0x7FFF,true);
        d.setInt16(46+i*4,r<0?r*0x8000:r*0x7FFF,true);
      }
      return b;
    }
    if(cmd==='vst_patch_flush')return true;
    return null;
  }}};
`;

/* peak of the WAV the app produced (INSTALL_RENDER catches the blob) */
const PEAK=async page=>page.evaluate(async()=>{
  window.__wavBlob=null;
  await renderWav();
  if(!window.__wavBlob)return-1;
  const dv=new DataView(await window.__wavBlob.arrayBuffer());
  let off=12,data=null;
  while(off<dv.byteLength-8){
    const id=String.fromCharCode(dv.getUint8(off),dv.getUint8(off+1),dv.getUint8(off+2),dv.getUint8(off+3));
    const sz=dv.getUint32(off+4,true);
    if(id==='data'){data={off:off+8,sz};break}
    off+=8+sz+(sz&1);
  }
  if(!data)return-1;
  let pk=0;
  for(let i=data.off;i<data.off+data.sz-1;i+=2)pk=Math.max(pk,Math.abs(dv.getInt16(i,true)/32768));
  return +pk.toFixed(4);
});

module.exports=async function(){
  const S=suite('plugin FX chains in a render');
  const{browser,page,errs}=await open({pre:PRE+';('+INSTALL_RENDER.toString()+')()'});
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;song.swing=0;
      song.instruments=[mkInst('sub','Bass')];
      const pat=song.patterns[curPat];pat.rows=16;
      pat.data.forEach(trk=>{for(let r=0;r<16;r++)trk[r]=null});
      pat.data[2][0]={n:36,i:0,v:64};        /* channel 3, a PORTED instrument — the stem is real audio */
      pat.data[2][8]={n:36,i:0,v:64};
      song.order=[curPat];
      song.trackset[2].dly=0;song.trackset[2].rvb=0;
    });

    S.head('with no chain anywhere, nothing crosses');
    const pA=await PEAK(page);
    S.ok('the render sounds',pA>.02,'peak '+pA);
    let jobs=await page.evaluate(()=>window.__fxjobs.length);
    S.ck('and eng_render_fx was never asked',jobs,0);

    S.head('a unity chain REPLACES the dry copy — it never sums with it');
    await page.evaluate(()=>{song.trackset[2].fxVsts=[{path:'C:/VST3/EQ.vst3',index:0,name:'EQ'}];window.__fxjobs=[];window.__fxgain=1});
    const pB=await PEAK(page);
    /* if the dry tail were still connected this would come out ~2x (+6 dB) — the render "phasing" */
    S.ok('unity-wet level equals the no-chain level',pB>pA*.8&&pB<pA*1.25,'peak '+pB+' vs dry '+pA);

    S.head('and the job describes the song, not a guess at it');
    const job=await page.evaluate(()=>{const j=window.__fxjobs[0];return j&&{track:j.track,bpm:j.bpm,start:+j.start.toFixed(3),links:j.links,rms:+j.rms.toFixed(4)}});
    S.ck('one job, for the one chain channel',await page.evaluate(()=>window.__fxjobs.length),1);
    S.ck('keyed by the mixer track',job&&job.track,2);
    S.ck('at the song\u2019s tempo',job&&job.bpm,120);
    /* 16 rows at 125 ms, pre-roll 8 rows + the .05 head */
    S.ok('with beat 0 named on the stem\u2019s timeline',job&&Math.abs(job.start-1.05)<.002,'start '+(job&&job.start));
    S.ck('the chain crosses with its live positions',job&&job.links.map(l=>l.pos),[0]);
    S.ok('and the stem carries the channel\u2019s audio',job&&job.rms>.001,'stem rms '+(job&&job.rms));

    S.head('a silent chain silences the channel — the wet copy really owns it');
    await page.evaluate(()=>{window.__fxgain=0});
    const pC=await PEAK(page);
    S.ok('nothing of the dry path survives',pC<.01,'peak '+pC);
    await page.evaluate(()=>{window.__fxgain=1});

    S.head('the MASTER chain hears the whole mix');
    await page.evaluate(()=>{song.trackset[2].fxVsts=[];song.masterFxVsts=[{path:'C:/VST3/Glue.vst3',index:0,name:'Glue'}];window.__fxjobs=[];window.__fxgain=0});
    const pD=await PEAK(page);
    const mjob=await page.evaluate(()=>{const j=window.__fxjobs[0];return j&&{track:j.track,rms:+j.rms.toFixed(4)}});
    S.ck('the job is the master\u2019s (track -1)',mjob&&mjob.track,-1);
    S.ok('the mix reached it',mjob&&mjob.rms>.001,'mix rms '+(mjob&&mjob.rms));
    S.ok('and a silent master chain is a silent file',pD<.01,'peak '+pD);
    await page.evaluate(()=>{song.masterFxVsts=[];window.__fxgain=1});

    S.head('when the engine cannot, the channel ships DRY and the render says so');
    await page.evaluate(()=>{song.trackset[2].fxVsts=[{path:'C:/VST3/EQ.vst3',index:0,name:'EQ'}];window.__fxfail=1});
    const pE=await PEAK(page);
    const said=await page.evaluate(()=>document.getElementById('status').textContent);
    S.ok('the channel still sounds',pE>pA*.8,'peak '+pE+' vs dry '+pA);
    S.ok('and the failure is named',/FAILED/.test(said)&&/channel 3/.test(said),said.slice(0,140));
    await page.evaluate(()=>{window.__fxfail=0;song.trackset[2].fxVsts=[]});

    S.head('a plugin INSTRUMENT on a chain channel goes through the chain — once');
    /* the dnb8 shape: Sytrus on channel 10 with a chain behind it. The part must reach the stem
       (pass 1's vstrParts), come back wet, and NOT also play dry in the final pass — a doubled
       part would read as ~2x here. */
    await page.evaluate(()=>{
      song.instruments[0].vst={path:'C:/VST3/Synth.vst3',index:0,name:'Synth'};
      window.__fxjobs=[];window.__vstjobs=0;window.__fxgain=1;
    });
    const pNoChain=await PEAK(page);          /* plugin part, no chain — the baseline */
    await page.evaluate(()=>{
      song.trackset[2].fxVsts=[{path:'C:/VST3/EQ.vst3',index:0,name:'EQ'}];
      /* plugin automation lanes: a vp sweep on the instrument, a vf value on the chain link —
         these must travel WITH the offline jobs, not poke the live instances */
      const pat=song.patterns[curPat];
      pat.auto={'2.vp0.100':Array.from({length:16},(_,r)=>r===0?0.25:(r===8?0.75:null)),
                '2.vf0.101':Array.from({length:16},(_,r)=>r===0?0.5:null)};
      window.__fxjobs=[];window.__vstjobs=0;
    });
    const pChain=await PEAK(page);
    const j2=await page.evaluate(()=>{const j=window.__fxjobs[0];return j&&{rms:+j.rms.toFixed(4)}});
    S.ok('the part is in the stem the chain was fed',j2&&j2.rms>.01,'stem rms '+(j2&&j2.rms));
    S.ok('unity chain keeps the part at its own level — not doubled',
      pChain>pNoChain*.8&&pChain<pNoChain*1.25,'peak '+pChain+' vs no-chain '+pNoChain);
    /* the chain render is two passes, but the part cache means the stem pass renders the part
       and the mix pass reuses it — one engine ask for the whole render */
    const vstAsks=await page.evaluate(()=>window.__vstjobs);
    S.ck('and the part crossed the engine once per render, cache included',vstAsks,1);

    S.head('plugin automation lanes travel with the jobs, timed');
    const auto=await page.evaluate(()=>({vp:window.__vstparams||[],
      vf:(window.__fxjobs[0]&&window.__fxjobs[0].links[0].params)||[]}));
    /* the vp lane: rows 0 and 8 — plus row 8 again from the pre-roll (it schedules the pattern's
       back half first, which is correct: the sweep is warm at the trim point like everything else) */
    S.ok('the vp sweep reached the instrument job',
      auto.vp.some(e=>e.id===100&&e.value===0.25)&&auto.vp.some(e=>e.id===100&&e.value===0.75),
      JSON.stringify(auto.vp));
    S.ok('  in time order',auto.vp.every((e,i)=>!i||e.when>=auto.vp[i-1].when),
      auto.vp.map(e=>+e.when.toFixed(3)).join(','));
    S.ok('the vf value reached its chain link',
      auto.vf.length===1&&auto.vf[0].id===101&&auto.vf[0].value===0.5,JSON.stringify(auto.vf));
    /* row 0 at 120 bpm behind an 8-row pre-roll and the .05 head = 1.05 s */
    S.ok('  at its row’s time',auto.vf[0]&&Math.abs(auto.vf[0].when-1.05)<.002,
      'when '+(auto.vf[0]&&auto.vf[0].when));
    await page.evaluate(()=>{song.instruments[0].vst=null;song.trackset[2].fxVsts=[];delete song.patterns[curPat].auto});

    S.head('a muted chain channel does not cost a chain render');
    await page.evaluate(()=>{song.trackset[2].fxVsts=[{path:'C:/VST3/EQ.vst3',index:0,name:'EQ'}];song.trackset[2].mute=true;window.__fxjobs=[]});
    await PEAK(page);
    S.ck('nothing was asked of the engine',await page.evaluate(()=>window.__fxjobs.length),0);
    await page.evaluate(()=>{song.trackset[2].mute=false;song.trackset[2].fxVsts=[]});

    S.head('a bounce carries plugin audio now — a Sytrus channel used to bounce to silence');
    await page.evaluate(()=>{song.instruments[0].vst={path:'C:/VST3/Synth.vst3',index:0,name:'Synth'};cursor.t=2;window.__vstjobs=0});
    const b=await page.evaluate(async()=>{
      const ins=await bounce('pat',{quiet:true});
      if(!ins||!ins.buffer)return{ok:false};
      let pk=0;const d=ins.buffer.getChannelData(0);
      for(let i=0;i<d.length;i++){const a=Math.abs(d[i]);if(a>pk)pk=a}
      return{ok:true,pk:+pk.toFixed(3),asks:window.__vstjobs};
    });
    S.ok('the bounced sample contains the plugin part',b.ok&&b.pk>.05,b.ok?('peak '+b.pk):'no instrument came back');
    S.ok('  rendered by the engine, once',b.asks===1,'eng_render_vst asked '+b.asks+'x');
    await page.evaluate(()=>{song.instruments.pop();song.instruments[0].vst=null});

    S.ok('no console errors',errs.length===0,errs.slice(0,3).join(' | '));
  }finally{await browser.close()}
  return S;
};
