/* Piano-roll auditioning: hear what you touch. The risk here isn't that a preview fails to play —
   it's that the new gesture eats an existing one, so the erase and select paths are checked as
   hard as the sound is. */
const{suite,open}=require('./lib');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));

module.exports=async function(){
  const S=suite('piano roll audition');
  const{browser,page,errs}=await open();
  try{
    const seed=()=>page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=120;song.lpb=4;
      const pat=song.patterns[0];pat.rows=16;pat.la=pat.lb=null;
      pat.data.forEach(c=>c.fill(null));
      for(const[r,n]of[[0,48],[4,52],[8,55]]){const c=ensureCell(0,r);c.n=n;c.i=0;c.v=50}
      cursor.t=0;cursor.r=0;
      if(prPan.hidden)prToggle();
      prRender();
      audio();
      /* count auditions instead of listening: wrap trig once */
      if(!window.__trigWrapped){
        window.__trigWrapped=true;window.__trigs=[];
        const real=trig;
        window.trig=function(inst,note,t,tr,vel){window.__trigs.push({note,tr,vel});return real.apply(null,arguments)};
      }
      window.__trigs.length=0;
      /* where a note sits on screen */
      window.__at=(r,n)=>{
        const b=prCv.getBoundingClientRect();
        return{clientX:b.left+PR.GUT+r*PR.CW+PR.CW/2,clientY:b.top+(PR.base+PR.NP-1-n)*PR.CH+PR.CH/2};
      };
    });
    await seed();

    S.head('clicking a note plays it');
    let r=await page.evaluate(()=>{
      window.__trigs.length=0;
      const p=__at(4,52);
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,...p}));
      document.dispatchEvent(new MouseEvent('mouseup'));
      return{trigs:__trigs.map(x=>x.note),count:__trigs.length};
    });
    S.ck('the note you clicked is the note you hear',r.trigs,[52]);

    r=await page.evaluate(()=>{
      window.__trigs.length=0;
      const p=__at(9,40);                     /* empty space — the roll draws a note here */
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,...p}));
      document.dispatchEvent(new MouseEvent('mouseup'));
      return{trigs:__trigs.map(x=>x.note),drawn:!!song.patterns[0].data[0].some(c=>c&&c.n===40)};
    });
    S.ok('drawing a new note still draws it',r.drawn);
    S.ck('  and sounds it once, not twice',r.trigs,[40]);

    S.head('Alt + right-drag scrubs through time');
    await seed();
    r=await page.evaluate(async()=>{
      window.__trigs.length=0;
      /* start left of everything, then sweep RIGHT along a single height that touches no note */
      const y=__at(0,20).clientY;
      const X=r2=>__at(r2,48).clientX;
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:2,altKey:true,clientX:X(0),clientY:y}));
      const started=!!PR.scrub;
      const first=__trigs.map(x=>x.note);
      for(const rr of[2,4,6,8]){
        prCv.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:X(rr),clientY:y}));
      }
      const heard=__trigs.map(x=>x.note);
      const lit=(PR.scrub.lit||[]).slice();
      const row=PR.scrub.row;
      document.dispatchEvent(new MouseEvent('mouseup'));
      return{started,first,heard,lit,row,ended:!PR.scrub,
             notesIntact:song.patterns[0].data[0].filter(c=>c&&c.n!=null&&c.n!==OFF).length};
    });
    S.ok('the scrub starts on Alt+right-press',r.started);
    S.ck('  and fires the row it starts on',r.first,[48]);
    S.ck('the pointer never touched a note, yet all three played',r.heard,[48,52,55]);
    S.ck('  the last row crossed is highlighted',[r.row,r.lit],[8,['0:8:55']]);
    S.ok('  it stops on release',r.ended);
    S.ck('  nothing is deleted or moved',r.notesIntact,3);

    r=await page.evaluate(async()=>{
      window.__trigs.length=0;
      const y=__at(0,20).clientY,X=r2=>__at(r2,48).clientX;
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:2,altKey:true,clientX:X(0),clientY:y}));
      window.__trigs.length=0;
      /* one big jump right across everything, then back again */
      prCv.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:X(9),clientY:y}));
      const forward=__trigs.map(x=>x.note);
      window.__trigs.length=0;
      prCv.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:X(0),clientY:y}));
      const back=__trigs.map(x=>x.note);
      document.dispatchEvent(new MouseEvent('mouseup'));
      return{forward,back};
    });
    S.ck('a fast flick still plays everything it passed',r.forward,[52,55]);
    S.ck('  and dragging back plays them in reverse',r.back,[55,52,48]);

    S.head('plain right-click is still the eraser');
    await seed();
    r=await page.evaluate(()=>{
      const p=__at(4,52);
      prCv.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,...p}));
      return{left:song.patterns[0].data[0].filter(c=>c&&c.n!=null&&c.n!==OFF).length,
             gone:!song.patterns[0].data[0].some(c=>c&&c.n===52)};
    });
    S.ck('it deletes the note under the pointer',[r.left,r.gone],[2,true]);

    await seed();
    r=await page.evaluate(()=>{
      const p=__at(4,52);
      prCv.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,altKey:true,...p}));
      return song.patterns[0].data[0].filter(c=>c&&c.n!=null&&c.n!==OFF).length;
    });
    S.ck('but Alt+right-click does NOT — that gesture is the sweep',r,3);

    S.head('dragging a note sounds each new pitch');
    await seed();
    r=await page.evaluate(async()=>{
      window.__trigs.length=0;
      const a=__at(0,48);
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,...a}));
      const afterClick=__trigs.length;
      for(const n of[49,50,50,51]){
        const p=__at(0,n);
        document.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,...p}));
      }
      const heard=__trigs.map(x=>x.note);
      document.dispatchEvent(new MouseEvent('mouseup'));
      return{afterClick,heard};
    });
    S.ck('the press itself plays the original pitch',r.heard.slice(0,1),[48]);
    S.ok('  then every new pitch as you pass over it',
      r.heard.length>=3&&r.heard.indexOf(50)>0&&r.heard.indexOf(51)>r.heard.indexOf(50),
      r.heard.join(', '));
    S.ok('  and a pitch you sit on is not retriggered',
      r.heard.filter(x=>x===50).length===1,r.heard.join(', '));

    S.head('previews do not hang around');
    await seed();
    r=await page.evaluate(async()=>{
      const p=__at(0,48);
      prCv.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0,...p}));
      document.dispatchEvent(new MouseEvent('mouseup'));
      const during=!!prVoice;
      prAuditionStop();
      return{during,after:!prVoice};
    });
    S.ok('a preview voice exists while it sounds',r.during);
    S.ok('  and is released afterwards',r.after);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
