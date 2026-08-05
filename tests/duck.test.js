/* Sidechain ducking. Claims about level are worthless unless you measure the output, so this
   renders the song through VOLT's own offline path and compares the loudness of a held pad
   right after a trigger note against the loudness just before the next one. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('sidechain duck');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(INSTALL_RENDER);

    /* two tracks: a trigger playing on every beat, a pad holding one long note underneath.
       If ducking works the pad's envelope dips right after each trigger note. */
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      await setTracks(2);
      song.bpm=120;song.lpb=4;
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      song.order=[0];
      song.instruments=[
        {name:'trig',type:'kick',params:{pitch:50,punch:1,decay:.2,drive:0}},
        /* a plain sub: no detune, no beating, so its RMS is flat and any dip is the ducker */
        {name:'pad',type:'sub',params:{harm:0,rel:.4}}
      ];
      song.instruments.forEach(normInst);
      for(let r=0;r<16;r+=4){const c=ensureCell(0,r);c.n=48;c.i=0;c.v=64}
      const p=ensureCell(1,0);p.n=48;p.i=1;p.v=64;   /* one long held note */
      song.trackset.forEach(normTs);
      window.__pad=song.trackset[1];
      /* measure the pad alone: mute the trigger's OUTPUT but keep its notes.
         (ducking reads note data, so a muted trigger must still duck — that's the test.) */
      song.trackset[0].mute=true;
      audio();
      renderGrid();applyMix();
    });

    /* pull the pad's envelope out of a render, sampled just after each trigger and just before
       the next one */
    const ENV=async()=>page.evaluate(async()=>{
      window.__wavBlob=null;
      await renderWav();
      const dv=new DataView(await window.__wavBlob.arrayBuffer());
      let off=12,fmt={},data=null;
      while(off<dv.byteLength-8){
        const id=String.fromCharCode(dv.getUint8(off),dv.getUint8(off+1),dv.getUint8(off+2),dv.getUint8(off+3));
        const sz=dv.getUint32(off+4,true);
        if(id==='fmt '){fmt.ch=dv.getUint16(off+10,true);fmt.sr=dv.getUint32(off+12,true);fmt.bits=dv.getUint16(off+22,true)}
        if(id==='data'){data={off:off+8,sz};break}
        off+=8+sz+(sz&1);
      }
      const by=fmt.bits/8,frames=Math.floor(data.sz/(by*fmt.ch));
      const rms=(from,to)=>{
        let s=0,n=0;
        for(let i=Math.max(0,from);i<Math.min(frames,to);i++){
          const v=dv.getInt16(data.off+i*by*fmt.ch,true)/32768;s+=v*v;n++;
        }
        return n?Math.sqrt(s/n):0;
      };
      const beat=60/song.bpm,sr=fmt.sr;
      /* renderWav keeps 30 ms of pre-roll ahead of the song start, so song time b*beat lands
         30 ms later in the file. Without this the "after" window straddles the hit and the dip
         averages away. */
      const TRIM=.03;
      const out=[];
      for(let b=1;b<4;b++){                       /* skip beat 0: the pad's own attack is there */
        const t=b*beat+TRIM;
        out.push({after:rms((t+.005)*sr,(t+.045)*sr),  /* inside the dip */
                  before:rms((t+beat-.05)*sr,(t+beat-.01)*sr)}); /* recovered, before the next */
      }
      return out;
    });

    S.head('a muted trigger track still ducks (ghost kick)');
    let r=await page.evaluate(()=>{__pad.duck=.8;__pad.duckSrc=0;__pad.duckRel=.16;applyMix();return __pad.duckSrc});
    S.ck('pad set to duck from track 1',r,0);
    let env=await ENV();
    let ratios=env.map(e=>+(e.after/(e.before||1e-9)).toFixed(3));
    S.ok('the pad drops right after every trigger note',ratios.every(x=>x<.55),'after/before '+ratios.join(', '));
    S.ok('  and recovers before the next one',env.every(e=>e.before>.01),
      'recovered RMS '+env.map(e=>e.before.toFixed(3)).join(', '));

    S.head('depth 0 = no ducking at all');
    await page.evaluate(()=>{__pad.duck=0;applyMix()});
    env=await ENV();
    ratios=env.map(e=>+(e.after/(e.before||1e-9)).toFixed(3));
    S.ok('the pad holds steady',ratios.every(x=>x>.8),'after/before '+ratios.join(', '));

    S.head('the source picker actually picks');
    await page.evaluate(()=>{__pad.duck=.8;__pad.duckSrc=1;applyMix()}); /* track 2 = the pad itself */
    env=await ENV();
    ratios=env.map(e=>+(e.after/(e.before||1e-9)).toFixed(3));
    S.ok('pointed at a silent track, nothing ducks',ratios.every(x=>x>.8),'after/before '+ratios.join(', '));
    await page.evaluate(()=>{__pad.duckSrc=0;applyMix()});
    env=await ENV();
    ratios=env.map(e=>+(e.after/(e.before||1e-9)).toFixed(3));
    S.ok('pointed back at the trigger, it ducks again',ratios.every(x=>x<.55),'after/before '+ratios.join(', '));

    S.head('release length changes the recovery');
    /* measured where the NEXT trigger is about to land: a slow release hasn't finished
       recovering by then, a fast one has */
    const recovery=async rel=>{
      await page.evaluate(v=>{__pad.duckRel=v;applyMix()},rel);
      const e=await ENV();
      return +(e.reduce((a,x)=>a+x.before,0)/e.length).toFixed(4);
    };
    const fast=await recovery(.02),slow=await recovery(.9);
    S.ok('a slow release is still down when the next hit lands',slow<fast*.9,
      'recovered to '+fast+' (fast) vs '+slow+' (slow)');

    S.head('one control: pick a channel, done');
    r=await page.evaluate(()=>{
      const t=1;
      cursor.t=t;song.trackset[t].duck=0;song.trackset[t].duckSrc=-1;
      trackPanel();
      const before={sel:!!$id('scSel'),amount:!!document.querySelector('#trackMix input[data-tk="duck"]'),
                    rel:!!document.querySelector('#trackMix input[data-tk="duckRel"]')};
      /* pick track 1 from the dropdown, exactly as a click would */
      const sel=$id('scSel');sel.value='0';sel.dispatchEvent(new Event('change',{bubbles:true}));
      const ts=song.trackset[t];
      const after={amount:!!document.querySelector('#trackMix input[data-tk="duck"]'),
                   rel:!!document.querySelector('#trackMix input[data-tk="duckRel"]'),
                   src:ts.duckSrc,depth:ts.duck,
                   badge:!!ghead.querySelector('.tcell[data-t="'+t+'"] .tbtn[data-sc].cON')};
      /* and off again */
      const s2=$id('scSel');s2.value='';s2.dispatchEvent(new Event('change',{bubbles:true}));
      const off={depth:song.trackset[t].duck,
                 amount:!!document.querySelector('#trackMix input[data-tk="duck"]'),
                 badge:!!ghead.querySelector('.tcell[data-t="'+t+'"] .tbtn[data-sc].cON')};
      return{before,after,off,opts:[...$id('scSel').options].map(o=>o.textContent)};
    });
    S.ck('off by default: just the one dropdown',[r.before.sel,r.before.amount,r.before.rel],[true,false,false]);
    S.ok('the list offers off, the other tracks, and classic',
      r.opts[0]==='off'&&r.opts[r.opts.length-1]==='any kick (classic)',r.opts.join(' / '));
    S.ck('picking a channel turns it on with a usable depth',[r.after.src,r.after.depth],[0,.45]);
    S.ck('  and only then shows Amount + Release',[r.after.amount,r.after.rel],[true,true]);
    S.ok('  the header C button lights up',r.after.badge);
    S.ck('choosing off puts it away again',[r.off.depth,r.off.amount,r.off.badge],[0,false,false]);

    S.head('the C button: one click on, one click off');
    r=await page.evaluate(()=>{
      const t=1,ts=song.trackset[t];
      ts.duck=.7;ts.duckSrc=0;delete ts.duckSet;
      renderHead();
      const btn=()=>ghead.querySelector('.tcell[data-t="'+t+'"] .tbtn[data-sc]');
      const lit=()=>btn().classList.contains('cON');
      const wasOn=lit();
      btn().click();
      const offState={lit:lit(),duck:ts.duck,remembered:ts.duckSet};
      btn().click();
      const backOn={lit:lit(),duck:ts.duck};
      return{wasOn,offState,backOn,inMixer:(toggleMixer(),!!mixch.querySelector('.mch[data-t="1"] .tbtn[data-sc]'))};
    });
    S.ok('it starts lit when sidechain is on',r.wasOn);
    /* the class alone proved nothing last time: #ghead .tbtn carries an id, so a plainer
       .cON rule lost to it and the button never changed colour on screen. Read the pixels. */
    const paint=await page.evaluate(()=>{
      const t=1,ts=song.trackset[t];
      const btn=()=>ghead.querySelector('.tcell[data-t="'+t+'"] .tbtn[data-sc]');
      const col=()=>{const c=getComputedStyle(btn());return{fg:c.color,bg:c.backgroundColor,bd:c.borderTopColor,txt:btn().textContent}};
      ts.duck=.5;renderHead();const on=col();
      ts.duck=0;renderHead();const off=col();
      const mref=getComputedStyle(ghead.querySelector('.tcell[data-t="'+t+'"] .tbtn[data-mu]')).color;
      ts.duck=.5;renderHead();
      return{on,off,mref};
    });
    /* don't hardcode a hex — each theme has its own amber. Yellow is simply r > g > b. */
    const rgb=s=>(s.match(/\d+/g)||[]).map(Number);
    const on=rgb(paint.on.fg);
    S.ok('ON is yellow, not the default grey',
      paint.on.fg!==paint.off.fg&&on[0]>on[1]&&on[1]>on[2]&&on[0]>150,
      'on '+paint.on.fg+' vs off '+paint.off.fg);
    S.ok('  with a filled background',paint.on.bg!==paint.off.bg,paint.on.bg+' vs '+paint.off.bg);
    S.ck('OFF matches the other buttons exactly',paint.off.fg,paint.mref);
    S.ck('the button is labelled SC',paint.on.txt,'SC');
    S.ck('one click switches it off',[r.offState.lit,r.offState.duck],[false,0]);
    S.ok('  remembering the depth you had',r.offState.remembered===.7,'kept '+r.offState.remembered);
    S.ck('another click restores that exact depth',[r.backOn.lit,r.backOn.duck],[true,.7]);
    S.ok('the mixer strip has the same button',r.inMixer);
    await page.evaluate(()=>{if(!mixer.hidden)toggleMixer()});

    S.head('new channels arrive with it on');
    r=await page.evaluate(()=>{
      const ts=mkTs();
      return{duck:ts.duck,src:ts.duckSrc,rel:ts.duckRel};
    });
    S.ok('a fresh track ducks by default',r.duck>0,'depth '+r.duck);
    S.ck('  from any kick instrument',r.src,-1);

    S.head('old songs are left exactly as they were');
    r=await page.evaluate(()=>{
      const legacy={vol:.8,pan:0,dly:0,rvb:0,duck:0,mute:false,solo:false}; /* saved with it off */
      normTs(legacy);
      const noField={vol:.8};                                               /* saved before duck existed */
      normTs(noField);
      return{kept:legacy.duck,invented:noField.duck};
    });
    S.ck('a track saved with sidechain off stays off',r.kept,0);
    S.ok('normTs never invents a depth',r.invented===undefined||r.invented===0,'got '+r.invented);

    S.head('a track never ducks itself');
    r=await page.evaluate(()=>{
      const ts=song.trackset[0];ts.duck=.9;ts.duckSrc=0;normTs(ts);
      let hit=false;
      const real=chans[0].duckG.gain.setTargetAtTime.bind(chans[0].duckG.gain);
      chans[0].duckG.gain.setTargetAtTime=(...a)=>{hit=true;return real(...a)};
      duck(AC.currentTime+.5,0,true);
      chans[0].duckG.gain.setTargetAtTime=real;
      ts.duck=0;
      return hit;
    });
    S.ok('the trigger track is skipped',r===false);

    S.head('track edits move the trigger with it');
    r=await page.evaluate(async()=>{
      song.patterns.forEach(p=>p.data.forEach(col=>col.fill(null))); /* no notes → no confirm dialogs */
      song.trackset.forEach(normTs);
      song.trackset[1].duckSrc=0;song.trackset[1].duck=.5;
      await setTracks(3);
      insertTrackAt(0);                      /* everything shifts right by one */
      const afterIns=song.trackset[2].duckSrc;
      await deleteTrack(0);                  /* and back */
      const afterDel=song.trackset[1].duckSrc;
      await deleteTrack(0);                  /* remove the trigger itself */
      const afterKill=song.trackset[0].duckSrc;
      return{afterIns,afterDel,afterKill};
    });
    S.ck('inserting a track before the trigger repoints it',r.afterIns,1);
    S.ck('  deleting it puts it back',r.afterDel,0);
    S.ck('  deleting the trigger falls back to auto',r.afterKill,-1);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
