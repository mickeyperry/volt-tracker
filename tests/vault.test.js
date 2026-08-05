/* Vault ideas across different rows/beat settings. A phrase is a length of MUSIC, not a number
   of rows — stash a bar at 4 rows/beat, paste it into an 8 rows/beat song, and it has to still
   be a bar. Proved by where the notes land, and by rendering the result. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('vault / rows-per-beat');
  const{browser,page,errs}=await open();
  try{
    S.head('an idea remembers the grid it was written on');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.lpb=4;$id('lpbSel').value=4;
      const pat=song.patterns[0];pat.rows=16;
      for(let i=0;i<4;i++){const c=ensureCell(0,i*4);c.n=48+i;c.i=0;c.v=50}
      sel={a:{t:0,c:0,r:0},b:{t:0,c:7,r:15}};
      const idea=captureIdea();
      return{lpb:idea.lpb,rows:idea.rows,notes:idea.data.map((row,i)=>row[0]&&row[0].n!=null?i:-1).filter(i=>i>=0)};
    });
    S.ck('the stash records rows/beat',r.lpb,4);
    S.ck('  and the phrase itself',[r.rows,r.notes],[16,[0,4,8,12]]);

    S.head('pasting into a finer grid stretches it');
    r=await page.evaluate(async()=>{
      const idea=captureIdea();
      await setLpb(8);                       /* same music, twice the rows */
      await ideaToClip(idea);
      return{rows:clip.data.length,
             notes:clip.data.map((row,i)=>row[0]&&row[0].n!=null?i:-1).filter(i=>i>=0),
             lpb:song.lpb};
    });
    S.ck('the song is now 8 rows/beat',r.lpb,8);
    S.ck('the phrase doubles in rows',r.rows,32);
    S.ck('  and the notes stay one beat apart',r.notes,[0,8,16,24]);

    S.head('and into a coarser grid it compresses');
    r=await page.evaluate(async()=>{
      const idea={kind:'volt-idea',v:1,lpb:8,rows:16,tracks:1,insts:{},
        data:Array.from({length:16},(_,i)=>[i%4===0?{n:48,i:0,v:50}:{}])};
      await setLpb(4);
      await ideaToClip(idea);
      return{rows:clip.data.length,notes:clip.data.map((row,i)=>row[0]&&row[0].n!=null?i:-1).filter(i=>i>=0)};
    });
    S.ck('half the rows',r.rows,8);
    S.ck('  notes still every half beat',r.notes,[0,2,4,6]);

    S.head('sub-row timing survives the stretch');
    r=await page.evaluate(async()=>{
      /* a note sitting a third of a row late at lpb 4 must still be a third of a BEAT late */
      const idea={kind:'volt-idea',v:1,lpb:4,rows:4,tracks:1,insts:{},
        data:[[{n:48,i:0,v:50}],[{n:50,i:0,v:50,c:'T',x:96}],[{}],[{}]]};
      await setLpb(8);
      await ideaToClip(idea);
      const at=[];clip.data.forEach((row,i)=>{const c=row[0];if(c&&c.n!=null)at.push([i,c.c||null,c.x||0])});
      return at;
    });
    S.ck('a half-row-late note lands a whole row later, dead on',r,[[0,null,0],[3,null,0]]);

    S.head('an idea with no lpb (saved before this) is treated as 4');
    r=await page.evaluate(async()=>{
      const idea={kind:'volt-idea',v:1,rows:8,tracks:1,insts:{},
        data:Array.from({length:8},(_,i)=>[i%4===0?{n:48,i:0,v:50}:{}])};
      await setLpb(8);
      await ideaToClip(idea);
      return clip.data.length;
    });
    S.ck('stretched as if it were 4',r,16);

    S.head('same grid = untouched');
    r=await page.evaluate(async()=>{
      await setLpb(4);
      const idea={kind:'volt-idea',v:1,lpb:4,rows:4,tracks:1,insts:{},
        data:[[{n:48,i:0,v:50}],[{}],[{n:52,i:0,v:50}],[{}]]};
      await ideaToClip(idea);
      return{same:clip.data===idea.data,rows:clip.data.length};
    });
    S.ok('the original array is passed straight through',r.same,r.rows+' rows');

    S.head('it actually sounds right');
    await page.evaluate(INSTALL_RENDER);
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;song.order=[0];
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      song.instruments=[{name:'k',type:'kick',params:{pitch:50,punch:1,decay:.15,drive:0}}];
      song.instruments.forEach(normInst);
      for(let i=0;i<4;i++){const c=ensureCell(0,i*4);c.n=48;c.i=0;c.v=64}
      sel={a:{t:0,c:0,r:0},b:{t:0,c:7,r:15}};
      const idea=captureIdea();
      const before=await window.__onsets();
      /* now clear, switch grid, paste the idea back, and listen again */
      await setLpb(8);
      song.patterns[0].data[0].fill(null);
      await ideaToClip(idea);
      cursor.t=0;cursor.r=0;pasteClip();
      const after=await window.__onsets();
      return{before:before.slice(0,4),after:after.slice(0,4)};
    });
    const drift=r.before.map((t,i)=>Math.abs(t-(r.after[i]==null?99:r.after[i])));
    S.ok('the pasted phrase hits at the same times',Math.max(...drift)<.012,
      'before '+r.before.join(', ')+'  →  after '+r.after.join(', ')+'  (max drift '+Math.max(...drift).toFixed(4)+'s)');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
