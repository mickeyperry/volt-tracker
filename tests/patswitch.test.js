/* How long a pattern switch takes to be HEARD, and whether the song's own turnover drifts.

   pump() books 160 ms of rows ahead — 1.6 SECONDS while the tab is hidden — and those rows carry
   whichever pattern was playing when they were booked. Before seqReflow() a manual switch took
   156-249 ms to reach the speakers, and 1605 ms in the second after coming back to a backgrounded
   tab. This is a REAL-TIME suite for that reason: an offline render cannot show it, because
   offline there is no clock to be late against. Everything here is measured off schedRow's actual
   booking times, not asserted. */
const{suite,open}=require('./lib');

const setup=()=>{
  audio();AC.resume();
  while(song.patterns.length<3)song.patterns.push(JSON.parse(JSON.stringify(song.patterns[0])));
  song.patterns[0].rows=32;song.patterns[1].rows=64;song.patterns[2].rows=16;  /* mixed lengths on purpose */
  /* track 0 plays EVERY row, so there is always something booked ahead to be un-booked */
  song.patterns.forEach(p=>{p.data.forEach((trk,t)=>{for(let r=0;r<p.rows;r++)trk[r]=(t===0||r%4===0)?{n:36,i:0,v:48}:null})});
  song.order=[0,1,2];
  window.__S=[];
  const _sr=schedRow;
  window.schedRow=function(pi,row,t){window.__S.push({pi,row,t,booked:AC.currentTime});_sr(pi,row,t)};
};

module.exports=async function(){
  const S=suite('pattern switch');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(setup);
    const rowDur=await page.evaluate(()=>60/song.bpm/song.lpb);

    S.head('a switch while a pattern loops');
    await page.evaluate(()=>play(false));
    const lats=[];
    for(let i=0;i<8;i++){
      await new Promise(r=>setTimeout(r,260+i*37));         /* land all over the booking window */
      await page.evaluate(()=>{window.__t0=AC.currentTime;curPatSet(curPat===0?1:0)});
      await new Promise(r=>setTimeout(r,320));
      lats.push(await page.evaluate(()=>{
        const f=window.__S.filter(s=>s.pi===curPat&&s.booked>=window.__t0-.002)[0];
        return f?f.t-window.__t0:null;
      }));
    }
    const worst=Math.max(...lats.map(x=>x==null?9:x));
    S.ok('every switch heard within one row + 40 ms',worst<=rowDur+.04,
      'worst '+(worst*1000).toFixed(0)+' ms  ·  one row = '+(rowDur*1000).toFixed(0)+' ms  ·  '+lats.map(x=>(x*1000).toFixed(0)).join('/'));
    S.ok('and none of them instant-and-wrong',lats.every(x=>x!=null&&x>0),'a switch must still land ON a row, not wherever the finger did');

    S.head('the re-book does not double anything');
    const dup=await page.evaluate(()=>{
      const seen={},d=[];
      for(const s of window.__S){const k=s.pi+':'+s.row+':'+s.t.toFixed(5);if(seen[k])d.push(k);seen[k]=1}
      return d;
    });
    S.ck('no row scheduled twice at the same instant',dup.length,0);

    S.head('stop stops');
    const st=await page.evaluate(()=>{
      const held=SEQ.fut.slice();
      stop();
      return{had:held.length,left:SEQ.fut.length,alive:held.filter(f=>f.v&&!f.v.dead).length};
    });
    S.ok('there were queued notes to catch',st.had>0,st.had+' booked ahead of the clock');
    S.ok('and stop un-booked every one of them',st.left===0&&st.alive===0,st.alive+' still live');

    S.head('a stall at the pattern change must not reach the ear');
    /* Drawing a pattern for the FIRST time is the most expensive frame of a session, and it
       lands exactly on the boundary. Without a deeper booking across it, the stall eats the
       160 ms cushion and rows get scheduled in the past — heard as a stumble on the first pass
       and never again, which is precisely how it was reported. */
    for(const block of [250,400]){
      const r=await page.evaluate(async(block)=>{
        if(!window.__cps)window.__cps=curPatSet;
        window.curPatSet=function(){const r=window.__cps.apply(this,arguments);
          const end=performance.now()+block;while(performance.now()<end);return r};
        let late=0;const real=schedRow;
        window.schedRow=function(pi,row,t){if(t-AC.currentTime<.005)late++;return real.apply(this,arguments)};
        window.__S=[];play(true);
        await new Promise(x=>setTimeout(x,7000));
        stop();window.schedRow=real;window.curPatSet=window.__cps;
        return late;
      },block);
      S.ck('no row scheduled late through a '+block+' ms stall',r,0);
    }

    S.head('the song’s own turnover — must not drift');
    await page.evaluate(()=>{window.__S=[];play(true)});
    await new Promise(r=>setTimeout(r,9000));
    const drift=await page.evaluate(()=>{
      const s=window.__S;stop();
      const rd=60/song.bpm/song.lpb;let w=0;
      for(let i=1;i<s.length;i++)w=Math.max(w,Math.abs((s[i].t-s[i-1].t)-rd));
      return{w,n:s.length};
    });
    S.ok('every row exactly one row after the last',drift.w<1e-6,
      drift.n+' rows across three patterns of 32/64/16 · worst error '+(drift.w*1e6).toFixed(1)+' ns');

    S.head('coming back from a hidden tab');
    const hid=await page.evaluate(async()=>{
      window.__S=[];
      Object.defineProperty(document,'hidden',{configurable:true,get:()=>true});
      play(false);
      await new Promise(r=>setTimeout(r,2200));               /* let it book the deep queue */
      const deep=SEQ.next-AC.currentTime;
      Object.defineProperty(document,'hidden',{configurable:true,get:()=>false});
      document.dispatchEvent(new Event('visibilitychange'));
      const t0=AC.currentTime;
      curPatSet(curPat===0?1:0);
      await new Promise(r=>setTimeout(r,600));
      const f=window.__S.filter(s=>s.pi===curPat&&s.booked>=t0-.002)[0];
      stop();
      return{deep,lat:f?f.t-t0:null};
    });
    S.ok('the deep queue really was deep',hid.deep>1,(hid.deep).toFixed(2)+' s booked while hidden');
    S.ok('and a switch straight after is still within a row',hid.lat!=null&&hid.lat<=rowDur+.04,
      (hid.lat*1000).toFixed(0)+' ms  (was 1605 ms)');

    S.ok('no console errors',errs.length===0,errs.slice(0,3).join(' | '));
  }finally{await browser.close()}
  return S;
};
