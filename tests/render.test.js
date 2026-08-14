/* What a WAV render leaves behind.
 *
 * `renderWav()` and the stem bounce both do the same violent thing: swap the global `AC` for an
 * OfflineAudioContext, rebuild the entire bus graph on it, render, and put everything back. The
 * putting-back is a hand-written list of globals, and a hand-written list is a list somebody will
 * forget to add to.
 *
 * They HAD forgotten three: `metOut`, `loudAn` and `loudHP`. `metOut` is the one that bit — after a
 * render it still pointed at a delay node inside the finished offline context, so the next
 * `metClick()` threw
 *
 *     InvalidAccessError: cannot connect to an AudioNode belonging to a different audio context
 *
 * from inside `schedRow` → `pump` → `play`. In other words: **with the metronome on, rendering a
 * WAV killed the transport until you reloaded the page.** Measured, not theorised — the exception
 * came out of `play()` itself, so the song never started.
 *
 * The check below mirrors buildGraph()'s list of globals. That is a second hand-written list and
 * it does not fix the underlying shape, but it does mean the *next* one to go missing shows up
 * here by name instead of as a silent dead meter or a transport that will not start.
 */
const{suite,open}=require('./lib');

/* Every node buildGraph() hangs off a module-level global. `eval` is out (the page's CSP forbids
   it), so the bag is written by hand in the page — one entry per global, MCH reduced to a node. */
const BAG=`({master,masterComp,MCH:(MCH&&MCH.fxIn),metOut,masterAn,masterAnL,masterAnR,
             loudAn,loudHP,dlyIn,dlyL,dlyR,rvbIn,spkOut})`;

module.exports=async function(){
  const S=suite('render: the graph survives it');
  const{browser,page,errs}=await open();
  try{
    /* A tiny song — this suite is about the swap, not the audio, so four rows keep the render
       under a second. */
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      song.patterns[curPat].rows=4;
      audio();
    });
    /* Returns the names of any graph global that is missing or belongs to a dead context. */
    const stale=()=>page.evaluate(`(()=>{const b=${BAG};
      return Object.keys(b).filter(k=>!b[k]||b[k].context!==AC)})()`);

    S.head('every global buildGraph() sets belongs to the live context');
    let r=await stale();
    S.ok('  they all start out live',r.length===0,r.join(' ')||'all live');

    S.head('...and still does after a render');
    await page.evaluate(()=>renderWav());
    r=await stale();
    S.ok('  a render puts every one of them back',r.length===0,r.join(' ')||'all live');

    S.head('the metronome still works afterwards — the bug this suite exists for');
    r=await page.evaluate(()=>{
      MET.on=true;
      try{metClick(AC.currentTime+.05,true);return null}catch(e){return e.name}
    });
    S.ck('  metClick does not throw across contexts',r,null);

    r=await page.evaluate(async()=>{
      MET.on=true;
      play(false);
      await new Promise(res=>setTimeout(res,300));
      const out={playing:SEQ.playing,advanced:SEQ.row>0||SEQ.q.length>0};
      stop();MET.on=false;
      return out;
    });
    S.ok('  and Play still starts the transport',r.playing===true);
    S.ok('  and rows are still being scheduled',r.advanced===true);

    S.head('a second render in the same session is fine too');
    await page.evaluate(()=>renderWav());
    r=await stale();
    S.ok('  nothing went stale the second time',r.length===0,r.join(' ')||'all live');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
