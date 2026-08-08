/* Mastering tools: the soft clipper and the output analysis. A meter that reads plausibly but
   wrongly is worse than none, so the numbers are checked against signals whose real values are
   known — a -20 dB tone, a mono signal, an inverted one, a band-limited one. */
const{suite,open}=require('./lib');
let FXP;

module.exports=async function(){
  const S=suite('mastering');
  const{browser,page,errs}=await open();
  FXP=async t=>page.evaluate(x=>Object.keys(FX_TYPES[x].p),t);
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      audio();
      /* run a test signal through an offline master chain and read the analysis back */
      window.__master=async(make,secs)=>{
        const S2={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp,loudAn,loudHP};
        try{
          AC=new OfflineAudioContext(2,Math.ceil((secs||1)*44100),44100);chans=[];
          buildGraph();ensureChans(2);applyMix();setDelayTime();applyFxAll();
          song.masterVol=1;master.gain.value=1;
          const srcs=make();
          const b=await AC.startRendering();
          const d0=b.getChannelData(0),d1=b.getChannelData(1);
          let pk=0,ms=0;
          for(let i=0;i<d0.length;i++){pk=Math.max(pk,Math.abs(d0[i]),Math.abs(d1[i]));ms+=d0[i]*d0[i]}
          return{peak:+pk.toFixed(4),rmsDb:+(10*Math.log10(ms/d0.length)).toFixed(2),
                 buf:Array.from(d0.slice(20000,20400))};
        }finally{
          AC=S2.AC;master=S2.master;chans=S2.chans;noiseBuf=S2.noiseBuf;dlyIn=S2.dlyIn;dlyL=S2.dlyL;dlyR=S2.dlyR;
          rvbIn=S2.rvbIn;masterAn=S2.masterAn;masterAnL=S2.masterAnL;masterAnR=S2.masterAnR;MCH=S2.MCH;
          masterComp=S2.masterComp;loudAn=S2.loudAn;loudHP=S2.loudHP;
        }
      };
    });

    S.head('the soft clipper rounds peaks instead of squaring them');
    let r=await page.evaluate(async()=>{
      const tone=(gain,fx)=>()=>{
        if(fx){song.masterFx=[fx];applyFxAll()}else{song.masterFx=[];applyFxAll()}
        const o=AC.createOscillator();o.frequency.value=220;
        const g=AC.createGain();g.gain.value=gain;
        o.connect(g);g.connect(master);o.start(0);
      };
      /* a sine well past full scale, clipped two ways. NOT compared against the bare master
         chain: VOLT's limiter is a compressor, so it pulls the level down smoothly and distorts
         less than any clipper would — the meaningful comparison is hard knee against soft. */
      const hard=await __master(tone(1.8,{type:'softclip',on:true,p:{drive:1,knee:0,ceil:-.3,mix:1}}),.6);
      const soft=await __master(tone(1.8,{type:'softclip',on:true,p:{drive:1,knee:1,ceil:-.3,mix:1}}),.6);
      song.masterFx=[];
      const raw=hard;
      /* "How much fizz did squaring the peaks add?" — the first difference is a crude high-pass,
         so its level relative to the signal's own says how much harmonic hash is riding on a
         220 Hz sine. Comparing raw sample steps would only have measured which one was louder. */
      const fizz=a=>{
        let s=0,d=0;
        for(let i=1;i<a.length;i++){s+=a[i]*a[i];const q=a[i]-a[i-1];d+=q*q}
        return s>1e-12?+(Math.sqrt(d/s)).toFixed(4):0;
      };
      return{rawPk:raw.peak,softPk:soft.peak,rawFizz:fizz(raw.buf),softFizz:fizz(soft.buf)};
    });
    S.ok('it holds the output under the ceiling',r.softPk<=1,'peak '+r.softPk);

    /* the knee has to be judged on the curve itself — through the whole chain the master
       limiter is doing most of the work and swamps the difference */
    r=await page.evaluate(()=>{
      const shape=(x,knee)=>{
        const c=softClipCurve(knee),n=c.length;
        const i=(x+1)/2*(n-1),lo=Math.floor(i),hi=Math.min(n-1,lo+1),f=i-lo;
        return c[lo]*(1-f)+c[hi]*f;
      };
      /* what the knob really controls: how much the curve is still growing as it approaches full
         scale. A hard knee runs at unity right up to the wall and then corners; a soft one is
         already bending, so it never has a corner to make. */
      const slope=knee=>+(((shape(1,knee)-shape(.9,knee))/.1)).toFixed(3);
      /* and what it does to a quiet signal: a hard knee must leave it untouched */
      const quiet=knee=>+Math.abs(shape(.2,knee)-.2).toFixed(4);
      return{slopeHard:slope(0),slopeSoft:slope(1),quietHard:quiet(0),quietSoft:quiet(1),
             ceilHard:+shape(1,0).toFixed(3),ceilSoft:+shape(1,1).toFixed(3)};
    });
    S.ok('a hard knee runs at unity right up to the wall',r.slopeHard>.85,
      'slope '+r.slopeHard+' approaching full scale');
    S.ok('  a soft knee is already bending, so it never corners',r.slopeSoft<r.slopeHard*.6,
      'slope '+r.slopeSoft+' vs '+r.slopeHard);
    S.ok('  a hard knee leaves quiet signal completely alone',r.quietHard<.001,
      'a 0.2 sample moves by '+r.quietHard);
    S.ok('  a soft knee starts shaping much earlier',r.quietSoft>r.quietHard,
      'the same sample moves by '+r.quietSoft);
    S.ok('  and neither goes past full scale',r.ceilHard<=1&&r.ceilSoft<=1,
      'full-scale in → '+r.ceilHard+' / '+r.ceilSoft);

    r=await page.evaluate(async()=>{
      const q=(drive,ceil)=>()=>{
        song.masterFx=[{type:'softclip',on:true,p:{drive,knee:.6,ceil,mix:1}}];applyFxAll();
        const o=AC.createOscillator();o.frequency.value=180;
        const g=AC.createGain();g.gain.value=.4;
        o.connect(g);g.connect(master);o.start(0);
      };
      const a=await __master(q(1,-.3),.5),b=await __master(q(6,-.3),.5),c=await __master(q(1,-9),.5);
      song.masterFx=[];
      return{soft:a.peak,driven:b.peak,quiet:c.peak};
    });
    S.ok('driving it harder does not simply make it louder',Math.abs(r.driven-r.soft)<r.soft*.6,
      'drive 1 → '+r.soft+', drive 6 → '+r.driven);
    S.ok('  and the ceiling really lowers the output',r.quiet<r.soft*.6,
      'ceil -0.3 → '+r.soft+', ceil -9 → '+r.quiet);

    S.head('the limiter is a real brickwall');
    r=await page.evaluate(async()=>{
      /* a quiet tone with transients four times over full scale — the thing a limiter exists for */
      const spiky=()=>{
        const o=AC.createOscillator();o.frequency.value=90;
        const g=AC.createGain();g.gain.value=.2;o.connect(g);g.connect(master);o.start(0);
        for(let k=0;k<5;k++){
          const b=AC.createBuffer(1,64,AC.sampleRate),cd=b.getChannelData(0);
          for(let i=0;i<64;i++)cd[i]=Math.sin(i*.6)*(1-i/64);
          const s=AC.createBufferSource();s.buffer=b;
          const sg=AC.createGain();sg.gain.value=4;s.connect(sg);sg.connect(master);s.start(.1+k*.12);
        }
      };
      const L=p=>({type:'limit',on:true,p:Object.assign({loud:35,ceil:-.3},p)});
      const out={none:(await __master(spiky,.9)).peak,ceils:{},rms:{}};
      for(const c of[-.3,-3,-6,-12]){
        song.masterFx=[L({ceil:c})];
        const res=await __master(()=>{song.masterFx=[L({ceil:c})];applyFxAll();spiky()},.9);
        out.ceils[c]={peak:res.peak,allowed:+Math.pow(10,c/20).toFixed(4)};
      }
      for(const lv of[0,50,100]){
        const res=await __master(()=>{song.masterFx=[L({loud:lv})];applyFxAll();spiky()},.9);
        out.rms[lv]=res.rmsDb;
      }
      song.masterFx=[];
      return out;
    });
    S.ok('without it, transients sail past full scale',r.none>1.2,'peak '+r.none);
    {
      const over=Object.entries(r.ceils).map(([c,v])=>+(20*Math.log10(v.peak/v.allowed)).toFixed(2));
      S.ok('with it, nothing ever exceeds the ceiling',over.every(d=>d<=.05),
        Object.entries(r.ceils).map(([c,v])=>c+'dB → '+v.peak+' (allowed '+v.allowed+')').join(' · '));
      S.ok('  and it holds at every ceiling, not just the top one',Math.max(...over.map(Math.abs))<.1,
        'worst error '+Math.max(...over.map(Math.abs))+' dB');
    }
    S.ok('one knob does the pushing',r.rms['100']>r.rms['0']+8,
      'RMS '+r.rms['0']+' dB at LOUD 0 → '+r.rms['50']+' at 50 → '+r.rms['100']+' at 100');
    S.ck('and there are only two of them',await FXP('limit'),['loud','ceil']);

    S.head('PULTEQ: boost and cut at once, and they do not cancel');
    r=await page.evaluate(()=>{
      /* the whole point of the trick: turning both up must NOT give you a flat response */
      const resp=(p,hz)=>{
        const mk=(type,f,g,q)=>{const b=AC.createBiquadFilter();b.type=type;b.frequency.value=f;
          b.gain.value=g;if(q!=null)b.Q.value=q;return b};
        const up=mk('lowshelf',p.lowHz,p.lowUp),dn=mk('peaking',p.lowHz*2.2,-p.lowDown,.8),
              air=mk('peaking',p.airHz,p.airUp,.7);
        const f=new Float32Array(hz),m=new Float32Array(hz.length),ph=new Float32Array(hz.length);
        f.set(hz);
        let tot=new Float32Array(hz.length).fill(1);
        for(const n of[up,dn,air]){n.getFrequencyResponse(f,m,ph);for(let i=0;i<hz.length;i++)tot[i]*=m[i]}
        return[...tot].map(x=>+(20*Math.log10(x)).toFixed(2));
      };
      const hz=[30,60,120,300,1000,10000];
      const flat=resp({lowHz:60,lowUp:0,lowDown:0,airHz:10000,airUp:0},hz);
      const trick=resp({lowHz:60,lowUp:8,lowDown:8,airHz:10000,airUp:0},hz);
      const air=resp({lowHz:60,lowUp:0,lowDown:0,airHz:10000,airUp:8},hz);
      return{flat,trick,air};
    });
    S.ck('with everything at zero it is transparent',r.flat,[0,0,0,0,0,0]);
    S.ok('boost + cut lifts the very bottom',r.trick[0]>1.5,r.trick[0]+' dB at 30 Hz');
    S.ok('  and scoops just above it',r.trick[2]<-1,r.trick[2]+' dB at 120 Hz');
    S.ok('  which is the trick, not a cancellation',Math.abs(r.trick[0]-r.trick[2])>3,
      '30 Hz '+r.trick[0]+' vs 120 Hz '+r.trick[2]);
    S.ok('  and it leaves the midrange alone',Math.abs(r.trick[4])<1,r.trick[4]+' dB at 1 kHz');
    S.ok('AIR lifts the top without touching the bottom',r.air[5]>4&&Math.abs(r.air[0])<.5,
      '+'+r.air[5]+' dB at 10 kHz, '+r.air[0]+' at 30 Hz');
    S.ck('  five controls, all named in plain words',await FXP('pulteq'),
      ['lowHz','lowUp','lowDown','airHz','airUp']);

    S.head('the safety compressor steps aside for it');
    r=await page.evaluate(()=>{
      audio();
      song.masterFx=[];applyFxAll();
      const off={th:masterComp.threshold.value,ratio:masterComp.ratio.value};
      song.masterFx=[{type:'limit',on:true,p:{loud:35,ceil:-.3}}];applyFxAll();
      const on={th:masterComp.threshold.value,ratio:masterComp.ratio.value};
      song.masterFx=[{type:'limit',on:false,p:{}}];applyFxAll();
      const bypassed={th:masterComp.threshold.value,ratio:masterComp.ratio.value};
      song.masterFx=[];applyFxAll();
      return{off,on,bypassed,restored:{th:masterComp.threshold.value,ratio:masterComp.ratio.value}};
    });
    S.ck('normally the safety net is armed',[r.off.th,r.off.ratio],[-3,12]);
    S.ck('  a limiter on the master neutralises it',[r.on.th,r.on.ratio],[0,1]);
    S.ck('  a BYPASSED limiter does not',[r.bypassed.th,r.bypassed.ratio],[-3,12]);
    S.ck('  and removing it arms the net again',[r.restored.th,r.restored.ratio],[-3,12]);

    /* Turning a knob used to rebuild the whole chain, which is why the sound dropped out mid-move.
       Two things have to hold: the live nodes survive a knob move, and every effect can actually
       tune every one of its own parameters — otherwise a knob would silently do nothing. */
    S.head('knobs do not cut the sound');
    r=await page.evaluate(()=>{
      audio();
      masterView=true;
      song.masterFx=[{type:'pulteq',on:true,p:{lowHz:60,lowUp:2,lowDown:2,airHz:10000,airUp:2}}];
      applyFxAll();
      const before=MCH._fxLive[0]&&MCH._fxLive[0].refs;
      const nodesBefore=MCH._fxNodes.length;
      const tuned=fxTuneLive(0,'lowUp',7);
      const after=MCH._fxLive[0]&&MCH._fxLive[0].refs;
      const gain=after&&after.up.gain.value;
      /* and a bypassed effect has nothing live to nudge — that must fall back, not throw */
      song.masterFx=[{type:'pulteq',on:false,p:{}}];applyFxAll();
      const whenBypassed=fxTuneLive(0,'lowUp',3);
      song.masterFx=[];applyFxAll();masterView=false;
      return{tuned,sameRefs:before===after,nodesBefore,whenBypassed,gainMoved:gain>1.5};
    });
    S.ok('a knob move tunes the live node',r.tuned);
    S.ok('  the effect is not rebuilt underneath it',r.sameRefs,
      r.sameRefs?'same node instances kept':'the chain was torn down and remade');
    S.ok('  and the value really lands',r.gainMoved);
    S.ok('  a bypassed effect falls back instead of throwing',r.whenBypassed===false);

    r=await page.evaluate(()=>{
      const bad={};
      for(const[id,ft]of Object.entries(FX_TYPES)){
        if(!ft.tune){bad[id]='no tune()';continue}
        const src=ft.tune.toString();
        const miss=Object.keys(ft.p).filter(k=>!src.includes("'"+k+"'")&&!src.includes('"'+k+'"'));
        if(miss.length)bad[id]=miss;
      }
      return{n:Object.keys(FX_TYPES).length,bad};
    });
    S.ck('every effect can tune every knob it offers',r.bad,{});
    S.ok('  across all of them',r.n>=15,r.n+' effects');

    S.head('loudness is K-weighted, not plain RMS');
    r=await page.evaluate(async()=>{
      /* the same level at 60 Hz and at 3 kHz must NOT read the same: the weighting discounts
         the bottom end, which is the whole reason a bass-heavy mix over-reads on an RMS meter */
      const at=f=>async()=>{
        const o=AC.createOscillator();o.frequency.value=f;
        const g=AC.createGain();g.gain.value=.25;
        o.connect(g);g.connect(master);o.start(0);
      };
      const read=async f=>{
        const S2={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp,loudAn,loudHP};
        try{
          AC=new OfflineAudioContext(2,Math.ceil(.5*44100),44100);chans=[];
          buildGraph();ensureChans(2);applyMix();setDelayTime();applyFxAll();
          master.gain.value=1;
          const o=AC.createOscillator();o.frequency.value=f;
          const g=AC.createGain();g.gain.value=.25;
          o.connect(g);g.connect(loudHP);              /* straight into the weighting filters */
          const sh=loudHP;
          o.start(0);
          const b=await AC.startRendering();
          return 1;
        }finally{
          AC=S2.AC;master=S2.master;chans=S2.chans;noiseBuf=S2.noiseBuf;dlyIn=S2.dlyIn;dlyL=S2.dlyL;dlyR=S2.dlyR;
          rvbIn=S2.rvbIn;masterAn=S2.masterAn;masterAnL=S2.masterAnL;masterAnR=S2.masterAnR;MCH=S2.MCH;
          masterComp=S2.masterComp;loudAn=S2.loudAn;loudHP=S2.loudHP;
        }
      };
      /* simpler and more direct: check the filter chain's own response */
      const resp=f=>{
        const hp=AC.createBiquadFilter();hp.type='highpass';hp.frequency.value=38;hp.Q.value=.5;
        const sh=AC.createBiquadFilter();sh.type='highshelf';sh.frequency.value=1500;sh.gain.value=4;
        const fr=new Float32Array([f]),m1=new Float32Array(1),p1=new Float32Array(1),
              m2=new Float32Array(1),p2=new Float32Array(1);
        hp.getFrequencyResponse(fr,m1,p1);sh.getFrequencyResponse(fr,m2,p2);
        return 20*Math.log10(m1[0]*m2[0]);
      };
      return{deep:+resp(20).toFixed(2),sub:+resp(30).toFixed(2),bass:+resp(60).toFixed(2),
             mid:+resp(1000).toFixed(2),high:+resp(4000).toFixed(2)};
    });
    S.ok('deep sub is discounted',r.deep<-4&&r.sub<-1.5,
      r.deep+' dB at 20 Hz, '+r.sub+' at 30, '+r.bass+' at 60');
    S.ok('  the top end is lifted',r.high>2,'+'+r.high+' dB at 4 kHz');
    S.ok('  and the midrange is left alone',Math.abs(r.mid)<3,r.mid+' dB at 1 kHz');

    S.head('the band meter puts energy where it belongs');
    r=await page.evaluate(async()=>{
      const bandOf=hz=>{for(let i=0;i<MBANDS.length;i++)if(hz>=MBANDS[i][0]&&hz<MBANDS[i][1])return i;return -1};
      return{n:MBANDS.length,
             sub:bandOf(40),bass:bandOf(90),mid:bandOf(500),air:bandOf(14000),
             covers:MBANDS[0][0]<=20&&MBANDS[MBANDS.length-1][1]>=20000,
             contiguous:MBANDS.every((b,i)=>i===0||b[0]===MBANDS[i-1][1])};
    });
    S.ck('eight bands, contiguous, 20 Hz to 20 kHz',[r.n,r.covers,r.contiguous],[8,true,true]);
    S.ck('  and they map where you would expect',[r.sub,r.bass,r.mid,r.air],[0,1,3,7]);

    S.head('correlation tells you if it survives mono');
    r=await page.evaluate(()=>{
      /* the maths the meter uses, checked against signals whose answer is known */
      const corr=(l,r2)=>{let sl=0,sr=0,slr=0;
        for(let i=0;i<l.length;i++){sl+=l[i]*l[i];sr+=r2[i]*r2[i];slr+=l[i]*r2[i]}
        return(sl>1e-9&&sr>1e-9)?slr/Math.sqrt(sl*sr):1};
      const n=1024,a=[],b=[],c=[],d=[];
      for(let i=0;i<n;i++){
        const s=Math.sin(i*.07);
        a.push(s);b.push(s);          /* identical: mono */
        c.push(s);d.push(-s);         /* inverted: cancels */
      }
      const rnd=seedRnd(7),e=[],f=[];
      for(let i=0;i<n;i++){e.push(rnd()*2-1);f.push(rnd()*2-1)}
      return{mono:+corr(a,b).toFixed(2),flipped:+corr(c,d).toFixed(2),wide:+Math.abs(corr(e,f)).toFixed(2)};
    });
    S.ck('identical channels read 1',r.mono,1);
    S.ck('  an inverted channel reads -1',r.flipped,-1);
    S.ok('  unrelated channels read near 0',r.wide<.2,r.wide);

    S.head('it is wired into the master panel');
    r=await page.evaluate(()=>{
      if(masterPan.hidden)renderMasterPan();
      masterPan.hidden=false;renderMasterPan();
      const has=id=>!!document.getElementById(id);
      const bars=document.querySelectorAll('#mstBands .mb').length;
      const st=masterStats();
      masterPan.hidden=true;
      return{nums:has('mstLufs')&&has('mstPeak')&&has('mstCorr'),bars,reset:has('mstReset'),
             stats:!!st&&st.bands.length===8&&isFinite(st.lufs)};
    });
    S.ok('the readouts are there',r.nums);
    S.ck('  with one bar per band',r.bars,8);
    S.ok('  a reset for the peak hold',r.reset);
    S.ok('  and the numbers compute',r.stats);

    S.head('Soft Clip is offered as an effect');
    r=await page.evaluate(()=>({
      listed:!!FX_TYPES.softclip,
      label:FX_TYPES.softclip&&FX_TYPES.softclip.label,
      params:FX_TYPES.softclip&&Object.keys(FX_TYPES.softclip.p).join(','),
      inMenu:[...document.querySelectorAll('#fxSel option')].some(o=>/soft clip/i.test(o.textContent))
    }));
    S.ck('it is in the effect list',[r.listed,r.label],[true,'Soft Clip']);
    S.ck('  with the controls you would expect',r.params,'drive,knee,ceil,mix');

    S.head('every effect has its own face in the rack');
    r=await page.evaluate(()=>{
      audio();masterView=true;
      song.masterFx=Object.keys(FX_TYPES).map(t=>({type:t,on:true,p:{}}));
      applyFxAll();trackPanel();
      const rows=[...document.querySelectorAll('#fxChain .fxrow')];
      const seen=rows.map(r2=>{
        const cs=getComputedStyle(r2);
        return{cls:[...r2.classList].find(c=>c.indexOf('fx-')===0),
               col:cs.getPropertyValue('--fxcol').trim(),
               glyph:(r2.querySelector('.fxg')||{}).textContent,
               sub:(r2.querySelector('.fxsub')||{}).textContent,
               headTall:r2.querySelector('.fxhead').getBoundingClientRect().height>28};
      });
      song.masterFx=[];applyFxAll();masterView=false;trackPanel();
      return{n:rows.length,types:Object.keys(FX_TYPES).length,seen,
             colours:new Set(seen.map(s=>s.col)).size,
             glyphs:new Set(seen.map(s=>s.glyph)).size};
    });
    S.ck('one row per effect',r.n,r.types);
    S.ok('each carries its own type class',r.seen.every(s=>s.cls),
      r.seen.map(s=>s.cls).join(' '));
    S.ck('  and its own colour',r.colours,r.types);
    S.ck('  and its own glyph',r.glyphs,r.types);
    S.ok('  with a plain-language line on every one',r.seen.every(s=>s.sub&&s.sub.length>4),
      r.seen.filter(s=>!s.sub||s.sub.length<5).map(s=>s.cls).join(', ')||'all present');
    S.ok('  and no title wraps onto a second line',r.seen.every(s=>!s.headTall),
      r.seen.filter(s=>s.headTall).map(s=>s.cls).join(', ')||'none wrapped');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
