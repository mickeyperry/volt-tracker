/* Ctrl+Alt+R — reshuffle the pitches already on a channel.
   The whole value of it is what it REFUSES to do: it never invents a pitch. A melodic channel
   draws only from the notes that channel already plays (so a bassline cannot wander out of key),
   and a SLICER draws from its own slices — a note past the last slice is a ghost, clamping onto
   whatever slice is nearest and playing something that is not in the kit. Mickey asked for it in
   exactly those terms: "if its slicer notes - not to go out of the scope of notes so we dont have
   ghost notes" (2026-08-15). */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('randomize / reshuffle notes');
  const{browser,page,errs}=await open();
  try{
    S.head('a melodic channel is reshuffled inside its own pitches');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('sub','Bass',{harm:.15,rel:.06})];
      const pat=song.patterns[curPat];
      /* four distinct pitches, twelve notes — enough that a shuffle almost surely moves one */
      const pitches=[36,40,43,48];
      for(let i=0;i<12;i++)pat.data[0][i*2]={n:pitches[i%4],i:0,v:48,c:null,x:0};
      const before=pat.data[0].map(c=>c&&c.n);
      cursor.t=0;clearSel();
      randomizeNotes();
      const after=pat.data[0].map(c=>c&&c.n);
      return{
        rows:after.map((n,i)=>[n,before[i]]).filter(([a,b])=>(a==null)!==(b==null)).length,
        count:after.filter(n=>n!=null).length,
        outside:after.filter(n=>n!=null&&pitches.indexOf(n)<0).length,
        moved:after.filter((n,i)=>n!=null&&n!==before[i]).length,
        vels:pat.data[0].filter(c=>c).every(c=>c.v===48&&c.i===0),
      };
    });
    S.ck('no note is added or removed',r.count,12);
    S.ck('  and none moves to a different row',r.rows,0);
    S.ck('  every pitch is one the channel already played',r.outside,0);
    S.ok('  and something actually changed',r.moved>0,r.moved+' of 12 moved');
    S.ok('  velocities and instruments are untouched',r.vels);

    S.head('a slicer stays inside its slices — no ghost notes');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      const b=new OfflineAudioContext(1,1000,44100).createBuffer(1,1000,44100);
      /* three slices from base 48: the only legal pitches are 48, 49 and 50 */
      song.instruments=[Object.assign(mkInst('sliced','Break'),{buffer:b,sliceBase:48,
        slices:[{s0:0,s1:.33},{s0:.33,s1:.66},{s0:.66,s1:1}]})];
      const pat=song.patterns[curPat];
      /* deliberately only ONE pitch in use — a melodic channel would have nothing to shuffle,
         but a slicer must still draw on every slice it owns */
      for(let i=0;i<16;i++)pat.data[0][i]={n:48,i:0,v:64,c:null,x:0};
      cursor.t=0;clearSel();
      const seen=new Set();
      for(let k=0;k<6;k++){randomizeNotes();pat.data[0].forEach(c=>{if(c&&c.n!=null)seen.add(c.n)})}
      const notes=pat.data[0].filter(c=>c).map(c=>c.n);
      return{
        lo:Math.min(...notes),hi:Math.max(...notes),
        distinct:[...seen].sort((a,b)=>a-b),
        count:notes.length,
      };
    });
    S.ck('nothing is lost',r.count,16);
    S.ok('  no note below the first slice',r.lo>=48,'lowest '+r.lo);
    S.ok('  and none past the last one — a ghost note is impossible',r.hi<=50,'highest '+r.hi);
    S.ok('  the whole kit gets used',r.distinct.length>1,r.distinct.join(','));

    S.head('a note-OFF, a disabled note and an empty channel are left alone');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('sub','Bass',{harm:.15,rel:.06})];
      const pat=song.patterns[curPat];
      pat.data[0][0]={n:36,i:0,v:48,c:null,x:0};
      pat.data[0][1]={n:40,i:0,v:48,c:null,x:0};
      pat.data[0][2]={n:-1,i:null,v:null,c:null,x:0};      /* note-OFF */
      pat.data[0][3]={n:43,i:0,v:48,c:null,x:0,d:1};       /* disabled */
      cursor.t=0;clearSel();
      for(let k=0;k<5;k++)randomizeNotes();
      const off=pat.data[0][2],dis=pat.data[0][3];
      cursor.t=1;                                          /* an empty channel */
      randomizeNotes();
      return{off:off.n,dis:dis.n,disFlag:dis.d,said:document.getElementById('status').textContent};
    });
    S.ck('the note-OFF is still a note-OFF',r.off,-1);
    S.ck('  the disabled note keeps its pitch',r.dis,43);
    S.ck('  and stays disabled',r.disFlag,1);
    S.ok('  an empty channel says so instead of doing nothing quietly',
      /no notes|nothing to shuffle/i.test(r.said||''),r.said);

    S.head('the BUILD-UP generator: an accelerating roll with a rising cutoff lane');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('snare','Sn',{tone:190,snap:.7,decay:.16})];
      curInst=0;cursor.t=2;clearSel();
      const pat=song.patterns[curPat];               /* 64 rows, channel 3 empty */
      buildUp();
      const cells=pat.data[2].map((c,r)=>c?{r,n:c.n,v:c.v,c:c.c,x:c.x,i:c.i}:null).filter(Boolean);
      const lane=(pat.auto||{})['2.fltF']||[];
      const laneVals=lane.map((v,r)=>v!=null?{r,v}:null).filter(Boolean);
      return{
        first:cells[0]&&cells[0].r,last:cells[cells.length-1],
        outside:cells.filter(c=>c.r<48).length,
        velsClimb:cells.every((c,k)=>!k||c.v>=cells[k-1].v),
        pitches:[...new Set(cells.map(c=>c.n))],
        insts:[...new Set(cells.map(c=>c.i))],
        rolls:cells.filter(c=>c.c==='R').length,
        laneClimbs:laneVals.length>4&&laneVals.every((p,k)=>!k||p.v>=laneVals[k-1].v),
        laneOutside:laneVals.filter(p=>p.r<48).length,
      };
    });
    S.ck('with no selection it takes the LAST 16 rows',r.first,48);
    S.ck('  and touches nothing before them',r.outside,0);
    S.ok('velocity climbs the whole way',r.velsClimb);
    S.ok('the roll accelerates into Rxx bursts',r.rolls>=4,r.rolls+' rows carry R');
    S.ok('  ripping hardest on the very last row',r.last&&r.last.c==='R'&&r.last.x===8,JSON.stringify(r.last));
    S.ok('one pitch, one instrument — a riser, not a melody',r.pitches.length===1&&r.insts.length===1,
      'pitches '+r.pitches.join(',')+' insts '+r.insts.join(','));
    S.ok('the cutoff lane rises with it, only inside the region',
      r.laneClimbs&&r.laneOutside===0,'climbs '+r.laneClimbs+' outside '+r.laneOutside);

    S.head('the ACID LINE generator stays in its scale, off the channel’s own root');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('acid303','Acid',{})];
      curInst=0;cursor.t=1;clearSel();
      const pat=song.patterns[curPat];
      pat.data[1][0]={n:50,i:0,v:48,c:null,x:0};   /* the channel already plays D — that is the root */
      acidLine();
      const cells=pat.data[1].map((c,r)=>c?{r,n:c.n,v:c.v,c:c.c}:null).filter(Boolean);
      const scale=[0,1,3,5,7,8,10];
      return{
        count:cells.length,rows:pat.rows,
        rootFirst:cells[0]&&cells[0].r===0&&cells[0].n===50&&cells[0].v===64,
        outOfScale:cells.filter(c=>scale.indexOf(((c.n-50)%12+12)%12)<0).length,
        cmds:[...new Set(cells.map(c=>c.c).filter(Boolean))],
        someRests:cells.length<pat.rows,
      };
    });
    S.ok('it writes a real line',r.count>=r.rows*.25,r.count+' notes in '+r.rows+' rows');
    S.ok('  landing on the one: the root leads, accented',r.rootFirst);
    S.ck('every pitch sits in phrygian off the root',r.outOfScale,0);
    S.ok('  slides are the only command it deals',r.cmds.every(c=>c==='G'),r.cmds.join(',')||'none');
    S.ok('  and it leaves room to breathe',r.someRests);

    S.head('the ROLLING BASS leaves the downbeat to the kick');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('psybass','Bass',{})];
      curInst=0;cursor.t=3;clearSel();
      const pat=song.patterns[curPat];
      rollingBass();
      const cells=pat.data[3].map((c,i)=>c?{r:i,n:c.n,v:c.v}:null).filter(Boolean);
      return{
        downbeats:cells.filter(c=>c.r%song.lpb===0).length,
        count:cells.length,expected:pat.rows-pat.rows/song.lpb,
        offRoot:cells.filter(c=>c.n!==36).every(c=>c.n===39||c.n===43),
        vels:[...new Set(cells.map(c=>c.v))].sort((a,b)=>a-b),
      };
    });
    S.ck('every downbeat row is empty — that row is the kick’s',r.downbeats,0);
    S.ck('  and every other 16th carries bass',r.count,r.expected);
    S.ok('the only notes off the root are the phrase-end 3rd/5th',r.offRoot);
    S.ok('  two velocity levels: the roll and the push',r.vels.length===2,r.vels.join(','));

    S.head('the DRUM FILL draws only on the channel’s own hits and ends on one');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('snare','Sn',{})];
      curInst=0;cursor.t=1;clearSel();
      const pat=song.patterns[curPat];
      pat.data[1][0]={n:48,i:0,v:40,c:null,x:0};
      pat.data[1][4]={n:50,i:0,v:40,c:null,x:0};
      drumFill();
      const region=pat.data[1].slice(pat.rows-8).map((c,i)=>c?{r:i,n:c.n,v:c.v}:null).filter(Boolean);
      const outside=pat.data[1].slice(8,pat.rows-8).filter(Boolean).length;
      return{
        pool:region.every(c=>c.n===48||c.n===50),
        outside,last:pat.data[1][pat.rows-1],
      };
    });
    S.ok('every fill pitch is one the channel already played',r.pool);
    S.ck('  rows between the seed notes and the fill are untouched',r.outside,0);
    S.ok('the fill ends on a full-velocity hit',r.last&&r.last.v===64,JSON.stringify(r.last));

    S.head('the GROOVE STAMP shapes velocities and touches nothing else');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('hat','H',{})];
      curInst=0;cursor.t=2;clearSel();
      const pat=song.patterns[curPat];
      for(let i=0;i<16;i++)pat.data[2][i]={n:60,i:0,v:48,c:null,x:0};
      grooveStamp();
      const cells=pat.data[2].slice(0,16);
      return{
        count:cells.filter(Boolean).length,
        pitches:cells.every(c=>c.n===60),
        beatsLoud:cells.filter((c,i)=>i%song.lpb===0).every(c=>c.v>=55),
        offbeatsSoft:cells.filter((c,i)=>i%song.lpb!==0&&i%song.lpb!==song.lpb/2).every(c=>c.v<=40),
      };
    });
    S.ck('no note added or removed',r.count,16);
    S.ok('  pitches untouched',r.pitches);
    S.ok('the beat is accented',r.beatsLoud);
    S.ok('  the offbeats are ghosts',r.offbeatsSoft);

    S.head('the CHORD STAMP grows lanes and stops the whole stack on an OFF');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('supersaw','Saw',{})];
      curInst=0;cursor.t=0;clearSel();
      const pat=song.patterns[curPat];
      pat.data[0][0]={n:48,i:0,v:50,c:null,x:0};
      pat.data[0][4]={n:-1,i:null,v:null,c:null,x:0};
      askPick=async()=>[3,7];                    /* the dialog, answered: Minor */
      await chordStamp();
      const lanes=polyLanes(0);
      return{
        lanes:lanes.length,
        third:pat.data[lanes[1]]&&pat.data[lanes[1]][0]&&pat.data[lanes[1]][0].n,
        fifth:pat.data[lanes[2]]&&pat.data[lanes[2]][0]&&pat.data[lanes[2]][0].n,
        offs:lanes.slice(1).every(l=>pat.data[l][4]&&pat.data[l][4].n===-1),
      };
    });
    S.ck('two lanes were grown for the minor stack',r.lanes,3);
    S.ck('the third lands on its lane',r.third,51);
    S.ck('  and the fifth on its own',r.fifth,55);
    S.ok('a note-OFF stops the whole chord, every lane',r.offs);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
