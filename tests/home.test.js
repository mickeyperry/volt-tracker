/* The Projects panel. It reads storage rather than the song, so the things worth testing are that
   it tells the truth about what's stored, that it stays live, and — most importantly — that it can
   never lose a project: clicking a closed one has to bring it back, not replace it.
   It docks in the left rail as a foldable section, so "open" means unfolded AND the rail shown. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('projects panel');
  const{browser,page,errs}=await open();
  try{
    S.head('it lives in the rail, and knows what is in this browser');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.name='first song';
      flushSave();
      await new Promise(r=>setTimeout(r,60));
      toggleHome(true);
      const p=document.getElementById('homeBody');
      const sec=document.querySelector('#side .sec-home');
      return{inRail:!!sec&&!!sec.closest('#side'),
             firstInRail:document.querySelector('#side .sec')===sec,
             foldable:!!sec.querySelector('h3'),
             shown:homeOpen(),lit:document.getElementById('homeBtn').classList.contains('on'),
             rows:p.querySelectorAll('[data-hproj]').length,
             text:p.textContent,
             title:sec.querySelector('h3').textContent.trim(),
             cur:!!p.querySelector('.hrow.cur')};
    });
    S.ok('it is a section of the left rail, not a floating panel',r.inRail);
    S.ok('  sitting at the top, above Instruments',r.firstInRail);
    S.ok('  with a header you can fold it by',r.foldable);
    S.ok('the panel opens and the button lights',r.shown&&r.lit);
    S.ok('it lists at least the project you are in',r.rows>=1,r.rows+' project row(s)');
    S.ok('the current project is marked',r.cur);
    S.ck('the rail section is called Projects',r.title,'Projects');
    S.ok('  and it groups what it shows',
      /saved/i.test(r.text)&&/Ideas/.test(r.text)&&/Samples/.test(r.text)&&/storage/i.test(r.text));
    S.ok('the open project shows its shape',/\d+ pat/.test(r.text)&&/\d+ trk/.test(r.text),
      (r.text.match(/\d+ pat[^<]*/)||[''])[0].trim());

    S.head('it follows the song, live');
    r=await page.evaluate(async()=>{
      song.name='renamed while open';
      const t=PM.tabs.find(t=>t.id===PM.active);if(t)t.name=song.name;
      const before=document.getElementById('homeBody').textContent;
      /* one more pattern, then save — the panel should redraw itself off the save alone */
      const pats=song.patterns.length;
      newPattern();
      flushSave();
      await new Promise(r=>setTimeout(r,500));   /* homeSoon debounce is 250ms */
      const after=document.getElementById('homeBody').textContent;
      return{before,after,pats,now:song.patterns.length};
    });
    S.ok('a save redraws the panel without reopening it',
      r.after.includes(r.now+' pat')&&!r.before.includes(r.now+' pat'),
      'went from '+r.pats+' to '+r.now+' patterns');
    S.ok('  and it picks up the new name',/renamed while open/.test(r.after));

    S.head('closed projects come BACK — never replaced');
    /* The one unforgivable failure would be a click here quietly overwriting a song, so this
       checks the reopened project is the same object, and that the one you were in survives. */
    r=await page.evaluate(async()=>{
      /* park a second project in storage with no tab, the way a stale key would look */
      const orphan='p_test_orphan';
      const parked=blankSong();
      parked.name='parked song';
      parked.bpm=123;
      parked.patterns[0].data[0][0]={n:60,i:0,v:64,c:null,x:0};
      localStorage.setItem('volt.proj.'+orphan,JSON.stringify(parked));
      const mineBefore=song.name, myTab=PM.active;
      renderHome();
      const row=document.getElementById('homeBody').querySelector('[data-hproj="'+orphan+'"]');
      const listedAsClosed=!!row&&/closed/i.test(row.textContent);
      const namedInList=!!row&&/parked song/.test(row.textContent);
      row.click();
      await new Promise(r=>setTimeout(r,400));
      const out={listedAsClosed,namedInList,mineBefore,
        nowName:song.name,nowBpm:song.bpm,
        note:song.patterns[0].data[0][0]&&song.patterns[0].data[0][0].n,
        tabs:PM.tabs.length,
        oldStillThere:PM.tabs.some(t=>t.id===myTab),
        oldStillInStorage:!!localStorage.getItem('volt.proj.'+myTab)};
      return out;
    });
    S.ok('a project with no tab is listed as closed',r.listedAsClosed);
    S.ok('  by its real name, without parsing the whole song',r.namedInList);
    S.ok('clicking it opens THAT song',r.nowName==='parked song'&&r.nowBpm===123,
      r.nowName+' @ '+r.nowBpm+' bpm');
    S.ok('  with its notes intact',r.note===60,'note '+r.note);
    S.ok('  it becomes a tab instead of replacing one',r.tabs>=2,r.tabs+' tabs');
    S.ok('  and the project you were in is untouched',r.oldStillThere&&r.oldStillInStorage);

    S.head('sizes and storage are reported, not invented');
    r=await page.evaluate(()=>({
      /* one decimal until it's big enough not to need one — 5.0 MB, but 12 MB */
      b:[hBytes(0),hBytes(900),hBytes(2048),hBytes(5242880),hBytes(12582912)],
      ago:[hAgo(0),hAgo(Date.now()-30000),hAgo(Date.now()-7200000)],
      ls:lsTotal(),
      bar:!!document.getElementById('homeBody').querySelector('.hbar i'),
      keys:projKeys().length,
    }));
    S.ck('bytes read like sizes',r.b,['0 B','900 B','2.0 KB','5.0 MB','12 MB']);
    S.ck('times read like times',r.ago,['','just now','2h ago']);
    S.ok('storage total is real',r.ls>0,hFmt(r.ls));
    S.ok('the storage bar is drawn',r.bar);
    S.ok('it sees every project key',r.keys>=2,r.keys+' keys');

    S.head('the samples section reports the store');
    r=await page.evaluate(async()=>{
      await homeScanSamples();
      return{s:HOME.samples,text:document.getElementById('homeBody').textContent};
    });
    S.ok('it reads the sample store without throwing',r.s&&typeof r.s.n==='number',
      r.s?r.s.n+' sample(s), '+r.s.dead+' unused':'null');
    S.ok('  and says so in the panel',/Samples/.test(r.text));

    S.head('folding it away stops all of its work');
    r=await page.evaluate(async()=>{
      toggleHome();                                   /* no argument = fold it back up */
      const p=document.getElementById('homeBody');
      const folded=!homeOpen();
      const lit=document.getElementById('homeBtn').classList.contains('on');
      /* a save while folded must not redraw anything */
      const before=p.innerHTML;
      flushSave();
      await new Promise(r=>setTimeout(r,400));
      return{folded,lit,unchanged:p.innerHTML===before,ticking:HOME.tick!=null,
             cls:document.querySelector('#side .sec-home').className};
    });
    S.ok('it folds and the button unlights',r.folded&&!r.lit,r.cls);
    S.ok('a save while folded does no work',r.unchanged);
    S.ok('the refresh timer is cancelled',!r.ticking);

    S.head('it only ever sees THIS build’s projects');
    /* Stable, beta and beta2 share one origin, so every build's projects sit in the same
       localStorage separated only by a key prefix. The first version of projKeys() matched
       loosely and listed the other builds' songs as phantom 0-byte projects that could never be
       opened. Plant a foreign key and make sure nothing reaches it. */
    r=await page.evaluate(()=>{
      /* Only keys that DON'T begin "volt." can be planted from in here — the shim owns
         Storage.prototype.setItem, so writing "volt.proj.x" would just create a real project of
         our own. Stable's key shape is covered by the prefix check below instead. */
      const foreign={
        'voltbeta.volt.proj.p_from_beta':'{"name":"SONG FROM ANOTHER BUILD","bpm":140}',
        'voltbeta99.volt.proj.p_future':'{"name":"SONG FROM A FUTURE BUILD","bpm":100}',
      };
      for(const k in foreign)localStorage.setItem(k,foreign[k]);
      const planted=[];
      for(let i=0;i<localStorage.length;i++){const k=localStorage.key(i);if(foreign[k])planted.push(k)}
      const mine=projKeys();
      renderHome();
      const listed=document.getElementById('homeBody').textContent;
      const pre=projPrefix();
      const out={
        planted,                                  /* prove they really landed raw */
        raws:mine.map(k=>k.raw),
        leakedName:/ANOTHER BUILD|FUTURE BUILD/.test(listed),
        pre,
        /* the shapes the other two builds write, neither of which may match our prefix */
        stableWouldMatch:'volt.proj.abc'.indexOf(pre)===0,
        betaWouldMatch:'voltbeta.volt.proj.abc'.indexOf(pre)===0,
        counted:lsTotal()>0,                      /* shared quota SHOULD include them */
      };
      for(const k in foreign)localStorage.removeItem(k);
      return out;
    });
    S.ck('the foreign keys really were planted raw',r.planted.length,2);
    S.ok('a project saved by another build is not listed',!r.leakedName);
    S.ok('  and projKeys() does not return it',!r.raws.some(k=>/^voltbeta\.|^voltbeta99\./.test(k)),
      r.raws.join(' , ')||'none');
    S.ok('  every key it does return is this build’s',r.raws.every(k=>k.indexOf('voltbeta2.volt.proj.')===0),
      r.raws.length+' project(s)');
    S.ck('the prefix it matches on is this build’s alone',r.pre,'voltbeta2.volt.proj.');
    S.ok('  stable’s key shape cannot match it',!r.stableWouldMatch);
    S.ok('  beta’s key shape cannot match it',!r.betaWouldMatch);
    S.ok('storage pressure still counts the whole origin',r.counted);

    S.head('its hotkey does not stand on anything else');
    /* Alt+Z was picked because it is the only free letter left. This check exists because the
       first attempt used Alt+J and silently ate the automation lane's shape tool — the panel
       opened and the tool just stopped working, with nothing to notice. */
    r=await page.evaluate(async()=>{
      const fire=code=>{const e=new KeyboardEvent('keydown',{code,altKey:true,bubbles:true,cancelable:true});
        document.dispatchEvent(e);return e.defaultPrevented};
      document.querySelector('#side .sec-home').classList.add('closed');homeSync();
      const zOpens=(fire('KeyZ'),homeOpen());
      /* every other Alt letter must leave this panel alone */
      document.querySelector('#side .sec-home').classList.add('closed');homeSync();
      const stolen=[];
      for(const c of 'ABCDEFGHIJKLMNOPQRSTUVWXY'){
        fire('Key'+c);
        if(homeOpen()){stolen.push(c);document.querySelector('#side .sec-home').classList.add('closed');homeSync()}
      }
      return{zOpens,stolen};
    });
    S.ok('Alt+Z opens it',r.zOpens);
    S.ok('  and no other Alt letter does',!r.stolen.length,r.stolen.length?'also opened by Alt+'+r.stolen.join(', Alt+'):'A–Y all left it alone');

    S.head('the jump always reveals — it never hides what you asked for');
    r=await page.evaluate(()=>{
      const sec=document.querySelector('#side .sec-home');
      sec.classList.add('closed');
      toggleHome(true);const fromFolded=homeOpen();
      toggleHome(true);const stillOpen=homeOpen();   /* asking again must not toggle it shut */
      document.body.classList.add('norail');          /* Alt+L hid the whole rail */
      const hiddenByRail=homeOpen();
      toggleHome(true);
      const railBack=!document.body.classList.contains('norail')&&homeOpen();
      return{fromFolded,stillOpen,hiddenByRail,railBack};
    });
    S.ok('the jump opens it when folded',r.fromFolded);
    S.ok('  and asking twice leaves it open',r.stillOpen);
    S.ok('a rail hidden by Alt+L counts as not open',!r.hiddenByRail);
    S.ok('  and the jump brings the rail back with it',r.railBack);

    S.head('folded state survives a reload, and keys off the section not its position');
    r=await page.evaluate(()=>{
      const sec=document.querySelector('#side .sec-home');
      sec.classList.add('closed');secsSave();
      const saved=JSON.parse(localStorage.getItem('volt.secs2')||'{}');
      /* the old format keyed by index — inserting Projects at the top would have shifted it */
      return{saved,byName:Object.keys(saved).every(k=>/^sec-/.test(k)),home:saved['sec-home']};
    });
    S.ok('state is stored per section, by name',r.byName,Object.keys(r.saved).join(', '));
    S.ck('  and remembers this one is folded',r.home,1);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};

/* tiny local echo of the panel's own formatter, so the note text reads the same */
function hFmt(n){return n<1048576?(n/1024).toFixed(0)+' KB':(n/1048576).toFixed(1)+' MB'}
