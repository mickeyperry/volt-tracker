/* Rows per beat. The promise is "your song sounds exactly the same, you just get a finer grid",
   so the important check is an actual audio render before and after, comparing note onsets. */
const{suite,open,INSTALL_RENDER}=require('./lib');

module.exports=async function(){
  const S=suite('rows per beat');
  const{browser,page,errs}=await open();
  try{
    await page.evaluate(INSTALL_RENDER);
    await page.evaluate(()=>{window.__bassOnly=()=>{
      song.trackset.forEach((ts,t)=>{ts.mute=(t!==1);ts.solo=false});song.order=[0];
    }});
    const fresh=()=>page.evaluate(async()=>{await loadSong(KITS['mine'].make());window.__bassOnly()});
    const shape=()=>page.evaluate(()=>{const p=song.patterns[curPat];
      return{lpb:song.lpb,rows:p.rows,beats:+(p.rows/song.lpb).toFixed(3),
        notes:p.data[1].filter(c=>c&&c.n!=null).length,ticks:p.data[1].filter(c=>c&&c.c==='T').length}});

    S.head('baseline (4 rows/beat)');
    await fresh();
    const base=await shape(),a4=await page.evaluate(()=>window.__onsets());
    S.note(JSON.stringify(base)+' · '+a4.length+' onsets rendered');

    for(const[lpb,factor]of[[8,2],[6,1.5]]){
      S.head('4 -> '+lpb+' rows/beat (x'+factor+')');
      await fresh();
      await page.evaluate(async n=>{await setLpb(n)},lpb);
      const s=await shape();
      S.ck('rows scale by '+factor,s.rows,Math.round(base.rows*factor));
      S.ck('same length in beats',s.beats,base.beats);
      S.ck('no notes lost',s.notes,base.notes);
      if(factor%1)S.ok('off-grid rows kept as Txx micro-delays',s.ticks>0,s.ticks+' cells carry a tick');
      const a=await page.evaluate(()=>window.__onsets());
      S.ck('same number of note onsets',a.length,a4.length);
      let worst=0;for(let i=0;i<Math.min(a.length,a4.length);i++)worst=Math.max(worst,Math.abs(a[i]-a4[i]));
      S.ok('THE MUSIC IS UNCHANGED (onset drift)',worst<=.006,(worst*1000).toFixed(1)+' ms worst case over '+a4.length+' notes');
    }

    S.head('loop region, automation, cursor');
    const r=await page.evaluate(async()=>{
      await loadSong(KITS['mine'].make());
      const p=song.patterns[curPat];
      p.la=8;p.lb=23;
      p.auto=p.auto||{};p.auto['1.vol']=new Array(p.rows).fill(null);
      p.auto['1.vol'][0]=.2;p.auto['1.vol'][1]=.6;p.auto['1.vol'][16]=1;
      cursor.r=12;
      await setLpb(8);
      const q=song.patterns[curPat],a=q.auto['1.vol'];
      return{la:q.la,lb:q.lb,cursor:cursor.r,pt0:a[0],pt1:a[2],mid:a[1],lone:a[32],loneNext:a[33],len:a.length,rows:q.rows};
    });
    S.ck('loop start scaled',r.la,16);
    S.ck('loop end scaled',r.lb,47);
    S.ck('cursor scaled',r.cursor,24);
    S.ck('automation points moved',[r.pt0,r.pt1,r.lone],[.2,.6,1]);
    S.ok('painted ramps filled in',r.mid!=null,'mid '+r.mid);
    S.ok('lone points stay lone',r.loneNext==null);
    S.ck('automation resized with the pattern',r.len,r.rows);

    S.head('undo + guard rails');
    const u=await page.evaluate(async()=>{
      await new Promise(x=>setTimeout(x,400));
      undo();
      await new Promise(x=>setTimeout(x,150));
      return{lpb:song.lpb,rows:song.patterns[curPat].rows,ui:document.getElementById('lpbSel').value};
    });
    S.ck('undo restores rows/beat + rows',[u.lpb,u.rows],[4,64]);
    S.ck('toolbar agrees',u.ui,'4');

    const g=await page.evaluate(async()=>{
      await loadSong(KITS['mine'].make());
      const el=document.getElementById('patLen');el.value=128;el.dispatchEvent(new Event('change'));
      await setLpb(16); /* 128 x4 = 512 rows — over the limit, must refuse and change nothing */
      return{lpb:song.lpb,rows:song.patterns[curPat].rows,ui:document.getElementById('lpbSel').value};
    });
    S.ck('refuses to blow the row limit',[g.lpb,g.rows,g.ui],[4,128,'4']);

    S.head('Shift+[ / Shift+] step the grid');
    await page.evaluate(async()=>{await loadSong(KITS['mine'].make());document.body.focus()});
    const step=async(key,shift)=>{
      await page.keyboard.down('Shift');await page.keyboard.press(key);await page.keyboard.up('Shift');
      await new Promise(x=>setTimeout(x,250));
      return page.evaluate(()=>({lpb:song.lpb,rows:song.patterns[curPat].rows,ui:document.getElementById('lpbSel').value,oct:octave}));
    };
    let k=await step('BracketRight');
    S.ck('Shift+] goes finer (4 -> 6)',[k.lpb,k.rows,k.ui],[6,96,'6']);
    k=await step('BracketRight');
    S.ck('  and again (6 -> 8)',[k.lpb,k.rows],[8,128]);
    k=await step('BracketLeft');
    S.ck('Shift+[ goes back coarser (8 -> 6)',[k.lpb,k.rows],[6,96]);
    const oct0=await page.evaluate(()=>octave);
    await page.keyboard.press('BracketRight');await new Promise(x=>setTimeout(x,80));
    const after=await page.evaluate(()=>({oct:octave,lpb:song.lpb}));
    S.ck('plain ] still changes the octave, not the grid',[after.oct,after.lpb],[oct0+1,6]);
    const stuck=await page.evaluate(async()=>{
      song.lpb=2;document.getElementById('lpbSel').value=2;
      await nudgeLpb(-1);
      return{lpb:song.lpb,status:$id('status').textContent};
    });
    S.ck('does not wrap past the coarse end',stuck.lpb,2);
    S.note(stuck.status);

    S.head('what it unlocks');
    const rates=await page.evaluate(()=>{
      const at=n=>{song.lpb=n;ARP.mode='arp';return arpRateList().map(i=>ARP_RATES[i].lab).join(' ')};
      const o={four:at(4),eight:at(8)};song.lpb=4;return o;
    });
    S.note('arp rates at 4: '+rates.four);
    S.note('arp rates at 8: '+rates.eight);
    S.ok('32nds + 16th triplets appear at 8',/1\/32/.test(rates.eight)&&/1\/16T/.test(rates.eight));

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
