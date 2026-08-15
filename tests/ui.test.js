/* Everyday things that are easy to break and annoying to discover by hand:
   the edit cursor staying visible while the transport runs, the background-tab audio guard,
   the crash reporter, and the song format stamp. */
const{suite,open}=require('./lib');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const chipFill=r=>r.fill?'gradient painted':'no gradient on the playing chip';

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

    S.head('pattern progress lives on the song strip, not over row 000');
    await page.evaluate(async()=>{
      await loadSong(blankSong());
      while(song.patterns.length<3)newPattern();
      song.order=[0,1,2];curPatSet(1);ordPos=1;renderOrder();
      const p=song.patterns[1];p.la=p.lb=null;
      document.body.focus();
    });
    r=await page.evaluate(()=>{
      const a=getComputedStyle(ghead,'::after');
      return{content:a.content,pp:getComputedStyle(ghead).getPropertyValue('--pp').trim()};
    });
    S.ok('the bar above row 000 is gone',r.content==='none'||r.content==='',
      'ghead::after content = '+JSON.stringify(r.content));

    await page.evaluate(()=>play(false,0));          /* looping ONE pattern */
    await sleep(500);
    r=await page.evaluate(()=>{
      const chips=[...document.querySelectorAll('#ordChips .chip')];
      return{lit:chips.map(c=>c.classList.contains('playing')),
             fill:chips[1]&&/gradient/.test(chips[1].style.background||''),
             playing:SEQ.playing,song:SEQ.songMode};
    });
    S.ck('looping a pattern lights its own slot',[r.lit,r.playing,r.song],[[false,true,false],true,false]);
    S.ok('  and fills it as the pattern runs',r.fill,chipFill(r));
    await page.evaluate(()=>stop());
    await sleep(120);
    r=await page.evaluate(()=>[...document.querySelectorAll('#ordChips .chip')].map(c=>c.classList.contains('playing')));
    S.ck('stopping clears it',r,[false,false,false]);

    S.head('the channel is wide enough for its buttons');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());renderGrid();renderHead();
      const cell=grows.children[0].querySelector('.tcell').getBoundingClientRect().width;
      const h=ghead.querySelector('.tcell[data-t="0"]');
      const ctl=h.querySelector('.thctl');
      return{cell:+cell.toFixed(1),head:+h.getBoundingClientRect().width.toFixed(1),
             needs:ctl.scrollWidth,has:ctl.clientWidth,
             btns:[...h.querySelectorAll('.tbtn')].map(b=>b.textContent).join(' ')};
    });
    S.ck('all four buttons are there',r.btns,'M S SC ❄');
    S.ok('  and none of them is clipped',r.needs<=r.has+1,r.needs+'px of buttons in '+r.has+'px');
    S.ck('  the header still matches its grid column',r.head,r.cell);

    /* ---- cutting a note-OFF actually cuts it -------------------------------------------------
       `eraseRect`'s guard re-inserts a terminator when the cut would otherwise let an earlier
       note ring past its end. Correct when you cut MUSIC; maddening when the thing you selected
       IS the terminator — it reappeared in the same cell and note-offs looked uncuttable
       (2026-08-15). Both halves are asserted here, because deleting the guard entirely would
       trade this bug for a note that never ends. */
    S.head('Ctrl+X removes a note-OFF, but still protects a cut note');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      const d=song.patterns[curPat].data;
      /* a note at row 0, its terminator at row 4 */
      d[0][0]={n:48,i:0,v:64};
      d[0][4]={n:OFF,i:null,v:null};
      /* select JUST the OFF and cut it */
      sel={a:{t:0,c:0,r:4},b:{t:0,c:7,r:4}};
      copySel(true);
      const offGone=!(d[0][4]&&d[0][4].n===OFF);
      const noteKept=!!(d[0][0]&&d[0][0].n===48);

      /* now the other half: a note at 8 ending at 12, cut the NOTE's rows including the OFF —
         the terminator must come back so the note before it does not run away */
      d[0][8]={n:50,i:0,v:64};
      d[0][10]={n:52,i:0,v:64};
      d[0][12]={n:OFF,i:null,v:null};
      sel={a:{t:0,c:0,r:10},b:{t:0,c:7,r:12}};
      copySel(true);
      /* the note at 8 used to end at row 10, where the next note began — so the terminator goes
         back THERE, keeping that note exactly the length it was */
      const guarded=!!(d[0][10]&&d[0][10].n===OFF);
      const noteAt8=!!(d[0][8]&&d[0][8].n===50);
      sel=null;
      return{offGone,noteKept,guarded,noteAt8};
    });
    S.ok('cutting a lone note-OFF removes it',r.offGone);
    S.ok('  and leaves the note before it alone',r.noteKept);
    S.ok('cutting a note WITH its OFF puts a terminator back at the old end',r.guarded);
    S.ok('  and the note before the cut keeps its length',r.noteAt8);

    /* ---- grid zoom ---------------------------------------------------------------------------
       Ctrl+wheel scales the grid's font size and EVERYTHING is derived from it. The assertion
       that matters is not the number but the invariant `calibrateRows` exists to protect: the
       placeholder height for skipped rows must still equal a real row's height, or a long
       pattern's scrollbar drifts and rows land in the wrong place. */
    S.head('Ctrl+wheel zooms the grid, and the lazy rows keep up');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.patterns[curPat].rows=64;renderGrid();
      const before=rowHpx;
      /* the wheel now coalesces into one rAF, so each step waits a frame */
      const frame=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
      /* the zoom GLIDES now — wait for the easing to arrive before measuring */
      const settle=async()=>{for(let i=0;i<40;i++){await frame();if(!gzRaf)break}};
      /* Ctrl+Alt = both axes, the proportional zoom */
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:-240,ctrlKey:true,altKey:true,bubbles:true,cancelable:true}));
      await settle();
      const inZoom=gridZoom,inH=rowHpx;
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:240,ctrlKey:true,altKey:true,bubbles:true,cancelable:true}));
      await settle();
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:240,ctrlKey:true,altKey:true,bubbles:true,cancelable:true}));
      await settle();
      const outZoom=gridZoom;
      /* a burst of events must cost ONE relayout, not twenty — that is the whole design */
      let renders=0;const realCal=calibrateRows;
      calibrateRows=()=>{renders++;realCal()};
      for(let i=0;i<12;i++)gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:-40,ctrlKey:true,bubbles:true,cancelable:true}));
      await frame();
      const burst=renders;          /* one frame of the glide, not twelve relayouts */
      await settle();
      calibrateRows=realCal;
      /* the placeholder the skipped rows use must match a real measured row */
      const declared=parseFloat(getComputedStyle(grows).getPropertyValue('--rowh'));
      const real=grows.children[0].getBoundingClientRect().height;
      /* a plain wheel must still scroll, not zoom */
      /* ---- the two axes, independently ----
         Row height is font-size x line-height; column width is font-size. So "rows only" must
         change height and leave width alone, and "columns only" the exact reverse. Measured off
         a real cell, not off the variables, because the variables are the thing under test. */
      zoomSet(1,1);await frame();
      const cellW=()=>grows.querySelector('.tcell').getBoundingClientRect().width;
      const rowH=()=>grows.children[0].getBoundingClientRect().height;
      const w0=cellW(),h0=rowH();
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:-300,ctrlKey:true,bubbles:true,cancelable:true}));
      await settle();
      const yOnly={w:cellW(),h:rowH()};
      resetGridZoom();await frame();
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:300,altKey:true,bubbles:true,cancelable:true}));
      await settle();
      const xOnly={w:cellW(),h:rowH()};
      resetGridZoom();await frame();
      const afterReset={w:cellW(),h:rowH()};

      /* ---- the header must stay glued to its column ----
         The M/S/SC/freeze buttons used to be fixed pixels, so narrowing the columns left them
         at full width and the header slid out of line with the notes underneath. Compare the
         header cell's left edge with the grid cell's, zoomed hard both ways. */
      const align=()=>{
        const h=[...document.querySelectorAll('#ghead .thead')];
        const c=[...grows.children[0].querySelectorAll('.tcell')];
        let worst=0;
        for(let i=0;i<Math.min(h.length,c.length);i++)
          worst=Math.max(worst,Math.abs(h[i].getBoundingClientRect().left-c[i].getBoundingClientRect().left));
        return worst;
      };
      resetGridZoom();await frame();
      const alignedAt100=align();
      zoomBy(0.55,'x',null);await settle();
      const alignedNarrow=align();
      resetGridZoom();zoomBy(2.2,'x',null);await settle();
      const alignedWide=align();
      resetGridZoom();await frame();

      const z0=gridZoom;
      gridwrap.dispatchEvent(new WheelEvent('wheel',{deltaY:-100,bubbles:true,cancelable:true}));
      await frame();
      const plain=gridZoom;
      zoomSet(1,1);
      return{before,inZoom,inH,outZoom,declared,real,z0,plain,burst,w0,h0,yOnly,xOnly,afterReset,
              alignedAt100,alignedNarrow,alignedWide};
    });
    S.ok('Ctrl+Alt+wheel up zooms both axes',r.inZoom>1,'zoom '+r.inZoom);
    S.ok('  and the rows actually got taller',r.inH>r.before,r.before+' -> '+r.inH);
    S.ok('  and down goes smaller',r.outZoom<r.inZoom,'zoom '+r.outZoom);
    S.ok('the skipped-row placeholder still matches a real row',
         Math.abs(r.declared-r.real)<0.6,r.declared+'px declared vs '+r.real+'px real');
    S.ck('a plain wheel scrolls instead of zooming',r.plain,r.z0);
    /* The glide paints once per FRAME, so two rAFs of waiting may legitimately show two.
       What must never happen is one relayout per wheel EVENT — that is the jank this design
       exists to avoid. */
    S.ok('  twelve wheel events coalesce into frames, not twelve relayouts',
         r.burst<=2,r.burst+' relayouts for 12 events');
    S.ok('Ctrl+wheel changes ROW HEIGHT only (FL’s gesture)',
         r.yOnly.h>r.h0+1&&Math.abs(r.yOnly.w-r.w0)<0.6,
         'h '+r.h0+'->'+r.yOnly.h+', w '+r.w0+'->'+r.yOnly.w);
    S.ok('Alt+wheel changes COLUMN WIDTH only',
         r.xOnly.w<r.w0-1&&Math.abs(r.xOnly.h-r.h0)<1.5,
         'w '+r.w0+'->'+r.xOnly.w+', h '+r.h0+'->'+r.xOnly.h);
    S.ok('the channel headers stay glued to their columns, narrow',
         r.alignedNarrow<1.5,'worst edge gap '+r.alignedNarrow+'px (100%: '+r.alignedAt100+'px)');
    S.ok('  and wide',r.alignedWide<1.5,'worst edge gap '+r.alignedWide+'px');
    S.ok('  and Ctrl+0 puts both back',
         Math.abs(r.afterReset.w-r.w0)<0.6&&Math.abs(r.afterReset.h-r.h0)<0.6,
         'w '+r.afterReset.w+' h '+r.afterReset.h);

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
