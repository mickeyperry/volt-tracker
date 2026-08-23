/* The motion, and the arithmetic underneath it.

   Two things here are easy to get wrong and impossible to see in a screenshot:
     1. setPlayRow no longer reads el.offsetTop — it works the scroll position out from cached
        row metrics. If that arithmetic drifts even a fraction per row the grid slowly rides up
        or down while you play, so it is checked against the real box positions, row by row.
     2. Every animation is a named Cool Mode part gated on a body class. An animation nobody can
        switch off is a bug on a slow machine, a projector, or a bad day. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('feel');
  const{browser,page,errs}=await open();
  try{
    /* CONTROL THE THING UNDER TEST. This suite asserts that motion happens, and motion is
       allowed to depend on the machine: Windows' "animate controls and elements inside windows"
       arrives here as prefers-reduced-motion, and on a machine with it off every check below
       failed while the app was behaving perfectly. Emulate the media feature rather than trust
       whatever the host is set to — and then assert the other side of it deliberately. */
    await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'no-preference'}]);
    S.head('the scroll arithmetic matches the layout it replaced');
    const rows=await page.evaluate(()=>{
      song.patterns[curPat].rows=64;renderGrid();calibrateRows();
      const out=[];
      for(const r of [0,1,7,16,33,63]){
        const el=grows.children[r];
        el.style.contentVisibility='visible';
        const real=el.offsetTop-gridwrap.clientHeight/2+40;   /* what it used to compute */
        el.style.contentVisibility='';
        out.push({r,real,calc:scrollForRow(r)});
      }
      return out;
    });
    /* offsetTop rounds to whole pixels and the cached arithmetic does not, so the ANCHOR is
       compared loosely and the SPACING — the part that would accumulate over 64 rows — exactly */
    const drift=Math.max(...rows.map(x=>Math.abs(x.real-x.calc)));
    S.ok('anchored where the layout says, within a pixel',drift<1.5,
      'worst '+drift.toFixed(3)+' px over rows '+rows.map(x=>x.r).join('/')+'  (offsetTop rounds, this does not)');
    const spacing=await page.evaluate(()=>{
      const r0=grows.children[0].getBoundingClientRect().top;
      let w=0;
      for(const r of [1,7,16,33,63]){
        const el=grows.children[r];el.style.contentVisibility='visible';
        const real=el.getBoundingClientRect().top-r0;el.style.contentVisibility='';
        w=Math.max(w,Math.abs(real-(scrollForRow(r)-scrollForRow(0))));
      }
      return w;
    });
    S.ok('row 63 is exactly 63 rows down — no creep',spacing<.05,
      'worst '+spacing.toFixed(4)+' px against the real box positions');
    const half=await page.evaluate(()=>scrollForRow(10.5)-scrollForRow(10));
    const full=await page.evaluate(()=>scrollForRow(11)-scrollForRow(10));
    S.ok('and half a row is half the distance — the glide needs that',Math.abs(half*2-full)<.01,
      half.toFixed(2)+' px vs a whole row at '+full.toFixed(2)+' px');

    S.head('no layout read left in the per-row path');
    const src=await page.evaluate(()=>setPlayRow.toString());
    S.ok('setPlayRow never touches offsetTop or clientHeight',
      !/offsetTop|clientHeight/.test(src),'cached metrics only');

    S.head('every animation has a switch');
    const parts=await page.evaluate(()=>COOL_PARTS.map(p=>p[0]));
    for(const k of ['glide','trail','hit','pulse'])
      S.ok('"'+k+'" is a named part',parts.indexOf(k)>=0,'');
    const gated=await page.evaluate(()=>{
      /* comments out first: one of them contains commas, and a comma is how a selector list
         is split — which is how this check once reported an English sentence as a CSS rule. */
      const css=[...document.querySelectorAll('style')].map(s=>s.textContent).join(String.fromCharCode(10)).replace(/[/][*][^]*?[*][/]/g,'');
      /* An animation is switchable either because its own selector names a cool- class, or
         because a `body:not(.cool-x) ...{animation:none}` rule can turn it off. */
      const kill=new Set(),bad=[],RULE=/([^{}]+)\{([^{}]*)\}/g;
      css.replace(RULE,(m,sel,body)=>{
        if(/animation:\s*none/.test(body))sel.split(',').forEach(x=>{
          const t=x.trim();
          if(/:not\(\.cool-/.test(t))kill.add(t.replace(/^body:not\(\.cool-[a-z]+\)\s*/,''));
        });
        return m;
      });
      css.replace(RULE,(m,sel,body)=>{
        if(!/animation:/.test(body)||/animation:\s*none/.test(body)||/@/.test(sel))return m;
        sel.split(',').forEach(x=>{const t=x.trim();if(!/cool-/.test(t)&&!kill.has(t))bad.push(t.slice(0,60))});
        return m;
      });
      return bad;
    });
    S.ok('nothing animates outside Cool Mode',gated.length===0,gated.join(' | '));

    S.head('turning a part off really removes it');
    const off=await page.evaluate(()=>{
      ['glide','trail','hit','pulse'].forEach(k=>COOL[k]=false);coolApply();
      const b=document.body.className,g=glideOn();
      ['glide','trail','hit','pulse'].forEach(k=>COOL[k]=true);coolApply();
      return{off:b,glide:g,on:document.body.className};
    });
    S.ok('the body classes go',!/cool-(glide|trail|hit|pulse)/.test(off.off),off.off);
    S.ok('and glideOn() agrees',off.glide===false,'');
    S.ok('they come back',/cool-glide/.test(off.on)&&/cool-trail/.test(off.on)&&/cool-hit/.test(off.on)&&/cool-pulse/.test(off.on),'');

    S.head('the cell flash lands on the cell you typed into');
    const fl=await page.evaluate(()=>{
      cursor.t=2;cursor.r=5;cursor.c=0;editStep=1;
      setNote(48,0);                                  /* writes row 5, cursor steps to row 6 */
      return{flashed:grows.children[5].children[3].classList.contains('flash'),
             note:!!(song.patterns[curPat].data[2][5]||{}).n,
             cursorMoved:cursor.r===6,
             elsewhere:[...grows.querySelectorAll('.flash')].length};
    });
    S.ck('the note was written',fl.note,true);
    S.ck('the cursor still steps on',fl.cursorMoved,true);
    S.ck('that cell is flashing',fl.flashed,true);
    S.ck('and nothing else is',fl.elsewhere,1);
    const flOff=await page.evaluate(()=>{
      COOL.hit=false;coolApply();
      cursor.t=3;cursor.r=9;cursor.c=0;setNote(50,0);
      const on=grows.children[9].children[4].classList.contains('flash');
      COOL.hit=true;coolApply();
      return on;
    });
    S.ck('with the switch off, no class at all',flOff,false);

    S.head('the glide really glides');
    const gl=await page.evaluate(async()=>{
      song.patterns[curPat].rows=64;renderGrid();calibrateRows();
      audio();AC.resume();setFollow(true);
      const seen=new Set(),rows=new Set();
      /* from the middle of the pattern: near row 0 the wanted scroll position is negative and
         the browser clamps it to 0, so nothing would move however smooth the maths is */
      play(false,30);
      const t0=performance.now();
      await new Promise(res=>{
        const tick=()=>{
          seen.add(Math.round(gridwrap.scrollTop*10));
          if(playRowEl)rows.add(playRowEl.dataset.r);
          performance.now()-t0<1400?requestAnimationFrame(tick):res();
        };requestAnimationFrame(tick);
      });
      stop();
      return{positions:seen.size,rows:rows.size};
    });
    S.ok('many scroll positions per row, not one jump each',gl.positions>gl.rows*3,
      gl.positions+' distinct scroll positions across '+gl.rows+' rows');

    S.head('the playhead flash');
    const ph=await page.evaluate(()=>{
      const g=document.getElementById('phGlow');
      COOL.trail=true;coolApply();
      setPlayRow(8);
      const a=g.style.transform,n=g.getAnimations().length;
      setPlayRow(20);
      const b=g.style.transform;
      COOL.trail=false;coolApply();
      g.getAnimations().forEach(x=>x.cancel());
      setPlayRow(30);
      const off=g.getAnimations().length,c=g.style.transform;
      COOL.trail=true;coolApply();
      return{a,b,c,n,off,inRows:!!g.closest('.prow')};
    });
    S.ok('it is not inside a row — that is what keeps it free',!ph.inRows,'lives in #gridwrap');
    S.ok('it fades when the playhead lands',ph.n>0,ph.n+' animation running');
    S.ok('and moves with the row',ph.a!==ph.b,ph.a+' → '+ph.b);
    S.ok('with the switch off it does nothing at all',ph.off===0&&ph.c===ph.b,'no animation, not moved');

    S.head('the system asking for less motion is obeyed — and can be overridden');
    await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'reduce'}]);
    const red=await page.evaluate(()=>{
      coolApply();
      const off={reduced:coolReduced(),glide:glideOn(),cls:document.body.className};
      /* the explicit, informed exception a person can make about their own DAW */
      COOL.force=true;coolApply();
      const on={glide:glideOn(),cls:document.body.className};
      COOL.force=false;coolApply();
      return{off,on};
    });
    S.ok('with reduce set, nothing animates however it is ticked',
      red.off.reduced&&red.off.glide===false&&!/cool-glide/.test(red.off.cls),red.off.cls||'(no classes)');
    S.ok('and "play them anyway" brings it back',
      red.on.glide===true&&/cool-glide/.test(red.on.cls),red.on.cls);
    await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'no-preference'}]);
    await page.evaluate(()=>coolApply());

    S.ok('no console errors',errs.length===0,errs.slice(0,3).join(' | '));
  }finally{await browser.close()}
  return S;
};
