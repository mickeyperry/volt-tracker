/* The per-channel spectrum. The interesting question isn't "does it draw" — it's whether it
   draws the RIGHT channel and puts energy at the right frequency, and whether it stays cheap. */
const{suite,open}=require('./lib');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));

module.exports=async function(){
  const S=suite('spectrum');
  const{browser,page,errs}=await open();
  try{
    S.head('it exists and follows the cursor');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      audio();
      cursor.t=0;trackPanel();
      const a0=specAnalyser()===chans[0].an;
      cursor.t=2;trackPanel();
      const a2=specAnalyser()===chans[2].an;
      masterView=true;const am=specAnalyser()===masterAn;masterView=false;trackPanel();
      return{a0,a2,am,canvas:!!$id('specCv')};
    });
    S.ok('the canvas is in the Track panel',r.canvas);
    S.ck('it reads the cursor channel, and the master in master view',[r.a0,r.a2,r.am],[true,true,true]);

    S.head('it measures the actual signal');
    /* feed one channel a known tone and check the energy lands in the right band */
    r=await page.evaluate(async()=>{
      const band=(hz,buf,an)=>{ /* index of the bin nearest hz */
        return Math.round(hz/(AC.sampleRate/2)*an.frequencyBinCount);
      };
      const an=chans[0].an;
      an.fftSize=2048;
      const osc=AC.createOscillator();osc.frequency.value=1000;
      const g=AC.createGain();g.gain.value=.3;
      osc.connect(g);g.connect(chans[0].in);osc.start();
      await new Promise(x=>setTimeout(x,300));
      const buf=new Uint8Array(an.frequencyBinCount);
      an.getByteFrequencyData(buf);
      let peak=0,at=0;
      for(let i=0;i<buf.length;i++)if(buf[i]>peak){peak=buf[i];at=i}
      const hz=at*(AC.sampleRate/2)/an.frequencyBinCount;
      /* and the neighbouring channel should see nothing */
      const an2=chans[1].an;const b2=new Uint8Array(an2.frequencyBinCount);
      an2.getByteFrequencyData(b2);
      let other=0;for(let i=0;i<b2.length;i++)if(b2[i]>other)other=b2[i];
      osc.stop();osc.disconnect();g.disconnect();
      return{hz:Math.round(hz),peak,other};
    });
    S.ok('a 1 kHz tone peaks at 1 kHz',Math.abs(r.hz-1000)<60,r.hz+' Hz, level '+r.peak);
    S.ok('  and the next channel stays empty',r.other<20,'peak '+r.other+' on the silent channel');

    S.head('drawing');
    r=await page.evaluate(async()=>{
      const cv=$id('specCv');
      const osc=AC.createOscillator();osc.frequency.value=200;
      const g=AC.createGain();g.gain.value=.4;osc.connect(g);g.connect(chans[0].in);osc.start();
      cursor.t=0;trackPanel();
      for(let i=0;i<40;i++){specDraw();await new Promise(x=>setTimeout(x,16))}
      const px=cv.getContext('2d').getImageData(0,0,cv.width,cv.height).data;
      let lit=0;for(let i=3;i<px.length;i+=4)if(px[i]>8)lit++;
      /* freeze / release */
      const froze=(specDraw(),$id('specWrap').click(),!!SPEC.frozen);
      $id('specWrap').click();
      const released=!SPEC.frozen;
      osc.stop();osc.disconnect();g.disconnect();
      return{lit,total:cv.width*cv.height,froze,released,size:[cv.width,cv.height]};
    });
    S.ok('the canvas has something on it',r.lit>200,r.lit+' lit pixels of '+r.total+' ('+r.size.join('x')+')');
    S.ok('clicking freezes a reference curve',r.froze);
    S.ok('  and clicking again releases it',r.released);

    S.head('it stays cheap');
    r=await page.evaluate(async()=>{
      await setTracks(32);
      const el=$id('patLen');el.value=128;el.dispatchEvent(new Event('change'));
      for(let t=0;t<32;t++)for(let rr=0;rr<128;rr+=2){const c=ensureCell(t,rr);c.n=36+((t*7+rr)%40);c.i=0;c.v=40}
      renderGrid();
      const t0=performance.now();
      for(let i=0;i<60;i++)specDraw();
      return +((performance.now()-t0)/60).toFixed(3);
    });
    S.ok('a spectrum frame costs under 3 ms',r<3,r+' ms per draw');

    S.head('hidden means free');
    r=await page.evaluate(()=>{
      const wrap=$id('specWrap');
      const before=wrap.offsetParent!==null;
      toggleRail();                       /* Alt+L — the whole rail goes away */
      const hidden=$id('specCv').offsetParent===null;
      const t0=performance.now();
      for(let i=0;i<200;i++)specDraw();    /* must bail immediately */
      const cost=(performance.now()-t0)/200;
      toggleRail();
      return{before,hidden,cost:+cost.toFixed(4)};
    });
    S.ok('it draws while the rail is open',r.before);
    S.ck('  and is not drawn when the rail is hidden',r.hidden,true);
    S.ok('  costing nothing then',r.cost<.05,r.cost+' ms per skipped frame');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
