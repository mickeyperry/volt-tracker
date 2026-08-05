/* Everyday things that are easy to break and annoying to discover by hand:
   the edit cursor staying visible while the transport runs, the background-tab audio guard,
   the crash reporter, and the song format stamp. */
const{suite,open}=require('./lib');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));

module.exports=async function(){
  const S=suite('ui / transport');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(()=>{
      if(!prPan.hidden)prToggle();
      setFollow(true);
      window.V={
        inView(r){const el=grows.children[r],gw=gridwrap;
          return el.offsetTop>=gw.scrollTop-1&&el.offsetTop+el.offsetHeight<=gw.scrollTop+gw.clientHeight+1},
        drawn(r){const el=grows.children[r];return !!el.querySelector('.cur')||el.className.indexOf('cur')>=0}
      };
    });

    S.head('the cursor stays visible while playing');
    let r=await page.evaluate(()=>{
      SEQ.playing=true;cursor.r=48;setPlayRow(48);
      moveCursor(0,-16,0);                        /* PgUp */
      const up={row:cursor.r,seen:V.inView(32),drawn:V.drawn(32),hold:followHold};
      const st=Math.round(gridwrap.scrollTop);
      setPlayRow(49);setPlayRow(50);
      const parked=Math.round(gridwrap.scrollTop)===st;
      setPlayRow(32);
      const resumed=!followHold;
      cursor.r=32;setPlayRow(4);
      moveCursor(0,16,0);                          /* PgDn */
      const down={row:cursor.r,seen:V.inView(48)};
      const last=song.patterns[curPat].rows-1;
      moveCursor(0,last-cursor.r,0);               /* End */
      const end={row:cursor.r,seen:V.inView(last)};
      moveCursor(0,-cursor.r,0);                   /* Home */
      const home={row:cursor.r,seen:V.inView(0),top:Math.round(gridwrap.scrollTop)};
      SEQ.playing=false;stop();
      return{up,parked,resumed,down,end,home};
    });
    S.ck('PgUp shows the cursor row',[r.up.row,r.up.seen,r.up.drawn],[32,true,true]);
    S.ok('the playhead does not drag the view back',r.parked);
    S.ok('following resumes when playback reaches it',r.resumed);
    S.ck('PgDn shows the cursor row',[r.down.row,r.down.seen],[48,true]);
    S.ck('End shows the last row',[r.end.row,r.end.seen],[63,true]);
    S.ck('Home goes to the very top',[r.home.row,r.home.seen,r.home.top],[0,true,0]);

    S.head('real playback, real keystrokes');
    await page.evaluate(()=>{const p=song.patterns[curPat];p.la=p.lb=null;cursor.r=40;document.body.focus();play(false,32)});
    await sleep(250);
    await page.keyboard.press('Home');
    await sleep(450);
    r=await page.evaluate(()=>({row:cursor.r,top:Math.round(gridwrap.scrollTop),seen:V.inView(0),playing:SEQ.playing}));
    S.ck('Home during playback parks at row 0',[r.row,r.top,r.seen,r.playing],[0,0,true,true]);
    await page.keyboard.press('End');
    await sleep(350);
    r=await page.evaluate(()=>({row:cursor.r,seen:V.inView(song.patterns[curPat].rows-1),playing:SEQ.playing}));
    S.ck('End during playback',[r.row,r.seen,r.playing],[63,true,true]);
    await page.evaluate(()=>stop());

    S.head('background tab does not starve the scheduler');
    r=await page.evaluate(async()=>{
      Object.defineProperty(document,'hidden',{configurable:true,get:()=>true});
      play(false,0);
      await new Promise(x=>setTimeout(x,200));
      const ahead=SEQ.next-AC.currentTime;
      stop();
      Object.defineProperty(document,'hidden',{configurable:true,get:()=>false});
      play(false,0);
      await new Promise(x=>setTimeout(x,200));
      const aheadVisible=SEQ.next-AC.currentTime;
      stop();
      return{hidden:+ahead.toFixed(3),visible:+aheadVisible.toFixed(3)};
    });
    S.ok('schedules ~1.6s ahead when hidden',r.hidden>1,r.hidden+'s hidden vs '+r.visible+'s visible');
    S.ok('stays tight when visible',r.visible<.35,r.visible+'s');

    S.head('themes');
    r=await page.evaluate(()=>{
      const bad={};
      for(const[id,t]of Object.entries(THEMES)){
        if(id==='volt')continue; /* the base theme is the stylesheet's own values */
        const miss=themeKeys.filter(k=>!(k in t.v));
        const junk=Object.entries(t.v).filter(([k,v])=>!/^(#[0-9a-f]{3,8}|rgba?\(|hsla?\()/i.test(v)).map(([k])=>k);
        if(miss.length||junk.length)bad[id]={miss,junk};
      }
      return{count:Object.keys(THEMES).length,opts:document.getElementById('themeSel').options.length,bad};
    });
    S.ck('every theme is complete and valid',r.bad,{});
    S.ck('the dropdown offers all of them',r.opts,r.count);

    S.head('view toggles');
    r=await page.evaluate(async()=>{
      await loadSong(KITS['mine'].make());
      const railW=()=>document.getElementById('side').offsetWidth;
      const w0=railW(),g0=gridwrap.clientWidth;
      toggleRail();const hidden={rail:railW(),wider:gridwrap.clientWidth>g0};
      toggleRail();const backW=railW();
      sel={a:{t:1,c:0,r:0},b:{t:2,c:7,r:16}};cursor.t=1;renderGrid();
      const cells=()=>[...document.querySelectorAll('#ghead .tcell[data-t]')].filter(e=>e.offsetParent).length;
      const all=cells(),mutes0=song.trackset.map(t=>t.mute?1:0).join('');
      focusTracks();
      const few=cells();
      cursor.t=1;moveCursor(1,0,0);const skip1=cursor.t;moveCursor(1,0,0);const skip2=cursor.t;
      const mutes1=song.trackset.map(t=>t.mute?1:0).join('');
      focusTracks();
      return{w0,hidden,backW,all,few,skip1,skip2,mutes0,mutes1,restored:cells(),focus:FOCUS};
    });
    S.ck('Alt+L hides the rail and widens the grid',[r.hidden.rail,r.hidden.wider],[0,true]);
    S.ck('  and brings it back',r.backW,r.w0);
    S.ck('Alt+H shows only the selected columns',[r.all,r.few],[8,2]);
    S.ck('  the cursor skips the hidden ones',[r.skip1,r.skip2],[2,1]);
    S.ck('  hiding never touches mute state',r.mutes1,r.mutes0);
    S.ck('  and it all comes back',[r.restored,r.focus],[8,null]);

    S.head('armed transport mode: Space plays what the lit button says');
    r=await page.evaluate(async()=>{
      await loadSong(KITS['mine'].make());
      /* an arrangement with the same pattern twice, so "which slot" is a real question */
      while(song.patterns.length<2)newPattern();
      song.order=[0,1,0];renderOrder();
      const lit=()=>({pat:$id('playPat').classList.contains('armed'),song:$id('playSong').classList.contains('armed')});
      armSet(true);
      const armedSong=lit();
      /* park the edit cursor on slot 3 (pattern 0 again) and play the song from there */
      ordPos=2;curPatSet(song.order[2]);
      play(true);
      const startedAt=SEQ.ord;                 /* must be 2, NOT 0 */
      stop();
      armSet(false);
      const armedPat=lit();
      return{armedSong,armedPat,startedAt};
    });
    S.ck('arming Song lights the Song button only',[r.armedSong.song,r.armedSong.pat],[true,false]);
    S.ck('  and arming Pattern swaps it',[r.armedPat.pat,r.armedPat.song],[true,false]);
    S.ck('song play starts at the slot you are editing',r.startedAt,2);

    S.head('\\ flips mode without stopping the transport');
    r=await page.evaluate(async()=>{
      const out={};
      armSet(false);
      curPatSet(1);
      play(false);
      await new Promise(x=>setTimeout(x,220));
      out.patRunning=SEQ.playing&&!SEQ.songMode;
      const t0=SEQ.next;
      flipMode();                               /* → SONG */
      out.stillPlaying=SEQ.playing;
      out.nowSong=SEQ.songMode;
      out.noRestart=SEQ.next===t0;              /* nothing was rescheduled */
      out.slot=SEQ.ord;                         /* slot 2 holds pattern 1 */
      out.armed=$id('playSong').classList.contains('armed');
      await new Promise(x=>setTimeout(x,220));
      flipMode();                               /* → PATTERN, onto whatever is sounding */
      out.backToPat=SEQ.playing&&!SEQ.songMode;
      out.locked=curPat===(lastQ?lastQ.pat:curPat);
      stop();
      /* empty arrangement must never leave you with silence */
      song.order=[];armSet(true);
      play(true);
      out.emptyFallsBack=SEQ.playing&&!SEQ.songMode;
      stop();
      song.order=[0];renderOrder();
      return out;
    });
    S.ok('pattern mode is running first',r.patRunning);
    S.ck('\\ switches to song mid-flight',[r.stillPlaying,r.nowSong],[true,true]);
    S.ok('  without rescheduling anything',r.noRestart);
    S.ck('  landing on the slot that holds the playing pattern',r.slot,1);
    S.ok('  and the Song button is armed',r.armed);
    S.ok('\\ back to pattern keeps playing',r.backToPat);
    S.ok('  locked onto the pattern that was sounding',r.locked);
    S.ok('an empty song order falls back to the pattern',r.emptyFallsBack);

    /* everything above drove flipMode() directly — prove the key itself is wired, from the grid,
       where every other letter is a note */
    await page.evaluate(()=>{song.order=[0,1];renderOrder();armSet(false);cursor.c=0;cursor.r=0;document.body.focus()});
    await page.keyboard.press('Backslash');
    r=await page.evaluate(()=>({armed:armSong,cell:JSON.stringify(cellAt(cursor.t,cursor.r))}));
    S.ok('the \\ key itself flips the armed mode',r.armed===true);
    S.ck('  and never writes a note',r.cell,'null');
    await page.keyboard.press('Backslash');
    S.ok('  and flips back',await page.evaluate(()=>armSong===false));
    /* the left rail owns the keyboard for edit keys — transport keys must still get through */
    await page.evaluate(()=>{const b=document.querySelector('#side button');b&&b.focus()});
    await page.keyboard.press('Backslash');
    S.ok('  and works with a left-rail panel focused',await page.evaluate(()=>armSong===true));
    await page.evaluate(()=>{document.body.focus();armSet(false)});

    S.head('crashes are visible, songs are stamped');
    r=await page.evaluate(async()=>{
      dispatchEvent(new ErrorEvent('error',{error:new Error('test blow-up'),message:'test blow-up'}));
      await new Promise(x=>setTimeout(x,50));
      const shown=$id('status').textContent;
      const s=JSON.parse(serialize(false));
      return{shown,v:s.v,lpb:s.lpb,captured:!!lastErr};
    });
    S.ok('an error reaches the status bar',/blow-up/.test(r.shown),r.shown);
    S.ok('  and is kept for Alt+/',r.captured);
    S.ck('saved songs carry a format version',r.v,1);
    S.ok('  and their rows/beat',r.lpb>0,'lpb '+r.lpb);

    /* the injected test error is expected — don't count it as a failure */
    const real=errs.filter(e=>!/blow-up/.test(e));
    S.ok('no unexpected console errors',real.length===0,real.join(' | '));
  }finally{await browser.close()}
  return S;
};
