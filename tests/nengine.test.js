/* The bridge from a VOLT pattern to the native engine.
 *
 * `nengPattern()` is the whole JS half of step 4: it turns the current pattern into the small
 * shape `native/desktop/src/engine.rs` deserialises. The Rust side is covered by its own tests and
 * by `live`, but the conversion is where the mistakes that matter live — an off-by-one on velocity
 * makes everything quiet, a missed note-OFF check makes the engine try to play note -1, and
 * silently dropping an unported instrument means Mickey hears a pattern with holes in it and has
 * no idea why.
 *
 * This runs in the browser, where `TAURI` is null and the palette entries are hidden — which is
 * itself worth asserting, because a desktop-only command that shows up in the web build is a
 * command that can only fail. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('native engine bridge');
  const{browser,page,errs}=await open();
  try{
    S.head('it converts a pattern into what the Rust engine accepts');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.bpm=140;song.lpb=4;
      /* Two ported instruments and one that is not, so the filter has something to do.
         Built exactly as the app builds them — knobs live in `.params` (mkInst merges the
         override there). The old version of this test hand-planted a `.p` field, which is also
         what nengPattern read — so the suite passed for weeks while the engine refused every
         native note in a real song for want of its parameters. Instruments here must be shaped
         like the app's or the test proves nothing about the app. */
      song.instruments=[
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
        mkInst('hat','H',{decay:.06,tone:.7}),
        mkInst('supersaw','S'),
      ];
      /* every real instrument is ported since 2026-08-14 — the filter under test is the
         MECHANISM, so this block plays a future unported type by pulling supersaw back out
         of the list for the second conversion below */
      const d=song.patterns[curPat].data;
      d[0][0]={n:48,i:0,v:64};       /* kick, full velocity — the column is 0..64 */
      d[0][4]={n:-1,i:0,v:64};       /* note-OFF — must not become a note */
      d[1][2]={n:60,i:1,v:32};       /* hat at half velocity */
      d[1][6]={n:60,i:1,c:'T',x:64}; /* hat with Txx and a BLANK velocity — the .8 default */
      d[2][0]={n:48,i:2,v:64};       /* supersaw — ported since 2026-08-14, crosses too */
      d[3][3]={n:60,i:1,v:48,d:1};   /* disabled — a hole, NOT a note-off */
      NENG.only=true;                /* ported cells cross for channels the ENGINE speaks for */
      const p=nengPattern();
      /* the unported MECHANISM, tested by making supersaw unported again for one conversion */
      const at=NENG_OK.indexOf('supersaw');NENG_OK.splice(at,1);
      const p2=nengPattern();
      NENG_OK.splice(at,0,'supersaw');
      NENG.only=false;
      const notes=p.cells.filter(c=>c.note>=0);
      return{p2instrs:p2.cells.filter(c=>c.note>=0).map(c=>c.instr),
             p2ducks:p2.ducks.length,
             bpm:p.bpm,lpb:p.lpb,rows:p.rows,n:p.cells.length,
             instrs:notes.map(c=>c.instr).sort(),
             notes:notes.map(c=>c.note),
             vels:notes.map(c=>+c.vel.toFixed(4)),
             txs:notes.map(c=>c.tx),
             rowsOf:notes.map(c=>c.row).sort((a,b)=>a-b),
             kickParams:(p.cells.find(c=>c.instr==='kick')||{}).params,
             offs:p.cells.filter(c=>c.note<0).map(c=>({row:c.row,track:c.track})),
             tracks:notes.map(c=>c.track),
             disabled:p.cells.filter(c=>c.row===3&&c.track===3).length,
             repeats:p.repeats};
    });
    S.ck('tempo comes from the song',r.bpm,140);
    S.ck('  and rows per beat with it',r.lpb,4);
    S.ok('every ported instrument crosses — all fourteen are, since 2026-08-14',
         r.instrs.join(',')==='hat,hat,kick,supersaw',r.instrs.join(','));
    S.ok('an unported type is left out, not sent as silence — the filter still guards the future',
         r.p2instrs.indexOf('supersaw')<0,r.p2instrs.join(','));
    S.ok('  but its duck trigger still crosses — unplayable must still pump the mix',
         r.p2ducks===4,r.p2ducks+' ducks (want 4: every note, offs excluded)');
    S.ck('a note-OFF crosses as its own event',r.n,5);
    S.ck('  on the lane it has to release, not as a note',r.offs,[{row:4,track:0}]);
    S.ok('  so no instrument note is negative',r.notes.every(n=>n>=0),JSON.stringify(r.notes));
    S.ok('every note names its lane, so the engine knows what to cut',
         r.tracks.join(',')==='0,1,1,2',r.tracks.join(','));
    S.ck('a disabled cell is a hole, not an off',r.disabled,0);
    /* the velocity column is 0..64 with 64 = full, exactly as schedRow reads it — the engine's
       voice REPLACES the browser's, so the two readings must be the same number */
    S.ok('velocity is the tracker\'s 0..64 column -> 0..1',
         r.vels.indexOf(1)>=0&&r.vels.indexOf(0.5)>=0,JSON.stringify(r.vels));
    S.ok('  and a blank velocity is .8, as the browser plays it',r.vels.indexOf(0.8)>=0,JSON.stringify(r.vels));
    S.ok('Txx crosses as 192nds for the clock to offset',r.txs.indexOf(64)>=0,JSON.stringify(r.txs));
    S.ok('  and notes without it send 0',r.txs.filter(x=>x===0).length===3,JSON.stringify(r.txs));
    S.ok('rows are preserved',r.rowsOf.join(',')==='0,0,2,6',r.rowsOf.join(','));
    S.ok('instrument parameters travel with the note — from .params, where the app keeps them',
         !!r.kickParams&&r.kickParams.pitch===50&&r.kickParams.punch===0.6,JSON.stringify(r.kickParams));

    S.head('the voice commands ride the cells');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('reese','R',{voices:2,detune:40,cutoff:150,res:0,lfoRate:.35,lfoAmt:.35,drive:.5,sub:0,rel:.09})];
      const d=song.patterns[curPat].data;
      d[0][0]={n:24,i:0,v:64};
      d[0][2]={n:36,i:0,v:64,c:'G',x:8};   /* glide, half a row */
      d[0][4]={n:36,i:0,v:64,c:'C',x:200}; /* filter cut */
      d[0][6]={n:36,i:0,v:64,c:'U',x:2};   /* slide up */
      d[0][8]={n:36,i:0,v:64,c:'T',x:96};  /* Txx must stay TIMING, not become a voice command */
      NENG.only=true;
      const cells=nengPattern().cells.filter(c=>c.note>=0).sort((a,b)=>a.row-b.row);
      NENG.only=false;
      return cells.map(c=>({row:c.row,cmd:c.cmd,cmdv:c.cmdv,tx:c.tx}));
    });
    S.ok('a plain note carries no command',r[0].cmd===''&&r[0].cmdv===0,JSON.stringify(r[0]));
    S.ok('Gxx crosses with its speed',r[1].cmd==='G'&&r[1].cmdv===8,JSON.stringify(r[1]));
    S.ok('Cxx crosses with its cut value',r[2].cmd==='C'&&r[2].cmdv===200,JSON.stringify(r[2]));
    S.ok('Uxx crosses with its depth',r[3].cmd==='U'&&r[3].cmdv===2,JSON.stringify(r[3]));
    S.ok('Txx stays timing: tx set, cmd empty',r[4].tx===96&&r[4].cmd==='',JSON.stringify(r[4]));

    S.head('which channels cross at all — nengWantsTrack');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      const count=()=>nengPattern().cells.filter(c=>c.note>=0).length;
      const plain=count();                               /* webview speaks for this channel */
      song.trackset[0].fxVsts=[{path:'C:/x.vst3',index:0,name:'X'}];
      const withFx=count();                              /* engine holds the only wet copy */
      song.trackset[0].fxVsts=[];
      NENG.only=true;
      const only=count();                                /* engine IS the speakers */
      const onlySong=nengPattern(null,true).cells.filter(c=>c.note>=0).length; /* song mode too, since 2026-08-14 */
      const ducksAnyway=nengPattern().ducks.length;
      NENG.only=false;
      return{plain,withFx,only,onlySong,ducksAnyway};
    });
    /* the doubling rule: a ported cell crosses ONLY when the engine's copy is the one being
       heard — otherwise the webview plays it and the engine playing it too is a flam/chorus */
    S.ck('a plain channel does NOT cross — the webview plays it',r.plain,0);
    S.ck('a channel with a VST effect crosses — the engine owns the wet copy',r.withFx,1);
    S.ck('ASIO-only crosses everything ported (pattern mode)',r.only,1);
    S.ck('  and in SONG mode too — all fourteen are ported, the sweep has no holes (2026-08-14)',r.onlySong,1);
    S.ck('duck triggers cross regardless — a trigger needs no voice',r.ducksAnyway,1);

    S.head('an empty pattern asks the engine to do nothing');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      return nengPattern().cells.length;
    });
    S.ck('no cells, so the command can say so instead of playing silence',r,0);

    S.head('the desktop-only commands stay out of the browser');
    r=await page.evaluate(()=>{
      const desk=COMMANDS.filter(c=>c.desk).map(c=>c.n);
      /* palRender() applies the filter; in a browser TAURI is null so none should survive */
      palOpen();
      const shown=palItems.filter(c=>c.desk).length;
      palClose();
      return{tauri:TAURI,defined:desk.length,shown};
    });
    S.ck('TAURI is null in a browser tab',r.tauri,null);
    S.ok('there are desktop-only commands to hide',r.defined>=6,r.defined+' of them');
    S.ck('  and none of them is offered here',r.shown,0);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}

  /* ---- the click path ------------------------------------------------------------------------
     Everything above tests the conversion. This tests the thing that actually breaks: what the
     palette commands SEND. A stubbed `window.__TAURI__` installed before the page loads makes the
     desktop branches live, so the commands can be run for real and their payloads inspected —
     which is as close to clicking the buttons as anything headless gets, and it catches the whole
     class of bug that matters here (wrong argument name, a field Rust cannot deserialise, a
     sample uploaded once per note instead of once). */
  const S2=suite('native engine · what the buttons send');
  const PRE=`
    window.__TAURI__={core:{invoke:async(cmd,args)=>{
      (window.__calls=window.__calls||[]).push({cmd,args});
      if(cmd==='eng_devices')return[
        {id:'w1',label:'Line Out (nio 2|4) via USB [WASAPI]',host:'WASAPI'},
        {id:'a1',label:'ASIO4ALL v2 [ASIO]',host:'ASIO'}];
      if(cmd==='eng_open')return{label:'ASIO4ALL v2 [ASIO]',sample_rate:44100,latency_ms:5.8};
      if(cmd==='eng_load_sample')return (args.pcm||[]).length;
      if(cmd==='eng_load_sample_raw'){
        /* raw body: [u32 len][json {id,channels}][pad to 4][f32le interleaved] — unpack it so
           the assertions below still read id/channels/pcm length off the call record */
        const u8=args instanceof Uint8Array?args:new Uint8Array(args);
        const dv=new DataView(u8.buffer,u8.byteOffset,u8.byteLength);
        const jlen=dv.getUint32(0,true),off=(4+jlen+3)&~3;
        const meta=JSON.parse(new TextDecoder().decode(u8.subarray(4,4+jlen)));
        const n=(u8.byteLength-off)/4;
        window.__calls[window.__calls.length-1]={cmd,args:{id:meta.id,channels:meta.channels,pcm:{length:n}}};
        return n/(meta.channels||1);
      }
      /* The scan's shape as of the VST2 work: plugins of both formats in one list, plus the
         count of 32-bit VST2s the scanner saw and cannot load — the UI must say so, not bury it. */
      if(cmd==='vst_scan')return{plugins:[
        {name:'Vital',vendor:'Vital Audio',path:'C:/VST3/Vital.vst3',index:0,fmt:'VST3'},
        {name:'Diva',vendor:'u-he',path:'C:/VST3/u-he/Diva.vst3',index:2,fmt:'VST3'},
        {name:'FabFilter One',vendor:'FabFilter',path:'C:/VSTPlugins/FabFilter One.dll',index:0,fmt:'VST2'}],
        skipped32:114};
      if(cmd==='vst_load')return args.index===2?'Diva':(/\.dll$/i.test(args.path)?'FabFilter One':'Vital');
      /* The plugin window. __noEditorSlot stands for a plugin with no editor of its own — a
         normal answer, not a failure, and the UI has to say which it is.
         (No backticks in here: this whole stub is a template literal, and one would end it.) */
      if(cmd==='vst_editor_open'){
        if(args.slot===window.__noEditorSlot)throw new Error('Thing has no window of its own');
        return null;
      }
      if(cmd==='vst_editor_close')return null;
      if(cmd==='eng_panic'){
        if(window.__panicFail)throw new Error('the audio thread is not answering');
        return null;
      }
      if(cmd==='fxvst_load')return 'Pro-Q 3';
      if(cmd==='fxvst_editor_open'||cmd==='fxvst_editor_close'||cmd==='fxvst_unload')return null;
      if(cmd==='vst_param_set'||cmd==='fxvst_param_set')return null;
      if(cmd==='vst_params'||cmd==='fxvst_params')return[
        {id:100,name:'Cutoff'},{id:101,name:'Resonance'}];
      if(cmd==='app_quit')return null; /* recorded like everything else; the page must not die */
      if(cmd==='lib_write')return null;
      if(cmd==='eng_mix'){
        /* kept across the whole block so the sequence numbers can be checked in order */
        (window.__allMix=window.__allMix||[]).push(args&&args.mix&&args.mix.seq);
        return null;
      }
      /* NOT lib_default. Returning a path from it switches on the song-folder mirror, and then
         every autosave serialises the whole song — samples and all — into this call log until
         the page falls over. The open-project test sets NSTORE.dir itself, deliberately. */
      if(cmd==='lib_list')return[
        {name:'older.volt',full:'x/older.volt',size:2048,modified:Date.now()-9e7},
        {name:'newest.volt',full:'x/newest.volt',size:4096,modified:Date.now()-1000}];
      if(cmd==='lib_read')return JSON.stringify(window.__songOnDisk);
      return null;
    }},event:{listen:async(name,cb)=>{(window.__evs=window.__evs||{})[name]=cb;return()=>{}}},
    dialog:{open:async()=>null}};
    /* the device picker is a LIST now, not a prompt() — the tests press Enter on it, which takes
       the highlighted row, which must be the ASIO one */
  `;
  const o2=await open({pre:PRE});
  try{
    const page=o2.page;
    let r=await page.evaluate(async()=>{
      const run=n=>COMMANDS.find(c=>c.n===n).f();
      await loadSong(blankSong());
      /* A sampler with real audio, and a kick, so both paths are exercised.
         The buffer comes from an OfflineAudioContext because the page's own `AC` does not exist
         until something asks for audio — and a sampler with no buffer is deliberately skipped by
         nengPattern(), so a null here would silently test nothing. */
      const b=new OfflineAudioContext(1,1000,44100).createBuffer(1,1000,44100);
      song.instruments=[
        Object.assign(mkInst('sampler','S'),{buffer:b,_shaSrc:'hash-abc',s0:.1,s1:.9,pad:0}),
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
        /* a SLICED instrument, because its PCM never crossed for the feature's whole life: the
           upload filtered on instr==='sampler', the engine refused every slice note ("sliced has
           no sample loaded"), and the channel went silent whenever the engine owned it — which
           is what "i put an fx over the channel and audio is not processing" was (2026-08-15) */
        Object.assign(mkInst('sliced','SL'),{buffer:b,_shaSrc:'hash-sliced',sliceBase:48,
          slices:[{s0:0,s1:.5},{s0:.5,s1:1}]}),
      ];
      song.patterns[curPat].data[0][0]={n:60,i:0,v:127};
      song.patterns[curPat].data[0][2]={n:60,i:0,v:80,c:'S',x:128};
      song.patterns[curPat].data[1][4]={n:48,i:1,v:127};
      song.patterns[curPat].data[2][6]={n:49,i:2,v:127};   /* note 49, base 48 → slice 1 */
      window.__calls=[];
      const dev=run('Native engine: choose output device…');
      await new Promise(r=>setTimeout(r,80));
      /* row 0 is "Windows default output" now — the no-homework option leads. One step down is
         the first real device, which the sort puts ASIO first. */
      const topRow=(document.querySelector('.pickRow.on')||{}).textContent||'';
      const inp=document.getElementById('dlgInput');
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true}));
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await dev;
      window.__topRow=topRow;
      await run('Native engine: play current pattern');
      await new Promise(r=>setTimeout(r,80));
      await run('Native engine: play current pattern');   /* twice: sample must NOT resend */
      await new Promise(r=>setTimeout(r,80));
      await run('Native engine: stop');
      const calls=window.__calls;
      const play=calls.filter(c=>c.cmd==='eng_play');
      return{
        tauri:!!TAURI,
        topRow:window.__topRow,
        /* `vst_scan` is dropped: the plugin list warms itself in the background at startup, so it
           can land anywhere in this log without meaning anything. `eng_mix` likewise — opening a
           device pushes the song's faders at it, which is correct and says nothing about ordering.
           What this assertion is about is the ORDER OF THE CLICKS — device, then open, then sample,
           then play. */
        order:calls.map(c=>c.cmd).filter(c=>c!=='vst_scan'&&c!=='eng_mix'),
        opened:(calls.find(c=>c.cmd==='eng_open')||{}).args,
        loads:calls.filter(c=>c.cmd==='eng_load_sample_raw').length,
        loadArgs:(calls.find(c=>c.cmd==='eng_load_sample_raw')||{}).args&&
                 (({id,channels})=>({id,channels}))(calls.find(c=>c.cmd==='eng_load_sample_raw').args),
        pcmLen:((calls.find(c=>c.cmd==='eng_load_sample_raw')||{args:{}}).args.pcm||[]).length,
        slicedLoad:calls.some(c=>c.cmd==='eng_load_sample_raw'&&c.args.id==='hash-sliced'),
        pattern:play.length?play[0].args.pattern:null,
      };
    });
    S2.ok('the desktop branch is live under a Tauri stub',r.tauri===true);
    S2.ok('the commands run in order',
      r.order.join(',')==='eng_devices,eng_open,eng_load_sample_raw,eng_load_sample_raw,eng_play,eng_play,eng_stop',
      r.order.join(','));
    S2.ok('the no-setup "Windows default output" leads the list',
      /Windows default output/.test(r.topRow||''),r.topRow);
    S2.ok('  and one step down is ASIO — the sort holds',!!r.opened&&r.opened.deviceId==='a1',
      JSON.stringify(r.opened));
    S2.ck('each sample is uploaded once, not per note and not per play',r.loads,2);
    S2.ok('  keyed by its hash, so shared audio is sent once',
      !!r.loadArgs&&r.loadArgs.id==='hash-abc',JSON.stringify(r.loadArgs));
    S2.ok('  as interleaved f32 frames',r.pcmLen===1000&&r.loadArgs.channels===1,
      r.pcmLen+' samples, '+(r.loadArgs||{}).channels+' ch');
    S2.ok('the pattern carries tempo and rows',
      !!r.pattern&&r.pattern.bpm>0&&r.pattern.rows>0,JSON.stringify(r.pattern&&{bpm:r.pattern.bpm,rows:r.pattern.rows}));
    S2.ck('  four notes: two sampler, one kick, one slice',r.pattern?r.pattern.cells.length:0,4);
    S2.ok('  sampler notes name their sample',
      r.pattern&&r.pattern.cells.filter(c=>c.sample==='hash-abc').length===2,
      JSON.stringify(r.pattern&&r.pattern.cells.map(c=>c.sample)));
    S2.ok('  the sliced instrument’s audio crosses too — the fx-over-slicer silence of 2026-08-15',
      r.slicedLoad===true&&r.pattern&&r.pattern.cells.some(c=>c.sample==='hash-sliced'),
      'load:'+r.slicedLoad+' cells:'+JSON.stringify(r.pattern&&r.pattern.cells.map(c=>c.sample)));
    S2.ok('  and the slice this note picks travels flat',
      (c=>!!c&&c.params.sliceCount===2&&c.params.sliceBase===48&&c.params.slice1s===0.5&&c.params.slice1e===1)
      (r.pattern&&r.pattern.cells.find(c=>c.sample==='hash-sliced')),
      JSON.stringify(r.pattern&&(r.pattern.cells.find(c=>c.sample==='hash-sliced')||{}).params));
    S2.ok('  trim travels as params, not as instrument fields',
      r.pattern&&r.pattern.cells[0].params.s0===0.1&&r.pattern.cells[0].params.s1===0.9,
      JSON.stringify(r.pattern&&r.pattern.cells[0].params));
    S2.ok('  Sxx becomes a 0..1 offset',
      r.pattern&&Math.abs(r.pattern.cells[1].params.off-128/255)<1e-6,
      JSON.stringify(r.pattern&&r.pattern.cells[1].params.off));
    S2.ok('  srcInst is stripped — Rust would reject it',
      r.pattern&&r.pattern.cells.every(c=>c.srcInst===undefined),
      JSON.stringify(r.pattern&&r.pattern.cells[0]));
    /* ---- VST instruments --------------------------------------------------------------------
       A plugin is the first thing VOLT plays that it did not write, and the wire to it is all
       payload: the slot number has to be the instrument index on both sides, the cell has to say
       `vst` rather than the VOLT instrument sitting underneath it, and Play has to load the plugin
       before it sends notes or the engine drops every one of them. None of that is visible by ear
       until it is wrong, so it is asserted here. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
        mkInst('sub','B',{harm:.15,rel:.06}),
      ];
      /* Instrument 0 (a kick) is SELECTED when the plugin is picked — the case that used to
         silently replace it. The plugin must land on a new slot and leave the kick alone. */
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      NENG.open=true;curInst=0;
      const before=song.instruments.length;
      window.__calls=[];

      const pick=COMMANDS.find(c=>c.n==='Add a VST instrument…').f();
      await new Promise(r=>setTimeout(r,80));
      /* the picker is the same list dialog the device chooser uses; Enter takes the highlighted
         row, and row 0 is the "no plugin" entry, so one step down is the first real plugin */
      const inp=document.getElementById('dlgInput');
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true}));
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await pick;

      const loadCall=window.__calls.find(c=>c.cmd==='vst_load');
      /* Adding a plugin should show it — see the assertions below. */
      await new Promise(r=>setTimeout(r,40));
      const openCall=window.__calls.find(c=>c.cmd==='vst_editor_open');
      /* Read straight after picking: the block below deliberately drives the transport flag around
         to test replay, so the value it has at the END says nothing about what picking did. */
      const transportOn=NENG.transport;
      const newSlot=song.instruments.length-1;
      const assigned=song.instruments[newSlot].vst;
      const kickUntouched=!song.instruments[0].vst&&song.instruments[0].type==='kick';
      /* now play the plugin: notes on a track pointing at the NEW instrument.
         v:32 is half velocity on the 0..64 column. NENG.only so the kick's cell crosses too —
         a plain channel's ported cells stay with the webview now (nengWantsTrack). */
      song.patterns[curPat].data[1][0]={n:60,i:newSlot,v:32};
      song.patterns[curPat].data[1][4]={n:-1,i:newSlot};
      NENG.only=true;
      const cells=nengPattern().cells;
      NENG.only=false;

      /* And now the same song played through the transport hook, with the engine's memory of the
         plugin cleared — which is what reopening a song, or switching audio device, actually looks
         like. The plugin must be reloaded before any notes are sent to it. */
      NENG_VST.clear();
      NENG.transport=true;window.__calls=[];
      play(false);await new Promise(r=>setTimeout(r,220));
      const order=window.__calls.map(c=>c.cmd);
      const sent=(window.__calls.find(c=>c.cmd==='eng_play')||{args:{}}).args.pattern;
      stop();await new Promise(r=>setTimeout(r,60));
      NENG.transport=false;curInst=0;

      return{
        loadArgs:loadCall&&loadCall.args,
        openedOnAdd:!!openCall,openedSlot:openCall&&openCall.args.slot,
        madeNewSlot:song.instruments.length===before+1,
        newSlot,kickUntouched,transportOn,
        assigned,
        vstCell:cells.find(c=>c.instr==='vst'),
        voltCell:cells.find(c=>c.instr==='kick'),
        offCell:cells.find(c=>c.note===-1),
        loadedBeforePlay:order.indexOf('vst_load')>=0&&order.indexOf('vst_load')<order.indexOf('eng_play'),
        sentVst:sent&&sent.cells.filter(c=>c.instr==='vst').length,
      };
    });
    S2.head('a VST instrument on a slot');
    /* The complaint this came from, in one line: "i added vst - diva it added it over 00 kicks".
       Adding an instrument must add an instrument. */
    S2.ok('picking a plugin ADDS an instrument rather than replacing the selected one',
          r.madeNewSlot===true&&r.kickUntouched===true);
    S2.ck('the slot it loads into IS the new instrument index',r.loadArgs&&r.loadArgs.slot,r.newSlot);
    /* And it turns the native engine on, because a plugin that is not followed makes no sound and
       the fix would otherwise be a second command in a menu nobody opens. */
    S2.ok('  and it switches the native transport on, so the plugin is actually audible',
          r.transportOn===true);
    S2.ok('  with the plugin path and class index',
          !!r.loadArgs&&r.loadArgs.path==='C:/VST3/Vital.vst3'&&r.loadArgs.index===0,
          JSON.stringify(r.loadArgs));
    S2.ok('  and it is remembered on the instrument, so the song carries it',
          !!r.assigned&&r.assigned.index===0&&r.assigned.name==='Vital',JSON.stringify(r.assigned));
    S2.ck('the note is sent as `vst`, not as the VOLT instrument underneath',
          r.vstCell&&r.vstCell.instr,'vst');
    S2.ck('  pointing at the same slot',r.vstCell&&r.vstCell.vst,r.newSlot);
    S2.ok('  carrying no ITYPES params — a plugin sound is its own state',
          !!r.vstCell&&Object.keys(r.vstCell.params||{}).length===0);
    S2.ok('  and velocity is the same 0..64 column every note uses',!!r.vstCell&&Math.abs(r.vstCell.vel-0.5)<1e-6,
          r.vstCell&&r.vstCell.vel);
    S2.ok('instruments without a plugin are untouched',!!r.voltCell&&r.voltCell.instr==='kick');
    S2.ok('a note-OFF still goes across with no instrument',!!r.offCell&&r.offCell.instr==='');
    /* The ordering bug this catches is total: notes sent to a slot with no plugin in it are not
       quietly late, they are silently dropped, and the whole track disappears. */
    S2.ok('Play loads the plugin BEFORE sending the pattern',r.loadedBeforePlay===true);
    S2.ck('  and the pattern reaches the engine with its plugin note',r.sentVst,1);
    /* The bug this pins was invisible for a day: the plugin window worked, and nothing ever asked
       for it. The log showed `vst_editor_open` called ZERO times across three sessions of loading
       plugins, because the only way to reach it was a button in a panel nobody had reason to open.
       Adding a plugin now shows it, exactly as every other DAW does. */
    S2.ok('adding a plugin opens its window without being asked',r.openedOnAdd===true,
          'vst_editor_open after add: '+r.openedOnAdd);
    S2.ck('  for the slot the plugin actually landed on',r.openedSlot,r.newSlot);

    /* ---- what a plugin on a slot LOOKS like ---------------------------------------------------
       Assigning a plugin used to change nothing visible: a status line that scrolled away, and an
       instrument list and parameter panel that carried on as if the slot were still VOLT's own —
       so the panel showed knobs that could not affect the sound. These assert the two places a
       user actually looks. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[
        mkInst('sub','Bass',{harm:.15,rel:.06}),
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
      ];
      curInst=0;renderInstList();renderParams();
      const plainRow=document.querySelector('#instList .irow').innerHTML;
      const plainKnobs=document.querySelectorAll('#params input[type=range]').length;

      song.instruments[0].vst={path:'C:/VST3/Diva.vst3',index:2,name:'Diva'};
      renderInstList();renderParams();
      const row=document.querySelector('#instList .irow');
      const badge=row.querySelector('.vstb');
      /* clicking the badge must open the plugin's window, not rename the instrument */
      window.__calls=[];
      badge.click();
      await new Promise(r=>setTimeout(r,40));
      const openedFromBadge=!!window.__calls.find(c=>c.cmd==='vst_editor_open');
      const panel=document.querySelector('#params .vstpan');
      const knobs=document.querySelectorAll('#params input[type=range]').length;
      const title=document.getElementById('parTitle').textContent;

      /* and taking it away puts the instrument back */
      const off=document.getElementById('vstOff');
      const hadRemove=!!off;
      if(off)off.click();
      await new Promise(r=>setTimeout(r,30));
      const backKnobs=document.querySelectorAll('#params input[type=range]').length;
      const backBadge=!!document.querySelector('#instList .vstb');
      return{plainHadBadge:/vstb/.test(plainRow),plainKnobs,openedFromBadge,
             badgeText:badge&&badge.textContent,badgeTitle:badge&&badge.getAttribute('title'),
             hasPanel:!!panel,panelText:panel?panel.textContent:'',knobs,title,
             hadRemove,backKnobs,backBadge,stillAssigned:!!song.instruments[0].vst};
    });
    S2.head('a plugin on a slot is visible, not just remembered');
    S2.ok('a plain instrument has no plugin badge',r.plainHadBadge===false);
    S2.ck('the instrument list shows the plugin name',r.badgeText,'Diva');
    S2.ok('  with the full plugin in the tooltip',/VST3: Diva/.test(r.badgeTitle||''),r.badgeTitle);
    /* And it is the second way in: the badge is the only thing on screen already saying "this is a
       plugin", so pressing it opens the plugin — not renames the instrument. */
    S2.ok('  and clicking the badge opens the plugin window',r.openedFromBadge===true);
    S2.ok('the params panel is replaced, not decorated',r.hasPanel===true);
    /* The important one: VOLT's knobs cannot be left on screen under a plugin. They would do
       nothing at all, and a control that does nothing is worse than no control. */
    S2.ck('  and the instrument knobs are gone',r.knobs,0);
    S2.ok('  (the same instrument shows '+r.plainKnobs+' knobs without a plugin)',r.plainKnobs>0);
    S2.ok('the panel says where it plays and what is still missing',
          /native engine/i.test(r.panelText)&&/not saved into the song/i.test(r.panelText),
          r.panelText);
    S2.ok('the title says VST3 rather than the VOLT instrument type',/\[VST3\]/.test(r.title),r.title);
    S2.ok('there is a way to remove it from the panel',r.hadRemove===true);
    S2.ok('  and removing it brings the instrument back',
          r.stillAssigned===false&&r.backBadge===false&&r.backKnobs>0,
          'knobs back: '+r.backKnobs);

    /* ---- VST2, next to the VST3s, indistinguishable --------------------------------------------
       The ship point of the VST2 work, asserted: a 64-bit VST2 DLL sits in the same picker, loads
       through the same command with just a path, and the format appears only as a label. The
       32-bit count the scanner set aside must be SAID — a picker that silently lists 232 of 346
       plugins reads as broken, not as honest. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      NENG.open=true;curInst=0;window.__calls=[];
      const pick=COMMANDS.find(c=>c.n==='Add a VST instrument…').f();
      await new Promise(r=>setTimeout(r,80));
      const msg=document.getElementById('dlgMsg').textContent;
      /* filter down to the VST2 — the sub-line carries the format, so this also proves a user can
         type "vst2" and see only that format */
      const inp=document.getElementById('dlgInput');
      inp.value='fabfilter';inp.dispatchEvent(new Event('input',{bubbles:true}));
      await new Promise(r=>setTimeout(r,30));
      const rowSub=(document.querySelector('.pickRow.on .pickSub')||{}).textContent||'';
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await pick;
      const loadCall=window.__calls.find(c=>c.cmd==='vst_load');
      const slot=song.instruments.length-1;
      const assigned=song.instruments[slot].vst;
      curInst=slot;renderInstList();renderParams();
      const badge=document.querySelector('#instList .irow:last-child .vstb');
      const title=document.getElementById('parTitle').textContent;
      const panel=document.querySelector('#params .vstpan');
      curInst=0;
      return{msg,rowSub,loadArgs:loadCall&&loadCall.args,assigned,
             badgeTitle:badge&&badge.getAttribute('title'),title,
             panelText:panel?panel.textContent:''};
    });
    S2.head('a VST2 next to the VST3s');
    S2.ok('the picker names the 32-bit plugins it cannot offer',/114 32-bit/.test(r.msg),r.msg);
    S2.ok('the row wears its format in the sub-line',/^VST2 · /.test(r.rowSub),r.rowSub);
    S2.ok('it loads by path alone — the same command, no format parameter',
          !!r.loadArgs&&/FabFilter One\.dll$/.test(r.loadArgs.path)&&r.loadArgs.index===0,
          JSON.stringify(r.loadArgs));
    S2.ok('the song remembers the format with the assignment',
          !!r.assigned&&r.assigned.fmt==='VST2'&&r.assigned.name==='FabFilter One',
          JSON.stringify(r.assigned));
    S2.ok('the badge tooltip says VST2',/^VST2: /.test(r.badgeTitle||''),r.badgeTitle);
    S2.ok('  and so does the params title',/\[VST2\]/.test(r.title),r.title);
    S2.ok('  over the same plugin panel the VST3s get',/native engine/i.test(r.panelText));

    /* ---- the plugin's own window ---------------------------------------------------------------
       The thing that turns a plugin from "makes its default noise" into an instrument: its real
       interface, with its presets. The window itself is a native Windows window made in Rust and
       cannot be exercised headlessly, so what is pinned here is the wire to it — that the button
       exists on a plugin slot, that it asks for the RIGHT slot (a plugin lands on a new
       instrument, so passing curInst blindly would open the wrong plugin's window), and that a
       plugin with no editor reports that plainly instead of looking broken. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
        Object.assign(mkInst('sub','Synth'),{}),
      ];
      song.instruments[1].vst={path:'C:/VST3/u-he/Diva.vst3',index:2,name:'Diva',fmt:'VST3'};
      curInst=1;renderParams();
      const btn=document.getElementById('vstUi');
      const hadBtn=!!btn;
      window.__calls=[];
      if(btn)btn.click();
      await new Promise(r=>setTimeout(r,60));
      const call=window.__calls.find(c=>c.cmd==='vst_editor_open');

      /* a plain instrument must NOT offer the button */
      curInst=0;renderParams();
      const btnOnPlain=!!document.getElementById('vstUi');

      /* the command-palette entry drives the same path */
      curInst=1;window.__calls=[];
      await COMMANDS.find(c=>c.n==='Open the plugin’s window').f();
      await new Promise(r=>setTimeout(r,60));
      const viaPalette=window.__calls.find(c=>c.cmd==='vst_editor_open');

      /* a plugin with no editor of its own — the stub refuses for this slot */
      song.instruments.push(Object.assign(mkInst('sub','NoUI'),
        {vst:{path:'C:/VST3/Thing.vst3',index:0,name:'Thing',fmt:'VST3'}}));
      const noUiSlot=song.instruments.length-1;
      window.__noEditorSlot=noUiSlot;
      window.__calls=[];
      await nengPluginWindow(noUiSlot);
      await new Promise(r=>setTimeout(r,60));
      const refusedMsg=document.getElementById('status').textContent;

      /* and asking on an instrument with no plugin at all */
      window.__calls=[];
      await nengPluginWindow(0);
      await new Promise(r=>setTimeout(r,40));
      const noPluginMsg=document.getElementById('status').textContent;
      const askedForSlot0=!!window.__calls.find(c=>c.cmd==='vst_editor_open');

      curInst=0;song.instruments.length=noUiSlot;
      return{hadBtn,btnOnPlain,slot:call&&call.args.slot,paletteSlot:viaPalette&&viaPalette.args.slot,
             refusedMsg,noPluginMsg,askedForSlot0};
    });
    S2.head('the plugin’s own window');
    S2.ok('a plugin slot offers a button to open it',r.hadBtn===true);
    S2.ok('  and an instrument without a plugin does not',r.btnOnPlain===false);
    /* The bug this exists to stop: a plugin lands on its OWN new instrument, so "the selected
       instrument" and "the instrument holding this plugin" are routinely different numbers. */
    S2.ck('it opens the window for the slot the plugin is on',r.slot,1);
    S2.ck('  and the command palette reaches the same slot',r.paletteSlot,1);
    /* Plenty of plugins genuinely have no editor. That is an answer, not a failure — and the
       difference has to be visible, or the button looks broken. */
    S2.ok('a plugin with no window says so in plain words',
          /no window of its own/i.test(r.refusedMsg||''),r.refusedMsg);
    S2.ok('asking on an instrument with no plugin says that instead',
          /no plugin/i.test(r.noPluginMsg||''),r.noPluginMsg);
    S2.ok('  and does not bother the engine about it',r.askedForSlot0===false);

    /* ---- the mixer reaches the engine ----------------------------------------------------------
       The native engine has the same channel strips and master chain in Rust, so a fader that moves
       on screen has to move there too. What is pinned here is the wire: that a mixer edit sends at
       all, that the payload carries every track plus the master and the delay time, that it is
       throttled rather than flooding at LFO frame rate, and that opening a device pushes the song's
       mix immediately — otherwise choosing ASIO silently resets everybody's mix to unity. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.bpm=140;song.masterVol=0.6;
      song.trackset[0]=Object.assign(mkTs(),{vol:0.42,pan:-0.5,dly:0.3,rvb:0.2,eq:{l:3,m:-2,h:1}});
      song.trackset[1]=Object.assign(mkTs(),{vol:0.9});

      window.__calls=[];NMIX.last=0;
      applyMix();
      await new Promise(r=>setTimeout(r,10));
      const first=window.__calls.filter(c=>c.cmd==='eng_mix');

      /* hammer it the way a mix LFO would, at frame rate */
      window.__calls=[];
      for(let i=0;i<30;i++){applyMix();await new Promise(r=>setTimeout(r,2))}
      await new Promise(r=>setTimeout(r,80));
      const burst=window.__calls.filter(c=>c.cmd==='eng_mix').length;

      /* opening a device must push the mix without waiting for an edit */
      window.__calls=[];NMIX.last=0;
      nengMix();
      await new Promise(r=>setTimeout(r,10));
      const onOpen=window.__calls.find(c=>c.cmd==='eng_mix');

      const m=(first[0]||{}).args&&first[0].args.mix;
      const want=song.trackset.length;
      /* every eng_mix seen in this block, in the order they were sent */
      const seqs=[];
      for(const c of (window.__allMix||[]))seqs.push(c);
      NENG.open=false;
      return{sent:first.length,burst,onOpen:!!onOpen,mix:m,want,seqs,
             t0:m&&m.tracks&&m.tracks[0],trackCount:m&&m.tracks&&m.tracks.length};
    });
    S2.head('the mixer reaches the native engine');
    S2.ok('a mixer edit sends the state',r.sent>=1,'calls: '+r.sent);
    S2.ck('  with every track in it',r.trackCount,r.want);
    S2.ok('  the track settings cross intact',
          !!r.t0&&r.t0.vol===0.42&&r.t0.pan===-0.5&&r.t0.dly===0.3&&r.t0.rvb===0.2,
          JSON.stringify(r.t0));
    S2.ok('  the EQ comes with it',!!r.t0&&r.t0.eq&&r.t0.eq.l===3&&r.t0.eq.m===-2&&r.t0.eq.h===1);
    S2.ck('  and the master fader',r.mix&&r.mix.master,0.6);
    /* The browser owns the tempo, so it owns this arithmetic — `setDelayTime`'s 60/bpm*0.75. */
    S2.ok('  and the ping-pong delay time, derived from the tempo',
          !!r.mix&&Math.abs(r.mix.delayTime-60/140*0.75)<1e-9,r.mix&&r.mix.delayTime);
    /* 30 edits in ~60 ms must not be 30 messages: applyMix runs from the mix LFOs at frame rate,
       and an unthrottled wire would spend the session serialising the whole mixer. */
    /* ~140 ms of hammering at one message per 40 ms is a handful, not thirty. The bound is loose on
       purpose — the point is the ORDER of magnitude, and pinning it tighter would make the test
       fail on a slow machine rather than on a real regression. */
    S2.ok('30 rapid edits are coalesced, not sent one for one',r.burst>0&&r.burst<=8,
          'messages: '+r.burst);
    S2.ok('opening a device pushes the mix straight away',r.onOpen===true);
    /* Nothing waits for eng_mix and Rust answers it on a thread pool, so two updates can arrive in
       either order. Each carries a counter so the engine can throw away one that overtook a newer
       one — without it a dropped fader could settle a step behind where it was left. */
    S2.ok('every mix update carries an increasing sequence number',
          r.seqs&&r.seqs.length>1&&r.seqs.every((v,i)=>i===0||v>r.seqs[i-1]),
          JSON.stringify(r.seqs));

    /* ---- mute and solo cross the wire ----------------------------------------------------------
       "when muting a channel that has a vst on it its not muting": the browser mutes by not
       scheduling notes, and a plugin channel's sound never came from the browser. So the mute
       button has to reach the NATIVE mixer — folded by trackAudible() (a poly lane follows its
       leader, one solo silences the rest) into each track's `mute` on the eng_mix payload, with
       `solo` sent as false so the engine's own solo logic stays out of it. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      const grab=()=>{const c=window.__calls.filter(x=>x.cmd==='eng_mix').pop();return c&&c.args.mix.tracks};
      window.__calls=[];NMIX.last=0;
      toggleMS(1,'mute');
      await new Promise(r=>setTimeout(r,10));
      const muted=grab();
      window.__calls=[];NMIX.last=0;
      toggleMS(1,'mute');           /* back off */
      await new Promise(r=>setTimeout(r,10));
      const unmuted=grab();
      window.__calls=[];NMIX.last=0;
      toggleMS(0,'solo');
      await new Promise(r=>setTimeout(r,10));
      const soloed=grab();
      const rawSolo=song.trackset[0].solo;   /* still true in the SONG — the wire carries the fold, not the flags */
      window.__calls=[];NMIX.last=0;
      clearMS();
      await new Promise(r=>setTimeout(r,10));
      const cleared=grab();
      NENG.open=false;
      return{muted:muted&&muted.map(t=>t.mute),
             unmuted:unmuted&&unmuted.map(t=>t.mute),
             soloed:soloed&&soloed.map(t=>t.mute),
             soloField:soloed&&soloed.map(t=>t.solo),
             cleared:cleared&&cleared.map(t=>t.mute),rawSolo};
    });
    S2.head('mute and solo reach the native mixer');
    S2.ok('muting a track sends eng_mix with that track muted',
          !!r.muted&&r.muted[1]===true&&r.muted[0]===false,JSON.stringify(r.muted));
    S2.ok('  and un-muting sends it back audible',!!r.unmuted&&r.unmuted.every(m=>m===false),
          JSON.stringify(r.unmuted));
    S2.ok('one solo mutes every other track on the wire',
          !!r.soloed&&r.soloed[0]===false&&r.soloed.slice(1).every(m=>m===true),
          JSON.stringify(r.soloed));
    S2.ok('  as folded mute — solo itself crosses as false',
          !!r.soloField&&r.soloField.every(s=>s===false),JSON.stringify(r.soloField));
    S2.ok('clear brings everything back',!!r.cleared&&r.cleared.every(m=>m===false),
          JSON.stringify(r.cleared));
    S2.ok('  and the song\'s own solo flag survives — the send folds a copy, never the song',
          r.rawSolo===true);

    /* ---- the insert chains ride in the mix snapshot (stage D) ---------------------------------
       The engine builds ts.fx and song.masterFx for real now (native/engine/src/fx.rs), which
       only works if they actually cross the wire. The per-track chain always travelled inside
       trackset; masterFx is new to the payload — and its absence is exactly the kind of silent
       serde miss the Rust side cannot error on. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.trackset[0].fx=[{type:'echo',p:{time:.5,fb:.3},on:true},{type:'crush',p:{bits:6},on:false}];
      song.masterFx=[{type:'limit',p:{loud:40}}];
      window.__calls=[];NMIX.last=0;
      nengMix();
      await new Promise(r=>setTimeout(r,10));
      const m=(window.__calls.find(c=>c.cmd==='eng_mix')||{}).args;
      NENG.open=false;
      return m&&m.mix&&{fx0:m.mix.tracks[0].fx,masterFx:m.mix.masterFx};
    });
    S2.head('the insert chains reach the native engine');
    S2.ok('a track\'s fx chain crosses inside its trackset',
          !!r&&!!r.fx0&&r.fx0.length===2&&r.fx0[0].type==='echo'&&r.fx0[0].p.time===0.5,
          JSON.stringify(r&&r.fx0));
    S2.ok('  bypass crosses with it',!!r&&r.fx0[1].on===false);
    S2.ok('  and the master chain rides alongside',
          !!r&&!!r.masterFx&&r.masterFx.length===1&&r.masterFx[0].type==='limit',
          JSON.stringify(r&&r.masterFx));

    /* ---- the panic ladder ---------------------------------------------------------------------
       Alt+P must silence the native side too — eng_panic when the engine answers, eng_close (and
       a marked-closed badge) when it does not. The old panic touched neither, which on the
       desktop build made it a placebo: the sound that sticks is a plugin's. */
    r=await page.evaluate(async()=>{
      audio();
      NENG.open=true;window.__panicFail=false;window.__calls=[];
      COMMANDS.find(c=>c.n==='Panic — stop and silence EVERYTHING').f();
      await new Promise(r=>setTimeout(r,30));
      const healthy=window.__calls.map(c=>c.cmd);
      window.__panicFail=true;window.__calls=[];
      panic();
      await new Promise(r=>setTimeout(r,60));
      const wedged=window.__calls.map(c=>c.cmd);
      const closedAfter=!NENG.open;
      window.__panicFail=false;NENG.open=false;
      return{healthy,wedged,closedAfter};
    });
    S2.head('panic reaches the engine, and escalates when it must');
    S2.ok('a healthy engine gets eng_panic',r.healthy.indexOf('eng_panic')>=0,r.healthy.join(','));
    S2.ok('  and does NOT get closed for answering',r.healthy.indexOf('eng_close')<0,r.healthy.join(','));
    S2.ok('a wedged engine gets the device dropped from under it',
          r.wedged.indexOf('eng_panic')>=0&&r.wedged.indexOf('eng_close')>r.wedged.indexOf('eng_panic'),
          r.wedged.join(','));
    S2.ok('  and the badge stops claiming a device is open',r.closedAfter===true);

    /* ---- a plugin slot stays silent when the engine is down -----------------------------------
       trig() used to fall back to the instrument underneath the moment NENG.open went false — so
       an engine fault turned every synth line into the raw `sub` preset ("it plays the note under
       a basic tool"). In the desktop build the slot now stays a silent voice either way; the
       sound's absence is reported and the engine reopens itself. */
    r=await page.evaluate(()=>{
      audio();
      const ins=Object.assign(mkInst('sub','V'),{vst:{path:'C:/VST3/Vital.vst3',index:0,name:'Vital'}});
      NENG.open=true;
      const up=trig(ins,60,AC.currentTime,0,1,0);
      NENG.open=false;
      const down=trig(ins,60,AC.currentTime,0,1,0);
      const plain=trig(Object.assign(mkInst('sub','S')),60,AC.currentTime,0,1,0);
      NENG.open=false;
      return{up:up.srcs.length,down:down.srcs.length,plain:plain.srcs.length};
    });
    S2.head('a plugin slot never plays the instrument underneath it');
    S2.ck('engine open: the slot is a silent bookkeeping voice',r.up,0);
    S2.ck('engine DOWN: still silent, never the basic tool',r.down,0);
    S2.ok('an ordinary instrument still sounds (the guard is the vst, not the build)',r.plain>0,
          'srcs: '+r.plain);

    /* ---- one plugin, one load, however many callers -------------------------------------------
       volt.log 2026-08-11 20:58: City Piano created twice into slot 8 within a second — the warm
       sync and Play's sync both saw an unloaded slot. Concurrent syncs must join the same load. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.instruments=[Object.assign(mkInst('sub','V'),{vst:{path:'C:/VST3/Vital.vst3',index:0,name:'Vital'}})];
      NENG_VST.clear();NENG_VST_BAD.clear();window.__calls=[];
      await Promise.all([nengVstSync(),nengVstSync(),nengVstSync()]);
      const loads=window.__calls.filter(c=>c.cmd==='vst_load').length;
      NENG.open=false;NENG_VST.clear();
      return{loads};
    });
    S2.head('concurrent syncs do not double-load a plugin');
    S2.ck('three racing syncs make exactly one vst_load',r.loads,1);

    /* ---- the sidechain crosses as its own trigger list ----------------------------------------
       Native strips have a duck gain and the engine schedules the envelope (Mixer::duck) — but
       only from what nengPattern sends. Every note is a trigger, INCLUDING notes the engine
       cannot play (unported instruments) and, one day, muted ghost kicks: the trigger list is
       not the playable-cell list. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
        mkInst('supersaw','S'),      /* made unported below — no cell crosses, but the duck must */
      ];
      const d=song.patterns[curPat].data;
      d[0][0]={n:48,i:0,v:64};
      d[1][2]={n:60,i:1,v:48,c:'T',x:96};
      d[0][4]={n:-1,i:0,v:0};        /* an OFF is not a trigger */
      NENG.only=true;                /* so the kick's cell crosses alongside its trigger */
      /* every real instrument is ported now — recreate "unported" for the trigger contract */
      const at=NENG_OK.indexOf('supersaw');NENG_OK.splice(at,1);
      const p=nengPattern();
      NENG_OK.splice(at,0,'supersaw');
      NENG.only=false;
      return{ducks:p.ducks,cells:p.cells.length};
    });
    S2.head('sidechain triggers ride with the pattern');
    S2.ck('two notes, two triggers — the OFF is not one',r.ducks.length,2);
    S2.ok('the kick says it is a kick (the legacy source-less trigger)',
          r.ducks[0].kick===true&&r.ducks[0].src===0&&r.ducks[0].row===0,JSON.stringify(r.ducks[0]));
    S2.ok('the unported instrument still triggers, off-grid Txx intact',
          r.ducks[1].kick===false&&r.ducks[1].src===1&&r.ducks[1].tx===96,JSON.stringify(r.ducks[1]));
    /* kick + the OFF cross as cells; the supersaw is filtered — yet it ducked above */
    S2.ok('  even though its cell never crossed (kick + OFF only)',r.cells===2,'cells: '+r.cells);

    /* ---- a VST effect on a mixer channel -------------------------------------------------------
       ts.fxVsts -> fxvst_load keyed by TRACK and chain POSITION, reconciled before Play exactly
       as instrument plugins are synced, one in-flight load per chain slot. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.trackset[2].fxVsts=[{path:'C:/VST3/WaveShell1-VST3 12.7_x64.vst3',index:41,name:'SSLComp Stereo',fmt:'VST3'}];
      NENG_FXVST.clear();NENG_FXVST_BAD.clear();window.__calls=[];
      await Promise.all([nengVstSync(),nengVstSync()]);
      const loads=window.__calls.filter(c=>c.cmd==='fxvst_load');
      const out={n:loads.length,args:loads[0]&&loads[0].args};
      NENG.open=false;NENG_FXVST.clear();song.trackset[2].fxVsts=[];
      return out;
    });
    S2.head('a plugin effect on a channel reaches the engine');
    S2.ck('two racing syncs load it exactly once',r.n,1);
    S2.ok('  keyed by the mixer track, with the class index and chain position',
          !!r.args&&r.args.track===2&&r.args.index===41&&r.args.pos===0,JSON.stringify(r.args));

    /* ---- a CHAIN: several effects on one channel, in series, in order ------------------------- */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.trackset[3].fxVsts=[
        {path:'C:/VST3/apQualizr2.vst3',index:0,name:'apQualizr2',fmt:'VST3'},
        {path:'C:/VST3/WaveShell1-VST3 12.7_x64.vst3',index:41,name:'L2 Stereo',fmt:'VST3'},
      ];
      NENG_FXVST.clear();NENG_FXVST_BAD.clear();window.__calls=[];
      await nengVstSync();
      const loads=window.__calls.filter(c=>c.cmd==='fxvst_load').map(c=>c.args);
      /* now remove link 0 — the reconciler must unload BOTH old slots' wrong contents and
         reload the survivor one position earlier */
      window.__calls=[];
      song.trackset[3].fxVsts.splice(0,1);
      await nengFxSync();
      const after={
        unloads:window.__calls.filter(c=>c.cmd==='fxvst_unload').map(c=>c.args),
        loads:window.__calls.filter(c=>c.cmd==='fxvst_load').map(c=>c.args),
      };
      NENG.open=false;NENG_FXVST.clear();song.trackset[3].fxVsts=[];
      return{loads,after};
    });
    S2.head('effects chain — several plugins on one channel');
    S2.ck('two links load as pos 0 and pos 1',r.loads.map(a=>a.pos).join(','),'0,1');
    S2.ok('  in the song\'s order',r.loads[0].index===0&&r.loads[1].index===41,JSON.stringify(r.loads));
    S2.ok('removing link 0 slides the survivor down to pos 0',
          r.after.loads.length===1&&r.after.loads[0].pos===0&&r.after.loads[0].index===41,
          JSON.stringify(r.after));
    S2.ok('  and both stale slots were unloaded first',
          r.after.unloads.some(a=>a.pos===0)&&r.after.unloads.some(a=>a.pos===1),
          JSON.stringify(r.after.unloads));

    /* ---- the master's VST chain is track -1 ---------------------------------------------------- */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      song.masterFxVsts=[{path:'C:/VST3/FabFilter Pro-L 2.vst3',index:0,name:'Pro-L 2',fmt:'VST3'}];
      NENG_FXVST.clear();NENG_FXVST_BAD.clear();window.__calls=[];
      await nengVstSync();
      const load=window.__calls.find(c=>c.cmd==='fxvst_load');
      NENG.open=false;NENG_FXVST.clear();song.masterFxVsts=[];
      return load&&load.args;
    });
    S2.head('the master bus takes a VST chain too');
    S2.ok('song.masterFxVsts syncs as track -1',!!r&&r.track===-1&&r.index===0,JSON.stringify(r));

    /* ---- a one-slot song from before chains loads as link 0 ----------------------------------- */
    r=await page.evaluate(async()=>{
      const s=blankSong();
      s.trackset[1].fxVst={path:'C:/VST3/old.vst3',index:3,name:'Old Slot',fmt:'VST3'};
      s.masterFxVst={path:'C:/VST3/oldm.vst3',index:0,name:'Old Master',fmt:'VST3'};
      await loadSong(s);
      return{chan:song.trackset[1].fxVsts,gone:song.trackset[1].fxVst===undefined,
             master:song.masterFxVsts,mgone:song.masterFxVst===undefined};
    });
    S2.head('songs saved with the single FX slot still load');
    S2.ok('the channel slot becomes link 0 of a chain',
          !!r.chan&&r.chan.length===1&&r.chan[0].name==='Old Slot'&&r.gone,JSON.stringify(r.chan));
    S2.ok('  and so does the master\'s',
          !!r.master&&r.master.length===1&&r.master[0].name==='Old Master'&&r.mgone,JSON.stringify(r.master));

    /* ---- plugin parameter automation ----------------------------------------------------------
       vp<slot>.<id> lanes drive vst_param_set, vf.<id> lanes drive fxvst_param_set (track, or -1
       from a master lane), values pass through 0..1 untouched, and a burst per key coalesces. */
    r=await page.evaluate(async()=>{
      NENG.open=true;window.__calls=[];
      applyVstAuto(3,'vp5.100',0.25);
      applyVstAuto(2,'vf.101',0.5);
      applyVstAuto('M','vf.100',0.75);
      const notOurs=applyVstAuto(1,'vol',0.5);
      for(let i=0;i<20;i++)applyVstAuto(3,'vp5.100',i/20); /* a dense lane, one key */
      await new Promise(r=>setTimeout(r,80));
      const sets=window.__calls.filter(c=>c.cmd==='vst_param_set'||c.cmd==='fxvst_param_set');
      NENG.open=false;
      return{
        first:sets[0]&&{cmd:window.__calls[0].cmd,args:sets[0].args},
        fx:sets.find(c=>c.cmd==='fxvst_param_set'&&c.args.track===2)&&sets.find(c=>c.cmd==='fxvst_param_set'&&c.args.track===2).args,
        master:sets.find(c=>c.cmd==='fxvst_param_set'&&c.args.track===-1)&&sets.find(c=>c.cmd==='fxvst_param_set'&&c.args.track===-1).args,
        notOurs,
        burst:sets.filter(c=>c.cmd==='vst_param_set').length,
        last:sets.filter(c=>c.cmd==='vst_param_set').pop().args.value,
      };
    });
    S2.head('plugin parameters automate');
    S2.ok('an instrument lane reaches its slot and param id',
          !!r.first&&r.first.args.slot===5&&r.first.args.id===100&&r.first.args.value===0.25,
          JSON.stringify(r.first));
    S2.ok('a channel-FX lane carries its track',!!r.fx&&r.fx.track===2&&r.fx.id===101,JSON.stringify(r.fx));
    S2.ok('a master lane sends track -1',!!r.master&&r.master.id===100,JSON.stringify(r.master));
    S2.ok('non-plugin keys are left for the built-in dispatch',r.notOurs===false);
    S2.ok('a dense lane coalesces (~30 Hz, trailing value kept)',
          r.burst>=2&&r.burst<=8&&Math.abs(r.last-19/20)<1e-9,
          r.burst+' sends, last '+r.last);

    /* ---- the assist channel: remote hands through the front door ------------------------------
       Claude writes assist-cmd.json into the song folder; the app applies a whitelist of mix ops
       through the same code paths the panels use. What is pinned: each op does what it says,
       results come back, unknown ops are refused by name, and nothing here can touch anything
       outside the whitelist. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.masterFxVsts=[{path:'C:/VST2/IL Maximus.dll',index:0,name:'IL Maximus',fmt:'VST2'}];
      song.trackset[1].vol=0;song.trackset[1].fltF=0;
      const results=assistApply({ops:[
        {op:'fx_remove',t:-1,pos:0},
        {op:'set_vol',t:1,v:.85},
        {op:'set_flt',t:1,v:1},
        {op:'set_param',i:3,k:'cutoff',v:150},
        {op:'set_param',i:3,k:'nope',v:1},
        {op:'evil_op'},
      ]});
      return{results,
             masterGone:(song.masterFxVsts||[]).length===0,
             vol:song.trackset[1].vol,flt:song.trackset[1].fltF,
             cutoff:song.instruments[3].params.cutoff};
    });
    S2.head('the assist channel applies whitelisted mix ops');
    S2.ok('a master FX link is removed',r.masterGone===true&&/removed IL Maximus from the master/.test(r.results[0]),r.results[0]);
    S2.ok('fader and filter restore',r.vol===0.85&&r.flt===1,JSON.stringify([r.vol,r.flt]));
    S2.ck('an instrument knob moves',r.cutoff,150);
    S2.ok('a parameter that does not exist is refused by name',/has no nope/.test(r.results[4]),r.results[4]);
    S2.ok('an unknown op is refused, not guessed at',/unknown op evil_op/.test(r.results[5]),r.results[5]);

    /* ---- the palette's direct plugin search --------------------------------------------------
       "just searching in ctrl space": typing a plugin's name must produce add-rows, and picking
       one must LOAD the plugin onto the cursor's channel — the whole flow, exactly as clicked,
       because the first ship of this died in three different silent ways. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;cursor.t=2;
      VSTS.list=[
        {name:'L2 Stereo',vendor:'Waves',path:'C:/VST3/WaveShell1-VST3 12.7_x64.vst3',index:41,fmt:'VST3',kind:'fx'},
        {name:'Vital',vendor:'Vital Audio',path:'C:/VST3/Vital.vst3',index:0,fmt:'VST3',kind:'inst'},
      ];
      window.__calls=[];
      palOpen();
      palQ.value='l2';palRender();
      const rows=palItems.filter(c=>/^\+ /.test(c.n)).map(c=>c.n);
      const row=palItems.find(c=>c.n==='+ L2 Stereo → this channel');
      palClose();
      /* the render in the middle is the point: between showing the row and the user pressing
         Enter, VOLT keeps rendering — and a render used to swap the fxVsts array out from under
         the held reference (normTs), which ate the add. renderMixer() here reproduces that. */
      mixer.hidden=false;renderMixer();
      let ferr=null;
      if(row){try{await row.f()}catch(e){ferr=String((e&&e.stack)||e)}}
      mixer.hidden=true;
      await new Promise(r=>setTimeout(r,30));
      const load=window.__calls.find(c=>c.cmd==='fxvst_load');
      const out={rows,ferr,statusNow:document.getElementById('status').textContent,
                 loadArgs:load&&load.args,
                 inSong:(song.trackset[2].fxVsts||[]).map(v=>v.name),
                 instFiltered:!palItems.some(c=>/Vital/.test(c.n))};
      /* and the no-scan case must SAY so instead of showing nothing */
      VSTS.list=null;VSTS.scanning=Promise.resolve([]);  /* keep the hint row's warm kick inert */
      palOpen();palQ.value='l2';palRender();
      out.hint=(palItems.find(c=>/scanning your plugins/.test(c.n))||{}).n||null;
      palClose();VSTS.list=[];
      NENG.open=false;song.trackset[2].fxVsts=[];NENG_FXVST.clear();
      return out;
    });
    S2.head('Ctrl+Space types a plugin name straight onto a channel');
    S2.ok('typing two letters surfaces add-rows',r.rows.length===2,JSON.stringify(r.rows));
    S2.ok('  instruments are not offered as channel FX',r.instFiltered===true);
    S2.ok('picking the row loads onto the CURSOR channel at the chain tail',
          !!r.loadArgs&&r.loadArgs.track===2&&r.loadArgs.pos===0,
          JSON.stringify(r.loadArgs)+' err='+r.ferr+' status='+r.statusNow);
    S2.ok('  and the song carries the link',r.inSong.join(',')==='L2 Stereo',JSON.stringify(r.inSong));
    S2.ok('an unscanned list SAYS it is scanning instead of showing nothing',
          !!r.hint,String(r.hint));

    /* ---- Alt+F4 is Windows' close, never the transport ---------------------------------------- */
    r=await page.evaluate(async()=>{
      stop(true);
      document.dispatchEvent(new KeyboardEvent('keydown',{key:'F4',altKey:true,bubbles:true,cancelable:true}));
      await new Promise(r=>setTimeout(r,30));
      const played=SEQ.playing;
      if(SEQ.playing)stop(true);
      return played;
    });
    S2.head('Alt+F4 belongs to Windows');
    S2.ok('the transport does not start on Alt+F4',r===false);

    /* ---- one plugin, one row ------------------------------------------------------------------ */
    r=await page.evaluate(()=>{try{
      const list=[
        {name:'SSLComp Stereo',vendor:'Waves',path:'C:/VST3/WaveShell1-VST3 12.7_x64.vst3',index:4,fmt:'VST3',kind:'fx'},
        {name:'SSLComp Stereo',vendor:'Waves',path:'C:/VST3/WaveShell1-VST3 12.10_x64.vst3',index:4,fmt:'VST3',kind:'fx'},
        {name:'SSLComp Stereo',vendor:'Waves',path:'C:/VST3/WaveShell1-VST3 12.8_x64.vst3',index:4,fmt:'VST3',kind:'fx'},
        {name:'Diva',vendor:'u-he',path:'C:/VST2/Diva.dll',index:0,fmt:'VST2',kind:'inst'},
        {name:'Diva',vendor:'u-he',path:'C:/VST3/Diva.vst3',index:0,fmt:'VST3',kind:'inst'},
      ];
      const d=vstDedupe(list);
      const kept=vstDedupe(list,'C:/VST2/Diva.dll',0);
      return{n:d.length,
             ssl:d.find(p=>p.name==='SSLComp Stereo').path,
             diva:d.find(p=>p.name==='Diva').fmt,
             keptDiva:kept.find(p=>p.name==='Diva').path};
    }catch(err){return{err:String(err)}}});
    if(r&&r.err)S2.ok('vstDedupe evaluate ran',false,r.err);
    S2.head('duplicate plugins collapse to one row');
    S2.ck('five entries become two',r.n,2);
    S2.ok('the newest WaveShell wins (12.10 beats 12.8 numerically)',
          /12\.10/.test(r.ssl),r.ssl);
    S2.ck('VST3 beats VST2 for the same synth',r.diva,'VST3');
    S2.ok('  unless the OLD one is what is loaded — then it stays visible',
          /Diva\.dll/.test(r.keptDiva),r.keptDiva);

    /* ---- the touch log: tweak a knob in the plugin, right-click in VOLT --------------------- */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NVP_TOUCH.target='i5';NVP_TOUCH.id=100;NVP_TOUCH.name='Cutoff';NVP_TOUCH.at=performance.now();
      const hit=nvpTouchFor('i5'),otherPlugin=nvpTouchFor('f2');
      NVP_TOUCH.at=performance.now()-999999;
      const stale=nvpTouchFor('i5');
      /* what the menu's "just tweaked" item does: name cached, lane created */
      VP_NAMES.set('i5',new Map([[100,'Cutoff']]));
      autoCreate(2,'vp5.100');
      const lane=!!(song.patterns[curPat].auto&&song.patterns[curPat].auto['2.vp5.100']);
      const label=autoLabel('vp5.100');
      delete song.patterns[curPat].auto['2.vp5.100'];
      NVP_TOUCH.target=null;
      return{hit:!!hit,otherPlugin:!!otherPlugin,stale:!!stale,lane,label};
    });
    S2.head('the last-touched knob is offered, and only where it belongs');
    S2.ok('a fresh touch answers for its own plugin',r.hit===true);
    S2.ok('  and not for a different one',r.otherPlugin===false);
    S2.ok('  and expires rather than haunting the menu',r.stale===false);
    S2.ok('the menu item makes a real lane',r.lane===true);
    S2.ok('  labelled with the knob\'s own name',/Cutoff/.test(r.label),r.label);

    /* ---- Alt+F4 asks only when the song was touched --------------------------------------------
       The Rust side emits volt:close-req; the page answers with app_quit (clean) or a question
       (edited). Either way it must END in app_quit or in staying — never a dead window. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());          /* loadSong clears EDITED */
      window.__calls=[];
      await window.__evs['volt:close-req']();
      await new Promise(r=>setTimeout(r,30));
      const clean=window.__calls.map(c=>c.cmd).filter(c=>c==='app_quit');

      EDITED=true;window.__calls=[];
      const done=window.__evs['volt:close-req']();
      await new Promise(r=>setTimeout(r,120));
      const askOpen=!!document.getElementById('dlgInput');
      /* first row is "Save and close" — Enter takes it */
      if(askOpen)document.getElementById('dlgInput')
        .dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await done;
      await new Promise(r=>setTimeout(r,60));
      const after=window.__calls.map(c=>c.cmd);
      return{clean:clean.length,askOpen,savedThenQuit:after.indexOf('app_quit')>=0};
    });
    S2.head('closing the window asks the right question');
    S2.ck('an untouched song closes at once',r.clean,1);
    S2.ok('an edited song gets the question',r.askOpen===true);
    S2.ok('  and "Save and close" ends in app_quit',r.savedThenQuit===true);

    /* ---- the Projects panel on desktop leads with FILES ----------------------------------------
       "why the fuck do we still have the browser storage shit" — on desktop the song folder is the
       front door and browser storage is labelled the old world. The stub's lib_list returns two
       songs; both must be listed, newest first, and clicking one must open through lib_read. */
    r=await page.evaluate(async()=>{
      NSTORE.dir='X:/songs';HOME.files=null;
      toggleHome(true);
      await new Promise(r=>setTimeout(r,120));
      const t=homeBody.textContent;
      const rowsEls=[...homeBody.querySelectorAll('[data-hfile]')];
      window.__calls=[];
      if(rowsEls[0])rowsEls[0].click();
      await new Promise(r=>setTimeout(r,120));
      const read=window.__calls.find(c=>c.cmd==='lib_read'&&c.args.name!=='assist-cmd.json'); /* the assist poller reads too */
      const out={
        hasFolderSec:/Your songs/.test(t),
        files:rowsEls.map(e=>e.dataset.hfile),
        /* The desktop panel should NOT be showing a browser-storage meter at all when the folder
           is the home — that 5 MB ceiling is a browser's problem and means nothing here. */
        lsMeterHidden:!/of ~5 MB/.test(t),
        tabsLabelled:/Open tabs/.test(t),
        readArgs:read&&read.args,
        openedTab:PM.tabs.length,
      };
      toggleHome(false);NSTORE.dir=null;HOME.files=null;
      return out;
    });
    S2.head('the Projects panel on desktop leads with the song folder');
    S2.ok('a "Your songs" file section exists',r.hasFolderSec===true);
    S2.ok('  newest first',r.files[0]==='newest.volt'&&r.files[1]==='older.volt',r.files.join(','));
    S2.ok('  the ~5 MB browser-storage meter is gone',r.lsMeterHidden===true);
    S2.ok('  what is left is called "Open tabs", not storage internals',r.tabsLabelled===true);
    S2.ok('clicking a file opens it through lib_read',
      !!r.readArgs&&r.readArgs.name==='newest.volt'&&r.readArgs.dir==='X:/songs',
      JSON.stringify(r.readArgs));

    /* ---- song mode ----------------------------------------------------------------------------
       "it plays only in pattern and not song mode". It used to refuse song mode outright, so a
       plugin was silent in the mode songs are actually arranged in. Now the browser drives the
       engine pattern by pattern: the one playing goes across on Play, and the next as the order
       turns over — each aligned, each for ONE pass so the engine never loops a pattern the song
       has already left. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      /* a channel FX plugin is what makes a ported channel the ENGINE's to play (nengWantsTrack) —
         without one, Play correctly sends the engine nothing and this test would prove that instead */
      song.trackset[0].fxVsts=[{path:'C:/x.vst3',index:0,name:'X'}];
      /* two short patterns, both with a note, arranged one after the other */
      song.patterns[curPat].rows=4;song.patterns[curPat].data.forEach(t=>t.length=4);
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      const p2=JSON.parse(JSON.stringify(song.patterns[curPat]));
      p2.data[0][0]={n:55,i:0,v:100};
      song.patterns.push(p2);
      song.order=[0,1];
      song.bpm=240;                                  /* 4 rows fly by, so the order turns over fast */
      NENG.open=true;NENG.transport=true;NENG.lat=5.8;
      window.__calls=[];
      play(true);                                    /* SONG mode */
      await new Promise(r=>setTimeout(r,700));
      const plays=window.__calls.filter(c=>c.cmd==='eng_play').map(c=>c.args.pattern);
      stop();await new Promise(r=>setTimeout(r,60));
      NENG.transport=false;song.trackset[0].fxVsts=[];NENG_FXVST.clear();
      return{
        sent:plays.length,
        firstNote:plays.length?(plays[0].cells[0]||{}).note:null,
        repeats:plays.length?plays[0].repeats:null,
        everyOneAligned:plays.every(p=>typeof p.start_in==='number'&&p.start_in>0),
        sawSecondPattern:plays.some(p=>(p.cells[0]||{}).note===55),
        /* the fresh flag: Play's own send flushes plugins, the order-turnover sends must NOT */
        freshFirst:plays.length?plays[0].fresh===true:null,
        freshNext:plays.slice(1).every(p=>!p.fresh),
      };
    });
    S2.head('song mode reaches the engine');
    S2.ok('Play in SONG mode sends a pattern at all',r.sent>=1,'sends: '+r.sent);
    S2.ck('  and it is the pattern the song starts on',r.firstNote,48);
    S2.ck('  sent for one pass, not looping forever',r.repeats,1);
    S2.ok('  every send is aligned, including the ones mid-song',r.everyOneAligned===true);
    /* The one that proves the order is actually being followed rather than pattern 0 repeated. */
    S2.ok('  the NEXT pattern is sent as the order turns over',r.sawSecondPattern===true,
          'sends: '+r.sent);
    S2.ok('  Play\'s own send is the fresh take (plugins flush)…',r.freshFirst===true);
    S2.ok('  …and the turnover sends are NOT — no mid-song state wipe',r.freshNext===true);

    /* The transport hook is the one change here that can touch Mickey's actual playback, so the
       thing worth proving is not that it works — it is that when it is OFF it does nothing at all,
       and that stopping never leaves a loop running on the audio thread. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.trackset[0].fxVsts=[{path:'C:/x.vst3',index:0,name:'X'}]; /* makes track 0 the engine's — see the song-mode test */
      NENG.transport=false;NENG.open=true;
      window.__calls=[];
      play(false);await new Promise(r=>setTimeout(r,120));
      const whileOff=window.__calls.filter(c=>c.cmd==='eng_play').length;
      stop();await new Promise(r=>setTimeout(r,60));
      const stoppedOff=window.__calls.filter(c=>c.cmd==='eng_stop').length;

      NENG.transport=true;
      window.__calls=[];
      play(false);await new Promise(r=>setTimeout(r,200));
      const whileOn=window.__calls.filter(c=>c.cmd==='eng_play');
      stop();await new Promise(r=>setTimeout(r,60));
      const stoppedOn=window.__calls.filter(c=>c.cmd==='eng_stop').length;
      NENG.transport=false;song.trackset[0].fxVsts=[];NENG_FXVST.clear();
      return{whileOff,stoppedOff,whileOn:whileOn.length,
             repeats:whileOn.length?whileOn[0].args.pattern.repeats:null,
             startIn:whileOn.length?whileOn[0].args.pattern.start_in:null,
             fromRow:whileOn.length?whileOn[0].args.pattern.from_row:null,stoppedOn,
             browserStillRan:!SEQ.playing};
    });
    S2.ck('with the hook off, Play sends nothing to the engine',r.whileOff,0);
    S2.ok('  and the browser transport is untouched',r.browserStillRan===true);
    S2.ck('with it on, Play sends the pattern',r.whileOn,1);
    S2.ck('  looping, like the browser does',r.repeats,0);
    /* The flam fix: the pattern must carry an aligned start rather than letting the engine use its
       fixed 0.15 s. `start_in` covers the latency gap (browser ~58 ms vs ASIO ~6) plus wherever
       the browser's lookahead has got to, so anything at or above the 50 ms floor is credible;
       what would be wrong is 0 (never sent) or something past a bar. `from_row` likewise must
       point at where the browser IS, not naively at row 0. */
    S2.ok('  aligned: start_in covers the latency gap',
      r.startIn!=null&&r.startIn>=0.05&&r.startIn<2,String(r.startIn));
    S2.ok('  aligned: from_row is where the browser is, wrapped to the pattern',
      typeof r.fromRow==='number'&&r.fromRow>=0&&r.fromRow<16,String(r.fromRow));
    S2.ok('Stop always stops the engine, on or off',r.stoppedOff>=1&&r.stoppedOn>=1,
      'off='+r.stoppedOff+' on='+r.stoppedOn);

    /* ---- cross-engine alignment is PER STACK --------------------------------------------------
       The 35 ms Mickey had to nudge by hand (2026-08-13) measured this rule into existence:
       ASIO's reported latency is real and belongs in the sum; the shared Windows path's is
       fiction, and there the engine and the webview leave through the same stack — equal real
       latency, correct by NOTHING. So on WASAPI, start_in must IGNORE NENG.lat entirely. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.trackset[0].fxVsts=[{path:'C:/x.vst3',index:0,name:'X'}];
      NENG.open=true;NENG.transport=true;
      const startIn=async()=>{
        window.__calls=[];
        play(false);await new Promise(r=>setTimeout(r,150));
        const p=(window.__calls.find(c=>c.cmd==='eng_play')||{args:{}}).args.pattern;
        stop();await new Promise(r=>setTimeout(r,50));
        return p?p.start_in:null;
      };
      NENG.dev={id:'w1',label:'x [WASAPI]',host:'WASAPI'};
      NENG.lat=5;   const wasapiLow=await startIn();
      NENG.lat=400; const wasapiHigh=await startIn();
      /* small deltas on the ASIO side, or start_in drops under its 50 ms scheduling floor and
         gets walked forward a row — which is correct behaviour and useless for this comparison */
      NENG.dev={id:'a1',label:'x [ASIO]',host:'ASIO'};
      NENG.lat=5;  const asioLow=await startIn();
      NENG.lat=25; const asioHigh=await startIn();
      NENG.transport=false;NENG.open=false;NENG.lat=null;NENG.dev=null;
      song.trackset[0].fxVsts=[];NENG_FXVST.clear();
      return{wasapiLow,wasapiHigh,asioLow,asioHigh};
    });
    S2.head('cross-engine alignment: per stack, not per driver claim');
    S2.ok('on WASAPI the reported latency is IGNORED (equal-real-latency rule)',
      r.wasapiLow!=null&&r.wasapiHigh!=null&&Math.abs(r.wasapiLow-r.wasapiHigh)<0.02,
      r.wasapiLow+' vs '+r.wasapiHigh);
    S2.ok('on ASIO it is honoured — 20 ms more reported = ~20 ms less start_in',
      r.asioLow!=null&&r.asioHigh!=null&&Math.abs((r.asioLow-r.asioHigh)-0.02)<0.012,
      r.asioLow+' vs '+r.asioHigh);

    /* ---- the playhead coming back -----------------------------------------------------------
       The engine emits `neng:row` as each row sounds. Two things matter and neither is "it moves
       the cursor": that it does NOTHING when the hook is off (the browser's transport must be
       exactly as it was), and that a row for a pattern you are not looking at does not paint a
       cursor on the grid you ARE looking at. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      const fire=r=>window.__evs['neng:row']({payload:r});
      const at=()=>{const e=document.querySelector('#grows .playrow');return e?[...grows.children].indexOf(e):-1};
      const subscribed=!!(window.__evs&&window.__evs['neng:row']);

      /* hook off: the event must be inert, whatever it says */
      NENG.transport=false;NENG.open=true;
      play(false);await new Promise(r=>setTimeout(r,60));
      const before=at();fire(9);
      const offRow=at(),offMoved=offRow!==before;
      stop();

      /* hook on, and the engine is playing the pattern on screen */
      NENG.transport=true;
      play(false);await new Promise(r=>setTimeout(r,60));
      NENG.pat=curPat;fire(9);
      const onRow=at();
      const lead=nengLead();
      /* a row from a pattern you have browsed away from clears the cursor rather than lying */
      NENG.pat=curPat+1;fire(5);
      const foreign=at();
      /* -1 retires it, and the browser's own cursor is free to take over again */
      NENG.pat=curPat;fire(-1);
      const cleared=at(),leadAfter=nengLead();
      stop();
      NENG.transport=false;NENG.open=false;NENG.row=-1;
      return{subscribed,offMoved,onRow,lead,foreign,cleared,leadAfter,stale:NENG.row};
    });
    S2.ok('the engine event channel is subscribed to at load',r.subscribed===true);
    S2.ok('with the hook off, a row event moves nothing',r.offMoved===false);
    S2.ck('with it on, the cursor goes where the engine says',r.onRow,9);
    S2.ok('  and the browser stops fighting it for the cursor',r.lead===true);
    S2.ck('a row for another pattern clears rather than mispaints',r.foreign,-1);
    S2.ck('-1 retires the cursor when the engine stops',r.cleared,-1);
    S2.ok('  and hands it back to the browser transport',r.leadAfter===false);

    /* ---- ASIO only ---------------------------------------------------------------------------
       The last step of handing the transport over, and deliberately the smallest one: a gain node
       between the mix and the speakers. What has to be true is not "it goes quiet" — it is that it
       can only ever go quiet while everything lines up, that Stop always brings it back, and that
       it cannot follow a WAV render into the offline graph and silence the file. */
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:127};
      audio();
      const spk=()=>+spkOut.gain.value.toFixed(3);
      const settle=()=>new Promise(res=>setTimeout(res,80));

      /* the hook on but ASIO-only off: both engines, full level */
      NENG.open=true;NENG.transport=true;NENG.only=false;
      play(false);await settle();
      const both=spk();
      stop();await settle();

      /* ASIO-only on, playing: the webview goes down */
      NENG.only=true;
      play(false);await settle();
      const muted=spk();
      /* ...and Stop brings it back, whatever else happened */
      stop();await settle();
      const afterStop=spk();

      /* it must NOT mute when there is no engine to hear instead */
      NENG.open=false;
      play(false);await settle();
      const noDevice=spk();
      stop();await settle();

      /* nor when the transport hook is off */
      NENG.open=true;NENG.transport=false;
      play(false);await settle();
      const noHook=spk();
      stop();await settle();

      /* SONG mode mutes too, since 2026-08-14 — nengWantsTrack sends the ASIO-only sweep in both
         transports now, so the mute and the sweep stay one decision */
      NENG.transport=true;
      song.order=[curPat,curPat];
      play(true);await settle();
      const songMode=spk();
      stop();await settle();
      song.order=[];

      /* and a render must be unaffected — it builds its own graph, and a muted bounce would be a
         silent file, which is the worst possible way for this to fail */
      NENG.transport=true;
      play(false);await settle();
      const beforeRender=spk();
      await renderWav();
      const afterRender=spk();
      stop();await settle();
      NENG.only=false;NENG.transport=false;NENG.open=false;
      await settle();
      return{both,muted,afterStop,noDevice,noHook,songMode,beforeRender,afterRender,back:spk()};
    });
    S2.ck('with ASIO-only off the webview is at full level',r.both,1);
    S2.ck('with it on, playing, the webview is silent',r.muted,0);
    S2.ck('Stop always brings the speakers back',r.afterStop,1);
    S2.ck('  it will not mute with no device open',r.noDevice,1);
    S2.ck('  nor with the transport hook off',r.noHook,1);
    S2.ck('  and in song mode it MUTES — the engine is sent everything there now (2026-08-14)',r.songMode,0);
    S2.ck('a render leaves the live graph exactly as it found it',r.afterRender,r.beforeRender);
    S2.ok('  and it was genuinely muted while that happened',r.beforeRender===0,'gain '+r.beforeRender);
    S2.ck('  and turning it all off restores the speakers',r.back,1);

    /* The claim above is about a gain node. THIS is about the file: render while ASIO-only is on
       and listen to the result. A silent bounce is the worst way this could fail and the one a
       gain-value assertion would never catch. */
    await page.evaluate(INSTALL_RENDER);
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      const d=song.patterns[curPat].data;
      d[0][0]={n:48,i:0,v:127};d[0][8]={n:48,i:0,v:127};
      song.patterns[curPat].rows=16;
      audio();
      NENG.open=true;NENG.transport=true;NENG.only=true;
      play(false);await new Promise(res=>setTimeout(res,80));
      const onsets=await window.__onsets();
      stop();
      NENG.only=false;NENG.transport=false;NENG.open=false;
      return onsets;
    });
    S2.ok('the WAV still has the notes in it while ASIO-only is on',r.length>=2,
      r.length+' onsets at '+JSON.stringify(r));

    /* ---- the two GUIs ---------------------------------------------------------------------
       Both used to be `prompt()` with a numbered menu, or nothing at all. What is worth asserting
       is not that a dialog appears — it is that the list is in the right ORDER (ASIO first,
       newest song first), because that is the whole reason a list beats a text box, and that
       picking a song actually loads it rather than just closing the dialog. */
    S2.head('choosing audio hardware from a list');
    r=await page.evaluate(async()=>{
      const shown=()=>[...document.querySelectorAll('#dlgBody .pickMain')].map(e=>e.textContent);
      const p=nengPickDevice();
      await new Promise(res=>setTimeout(res,60));
      const order=shown();
      const subs=[...document.querySelectorAll('#dlgBody .pickSub')].map(e=>e.textContent);
      /* filtering is the other half of a list: type, and the wrong ones go */
      const inp=document.getElementById('dlgInput');
      inp.value='nio';inp.dispatchEvent(new Event('input'));
      const filtered=shown();
      inp.value='';inp.dispatchEvent(new Event('input'));
      /* Enter takes the highlighted row — the ASIO one, because it is first */
      inp.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await p;
      return{order,subs,filtered,opened:(window.__calls.filter(c=>c.cmd==='eng_open').pop()||{}).args,
             gone:document.getElementById('dlgWrap').hidden};
    });
    S2.ok('the zero-setup default output leads the list',/Windows default output/.test(r.order[0]),r.order.join(' | '));
    S2.ok('  and ASIO right behind it, where the low-latency crowd looks',/ASIO4ALL/.test(r.order[1]),r.order.join(' | '));
    S2.ok('  each row says which API it is',r.subs.some(s=>/^ASIO/.test(s))&&r.subs.some(s=>/^WASAPI/.test(s)),
      r.subs.join(' | '));
    S2.ck('typing filters the list',r.filtered.length,1);
    S2.ok('Enter opens the highlighted device — the default, as row 0',
      !!r.opened&&r.opened.deviceId==='DEFAULT',JSON.stringify(r.opened));
    S2.ok('  and the dialog closes behind it',r.gone===true);

    S2.head('opening a song from the song folder');
    r=await page.evaluate(async()=>{
      /* a real song, so the open path is exercised rather than a stub returning {} */
      window.__songOnDisk=JSON.parse(serialize(true));
      window.__songOnDisk.name='from the folder';
      NSTORE.dir='C:/Users/Mickey/Documents/VOLT';
      window.__calls=[];
      const p=openProject();
      await new Promise(res=>setTimeout(res,60));
      const rows=[...document.querySelectorAll('#dlgBody .pickMain')].map(e=>e.textContent);
      const subs=[...document.querySelectorAll('#dlgBody .pickSub')].map(e=>e.textContent);
      document.getElementById('dlgInput')
        .dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await p;
      await new Promise(res=>setTimeout(res,120));
      return{rows,subs,name:song.name,
             read:(window.__calls.find(c=>c.cmd==='lib_read'&&c.args.name!=='assist-cmd.json')||{}).args,
             btn:!document.getElementById('openBtn').hidden};
    });
    S2.ok('the Open button is there in the desktop build',r.btn===true);
    S2.ok('newest song first — it is nearly always the one you want',
      r.rows[0]==='newest'&&r.rows[1]==='older',r.rows.join(' | '));
    S2.ok('  with when and how big, not just a filename',/today|ago|yesterday/.test(r.subs[0])&&/KB|MB/.test(r.subs[0]),
      r.subs.join(' | '));
    S2.ok('Enter reads the one you picked',!!r.read&&r.read.name==='newest.volt',JSON.stringify(r.read));
    S2.ck('  and the song is actually loaded, not just fetched',r.name,'from the folder');

    /* ---- a plugin assignment is part of the song, everywhere ------------------------------
       The vanish of 2026-08-13: snapState() left `ins.vst` (and the master's VST chain) out of
       the snapshot, applyState() rebuilt instruments without it, and the save scheduled 700 ms
       after an undo made the loss permanent. One Ctrl+Z after adding a synth and it was quietly
       a bare `sub` again — "the vst inst i put are vanishing". These fail by name if any of the
       three legs (snapshot, restore, engine reconcile) regresses. */
    S2.head('a plugin assignment survives undo');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.instruments[0].vst={path:'C:/VST3/Vital.vst3',index:0,name:'Vital',fmt:'VST3'};
      song.masterFxVsts=[{path:'C:/VST3/u-he/Diva.vst3',index:2,name:'Diva',fmt:'VST3'}];
      /* the state WITH the plugin becomes an undo point… */
      pushUndoSoon();await new Promise(res=>setTimeout(res,300));
      /* …then an ordinary edit on top of it… */
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      pushUndoSoon();await new Promise(res=>setTimeout(res,300));
      /* …and undoing the edit must give back the cell-less song WITH its plugins */
      undo();
      const ins=song.instruments[0];
      return{cell:song.patterns[curPat].data[0][0],
             vst:ins&&ins.vst&&ins.vst.name,
             mfv:(song.masterFxVsts||[]).map(v=>v.name).join(',')};
    });
    S2.ck('the edit itself is undone',r.cell,null);
    S2.ck('the instrument keeps its plugin through the undo',r.vst,'Vital');
    S2.ck('  and the master keeps its VST chain',r.mfv,'Diva');

    S2.head('the engine follows the song when a plugin is taken away');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.instruments[0].vst={path:'C:/VST3/Vital.vst3',index:0,name:'Vital',fmt:'VST3'};
      NENG.open=true;NENG_VST.clear();NENG_VST_BAD.clear();window.__calls=[];
      await nengVstSync();
      const loaded=window.__calls.filter(c=>c.cmd==='vst_load').length;
      /* the song stops claiming it — an undo, a delete, a replace all look like this */
      delete song.instruments[0].vst;
      window.__calls=[];
      await nengVstSync();
      const unloads=window.__calls.filter(c=>c.cmd==='vst_unload').map(c=>c.args.slot);
      const size=NENG_VST.size;
      NENG.open=false;NENG_VST.clear();
      return{loaded,unloads,size};
    });
    S2.ck('the sync loads what the song claims',r.loaded,1);
    S2.ck('  and UNLOADS what it no longer does — one owner includes "no longer"',r.unloads,[0]);
    S2.ck('  leaving the engine map empty',r.size,0);

    /* ---- the buffer knob ------------------------------------------------------------------
       FL's no-hiccups slider, as a row in the device picker. What matters is that the choice is
       real: the row shows the current setting, and every eng_open carries it as the hint. */
    S2.head('the buffer length rides every open');
    r=await page.evaluate(async()=>{
      NENG.buf=11.6;
      const p=nengPickDevice();
      await new Promise(res=>setTimeout(res,60));
      const rows=[...document.querySelectorAll('#dlgBody .pickMain')].map(e=>e.textContent);
      document.getElementById('dlgInput')
        .dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));
      await p;
      const opened=(window.__calls.filter(c=>c.cmd==='eng_open').pop()||{}).args;
      NENG.buf=0;
      return{row:rows.find(t=>/Buffer length/.test(t))||'',opened};
    });
    S2.ok('the picker offers the buffer knob, wearing the current setting',/11\.6 ms/.test(r.row),r.row);
    S2.ok('  and eng_open asks for exactly that buffer',!!r.opened&&r.opened.bufMs===11.6,JSON.stringify(r.opened));

    /* ---- finding a synth by name, and putting it somewhere ---------------------------------
       "search for vst generator like diva and it will be found — make it replace the selected
       instrument after asking, and make it undoable." All three halves are asserted: the rows
       exist, replacing keeps the slot (rather than adding one), and undo restores what was there. */
    S2.head('the palette finds instruments, not only effects');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;
      /* the palette searches the SCAN — planted here, because an earlier block in this suite
         deliberately leaves the list empty */
      VSTS.list=[{name:'Diva',vendor:'u-he',path:'C:/VST3/u-he/Diva.vst3',index:2,fmt:'VST3',kind:'inst'}];
      song.instruments=[mkInst('kick','Kick',{pitch:50,punch:.6,decay:.4,drive:.3})];
      curInst=0;
      const rows=palVstRows('diva').map(x=>x.n);
      return{rows};
    });
    S2.ok('a generator search offers to replace the selected instrument',
      r.rows.some(t=>/Diva → replace 00 “Kick”/.test(t)),r.rows.join(' | '));
    S2.ok('  and to add it as its own instrument',r.rows.some(t=>/Diva → new instrument/.test(t)),r.rows.join(' | '));

    S2.head('replacing an instrument with a plugin is undoable');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      NENG.open=true;NENG_VST.clear();NENG_VST_BAD.clear();
      song.instruments=[mkInst('kick','Kick',{pitch:50,punch:.6,decay:.4,drive:.3})];
      curInst=0;
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      pushUndoSoon();await new Promise(res=>setTimeout(res,300));
      /* askConfirm puts a dialog on screen — answer it the way a user would */
      const p=nengReplaceInstVst(0,{name:'Diva',path:'C:/VST3/u-he/Diva.vst3',index:2,fmt:'VST3'});
      await new Promise(res=>setTimeout(res,60));
      const btns=[...document.querySelectorAll('#dlgBtns button')].map(b=>b.textContent);
      const yes=[...document.querySelectorAll('#dlgBtns button')].find(b=>/Replace/.test(b.textContent));
      if(yes)yes.click();
      await p;
      const after={count:song.instruments.length,vst:song.instruments[0].vst&&song.instruments[0].vst.name,
                   name:song.instruments[0].name};
      undo();
      const back={count:song.instruments.length,vst:song.instruments[0].vst&&song.instruments[0].vst.name,
                  type:song.instruments[0].type,name:song.instruments[0].name};
      NENG.open=false;NENG_VST.clear();
      return{btns,after,back};
    });
    S2.ok('it asks before replacing anything',r.btns.some(t=>/Replace/.test(t)),r.btns.join(' | '));
    S2.ck('the slot becomes the plugin — no extra instrument appears',r.after.count,1);
    S2.ck('  and it is the plugin that was searched for',r.after.vst,'Diva');
    S2.ck('Ctrl+Z gives the instrument back',r.back.vst,undefined);
    S2.ck('  as what it was before',r.back.type,'kick');
    S2.ck('  under its own name again',r.back.name,'Kick');

    /* ---- a late song-mode send joins the bar instead of skipping it -----------------------
       nengSongStep fires as the order turns over. If sample uploads or plugin syncs make it
       late: with the webview sounding, skipping is right (a late copy flams); in ASIO ONLY the
       engine is all there is, and skipping means a SILENT BAR with the transport running — so
       it is sent regardless.
       **It is sent FROM ROW 0, and that is the point of it since 2026-08-15.** A song step means
       "here is the next pattern, from its top": the engine PARKS it and starts it on its own
       timeline exactly one row after the outgoing pattern's last row (the seam — see the queued
       pattern in native/desktop/src/engine.rs and `a_pattern_change_lands_exactly_one_row_after_
       the_last` in clock.rs). It used to walk `from_row` forward to compensate for the send being
       late, which is what made a transition audibly wander: it aimed with a wall clock AND
       dropped rows off the front of the new pattern. `start_in` survives only as the fallback for
       a send that lands when the engine is no longer playing — and starting late at the top beats
       starting on time in the middle. */
    /* ---- editing while it plays reaches the engine ----------------------------------------
       The engine holds the cells it was handed at Play; the browser's scheduler re-reads the song
       every row. So on an engine-owned channel, editing changed NOTHING until the next Play —
       "the audio is not getting played at realtime … i need to stop playback and play again in
       order to hear the updated pattern" (2026-08-15). `eng_cells` replaces the notes without
       touching the clock, and the pattern is stamped so the engine can refuse an edit aimed at a
       pattern it is not playing. The ARP is tested alongside because it is the case that first
       showed the hole: it rewrites notes live and never calls saveSoon, which is where the hook
       hangs — so it needed its own call and would silently regress without a test. */
    S2.head('editing while the engine plays is heard without a restart');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('sub','Bass',{harm:.15,rel:.06})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      audio();
      NENG.open=true;NENG.transport=true;NENG.only=true;SEQ.playing=true;
      window.__calls=[];
      song.patterns[curPat].data[0][8]={n:55,i:0,v:64};   /* the edit */
      saveSoon();
      await new Promise(r=>setTimeout(r,400));
      const cells=window.__calls.filter(c=>c.cmd==='eng_cells');
      const sent=cells.length?cells[cells.length-1].args.pattern:null;
      /* and no Play: a live edit must not restart the transport */
      const plays=window.__calls.filter(c=>c.cmd==='eng_play').length;
      NENG.only=false;NENG.transport=false;SEQ.playing=false;NENG.open=false;
      return{sentCells:cells.length>0,plays,
             pat:sent&&sent.pat,notes:sent?sent.cells.filter(c=>c.note>=0).length:0,
             hasArpHook:/nengCellsSoon\(\)/.test(arpApply.toString())};
    });
    S2.ok('the edit is sent to the engine',r.sentCells);
    S2.ck('  as the pattern it belongs to',r.pat,0);
    S2.ck('  carrying both notes',r.notes,2);
    S2.ck('  and WITHOUT restarting the transport',r.plays,0);
    S2.ok('the arp’s live rewrite calls the same hook — it never goes through saveSoon',
      r.hasArpHook);

    S2.head('a late song-mode send joins the bar instead of skipping it');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3})];
      song.patterns[curPat].data[0][0]={n:48,i:0,v:64};
      audio();
      NENG.open=true;SEQ.playing=true;
      window.__calls=[];NENG.only=false;
      await nengSongStep(curPat,AC.currentTime-0.01);   /* already in the past */
      const skipped=window.__calls.filter(c=>c.cmd==='eng_play').length;
      window.__calls=[];NENG.only=true;
      await nengSongStep(curPat,AC.currentTime-0.01);
      const sent=(window.__calls.find(c=>c.cmd==='eng_play')||{}).args;
      NENG.only=false;SEQ.playing=false;NENG.open=false;
      return{skipped,sent:sent&&{from:sent.pattern.from_row,startIn:sent.pattern.start_in}};
    });
    S2.ck('with the webview sounding, a late send is skipped — it would flam',r.skipped,0);
    S2.ok('in ASIO only it is still sent — never a silent bar',!!r.sent,JSON.stringify(r.sent));
    S2.ok('  from the TOP of the pattern, so the engine can seam it',
      !!r.sent&&r.sent.from===0,JSON.stringify(r.sent));
    S2.ok('  with a startable fallback time for when the engine has already stopped',
      !!r.sent&&r.sent.startIn>=0.05,JSON.stringify(r.sent));

    /* ---- the webview must not SYNTHESISE what the engine is playing ------------------------
       Muting spkOut hides the browser's copy; it does not stop it being computed. While trig()
       only checked for an FX plugin, ASIO-only ran two complete mixes on one machine — the
       engine's and a silent browser one — and the wasted DSP starved the audio callback into
       crackle on busy bars (2026-08-14, "the snare fucks it up with distortions like crazy").
       Counting NODES is the only honest assertion here: a gain-value check passes either way. */
    S2.head('an engine-owned channel costs the webview no DSP');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.instruments=[mkInst('reese','R',{voices:3,detune:40,cutoff:150,res:0,lfoRate:.35,
                                            lfoAmt:.35,drive:.5,sub:.5,rel:.09})];
      audio();
      /* count real oscillators built, by watching the context itself */
      const realOsc=AC.createOscillator.bind(AC);
      let built=0;
      AC.createOscillator=()=>{built++;return realOsc()};
      /* trig(inst, note, t, tr, vel, …) — track 0, so ensureChans has a strip for it */
      ensureChans(TRACKS);
      const countFor=(only)=>{
        NENG.only=only;built=0;
        trig(song.instruments[0],48,AC.currentTime+.05,0,.8);
        return built;
      };
      NENG.open=true;NENG.transport=true;SEQ.playing=true;
      const engineOwns=countFor(true);     /* ASIO only — the engine plays this channel */
      const webviewOwns=countFor(false);   /* +web — the webview still owns it */
      SEQ.playing=false;NENG.open=false;NENG.transport=false;NENG.only=false;
      AC.createOscillator=realOsc;
      return{engineOwns,webviewOwns};
    });
    S2.ck('ASIO-only: the webview builds NO oscillators for that channel',r.engineOwns,0);
    S2.ok('  but it still does when it owns the channel itself',r.webviewOwns>0,
      r.webviewOwns+' oscillators');

    S2.ok('no console errors',o2.errs.length===0,o2.errs.join(' | '));
  }finally{await o2.browser.close()}

  S.checks+=S2.checks;S.fails+=S2.fails;
  /* ---- a render is the WHOLE mix now ----
     Plugin instruments render through eng_render_vst (tests/vstrender.test.js) and plugin FX
     chains through eng_render_fx (tests/fxrender.test.js), so renderMissing() has nothing left
     to warn about — the "these will be missing" dialog is gone, and a chain that FAILS is named
     in the status line instead of being announced in advance. */
  {
    const o3=await open({pre:PRE});
    try{
      const page=o3.page;
      const r=await page.evaluate(async()=>{
        await loadSong(blankSong());
        song.instruments[0].vst={path:'C:/vst/Serum.dll',name:'Serum'};
        song.instruments[0].name='Lead';
        const pat=song.patterns[0];
        pat.data[0][0]={n:48,i:0,v:48};
        song.trackset[2].fxVsts=[{path:'C:/vst/Pro-Q.dll'}];
        song.masterFxVsts=[{path:'C:/vst/L2.dll'}];
        const withPlugins=renderMissing();
        song.instruments[0].vst=null;song.trackset[2].fxVsts=[];song.masterFxVsts=[];
        return{withPlugins,clean:renderMissing().length};
      });
      S.head('a render no longer has anything it cannot include');
      S.ck('nothing is listed as missing, plugins and chains included',r.withPlugins.length,0);
      S.ck('and with no plugins the answer is the same',r.clean,0);
    }finally{await o3.browser.close()}
  }

  return S;
};
