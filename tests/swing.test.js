/* Swing. The offsets are arithmetic, so they're checked directly — but feel is the whole point,
   so the last section renders audio and measures where the hits actually land. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('swing');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;song.order=[0];
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      song.instruments=[{name:'k',type:'kick',params:{pitch:50,punch:1,decay:.12,drive:0}}];
      song.instruments.forEach(normInst);
      song.trackset.forEach(normTs);
    });

    S.head('which rows move');
    let r=await page.evaluate(()=>{
      song.swing=1;song.swingRes=8;                    /* 1/8 swing, 4 rows per beat → unit 2 */
      const rd=rowDur();
      const off=[];for(let i=0;i<8;i++)off.push(+(swingOff(0,i)/rd).toFixed(3));
      return{off,unit:swingUnit(),rd:+rd.toFixed(4)};
    });
    S.ck('the unit is half a beat',r.unit,2);
    S.ck('only the off-eighths are nudged, by one row',r.off,[0,0,1,0,0,0,1,0]);

    r=await page.evaluate(()=>{
      song.swingRes=16;                                 /* every other sixteenth */
      const rd=rowDur(),off=[];
      for(let i=0;i<8;i++)off.push(+(swingOff(0,i)/rd).toFixed(3));
      return{off,unit:swingUnit()};
    });
    S.ck('1/16 halves the unit',r.unit,1);
    S.ck('  and moves every other row, by half of one',r.off,[0,.5,0,.5,0,.5,0,.5]);

    S.head('amount scales it');
    r=await page.evaluate(()=>{
      song.swingRes=8;
      const rd=rowDur(),out={};
      [0,.25,.5,1].forEach(a=>{song.swing=a;out[a]=+(swingOff(0,2)/rd).toFixed(3)});
      return out;
    });
    S.ck('0% is dead straight',r['0'],0);
    S.ck('  50% is half the shuffle',r['0.5'],.5);
    S.ck('  100% is a full row late (triplet feel)',r['1'],1);

    S.head('a channel can go its own way');
    r=await page.evaluate(()=>{
      song.swing=1;
      const rd=rowDur();
      song.trackset[0].swing=0;                        /* straight kick... */
      song.trackset[1].swing=null;                     /* ...under a shuffled everything else */
      const straight=+(swingOff(0,2)/rd).toFixed(3),followed=+(swingOff(1,2)/rd).toFixed(3);
      song.trackset[1].swing=.5;
      const own=+(swingOff(1,2)/rd).toFixed(3);
      const shown={own:tkPv(song.trackset[1],'swing'),inherited:(song.trackset[1].swing=null,tkPv(song.trackset[1],'swing'))};
      return{straight,followed,own,shown};
    });
    S.ck('an override of 0 keeps that track straight',r.straight,0);
    S.ck('  null follows the song',r.followed,1);
    S.ck('  its own value wins',r.own,.5);
    S.ck('the panel says which is which',[r.shown.own,r.shown.inherited],['50%','global 100%']);

    S.head('nowhere to go = no swing (never silently wrong)');
    r=await page.evaluate(async()=>{
      song.trackset.forEach(t=>t.swing=null);           /* drop the overrides set just above */
      song.swing=1;song.swingRes=16;
      await setLpb(2);                                  /* 1/16 needs 4+ rows per beat */
      const a={unit:swingUnit(),off:swingOff(0,1)};
      await setLpb(4);
      const b={unit:swingUnit(),off:+(swingOff(0,1)/rowDur()).toFixed(3)};
      return{a,b};
    });
    S.ck('2 rows/beat can not hold a 1/16 shuffle',[r.a.unit,r.a.off],[0,0]);
    S.ck('  4 rows/beat can',[r.b.unit,r.b.off],[1,.5]);

    S.head('the notes themselves never move');
    r=await page.evaluate(()=>{
      song.swing=1;song.swingRes=8;
      const c=ensureCell(0,2);c.n=48;c.i=0;c.v=64;
      return{row:JSON.stringify(song.patterns[0].data[0][2]),cmd:c.c,x:c.x};
    });
    S.ck('the cell keeps no swing command',[r.cmd,r.x],[null,0]);

    S.head('you can hear it');
    await page.evaluate(INSTALL_RENDER);
    r=await page.evaluate(async()=>{
      const pat=song.patterns[0];
      pat.data[0].fill(null);
      for(let i=0;i<8;i+=2){const c=ensureCell(0,i);c.n=48;c.i=0;c.v=64}
      song.trackset.forEach(t=>t.swing=null);
      song.swing=0;const straight=await window.__onsets();
      song.swing=1;const swung=await window.__onsets();
      return{straight:straight.slice(0,4),swung:swung.slice(0,4)};
    });
    const gaps=a=>a.slice(1).map((x,i)=>+(x-a[i]).toFixed(3));
    const gs=gaps(r.straight),gw=gaps(r.swung);
    S.ok('straight is evenly spaced',new Set(gs).size===1||Math.max(...gs)-Math.min(...gs)<.01,gs.join(', '));
    S.ok('swung is long-short-long',gw[0]>gw[1]*2,gw.join(', ')+'  (was '+gs.join(', ')+')');
    S.ok('  and the downbeats have not moved',Math.abs(r.straight[0]-r.swung[0])<.01&&Math.abs(r.straight[2]-r.swung[2])<.01,
      'beat 1 '+r.straight[0]+'→'+r.swung[0]+' · beat 2 '+r.straight[2]+'→'+r.swung[2]);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
