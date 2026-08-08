/* Bounce. The whole claim is "this instrument sounds like that channel did", so the checks are
   about the AUDIO: that the bounce contains the right hits at the right times, that it carries
   the channel's processing and its sidechain ducking, and that it does NOT contain the other
   channels or the master FX. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('bounce');
  const{browser,page,errs}=await open();
  try{
    /* two channels: a trigger on every beat, a steady tone under it */
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      await setTracks(2);
      song.bpm=120;song.lpb=4;song.order=[0];
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      pat.data.forEach(c=>c.fill(null));
      song.instruments=[
        {name:'trig',type:'kick',params:{pitch:50,punch:1,decay:.2,drive:0}},
        {name:'tone',type:'sub',params:{harm:0,rel:.4}}
      ];
      song.instruments.forEach(normInst);
      for(let r=0;r<16;r+=4){const c=ensureCell(0,r);c.n=48;c.i=0;c.v=64}
      const p=ensureCell(1,0);p.n=48;p.i=1;p.v=64;
      song.trackset.forEach(normTs);
      song.trackset.forEach(t=>{t.duck=0});
      audio();applyMix();renderGrid();
      /* measuring helpers */
      window.RMS=(buf,from,to)=>{
        const d=buf.getChannelData(0);let s=0,n=0;
        for(let i=Math.max(0,Math.round(from*buf.sampleRate));i<Math.min(d.length,Math.round(to*buf.sampleRate));i++){s+=d[i]*d[i];n++}
        return n?Math.sqrt(s/n):0;
      };
      window.PEAK=buf=>{const d=buf.getChannelData(0);let m=0;for(let i=0;i<d.length;i++)if(Math.abs(d[i])>m)m=Math.abs(d[i]);return m};
    });

    S.head('it renders the channel you are on');
    let r=await page.evaluate(async()=>{
      cursor.t=1;                                     /* the tone */
      const res=await renderBounce(1,'pat');
      return{secs:+(res.buf.length/res.buf.sampleRate).toFixed(2),
             peak:+PEAK(res.buf).toFixed(3),label:res.label,
             ch:res.buf.numberOfChannels};
    });
    S.ok('there is audio in it',r.peak>.05,'peak '+r.peak);
    S.ok('  about the right length',r.secs>1.5&&r.secs<6,r.secs+'s for a 2 s pattern plus tail');
    S.ck('  in stereo, labelled',[r.ch,/pattern/.test(r.label)],[2,true]);

    S.head('and only that channel');
    r=await page.evaluate(async()=>{
      /* bounce the TRIGGER track: it hits 4 times, so its bounce has 4 bursts and gaps between */
      cursor.t=0;
      const res=await renderBounce(0,'pat');
      const beat=60/song.bpm;
      const onHit=RMS(res.buf,.01,.06),between=RMS(res.buf,beat*.6,beat*.9);
      return{onHit:+onHit.toFixed(4),between:+between.toFixed(4)};
    });
    S.ok('the trigger bounce is loud on the beat',r.onHit>.02,'RMS '+r.onHit);
    S.ok('  and quiet between hits — the other channel is not in it',r.between<r.onHit*.25,
      'between-hits RMS '+r.between+' vs '+r.onHit);

    S.head('the channel’s own processing is baked in');
    r=await page.evaluate(async()=>{
      cursor.t=1;
      song.trackset[1].vol=.9;applyMix();
      const loud=PEAK((await renderBounce(1,'pat')).buf);
      song.trackset[1].vol=.15;applyMix();
      const quiet=PEAK((await renderBounce(1,'pat')).buf);
      song.trackset[1].vol=.9;applyMix();
      return{loud:+loud.toFixed(3),quiet:+quiet.toFixed(3)};
    });
    S.ok('the fader is part of the bounce',r.quiet<r.loud*.4,r.loud+' at 90% vs '+r.quiet+' at 15%');

    S.head('sidechain ducking survives — the muted trigger still pumps');
    r=await page.evaluate(async()=>{
      const ts=song.trackset[1];
      ts.duck=0;applyMix();
      const flat=await renderBounce(1,'pat');
      ts.duck=.85;ts.duckSrc=0;ts.duckRel=.16;applyMix();
      const pumped=await renderBounce(1,'pat');
      const beat=60/song.bpm;
      const dip=b=>+(RMS(b,beat+.005,beat+.045)/RMS(b,beat*1.8,beat*1.95)).toFixed(3);
      ts.duck=0;applyMix();
      return{flat:dip(flat.buf),pumped:dip(pumped.buf)};
    });
    S.ok('without ducking the tone is steady',r.flat>.85,'ratio '+r.flat);
    S.ok('  with it the bounce dips on every trigger note',r.pumped<r.flat*.7,
      'ratio '+r.pumped+' vs '+r.flat+' — and the trigger channel is silent in the file');

    S.head('master FX stay out of a single channel');
    r=await page.evaluate(async()=>{
      cursor.t=1;
      const before=PEAK((await renderBounce(1,'pat')).buf);
      song.masterVol=.1;
      const after=PEAK((await renderBounce(1,'pat')).buf);
      song.masterVol=.75;applyMix();
      return{before:+before.toFixed(3),after:+after.toFixed(3)};
    });
    S.ok('the master fader does not scale a channel bounce',Math.abs(r.before-r.after)<.02,
      r.before+' vs '+r.after+' with the master at 10%');

    S.head('scopes');
    r=await page.evaluate(async()=>{
      sel={a:{t:1,c:0,r:0},b:{t:1,c:7,r:3}};
      const selSteps=bounceSteps('sel').steps.length;   /* measured WHILE the selection exists */
      const one=(await renderBounce(1,'sel')).buf.duration;
      sel=null;
      const all=(await renderBounce(1,'pat')).buf.duration;
      const steps={sel:selSteps,pat:bounceSteps('pat').steps.length,
                   song:bounceSteps('song').steps.length};
      return{one:+one.toFixed(2),all:+all.toFixed(2),steps};
    });
    S.ck('a 4-row selection bounces 4 rows, not the pattern',[r.steps.sel,r.steps.pat],[4,16]);
    S.ok('  and the whole song covers the order',r.steps.song>=16,r.steps.song+' rows');
    S.ok('  a 4-row bounce is shorter than a 16-row one',r.one<r.all,r.one+'s vs '+r.all+'s');

    /* Mickey heard this before any test did: the bounce sat a hair ahead of the metronome.
       Two causes, both silent killers — a wrap-around pre-roll baking the loop's tail into the
       first milliseconds, and ~12 ms of DynamicsCompressor lookahead pushing everything late. */
    S.head('it starts exactly on the beat');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);pat.data[1].fill(null);
      for(const k of[0,4]){const c=ensureCell(0,k);c.n=48;c.i=0;c.v=64}
      const b=(await renderBounce(0,'pat')).buf,d=b.getChannelData(0),sr=b.sampleRate;
      let on=-1;for(let i=0;i<d.length;i++)if(Math.abs(d[i])>.001){on=i/sr;break}
      let second=-1;for(let i=Math.round(.4*sr);i<d.length;i++)if(Math.abs(d[i])>.05){second=i/sr;break}
      return{on:+on.toFixed(5),second:+second.toFixed(4)};
    });
    S.ok('a note on row 0 sounds at sample zero',r.on>=0&&r.on<.002,r.on*1000+' ms in');
    S.ok('  and the next beat is exactly a beat later',Math.abs(r.second-.5)<.003,r.second+'s');

    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);
      const c=ensureCell(0,15);c.n=48;c.i=0;c.v=64;   /* ONLY the last row */
      const b=(await renderBounce(0,'pat')).buf,d=b.getChannelData(0),sr=b.sampleRate;
      let head=0;for(let i=0;i<Math.round(sr*.05);i++)head=Math.max(head,Math.abs(d[i]));
      let on=-1;for(let i=0;i<d.length;i++)if(Math.abs(d[i])>.001){on=i/sr;break}
      return{head:+head.toFixed(6),on:+on.toFixed(4)};
    });
    S.ck('nothing bleeds into the top from the loop’s tail',r.head,0);
    S.ok('  and the one note lands where it was written',Math.abs(r.on-1.875)<.005,r.on+'s (row 15)');

    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);
      const c=ensureCell(0,4);c.n=48;c.i=0;c.v=64;
      const b=(await renderBounce(0,'pat')).buf,d=b.getChannelData(0),sr=b.sampleRate;
      let on=-1;for(let i=0;i<d.length;i++)if(Math.abs(d[i])>.001){on=i/sr;break}
      return +on.toFixed(4);
    });
    S.ok('a region that starts with a rest keeps the rest',Math.abs(r-.5)<.005,
      r+'s — the head is never trimmed, or the music would shift early');

    S.head('the tail is kept, the silence after it is not');
    r=await page.evaluate(async()=>{
      const res=await renderBounce(1,'pat');
      const b=res.buf,d=b.getChannelData(0);
      let last=0;for(let i=d.length-1;i>0;i--)if(Math.abs(d[i])>1e-4){last=i;break}
      return{trailingSilence:+((d.length-last)/b.sampleRate).toFixed(3)};
    });
    S.ok('no long dead air on the end',r.trailingSilence<.2,r.trailingSilence+'s of silence after the last sound');

    S.head('it becomes a usable instrument');
    r=await page.evaluate(async()=>{
      const n0=song.instruments.length;
      cursor.t=1;
      HTMLAnchorElement.prototype.click=function(){}; /* swallow the download in headless */
      const ins=await bounce('pat');
      return{added:song.instruments.length-n0,type:ins.type,hasAudio:!!ins.sampleB64,
             decoded:!!ins.buffer,name:ins.name,file:ins.sampleName,
             selected:curInst===song.instruments.length-1,
             listed:document.querySelectorAll('#instList .ins').length>=song.instruments.length};
    });
    S.ck('one new sampler instrument',[r.added,r.type],[1,'sampler']);
    S.ok('  carrying its audio, decoded and ready to play',r.hasAudio&&r.decoded,r.name+' / '+r.file);
    S.ok('  selected, so you can hear it straight away',r.selected);

    S.head('the transport is left exactly as it was');
    r=await page.evaluate(async()=>{
      song.trackset[0].mute=true;song.trackset[1].solo=true;
      const mv=song.masterVol;
      await renderBounce(1,'pat');
      return{mute:song.trackset[0].mute,solo:song.trackset[1].solo,mv:song.masterVol===mv,
             rendering,bouncing,live:!!AC&&AC.constructor.name!=='OfflineAudioContext'};
    });
    S.ck('mute and solo are restored',[r.mute,r.solo],[true,true]);
    S.ok('  master volume too',r.mv);
    S.ok('  the flags are cleared and the live context is back',!r.rendering&&!r.bouncing&&r.live);

    S.head('bounce to a new channel leaves the source alone');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);pat.data[1].fill(null);
      for(const k of[0,4,8,12]){const c=ensureCell(1,k);c.n=48;c.i=1;c.v=50}
      const ts=song.trackset[1];
      ts.vol=.4;ts.pan=-.3;ts.rvb=.25;ts.duck=.5;ts.duckSrc=0;ts.eq.l=3;
      ts.fx=[{type:'autofilter',on:true,p:{}}];
      const before={tracks:TRACKS,vol:ts.vol,pan:ts.pan,rvb:ts.rvb,duck:ts.duck,eq:ts.eq.l,
                    fx:ts.fx.length,notes:[0,4,8,12].map(k=>cellAt(1,k).n),
                    anyDisabled:[0,4,8,12].some(k=>cellAt(1,k).d)};
      cursor.t=1;
      HTMLAnchorElement.prototype.click=function(){};
      const pos=await bounceToChannel('pat');
      const nts=song.trackset[pos];
      return{pos,before,
        after:{tracks:TRACKS,vol:ts.vol,pan:ts.pan,rvb:ts.rvb,duck:ts.duck,eq:ts.eq.l,
               fx:(ts.fx||[]).length,notes:[0,4,8,12].map(k=>cellAt(1,k).n),
               anyDisabled:[0,4,8,12].some(k=>cellAt(1,k).d)},
        chan:{vol:nts.vol,duck:nts.duck,dly:nts.dly,rvb:nts.rvb,name:nts.name,
              row0:cellAt(pos,0)&&{n:cellAt(pos,0).n,i:cellAt(pos,0).i},
              empty:song.patterns[0].data[pos].filter(c=>c&&c.n!=null).length}};
    });
    S.ck('a channel is added right beside the source',[r.after.tracks,r.pos],[r.before.tracks+1,2]);
    S.ck('the source track is untouched',
      [r.after.vol,r.after.pan,r.after.rvb,r.after.duck,r.after.eq,r.after.fx],
      [r.before.vol,r.before.pan,r.before.rvb,r.before.duck,r.before.eq,r.before.fx]);
    S.ck('  and so are its notes',[r.after.notes,r.after.anyDisabled],[r.before.notes,false]);
    S.ck('the new channel is neutral — the bounce already has all that baked in',
      [r.chan.vol,r.chan.duck,r.chan.dly,r.chan.rvb],[1,0,0,0]);
    S.ck('  with one hit at row 0 and nothing else',[r.chan.row0.n,r.chan.empty],[48,1]);
    S.ok('  and a name that says where it came from',/↓/.test(r.chan.name),r.chan.name);

    S.head('freeze swaps the track over — reversibly');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);pat.data[1].fill(null);
      for(const k of[0,4,8,12]){const c=ensureCell(1,k);c.n=48;c.i=1;c.v=50}
      const ts=song.trackset[1];
      ts.vol=.4;ts.pan=-.5;ts.rvb=.3;ts.duck=.6;ts.duckSrc=0;ts.eq.l=4;
      ts.fx=[{type:'autofilter',on:true,p:{}}];
      pat.auto={'1.p.cutoff':new Array(16).fill(.5),'0.p.cutoff':new Array(16).fill(.2)};
      const before={vol:ts.vol,pan:ts.pan,rvb:ts.rvb,duck:ts.duck,eq:ts.eq.l,fx:ts.fx.length,
                    lanes:Object.keys(pat.auto).sort(),notes:[0,4,8,12].map(k=>cellAt(1,k).n)};
      cursor.t=1;
      const n0=song.instruments.length;
      await freezeToggle();
      const f=pat.frozen&&pat.frozen[1];
      const after={
        added:song.instruments.length-n0,
        strip:{vol:ts.vol,pan:ts.pan,rvb:ts.rvb,duck:ts.duck,eq:ts.eq.l,fx:(ts.fx||[]).length},
        lanes:Object.keys(pat.auto).sort(),
        disabled:[4,8,12].every(k=>cellAt(1,k).d===1),
        row0:{n:cellAt(1,0).n,i:cellAt(1,0).i,d:cellAt(1,0).d},
        pointsAtBounce:f&&song.instruments[f.inst]&&song.instruments[f.inst].type==='sampler',
        marker:!!ghead.querySelector('.tcell[data-t="1"] .tbtn[data-frz].fON')
      };
      await freezeToggle();                       /* thaw */
      const back={vol:ts.vol,pan:ts.pan,rvb:ts.rvb,duck:ts.duck,eq:ts.eq.l,fx:(ts.fx||[]).length,
                  lanes:Object.keys(pat.auto).sort(),
                  notes:[0,4,8,12].map(k=>{const c=cellAt(1,k);return c?c.n:null}),
                  anyDisabled:[0,4,8,12].some(k=>{const c=cellAt(1,k);return c&&c.d}),
                  frozen:!!(pat.frozen&&pat.frozen[1]),
                  instrumentKept:song.instruments.length-n0,
                  marker:!!ghead.querySelector('.tcell[data-t="1"] .tbtn[data-frz].fON')};
      return{before,after,back};
    });
    S.ck('one bounce instrument is made',r.after.added,1);
    S.ok('  and row 0 plays it',r.after.row0.n===48&&r.after.pointsAtBounce,
      'row 0 → instrument '+r.after.row0.i);
    S.ok('  the original notes are disabled, not deleted',r.after.disabled);
    S.ck('the strip is flattened so nothing is processed twice',
      [r.after.strip.vol,r.after.strip.pan,r.after.strip.rvb,r.after.strip.duck,r.after.strip.eq,r.after.strip.fx],
      [1,0,0,0,0,0]);
    S.ck('  and this track’s automation lanes are set aside',r.after.lanes,['0.p.cutoff']);
    S.ok('  the header shows it is frozen',r.after.marker);

    S.ck('thawing restores every mixer value',
      [r.back.vol,r.back.pan,r.back.rvb,r.back.duck,r.back.eq,r.back.fx],
      [r.before.vol,r.before.pan,r.before.rvb,r.before.duck,r.before.eq,r.before.fx]);
    S.ck('  the automation lanes come back',r.back.lanes,r.before.lanes);
    S.ck('  the notes are playable again',[r.back.notes,r.back.anyDisabled],[r.before.notes,false]);
    S.ck('  nothing is left frozen, and the marker goes',[r.back.frozen,r.back.marker],[false,false]);
    S.ck('  the bounce stays in your instruments',r.back.instrumentKept,1);

    /* the point of flattening the strip: a frozen track has to sound like it did. If the fader,
       EQ or FX were left in place the bounce would be processed a second time on the way out. */
    S.head('frozen sounds like it did before');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.auto={};
      pat.data[1].fill(null);
      for(const k of[0,4,8,12]){const c=ensureCell(1,k);c.n=48;c.i=1;c.v=50}
      const ts=song.trackset[1];
      ts.vol=.45;ts.pan=0;ts.eq.l=5;ts.rvb=0;ts.duck=0;ts.fx=[];
      cursor.t=1;
      const env=b=>{ /* loudness of each beat */
        const sr=b.sampleRate,d=b.getChannelData(0),out=[];
        for(let k=0;k<4;k++){
          let s=0,n=0;
          for(let i=Math.round((k*.5+.005)*sr);i<Math.round((k*.5+.2)*sr)&&i<d.length;i++){s+=d[i]*d[i];n++}
          out.push(n?Math.sqrt(s/n):0);
        }
        return out;
      };
      const live=env((await renderBounce(1,'pat')).buf);
      await freezeToggle();
      const frozen=env((await renderBounce(1,'pat')).buf);
      await freezeToggle();
      return{live:live.map(x=>+x.toFixed(4)),frozen:frozen.map(x=>+x.toFixed(4))};
    });
    {
      const diff=r.live.map((x,i)=>x?Math.abs(r.frozen[i]-x)/x:0);
      S.ok('every beat comes out at the same level',Math.max(...diff)<.12,
        'live '+r.live.join(', ')+'  →  frozen '+r.frozen.join(', ')
        +'  (worst '+(Math.max(...diff)*100).toFixed(1)+'% off)');
    }

    S.head('the ❄ button does it too');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[1].fill(null);pat.auto={};
      for(const k of[0,4]){const c=ensureCell(1,k);c.n=48;c.i=1;c.v=50}
      cursor.t=0;                                   /* cursor deliberately on ANOTHER track */
      renderHead();
      const btn=()=>ghead.querySelector('.tcell[data-t="1"] .tbtn[data-frz]');
      const lit=()=>btn().classList.contains('fON');
      const before=lit();
      btn().click();
      await new Promise(x=>setTimeout(x,1200));
      const on={lit:lit(),frozen:!!(song.patterns[0].frozen&&song.patterns[0].frozen[1]),
                cursorMoved:cursor.t!==0};
      btn().click();
      await new Promise(x=>setTimeout(x,600));
      const off={lit:lit(),frozen:!!(song.patterns[0].frozen&&song.patterns[0].frozen[1])};
      return{before,on,off,inMixer:(toggleMixer(),!!mixch.querySelector('.mch[data-t="1"] .tbtn[data-frz]'))};
    });
    S.ok('it starts unlit',!r.before);
    S.ck('clicking it freezes THAT track, not the cursor’s',[r.on.lit,r.on.frozen],[true,true]);
    S.ok('  and the cursor is left where it was',!r.on.cursorMoved);
    S.ck('clicking again thaws it',[r.off.lit,r.off.frozen],[false,false]);
    S.ok('the mixer strip has the button too',r.inMixer);
    await page.evaluate(()=>{if(!mixer.hidden)toggleMixer()});

    S.head('freeze leaves notes you had disabled yourself alone');
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[1].fill(null);
      for(const k of[0,4,8]){const c=ensureCell(1,k);c.n=48;c.i=1;c.v=50}
      cellAt(1,8).d=1;                             /* you turned this one off before freezing */
      cursor.t=1;
      await freezeToggle();
      await freezeToggle();
      return{four:cellAt(1,4).d?1:0,eight:cellAt(1,8).d?1:0};
    });
    S.ck('one you disabled stays disabled after a thaw',[r.four,r.eight],[0,1]);

    S.head('the hotkey');
    r=await page.evaluate(()=>{
      /* Firefox owns Alt+F/E/V/S/T/H/B for its menu bar and won't reliably let the page cancel
         them, so none of VOLT's Alt bindings may use those letters. */
      const src=document.documentElement.innerHTML;
      const bad=[];
      src.split('\n').forEach(l=>{
        if(l.indexOf('//')===0||/^\s*\/\*/.test(l))return;
        const alt=/(^|[^!])e\.altKey/.test(l);           /* e.altKey, not !e.altKey */
        if(!alt)return;
        /* direct bindings on this line, and the panel-jump lookup table */
        [...l.matchAll(/e\.code==='Key([FEVSTHB])'/g)].forEach(m=>bad.push(m[1]));
        [...l.matchAll(/\bKey([FEVSTHB])\s*:/g)].forEach(m=>bad.push(m[1]));
      });
      /* the panel-jump table sits on the line AFTER its `if (e.altKey…)` — check it too */
      const tbl=src.match(/\{Key[A-Z]:'inst'[^}]*\}/);
      if(tbl)[...tbl[0].matchAll(/\bKey([FEVSTHB])\s*:/g)].forEach(m=>bad.push(m[1]));
      return{bad:[...new Set(bad)],open:typeof bounceDlg==='function'};
    });
    S.ck('no Alt binding collides with a Firefox menu',r.bad,[]);
    await page.evaluate(()=>{
      document.body.focus();
      window.__dlg=null;
      const real=askDialog;
      window.askDialog=(msg,btns)=>{window.__dlg={msg,labels:btns.map(b=>b.label)};return Promise.resolve('')};
    });
    await page.keyboard.down('Alt');await page.keyboard.press('KeyR');await page.keyboard.up('Alt');
    r=await page.evaluate(()=>window.__dlg);
    S.ok('Alt+R opens the bounce dialog',!!r&&/Bounce/.test(r.msg),r?r.msg.split('\n')[0]:'nothing happened');
    S.ok('  offering the scopes',!!r&&r.labels.some(l=>/pattern/i.test(l))&&r.labels.some(l=>/mix/i.test(l)),
      r?r.labels.join(' · '):'');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
