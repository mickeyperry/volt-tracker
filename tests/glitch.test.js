/* GLTCH-9, the burst percussion engine. Everything here is measured from rendered audio: that a
   note really is a burst of several attacks, that the controls do what they say, that the presets
   are all in the same loudness ballpark, and that a note always renders identically — which is
   what lets you trust a WAV export. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('GLTCH-9');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      /* render one note of an instrument, offline, and hand back some measurements */
      window.__hit=async(params,note)=>{
        const ins=mkInst('glitch','x',params);
        const S2={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp};
        try{
          AC=new OfflineAudioContext(2,Math.ceil(2.5*44100),44100);chans=[];
          buildGraph();ensureChans(2);applyMix();setDelayTime();applyFxAll();
          const v=trig(ins,note==null?48:note,.05,0,1,0);
          if(v)v.release(2);
          const b=await AC.startRendering();
          const d=b.getChannelData(0),sr=b.sampleRate;
          let pk=0;for(let i=0;i<d.length;i++)pk=Math.max(pk,Math.abs(d[i]));
          const w=Math.round(sr*.002),env=[];
          for(let i=0;i+w<d.length;i+=w){let s=0;for(let j=0;j<w;j++)s+=d[i+j]*d[i+j];env.push(Math.sqrt(s/w))}
          let attacks=0,last=-99,first=-1,end=0;
          for(let i=1;i<env.length;i++)
            if(env[i]>pk*.1&&env[i]-env[i-1]>pk*.05&&i-last>2){attacks++;last=i;if(first<0)first=i*w/sr}
          for(let i=d.length-1;i>0;i--)if(Math.abs(d[i])>pk*.01){end=i/sr;break}
          /* rough brightness: zero crossings per second */
          let zc=0;for(let i=1;i<d.length;i++)if((d[i-1]<0)!==(d[i]<0))zc++;
          return{peak:+pk.toFixed(4),attacks,onset:+first.toFixed(4),length:+(end-.05).toFixed(3),
                 zc:Math.round(zc/(b.duration)),
                 slice:Array.from(d.slice(2300,4200))};
        }finally{
          AC=S2.AC;master=S2.master;chans=S2.chans;noiseBuf=S2.noiseBuf;dlyIn=S2.dlyIn;dlyL=S2.dlyL;dlyR=S2.dlyR;
          rvbIn=S2.rvbIn;masterAn=S2.masterAn;masterAnL=S2.masterAnL;masterAnR=S2.masterAnR;MCH=S2.MCH;masterComp=S2.masterComp;
        }
      };
    });

    S.head('one note, a burst of hits');
    let r=await page.evaluate(async()=>{
      const base={src:0,pitch:900,sweep:-.4,gap:.05,accel:0,decay:.02,fall:0,cutoff:9000,res:4,crush:0,chaos:0,rel:.02};
      const one=await __hit(Object.assign({},base,{hits:1}));
      const six=await __hit(Object.assign({},base,{hits:6}));
      return{one:one.attacks,six:six.attacks,oneLen:one.length,sixLen:six.length};
    });
    S.ck('hits 1 gives a single attack',r.one,1);
    S.ok('  hits 6 gives several',r.six>=5,r.six+' attacks');
    S.ok('  and the burst lasts longer',r.sixLen>r.oneLen*2,r.oneLen+'s vs '+r.sixLen+'s');

    S.head('gap and accel shape the burst');
    r=await page.evaluate(async()=>{
      const base={src:0,pitch:900,sweep:-.4,hits:5,accel:0,decay:.015,fall:0,cutoff:9000,res:4,crush:0,chaos:0,rel:.02};
      const slow=await __hit(Object.assign({},base,{gap:.09}));
      const fast=await __hit(Object.assign({},base,{gap:.02}));
      const speedUp=await __hit(Object.assign({},base,{gap:.09,accel:.9}));
      const dragOut=await __hit(Object.assign({},base,{gap:.05,accel:-.9}));
      return{slow:slow.length,fast:fast.length,speedUp:speedUp.length,dragOut:dragOut.length};
    });
    S.ok('a wider gap spreads the burst out',r.slow>r.fast*2,r.fast+'s at gap .02 vs '+r.slow+'s at .09');
    S.ok('  accel up pulls it in tighter',r.speedUp<r.slow,r.speedUp+'s vs '+r.slow+'s');
    S.ok('  accel down drags it out',r.dragOut>r.fast,r.dragOut+'s');

    S.head('the four sources really are different');
    r=await page.evaluate(async()=>{
      const base={pitch:800,sweep:-.4,hits:1,gap:.05,accel:0,decay:.12,fall:0,cutoff:12000,res:4,crush:0,chaos:0,rel:.05};
      const out={};
      for(const s of[0,1,2,3]){const h=await __hit(Object.assign({},base,{src:s}));out[s]={zc:h.zc,peak:h.peak}}
      return out;
    });
    S.ok('noise is far busier than the tuned sources',r['0'].zc>r['3'].zc*3,
      'noise '+r['0'].zc+' crossings/s vs sine '+r['3'].zc);
    S.ok('  each source makes sound',[0,1,2,3].every(s=>r[s].peak>.05),
      [0,1,2,3].map(s=>r[s].peak).join(', '));
    S.ok('  and none of them clips',[0,1,2,3].every(s=>r[s].peak<1.2),
      'peaks '+[0,1,2,3].map(s=>r[s].peak).join(', '));

    S.head('chaos scatters the grains, and only when you ask');
    r=await page.evaluate(async()=>{
      const base={src:1,pitch:700,sweep:-.4,hits:6,gap:.04,accel:.2,decay:.03,fall:.3,cutoff:8000,res:6,crush:0,rel:.02};
      const tidy=await __hit(Object.assign({},base,{chaos:0}));
      const tidy2=await __hit(Object.assign({},base,{chaos:0}));
      const wild=await __hit(Object.assign({},base,{chaos:.9}));
      const same=(a,b)=>a.every((x,i)=>x===b[i]);
      return{stable:same(tidy.slice,tidy2.slice),changed:!same(tidy.slice,wild.slice)};
    });
    S.ok('chaos 0 is perfectly regular',r.stable);
    S.ok('  and turning it up changes the sound',r.changed);

    S.head('the same note always renders the same');
    r=await page.evaluate(async()=>{
      const p={src:0,pitch:1200,sweep:-.5,hits:7,gap:.03,accel:.5,decay:.02,fall:.5,cutoff:9000,res:14,crush:.4,chaos:.7,rel:.02};
      const a=await __hit(p,48),b=await __hit(p,48),c=await __hit(p,53);
      const same=(x,y)=>x.every((v,i)=>v===y[i]);
      return{repeatable:same(a.slice,b.slice),byNote:!same(a.slice,c.slice),loud:a.peak};
    });
    S.ok('twice through the renderer, identical samples',r.repeatable,'peak '+r.loud);
    S.ok('  a different note gives a different mangle',r.byNote);

    S.head('noise is seeded, so exports match what you heard');
    r=await page.evaluate(async()=>{
      /* this covers hats, snares and claps too, which used a fresh random buffer per render */
      const grab=async()=>{
        const S2={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp};
        try{
          AC=new OfflineAudioContext(2,Math.ceil(.5*44100),44100);chans=[];buildGraph();
          return Array.from(noiseBuf.getChannelData(0).slice(0,500));
        }finally{
          AC=S2.AC;master=S2.master;chans=S2.chans;noiseBuf=S2.noiseBuf;dlyIn=S2.dlyIn;dlyL=S2.dlyL;dlyR=S2.dlyR;
          rvbIn=S2.rvbIn;masterAn=S2.masterAn;masterAnL=S2.masterAnL;masterAnR=S2.masterAnR;MCH=S2.MCH;masterComp=S2.masterComp;
        }
      };
      const a=await grab(),b=await grab();
      let mean=0;a.forEach(x=>mean+=x);mean/=a.length;
      let vari=0;a.forEach(x=>vari+=(x-mean)*(x-mean));vari/=a.length;
      return{same:a.every((x,i)=>x===b[i]),mean:+mean.toFixed(3),sd:+Math.sqrt(vari).toFixed(3)};
    });
    S.ok('two audio contexts get the identical noise buffer',r.same);
    S.ok('  and it is still white noise',Math.abs(r.mean)<.1&&r.sd>.4&&r.sd<.75,
      'mean '+r.mean+', sd '+r.sd);

    S.head('the presets are usable and evenly matched');
    r=await page.evaluate(async()=>{
      const out={};
      for(const pre of PRESETS.filter(p=>p.type==='glitch')){
        const h=await __hit(pre.p);
        out[pre.name]={peak:h.peak,attacks:h.attacks,len:h.length};
      }
      return out;
    });
    {
      const names=Object.keys(r);
      const peaks=names.map(n=>r[n].peak);
      S.ck('there are several to start from',names.length,8);
      S.ok('all of them make a sound',peaks.every(p=>p>.15),
        names.map(n=>n+' '+r[n].peak).join(' · '));
      S.ok('  none clips',peaks.every(p=>p<1),'loudest '+Math.max(...peaks));
      S.ok('  and they sit within about 3x of each other',Math.max(...peaks)/Math.min(...peaks)<3.2,
        'quietest '+Math.min(...peaks)+', loudest '+Math.max(...peaks));
      S.ok('  the ratchets really ratchet',r['Machine Gun'].attacks>=3&&r['Ratchet Tick'].attacks>=3,
        'Machine Gun '+r['Machine Gun'].attacks+', Ratchet Tick '+r['Ratchet Tick'].attacks);
      S.ok('  and the one-shot does not',r['Laser Zap'].attacks===1,'Laser Zap '+r['Laser Zap'].attacks);
    }

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
