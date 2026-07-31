/* Performance guard rails. Thresholds are deliberately loose — this is here to catch a 10x
   regression (like the whole grid repainting every playhead row), not to police milliseconds on
   whatever machine happens to run it. Numbers in the comments are what a 2026 desktop measured. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('performance');
  const{browser,page,errs}=await open({w:1600,h:900});
  try{
    await page.evaluate(async()=>{
      await setTracks(32);
      const el=document.getElementById('patLen');el.value=128;el.dispatchEvent(new Event('change'));
      for(let t=0;t<32;t++)for(let r=0;r<128;r+=2){
        const c=ensureCell(t,r);c.n=36+((t*7+r)%40);c.i=t%song.instruments.length;c.v=40;
      }
      renderGrid();
    });
    const nodes=await page.evaluate(()=>document.querySelectorAll('#grows *').length);
    S.note('stress pattern: 32 tracks x 128 rows, '+nodes+' DOM nodes');

    S.head('off-screen rows are skipped, and the placeholder is pixel-exact');
    const geo=await page.evaluate(()=>{
      const first=grows.children[0].getBoundingClientRect().height;
      const gaps=[];
      for(const i of [1,2,64,100,127])gaps.push(grows.children[i].offsetTop-grows.children[i-1].offsetTop);
      return{rowH:+first.toFixed(2),gaps,
        rowh:getComputedStyle(grows).getPropertyValue('--rowh').trim(),
        roww:getComputedStyle(grows).getPropertyValue('--roww').trim(),
        widthsMatch:Math.round(grows.children[0].getBoundingClientRect().width)===Math.round(grows.children[100].getBoundingClientRect().width),
        scrollable:grows.scrollWidth>gridwrap.clientWidth};
    });
    S.ok('rendered and skipped rows are the same height',new Set(geo.gaps).size===1,'gaps '+geo.gaps.join(','));
    S.ok('the placeholder matches a real row',parseFloat(geo.rowh)>2&&Math.abs(parseFloat(geo.rowh)-geo.rowH)<.5,geo.rowh+' vs measured '+geo.rowH+'px');
    S.ok('rows keep their full width (horizontal scrolling works)',geo.widthsMatch&&geo.scrollable,geo.roww);

    S.head('render costs');
    const t=await page.evaluate(()=>{
      const time=(fn,n)=>{const t0=performance.now();for(let i=0;i<n;i++)fn();return(performance.now()-t0)/n};
      const out={};
      out.grid=+time(()=>renderGrid(),3).toFixed(1);
      out.cursor=+time(()=>moveCursor(0,1,0),40).toFixed(2);
      sel={a:{t:0,c:0,r:0},b:{t:7,c:7,r:song.patterns[curPat].rows-1}};
      out.rows=+time(()=>renderRows(0,song.patterns[curPat].rows-1),3).toFixed(1);
      sel=null;
      return out;
    });
    S.ok('renderGrid() under 120 ms',t.grid<120,t.grid+' ms   (was 262 before row skipping)');
    S.ok('a cursor move under 8 ms',t.cursor<8,t.cursor+' ms');
    S.ok('re-rendering every row under 100 ms',t.rows<100,t.rows+' ms   (was 237)');

    S.head('frame rate during playback');
    const fps=await page.evaluate(async()=>{
      const frames=[];let last=performance.now(),id=0;
      const tick=()=>{const n=performance.now();frames.push(n-last);last=n;id=requestAnimationFrame(tick)};
      id=requestAnimationFrame(tick);
      play(false,0);
      await new Promise(r=>setTimeout(r,3200));
      stop();cancelAnimationFrame(id);
      frames.sort((a,b)=>a-b);
      return{median:+(1000/frames[Math.floor(frames.length/2)]).toFixed(1),worst:+frames[frames.length-1].toFixed(0)};
    });
    S.ok('at least 25 fps on the worst-case song',fps.median>=25,fps.median+' fps   (was 8.7)');
    S.ok('no frame gap over 150 ms',fps.worst<150,fps.worst+' ms worst gap');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
