/* The arp: generation, the live preview's undo safety, and real triplet timing. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('arp');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(()=>{
      window.T={
        seed(t,notes){
          const pat=song.patterns[curPat];
          pat.la=pat.lb=null;
          pat.data[t]=new Array(pat.rows).fill(null);
          notes.forEach(([r,n,v])=>{const c=ensureCell(t,r);c.n=n;c.i=0;if(v!=null)c.v=v});
          cursor.t=t;cursor.r=0;sel=null;PR.sel=[];
          if(prPan.hidden)prToggle();
          PR.sel=prNotes().map(x=>({t:x.t,r:x.r,n:x.n}));
          ARP.audi=false;
        },
        lane(t){return JSON.stringify(song.patterns[curPat].data[t])},
        notes(t){return prLaneNotes(t).map(x=>{const c=cellAt(t,x.r)||{};return[x.r,x.n,c.c||'-',c.x||0]})}
      };
    });

    S.head('arpeggiate');
    let r=await page.evaluate(()=>{
      T.seed(0,[[0,48],[2,52],[4,55]]);
      arpOpen();
      ARP.mode='arp';ARP.rateA=6;ARP.dir='up';ARP.oct=2;ARP.gate=80;ARP.steps=8;ARP.a7=false;ARP.a9=false;
      arpApply();
      const up=T.notes(0).map(x=>x[1]).join(',');
      ARP.dir='down';arpApply();const down=T.notes(0).map(x=>x[1]).join(',');
      ARP.dir='up';ARP.a7=true;arpApply();const seventh=T.notes(0).map(x=>x[1]).join(',');
      const majLabel=arpAddLabel();
      ARP.a7=false;arpApply();
      arpClose(false);
      /* same again on a bare root+5th — no third, so the 7th should come out flat */
      T.seed(0,[[0,45],[2,52]]); /* one lane holds one note per row — spread them out */
      arpOpen();ARP.mode='arp';ARP.rateA=6;ARP.dir='up';ARP.oct=1;ARP.steps=4;ARP.a7=true;arpApply();
      const flat=T.notes(0).map(x=>x[1]).join(',');
      const flatLabel=arpAddLabel();
      ARP.a7=false;arpClose(false);
      return{up,down,seventh,majLabel,flat,flatLabel};
    });
    S.ck('cycles the chord upwards over 2 octaves',r.up,'48,52,55,60,64,67,48,52');
    S.ck('and downwards',r.down,'67,64,60,55,52,48,67,64');
    S.ck('major triad gets a major 7th',r.seventh,'48,52,55,59,60,64,67,71');
    S.ck('  labelled',r.majLabel,'maj7');
    S.ck('root+5th with no third gets the flat 7',r.flat,'45,52,55,45');
    S.ck('  labelled',r.flatLabel,'b7');

    S.head('live preview is safe');
    r=await page.evaluate(async()=>{
      T.seed(0,[[0,48],[2,52],[4,55]]);   /* snapshot the UNTOUCHED lane, then open on top of it */
      const before=T.lane(0);
      arpOpen();ARP.mode='arp';ARP.rateA=6;ARP.steps=8;arpApply();
      const el=$id('arpGate');
      for(let i=0;i<120;i++){el.value=10+(i*7)%190;el.dispatchEvent(new Event('input',{bubbles:true}))}
      await new Promise(x=>setTimeout(x,200));
      const during=song.patterns[curPat].data[0].filter(Boolean).length;
      arpClose(false);
      return{restored:T.lane(0)===before,during,open:ARP.open,iv:ARP.iv};
    });
    S.ok('cancel restores the lane byte-for-byte',r.restored,'(after 120 live changes)');
    S.ck('panel closed, no timer left running',[r.open,r.iv],[false,null]);

    S.head('repeat');
    r=await page.evaluate(()=>{
      T.seed(0,[[0,48,20],[2,52,40],[4,55,60]]);
      arpOpen();
      ARP.mode='rep';ARP.rateR=9;ARP.gate=100;ARP.steps=0;ARP.xp=0;ARP.velT=false;
      arpApply();
      const rows=T.notes(0).map(x=>x[0]).join(',');
      const vels=prLaneNotes(0).map(x=>(cellAt(0,x.r)||{}).v).join(',');
      arpClose(false);
      return{rows,vels};
    });
    S.ck('phrase repeats on its own length',r.rows,'0,2,4,8,10,12,16,18,20,24,26,28,32,34,36,40,42,44,48,50,52,56,58,60');
    S.ok('each copy keeps its own velocities',/^20,40,60,20,40,60/.test(r.vels),r.vels.slice(0,20)+'…');

    S.head('rates the grid can actually hold');
    r=await page.evaluate(()=>{ARP.mode='arp';return{list:arpRateList().map(i=>ARP_RATES[i].lab).join(' '),lpb:song.lpb}});
    S.ck('nothing finer than one row is offered at '+r.lpb,r.list,'1/1 1/2 1/4 1/4T 1/8 1/8T 1/16');

    S.head('triplets are real (rendered)');
    await page.evaluate(INSTALL_RENDER);
    r=await page.evaluate(async()=>{
      song.trackset.forEach((ts,t)=>{ts.mute=(t!==0);ts.solo=false});song.order=[0];
      const pat=song.patterns[curPat];pat.la=pat.lb=null;
      const ii=song.instruments.findIndex(x=>x.type==='pluck'||x.type==='psybass');
      pat.data[0]=new Array(pat.rows).fill(null);
      [[0,48],[0,52],[0,55]].forEach(([rr,n],k)=>{const c=ensureCell(k?polyAddLane(0,false):0,rr);c.n=n;c.i=ii<0?1:ii;c.v=50});
      cursor.t=0;sel=null;PR.sel=[];if(prPan.hidden)prToggle();
      PR.sel=prNotes().map(x=>({t:x.t,r:x.r,n:x.n}));ARP.audi=false;
      arpOpen();ARP.mode='arp';ARP.rateA=5;ARP.dir='up';ARP.oct=1;ARP.gate=55;ARP.steps=9;arpApply();
      const step=arpStepRows()*rowDur();
      arpClose(true);
      const onsets=await window.__onsets();
      return{step,onsets};
    });
    const iv=r.onsets.slice(1,9).map((t,i)=>t-r.onsets[i]);
    const mean=iv.reduce((s,x)=>s+x,0)/(iv.length||1);
    S.ok('1/8 triplets play evenly at the right spacing',Math.abs(mean-r.step)<.008&&(Math.max(...iv)-Math.min(...iv))<.02,
      mean.toFixed(4)+'s vs '+r.step.toFixed(4)+'s wanted, spread '+((Math.max(...iv)-Math.min(...iv))*1000).toFixed(1)+'ms');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
