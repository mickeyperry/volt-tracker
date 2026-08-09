/* Metronome ↔ mix alignment. The click bypasses the mix bus (that's what keeps it off the
   meters and out of renders), so it has to be delayed by exactly the mix path's latency —
   channel compressor + master FX + safety limiter — or every beat flams. Claims about timing
   are worthless unless you measure the output: both paths are rendered through the real graph
   and the onsets compared. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('metronome sync');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      audio();
      /* one probe per path, through a throwaway replica of the live graph at the live sample
         rate — the same way the app itself measures. 'mix' = a one-sample impulse into a
         channel (a note with no attack of its own); 'click' = the real metClick through its
         delay. Returns seconds relative to the scheduled time. */
      window.__probe=async kind=>{
        const SR=AC.sampleRate;
        const s={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp,metOut,loudAn,loudHP};
        let pr=null;
        try{
          AC=new OfflineAudioContext(2,Math.ceil(.6*SR),SR);chans=[];
          buildGraph();ensureChans(1);applyMix();applyFxAll();
          const ch=chans[0];ch.vg.gain.value=1;ch.duckG.gain.value=1;buildFx(ch,null);
          if(kind==='mix'){
            /* 1 ms step, same probe the app uses — Firefox mangles single-sample buffers */
            const one=AC.createBuffer(1,Math.max(8,Math.round(SR*.001)),SR);one.getChannelData(0).fill(1);
            const p=AC.createBufferSource();p.buffer=one;p.connect(ch.in);p.start(.05);
          }else{
            metClick(.05,false);
          }
          pr=AC.startRendering();
        }finally{
          AC=s.AC;master=s.master;chans=s.chans;noiseBuf=s.noiseBuf;dlyIn=s.dlyIn;dlyL=s.dlyL;dlyR=s.dlyR;rvbIn=s.rvbIn;
          masterAn=s.masterAn;masterAnL=s.masterAnL;masterAnR=s.masterAnR;MCH=s.MCH;masterComp=s.masterComp;metOut=s.metOut;loudAn=s.loudAn;loudHP=s.loudHP;
        }
        if(!pr)return-1;
        const d=(await pr).getChannelData(0);
        let peak=0;for(let i=0;i<d.length;i++){const a=Math.abs(d[i]);if(a>peak)peak=a}
        const thr=Math.max(1e-5,peak*.02);   /* relative, like the app: levels differ per browser */
        if(peak<=1e-5)return-1;
        for(let i=0;i<d.length;i++)if(Math.abs(d[i])>thr)return i/SR-.05;
        return-1;
      };
    });

    const run=async fx=>page.evaluate(async fxs=>{
      song.masterFx=fxs;applyFxAll();
      await metLatMeasure();                      /* what the app decides */
      const mix=await window.__probe('mix');      /* where a note actually lands */
      const click=await window.__probe('click');  /* where the click actually lands */
      return{lat:MET.lat,mix,click,delay:metOut.delayTime.value};
    },fx);

    const ms=v=>(v*1000).toFixed(1)+' ms';

    S.head('clean master: two compressors of lookahead, measured not guessed');
    let r=await run([]);
    S.ok('measured latency is real (≈2 compressors)',r.lat>.006&&r.lat<.04,ms(r.lat));
    S.ok('the click delay follows the measurement',Math.abs(r.delay-r.lat)<1e-4,'delay '+ms(r.delay));
    S.ok('the click lands ON the note',r.mix>=0&&r.click>=0&&Math.abs(r.click-r.mix)<.0025,
      'mix '+ms(r.mix)+' vs click '+ms(r.click));
    const clean=r;

    S.head('Limiter L∞ on the master: a ScriptProcessor buffer of extra latency');
    /* Here live and offline part ways: a live ScriptProcessor's latency includes main-thread
       dispatch alignment, so it sits a few ms above the offline render's figure and differs
       per instance (measured 56–62 ms against 56.2 offline). The click follows the LIVE mix,
       so the assertions are plausibility and stability, not sample-equality — that level of
       exactness only exists for the compressors. */
    r=await run([{type:'limit',on:true,p:{}}]);
    S.ok('the limiter adds roughly its buffer in latency',
      r.lat>clean.lat+.02&&r.lat<clean.lat+.09,ms(r.lat)+' vs clean '+ms(clean.lat));
    S.ok('the offline render agrees about the ballpark',r.mix>clean.lat+.02&&r.mix<clean.lat+.09,
      'offline mix onset '+ms(r.mix));
    S.ok('the click follows the live figure',Math.abs(r.delay-r.lat)<1e-4,'delay '+ms(r.delay));
    const again=await run([{type:'limit',on:true,p:{}}]);
    S.ok('a second measurement lands within alignment jitter',Math.abs(again.lat-r.lat)<.008,
      ms(r.lat)+' then '+ms(again.lat));

    S.head('and off again: the delay comes back down');
    r=await run([]);
    S.ok('back to the clean figure',Math.abs(r.lat-clean.lat)<.002,ms(r.lat));

    S.head('adding the Limiter THROUGH THE PANEL, like a person does');
    /* fxRebuildCur is the UI's rebuild path and it deliberately skips applyFxAll (rebuilding
       every track chain mid-play causes dropouts) — this is exactly where the fix went stale */
    r=await page.evaluate(async()=>{
      song.masterFx=[];applyFxAll();await metLatMeasure();
      const before=metOut.delayTime.value;
      masterView=true;
      song.masterFx.push({type:'limit',on:true,p:{}});
      fxRebuildCur();                                 /* what the + FX button ends up calling */
      masterView=false;
      await new Promise(res=>setTimeout(res,2200));   /* debounce (120 ms) + the ~0.6 s live measurement */
      const out={before,after:metOut.delayTime.value,safety:masterComp.ratio.value};
      song.masterFx=[];applyFxAll();
      return out;
    });
    S.ok('the click delay grows to match',r.after>r.before+.015,
      ms(r.before)+' → '+ms(r.after));
    S.ok('the safety net steps aside too',r.safety===1,'ratio '+r.safety);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
