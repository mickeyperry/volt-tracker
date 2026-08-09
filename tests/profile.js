#!/usr/bin/env node
/* Not a test — a profiler. `node tests/profile.js` prints where the time actually goes on a
   realistic song, so a responsiveness change can be argued from numbers instead of a hunch.
   Kept in tests/ because it needs the same headless-Chrome boot as the suites. */
const{open}=require('./lib');

const ms=v=>(v==null?'--':(+v).toFixed(1).padStart(7)+' ms');

module.exports=async function(){
  const{browser,page,errs}=await open({w:1600,h:900});
  try{
    /* A song shaped like one of Mickey's: wide, long, and with enough instruments and patterns
       that undo snapshots are real work rather than a toy. */
    const build=async(tracks,rows,pats)=>page.evaluate(async(tracks,rows,pats)=>{
      await loadSong(blankSong());
      await setTracks(tracks);
      const el=document.getElementById('patLen');el.value=rows;el.dispatchEvent(new Event('change'));
      while(song.patterns.length<pats)newPattern();
      for(let p=0;p<pats;p++){
        song.patterns[p].rows=rows;
        for(let t=0;t<tracks;t++)for(let r=0;r<rows;r+=2){
          const c=ensureCellIn(p,t,r);c.n=36+((t*7+r)%40);c.i=t%song.instruments.length;c.v=40;
        }
      }
      song.order=song.patterns.map((_,i)=>i);
      curPatSet(0);fillPendingRows(true);
      return{cells:tracks*rows/2*pats,nodes:document.querySelectorAll('#grows *').length};
    },tracks,rows,pats);

    const timeIt=async(label,body,n)=>{
      const v=await page.evaluate((body,n)=>{
        const fn=new Function(body);
        for(let i=0;i<2;i++)fn();                       /* warm */
        const t=[];
        for(let i=0;i<n;i++){const t0=performance.now();fn();t.push(performance.now()-t0)}
        t.sort((a,b)=>a-b);
        return t[Math.floor(n/2)];                      /* median — one GC pause shouldn't set the number */
      },body,n);
      console.log('   '+label.padEnd(46)+ms(v));
      return v;
    };

    console.log('\nVOLT profile · beta.html\n');

    for(const[tracks,rows,pats] of [[8,64,4],[16,128,8],[32,128,8]]){
      const info=await build(tracks,rows,pats);
      console.log(`\n[${tracks} tracks x ${rows} rows x ${pats} patterns]  ${info.cells} cells, ${info.nodes} DOM nodes`);

      console.log('  -- pattern switch');
      await timeIt('curPatSet (the whole switch)','curPatSet(curPat?0:1)',9);
      await timeIt('  patchGrid alone','curPat=curPat?0:1;patchGrid()',9);
      await timeIt('  the all-rows patch loop only',
        'curPat=curPat?0:1;const p=song.patterns[curPat];for(let r=0;r<p.rows;r++)if(!patchRow(r))renderRow(r)',9);
      await timeIt('  renderHead+PatSel+Order only','renderHead();renderPatSel();renderOrder()',9);
      const vis=await page.evaluate(()=>{
        const rh=rowHpx||19.5,vh=gridwrap.clientHeight||700;
        return{visible:Math.ceil(vh/rh)+25,total:song.patterns[curPat].rows};
      });
      console.log('        (only ~'+vis.visible+' of '+vis.total+' rows are on screen)');

      console.log('  -- edit cost (what every keystroke pays)');
      await timeIt('snapState (undo snapshot)','snapState()',9);
      await timeIt('serialize(false) (autosave JSON)','serialize(false)',9);
      await timeIt('serialize(true) (with samples)','serialize(true)',5);
      const sz=await page.evaluate(()=>({snap:snapState().length,ser:serialize(false).length,undo:undoStack.length}));
      console.log('        snapshot '+(sz.snap/1e6).toFixed(2)+' MB · autosave JSON '+(sz.ser/1e6).toFixed(2)+' MB · undo depth '+sz.undo);

      const ls=await page.evaluate(()=>{
        const s=serialize(false);
        const t0=performance.now();
        try{localStorage.setItem('voltbeta.__probe',s)}catch(e){return -1}
        const w=performance.now()-t0;
        localStorage.removeItem('voltbeta.__probe');
        return w;
      });
      console.log('   '+'localStorage write (synchronous!)'.padEnd(46)+ms(ls<0?null:ls));

      console.log('  -- grid');
      await timeIt('renderGrid (full rebuild)','renderGrid()',5);
      await timeIt('renderRow x1','renderRow(3)',40);
      await timeIt('moveCursor','moveCursor(0,1,0)',40);
    }

    console.log('\n  -- playback frame gaps, with autosave firing');
    const fps=await page.evaluate(async()=>{
      const frames=[];let last=performance.now(),id=0;
      const tick=()=>{const n=performance.now();frames.push(n-last);last=n;id=requestAnimationFrame(tick)};
      id=requestAnimationFrame(tick);
      play(false,0);
      /* poke an edit every 400 ms so saveSoon/pushUndoSoon fire mid-playback, like real editing */
      const poke=setInterval(()=>{const c=ensureCell(0,(Math.random()*8|0));c.v=32+(Math.random()*30|0);saveSoon()},400);
      await new Promise(r=>setTimeout(r,4000));
      clearInterval(poke);stop();cancelAnimationFrame(id);
      frames.sort((a,b)=>a-b);
      const over=n=>frames.filter(f=>f>n).length;
      return{median:+(1000/frames[Math.floor(frames.length/2)]).toFixed(1),
             worst:+frames[frames.length-1].toFixed(0),
             over50:over(50),over100:over(100)};
    });
    console.log('   median '+fps.median+' fps · worst gap '+fps.worst+' ms · gaps >50ms: '+fps.over50+' · >100ms: '+fps.over100);

    if(errs.length)console.log('\n   CONSOLE ERRORS: '+errs.join(' | '));
    console.log('');
  }finally{await browser.close()}
};

if(require.main===module)module.exports().catch(e=>{console.error(e);process.exit(1)});
