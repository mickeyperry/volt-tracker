/* Song sections: named ranges drawn over the pattern chips. Two things matter — the bars line up
   with the chips they cover (including when the strip wraps), and the ranges stay glued to the
   right music when the arrangement moves underneath them. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('song sections');
  const{browser,page,errs}=await open({w:1400,h:800});
  try{
    const setup=()=>page.evaluate(async()=>{
      await loadSong(blankSong());
      while(song.patterns.length<6)newPattern();
      song.order=[0,1,2,3,4,5];
      song.sections=[{a:0,b:1,name:'intro',hue:168},{a:2,b:3,name:'drop',hue:262}];
      curPatSet(0);ordPos=0;renderOrder();
    });

    S.head('a section owns a range of slots');
    await setup();
    let r=await page.evaluate(()=>({
      owners:[0,1,2,3,4,5].map(k=>{const s=secAt(k);return s?s.name:null}),
      bars:[...document.querySelectorAll('#secLayer .secBar')].map(b=>b.textContent),
      grips:document.querySelectorAll('#secLayer .secGrip').length
    }));
    S.ck('slots map to their section',r.owners,['intro','intro','drop','drop',null,null]);
    S.ok('one bar each, named, with its slot count and duration',
      /^intro2 · \d+:\d\d$/.test(r.bars[0])&&/^drop2 · \d+:\d\d$/.test(r.bars[1]),r.bars.join(' | '));
    S.ck('  and a grip on each end',r.grips,4);

    S.head('the bars line up with the chips');
    r=await page.evaluate(()=>{
      const chips=[...document.querySelectorAll('#ordChips .chip')].map(c=>c.getBoundingClientRect());
      const bars=[...document.querySelectorAll('#secLayer .secBar')].map(b=>b.getBoundingClientRect());
      return{
        leftOk:Math.abs(bars[0].left-chips[0].left)<2&&Math.abs(bars[1].left-chips[2].left)<2,
        rightOk:Math.abs(bars[0].right-chips[1].right)<2&&Math.abs(bars[1].right-chips[3].right)<2,
        above:bars[0].bottom<=chips[0].top+1,
        gap:+(chips[0].top-bars[0].bottom).toFixed(1)
      };
    });
    S.ok('left edges match the first chip',r.leftOk);
    S.ok('right edges match the last chip',r.rightOk);
    S.ok('  and the bar sits above the row',r.above,r.gap+'px gap');

    S.head('a section that wraps is drawn as one bar per row');
    r=await page.evaluate(async()=>{
      song.order=[];for(let i=0;i<40;i++)song.order.push(i%6);
      song.sections=[{a:0,b:39,name:'all',hue:168}];
      renderOrder();
      const chips=[...document.querySelectorAll('#ordChips .chip')];
      const rows=new Set(chips.map(c=>c.offsetTop)).size;
      const bars=[...document.querySelectorAll('#secLayer .secBar')];
      return{rows,bars:bars.length,firstLabelled:/all/.test(bars[0].textContent)};
    });
    S.ok('the strip really did wrap',r.rows>1,r.rows+' visual rows');
    S.ck('one bar per row it crosses',r.bars,r.rows);
    S.ok('  only the first carries the name',r.firstLabelled);

    S.head('dragging an end resizes it, in whole slots');
    await setup();
    r=await page.evaluate(()=>{
      const grip=[...document.querySelectorAll('#secLayer .secGrip')].find(g=>g.dataset.g==='R'&&g.dataset.s==='0');
      const chip=document.querySelectorAll('#ordChips .chip')[3];
      const cr=chip.getBoundingClientRect();
      grip.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}));
      dispatchEvent(new MouseEvent('mousemove',{clientX:cr.left+cr.width/2,clientY:cr.top+cr.height/2}));
      const stretched=[secs()[0].a,secs()[0].b];
      dispatchEvent(new MouseEvent('mouseup'));
      return{stretched,neighbour:[secs()[1].a,secs()[1].b]};
    });
    S.ck('it stops at the next section instead of overlapping',r.stretched,[0,1]);
    S.ck('  and the neighbour is untouched',r.neighbour,[2,3]);

    r=await page.evaluate(()=>{
      secs()[1].a=3;secs()[1].b=3;renderOrder();          /* free up slot 2 */
      const grip=[...document.querySelectorAll('#secLayer .secGrip')].find(g=>g.dataset.g==='R'&&g.dataset.s==='0');
      const chip=document.querySelectorAll('#ordChips .chip')[2];
      const cr=chip.getBoundingClientRect();
      grip.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}));
      dispatchEvent(new MouseEvent('mousemove',{clientX:cr.left+cr.width/2,clientY:cr.top+cr.height/2}));
      dispatchEvent(new MouseEvent('mouseup'));
      return[secs()[0].a,secs()[0].b];
    });
    S.ck('into free space it grows',r,[0,2]);

    S.head('walking them');
    await setup();
    r=await page.evaluate(()=>{
      const out=[];
      ordPos=0;curPatSet(song.order[0]);
      secGo(1);out.push(ordPos);
      secGo(1);out.push(ordPos);      /* last one — stays */
      secGo(-1);out.push(ordPos);
      ordPos=3;curPatSet(song.order[3]);
      secGo(-1);out.push(ordPos);     /* mid-section → back to its own start */
      return out;
    });
    S.ck('forward, clamp, back, and back-to-start',r,[2,2,0,2]);

    await setup();
    await page.evaluate(()=>document.body.focus());
    await page.keyboard.down('Control');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.up('Control');
    r=await page.evaluate(()=>({pos:ordPos,pat:curPat}));
    S.ck('the real Ctrl+→ jumps',[r.pos,r.pat],[2,2]);

    S.head('ranges follow the music when slots move');
    const drag=(from,to)=>page.evaluate(([from,to])=>{
      const absorb=secAt(to);
      const v=song.order.splice(from,1)[0];song.order.splice(to,0,v);
      secShift(i=>i===from?to:(from<to&&i>from&&i<=to)?i-1:(from>to&&i>=to&&i<from)?i+1:i,from,absorb);
      return secs().map(s=>[s.name,s.a,s.b]);
    },[from,to]);

    await setup();
    r=await drag(0,5);
    /* slot 0 leaves "intro" and travels past everything. The section must NOT stretch across the
       whole song to chase it — it closes up around what's left. */
    S.ck('a slot dragged out leaves its section behind',r,[['intro',0,0],['drop',1,2]]);

    /* the one that was wrong: pull a pattern out of the intro and drop it inside the drop.
       The intro must close up with no hole, and the drop must take the new pattern in — not
       leave it stranded between two sections with its own tail orphaned. */
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      while(song.patterns.length<8)newPattern();
      song.order=[0,1,2,3,4,5,6,7];
      song.sections=[{a:0,b:3,name:'intro',hue:168},{a:4,b:7,name:'drop',hue:262}];
      renderOrder();
    });
    r=await drag(1,5);
    S.ck('dragging a pattern into another section: no gap, both stay whole',r,[['intro',0,2],['drop',3,7]]);
    r=await page.evaluate(()=>[0,1,2,3,4,5,6,7].map(k=>{const s=secAt(k);return s?s.name:null}));
    S.ck('  every slot still belongs to one of them',r,
      ['intro','intro','intro','drop','drop','drop','drop','drop']);

    r=await drag(6,0); /* and back the other way, into the middle of the intro */
    S.ck('dragging one back re-balances them',r,[['intro',0,3],['drop',4,7]]);

    await setup();
    r=await page.evaluate(()=>{
      const k=4;song.order.splice(k,1);                   /* a slot outside every section */
      secShift(i=>i===k?-1:(i>k?i-1:i));
      return secs().map(s=>[s.name,s.a,s.b]);
    });
    S.ck('deleting a slot after them changes nothing',r,[['intro',0,1],['drop',2,3]]);

    await setup();
    r=await page.evaluate(()=>{
      const k=0;song.order.splice(k,1);
      secShift(i=>i===k?-1:(i>k?i-1:i));
      return secs().map(s=>[s.name,s.a,s.b]);
    });
    S.ck('deleting a slot inside one shrinks it and pulls the rest back',r,[['intro',0,0],['drop',1,2]]);

    S.head('old start-marker sections still load');
    r=await page.evaluate(async()=>{
      const j=JSON.parse(serialize(true));
      j.order=[0,1,2,3];
      j.sections=[{at:0,name:'old',hue:168},{at:2,name:'newer',hue:42}];
      await loadSong(j);
      return song.sections.map(s=>[s.name,s.a,s.b]);
    });
    S.ck('they come back as ranges',r,[['old',0,0],['newer',2,2]]);

    S.head('junk can not break the strip');
    r=await page.evaluate(async()=>{
      const j=JSON.parse(serialize(true));
      j.order=[0,1,2,3];
      j.sections=[{a:99,b:120,name:'ghost',hue:10},{a:1,b:0,name:'backwards',hue:5},
                  {a:0,b:2,name:'x'.repeat(90),hue:NaN},null,'nope'];
      await loadSong(j);
      return{n:song.sections.length,names:song.sections.map(s=>s.name.length),
             hue:song.sections[0]&&song.sections[0].hue,drew:!!document.getElementById('secLayer')};
    });
    S.ok('out-of-range, reversed and null are dropped',r.n<=1,r.n+' kept');
    S.ok('  names capped, hue repaired',(!r.names.length||r.names[0]<=24)&&(r.hue===undefined||Number.isFinite(r.hue)),'hue '+r.hue);
    S.ok('  and it still renders',r.drew);

    S.head('undo covers them');
    await setup();
    r=await page.evaluate(async()=>{
      pushUndoSoon();await new Promise(x=>setTimeout(x,300));
      const before=secs().length;
      secs().push({a:4,b:5,name:'added',hue:96});renderOrder();
      pushUndoSoon();await new Promise(x=>setTimeout(x,300));
      undo();
      return{before,after:secs().length,names:secs().map(s=>s.name).join(',')};
    });
    S.ck('Ctrl+Z takes one back out',[r.before,r.after],[2,2]);
    S.ck('  leaving the originals',r.names,'intro,drop');

    S.head('how long is the song');
    await setup();
    r=await page.evaluate(()=>{
      song.bpm=120;song.lpb=4;                       /* a 64-row pattern = 8 s */
      song.patterns.forEach(p=>p.rows=64);
      renderOrder();
      return{one:+patSecs(0).toFixed(3),total:+songSecs().toFixed(3),
             sec:+secSecs(secs()[0]).toFixed(3),
             fmt:[fmtTime(0),fmtTime(8),fmtTime(95),fmtTime(3*60+42)],
             shown:document.getElementById('songClock').textContent};
    });
    S.ck('a 64-row pattern at 120/4 is 8 seconds',r.one,8);
    S.ck('  six of them make 48',r.total,48);
    S.ck('  a two-slot section is 16',r.sec,16);
    S.ck('formatting',r.fmt,['0:00','0:08','1:35','3:42']);
    S.ok('the strip shows it',/0:48/.test(r.shown),r.shown);
    r=await page.evaluate(()=>{song.bpm=240;renderOrder();return document.getElementById('songClock').textContent});
    S.ok('and it follows the tempo',/0:24/.test(r),'at 240 BPM: '+r);

    S.head('selecting several slots');
    await setup();
    r=await page.evaluate(()=>{
      /* every click re-renders the strip, so the old nodes are detached — re-query each time */
      const chip=k=>document.querySelectorAll('#ordChips .chip')[k];
      chip(1).dispatchEvent(new MouseEvent('click',{bubbles:true}));
      const one=ordSelSorted().length;
      chip(4).dispatchEvent(new MouseEvent('click',{bubbles:true,shiftKey:true}));
      const range=ordSelSorted();
      chip(2).dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:true}));
      const toggled=ordSelSorted();
      const painted=[...document.querySelectorAll('#ordChips .chip')].map(c=>c.classList.contains('msel'));
      return{one,range,toggled,painted};
    });
    S.ck('a plain click selects nothing extra',r.one,0);
    S.ck('shift+click takes the range',r.range,[1,2,3,4]);
    S.ck('  ctrl+click drops one out',r.toggled,[1,3,4]);
    S.ck('  and they are marked',r.painted,[false,true,false,true,true,false]);

    S.head('the lasso');
    await setup();
    r=await page.evaluate(()=>{
      const chips=document.querySelectorAll('#ordChips .chip');
      const a=chips[1].getBoundingClientRect(),b=chips[3].getBoundingClientRect();
      const col=document.getElementById('ordCol');
      col.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,ctrlKey:true,button:0,
        clientX:a.left+2,clientY:a.top+2}));
      dispatchEvent(new MouseEvent('mousemove',{clientX:b.right-2,clientY:b.bottom-2}));
      const picked=ordSelSorted();
      const band=!!document.getElementById('ordLasso');
      dispatchEvent(new MouseEvent('mouseup'));
      return{picked,band,gone:!document.getElementById('ordLasso')};
    });
    S.ck('ctrl+drag grabs everything it crosses',r.picked,[1,2,3]);
    S.ok('  it draws a band while dragging',r.band);
    S.ok('  and cleans it up after',r.gone);

    S.head('acting on the selection');
    await setup();
    r=await page.evaluate(()=>{
      ordSelSet([4,5]);
      ordFocus();
      const before=song.order.length;
      document.getElementById('ordCol').dispatchEvent(new KeyboardEvent('keydown',{key:'Delete',bubbles:true}));
      return{before,after:song.order.length,order:song.order.slice(),secs:secs().map(s=>[s.name,s.a,s.b])};
    });
    S.ck('Delete removes them all',[r.before,r.after],[6,4]);
    S.ck('  leaving the rest',r.order,[0,1,2,3]);
    S.ck('  and the sections intact',r.secs,[['intro',0,1],['drop',2,3]]);

    r=await page.evaluate(()=>{
      ordSelSet([0,1]);ordFocus();
      const col=document.getElementById('ordCol');
      col.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyC',ctrlKey:true,bubbles:true}));
      const copied=ordClip.slice();
      col.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyV',ctrlKey:true,bubbles:true}));
      return{copied,order:song.order.slice(),sel:ordSelSorted(),secs:secs().map(s=>[s.name,s.a,s.b])};
    });
    S.ck('Ctrl+C then Ctrl+V inserts after the selection',r.order,[0,1,0,1,2,3]);
    S.ck('  the pasted slots end up selected',r.sel,[2,3]);
    S.ck('  and later sections move out of the way',r.secs,[['intro',0,1],['drop',4,5]]);

    r=await page.evaluate(()=>{
      ordSelSet([0]);ordFocus();
      document.getElementById('ordCol').dispatchEvent(new KeyboardEvent('keydown',{code:'KeyD',ctrlKey:true,bubbles:true}));
      return song.order.slice();
    });
    S.ck('Ctrl+D duplicates the block in place',r,[0,0,1,0,1,2,3]);

    r=await page.evaluate(()=>{
      song.order=[0];song.sections=[];renderOrder();
      ordSelSet([0]);ordFocus();
      document.getElementById('ordCol').dispatchEvent(new KeyboardEvent('keydown',{key:'Delete',bubbles:true}));
      return song.order.length;
    });
    S.ck('the last slot can not be deleted',r,1);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
