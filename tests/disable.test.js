/* Disabled notes. The claim is "still there, but silent" — so the test both inspects the data
   and renders the audio, because a flag that the scheduler quietly ignored in one code path
   (or that got garbage-collected on the next edit) would look fine in the grid and be wrong. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('disabled notes');
  const{browser,page,errs}=await open();
  try{
    const seed=()=>page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;song.order=[0];
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      song.instruments=[{name:'k',type:'kick',params:{pitch:50,punch:1,decay:.12,drive:0}}];
      song.instruments.forEach(normInst);
      pat.data.forEach(col=>col.fill(null));
      for(let i=0;i<4;i++){const c=ensureCell(0,i*4);c.n=48;c.i=0;c.v=64}
      sel=null;cursor.t=0;cursor.r=0;
      renderGrid();
    });

    S.head('toggling one cell');
    await seed();
    let r=await page.evaluate(()=>{
      cursor.r=4;toggleDisable();
      const c=cellAt(0,4);
      return{d:c.d,note:c.n,inst:c.i,vol:c.v,
             painted:grows.children[4].querySelector('.tcell[data-t="0"]').classList.contains('dis')};
    });
    S.ck('the flag is set and nothing else changed',[r.d,r.note,r.inst,r.vol],[1,48,0,64]);
    S.ok('  and the cell is drawn struck through',r.painted);

    r=await page.evaluate(()=>{toggleDisable();const c=cellAt(0,4);return{d:c.d,note:c.n}});
    S.ck('pressing again puts it back',[r.d,r.note],[undefined,48]);

    /* the previous version of this test drove toggleDisable() directly and passed while the KEY
       did nothing at all once a left-rail panel had focus. Press the real thing, from both. */
    S.head('the real Alt+N, wherever focus happens to be');
    await seed();
    await page.evaluate(()=>{cursor.r=4;document.body.focus()});
    await page.keyboard.down('Alt');await page.keyboard.press('KeyN');await page.keyboard.up('Alt');
    S.ck('from the grid',await page.evaluate(()=>cellAt(0,4).d),1);
    await page.evaluate(()=>{const b=document.querySelector('#side button');if(b)b.focus()});
    await page.keyboard.down('Alt');await page.keyboard.press('KeyN');await page.keyboard.up('Alt');
    S.ck('  and with a left-rail panel focused',await page.evaluate(()=>cellAt(0,4).d),undefined);
    await page.keyboard.down('Alt');await page.keyboard.press('KeyN');await page.keyboard.up('Alt');
    S.ck('  toggling once per press, not twice',await page.evaluate(()=>cellAt(0,4).d),1);
    await page.evaluate(()=>{document.body.focus();delete cellAt(0,4).d});

    S.head('a block, and the mixed case');
    await seed();
    r=await page.evaluate(()=>{
      cellAt(0,8).d=1;                                  /* one already off */
      sel={a:{t:0,c:0,r:0},b:{t:0,c:7,r:15}};
      toggleDisable();                                   /* mixed → everything off */
      const off=[0,4,8,12].map(k=>cellAt(0,k).d?1:0);
      toggleDisable();                                   /* all off → everything on */
      const on=[0,4,8,12].map(k=>cellAt(0,k).d?1:0);
      sel=null;
      return{off,on};
    });
    S.ck('a mixed selection turns them all off',r.off,[1,1,1,1]);
    S.ck('  and the next press turns them all on',r.on,[0,0,0,0]);

    S.head('it survives everything that touches a cell');
    await seed();
    r=await page.evaluate(()=>{
      cursor.r=4;toggleDisable();
      /* editing a neighbouring field must not sweep the cell away */
      const c=cellAt(0,4);c.n=null;c.i=null;c.v=null;pruneCell(0,4);
      const kept=!!cellAt(0,4);
      cellAt(0,4).n=48;cellAt(0,4).i=0;
      /* copy / paste */
      sel={a:{t:0,c:0,r:4},b:{t:0,c:7,r:4}};copySel(false);sel=null;
      cursor.t=0;cursor.r=1;pasteClip();
      const pasted=cellAt(0,1).d;
      /* and pasting a LIVE note over a disabled cell must clear it */
      cellAt(0,4).d=1;
      sel={a:{t:0,c:0,r:0},b:{t:0,c:7,r:0}};copySel(false);sel=null;
      cursor.r=4;pasteClip();
      const cleared=cellAt(0,4).d;
      return{kept,pasted,cleared};
    });
    S.ok('an emptied-but-disabled cell is not pruned away',r.kept);
    S.ck('copy/paste carries the flag',r.pasted,1);
    S.ck('  and pasting a live note clears it',r.cleared,undefined);

    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      const pat=song.patterns[0];pat.rows=16;
      pat.data.forEach(col=>col.fill(null));
      const c=ensureCell(0,4);c.n=48;c.i=0;c.v=64;c.d=1;
      await setLpb(8);                                   /* rescale the whole pattern */
      const moved=song.patterns[0].data[0].findIndex(x=>x&&x.n===48);
      return{row:moved,d:song.patterns[0].data[0][moved].d};
    });
    S.ck('rows/beat rescale keeps it',[r.row,r.d],[8,1]);

    r=await page.evaluate(async()=>{
      const j=JSON.parse(serialize(true));
      await loadSong(j);
      const c=song.patterns[0].data[0].find(x=>x&&x.n===48);
      return c&&c.d;
    });
    S.ck('saving and loading keeps it',r,1);

    S.head('undo');
    await seed();
    r=await page.evaluate(async()=>{
      pushUndoSoon();await new Promise(x=>setTimeout(x,300));
      cursor.r=4;toggleDisable();
      pushUndoSoon();await new Promise(x=>setTimeout(x,300));
      const off=cellAt(0,4).d?1:0;
      undo();
      return{off,after:cellAt(0,4).d?1:0};
    });
    S.ck('Ctrl+Z brings the note back on',[r.off,r.after],[1,0]);

    S.head('the piano roll agrees');
    await seed();
    r=await page.evaluate(()=>{
      if(prPan.hidden)prToggle();
      cellAt(0,4).d=1;
      const ns=prNotes();
      const dis=ns.filter(n=>n.dis).map(n=>n.r);
      const live=ns.filter(n=>!n.dis).map(n=>n.r);
      /* a disabled note must not cut the one before it: row 0's note still runs past row 4 */
      const first=ns.find(n=>n.r===0);
      if(!prPan.hidden)prToggle();
      return{dis,live,firstLen:first&&first.len};
    });
    S.ck('it shows as a disabled note',r.dis,[4]);
    S.ck('  the others are unaffected',r.live,[0,8,12]);
    S.ok('  and the note before it rings on through',r.firstLen>4,'length '+r.firstLen+' rows');

    S.head('and it is actually silent');
    await page.evaluate(INSTALL_RENDER);
    await seed();
    r=await page.evaluate(async()=>{
      const all=await window.__onsets();
      cellAt(0,4).d=1;cellAt(0,12).d=1;
      const some=await window.__onsets();
      cellAt(0,4).d=0;delete cellAt(0,4).d;
      delete cellAt(0,12).d;
      const back=await window.__onsets();
      return{all:all.slice(0,4),some:some.slice(0,4),back:back.slice(0,4)};
    });
    S.ck('four notes render four hits',r.all.length,4);
    S.ck('  disabling two leaves two',r.some.length,2);
    S.ok('  and they are the ones that were left on',
      Math.abs(r.some[0]-r.all[0])<.01&&Math.abs(r.some[1]-r.all[2])<.01,
      'kept '+r.some.join(', ')+' out of '+r.all.join(', '));
    S.ck('re-enabling brings them back',r.back.length,4);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
