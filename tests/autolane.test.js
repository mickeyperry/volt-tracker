/* Automation lanes: zoom, the shape generator and level scaling. The shapes are checked by their
   VALUES rather than by "a dialog opened" — a sine that isn't a sine is worse than no sine. */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('automation lanes');
  const{browser,page,errs}=await open({w:1400,h:900});
  try{
    const seed=()=>page.evaluate(async()=>{
      await loadSong(blankSong());
      const pat=song.patterns[0];pat.rows=32;
      pat.auto={'0.p.cutoff':new Array(32).fill(.5)};
      autoSel=null;
      if(autoPan.hidden)autoToggle();
      renderAuto();
    });
    await seed();

    S.head('zoom');
    let r=await page.evaluate(()=>{
      const w=()=>autoLanes.style.getPropertyValue('--astepW');
      const start=w();
      autoZoom(2);const zin=w();
      autoZoom(.25);const zout=w();
      AZOOM=16;autoApplyZoom();
      autoFit();
      const fitted=parseFloat(w());
      const rows=song.patterns[0].rows;
      const lane=autoLanes.querySelector('.alane');
      const label=lane.querySelector('.al').offsetWidth;
      return{start,zin,zout,fitted,rows,fitsOnScreen:fitted*rows+label<=autoPan.clientWidth+2,
             buttons:['azIn','azOut','azFit'].every(id=>!!document.getElementById(id))};
    });
    S.ok('the controls are there',r.buttons);
    S.ok('zooming in widens the steps',parseFloat(r.zin)>parseFloat(r.start),r.start+' → '+r.zin);
    S.ok('  and out narrows them',parseFloat(r.zout)<parseFloat(r.start),r.zout);
    S.ok('fit puts the whole pattern on screen',r.fitsOnScreen,
      r.fitted+'px x '+r.rows+' rows inside the panel');

    r=await page.evaluate(()=>{
      AZOOM=16;autoApplyZoom();
      autoZoom(100);const hi=AZOOM;
      autoZoom(.0001);const lo=AZOOM;
      AZOOM=16;autoApplyZoom();
      return{hi,lo};
    });
    S.ok('it cannot be zoomed into uselessness',r.hi<=48&&r.lo>=3,'clamped to '+r.lo+'..'+r.hi+'px');

    S.head('the shape generator draws real waveforms');
    r=await page.evaluate(()=>{
      const arr=song.patterns[0].auto['0.p.cutoff'];
      const draw=(shape,cycles,lo,hi,phase)=>{
        Object.assign(ASHAPE,{shape,cycles,phase:phase||0,lo,hi});
        laneShapeDlg('0.p.cutoff',10,10);
        document.getElementById('shApply').click();
        return arr.slice();
      };
      const sine=draw('sine',1,0,1);
      const ramp=draw('ramp',1,0,1);
      const saw=draw('saw',1,0,1);
      const sq=draw('square',1,0,1);
      const tri=draw('tri',1,0,1);
      const band=draw('sine',1,.25,.75);
      const two=draw('sine',2,0,1);
      const rnd1=draw('random',4,0,1);
      const rnd2=draw('random',4,0,1);
      const zc=a=>{let n=0;for(let i=1;i<a.length;i++)if((a[i-1]<.5)!==(a[i]<.5))n++;return n};
      return{
        sineStart:sine[0],sinePeak:Math.max(...sine),sineMin:Math.min(...sine),
        rampRising:ramp[0]<ramp[8]&&ramp[8]<ramp[16]&&ramp[16]<ramp[24],
        sawFalling:saw[0]>saw[8]&&saw[8]>saw[16],
        squareVals:[...new Set(sq)].sort(),
        /* a triangle peaks in the MIDDLE — rows 8 and 24 are both halfway, so comparing those
           two proves nothing */
        triUpDown:tri[0]<tri[16]&&tri[16]>tri[31],triPeak:tri[16],
        bandLo:Math.min(...band),bandHi:Math.max(...band),
        oneCross:zc(sine),twoCross:zc(two),
        randomRepeatable:rnd1.every((v,i)=>v===rnd2[i]),
        rounded:sine.every(v=>Math.abs(v*100-Math.round(v*100))<1e-9)
      };
    });
    S.ok('a sine starts low, peaks and comes back',
      r.sineStart<.05&&r.sinePeak>.95&&r.sineMin<.05,
      'start '+r.sineStart+', peak '+r.sinePeak);
    S.ok('  ramp rises all the way',r.rampRising);
    S.ok('  saw falls all the way',r.sawFalling);
    S.ck('  square is only two values',r.squareVals,[0,1]);
    S.ok('  triangle climbs to a peak in the middle and back',r.triUpDown,'peak '+r.triPeak+' at the halfway row');
    S.ok('low and high really bound it',r.bandLo>=.24&&r.bandHi<=.76,
      'asked for 0.25–0.75, got '+r.bandLo+'–'+r.bandHi);
    S.ok('two cycles cross twice as often',r.twoCross>=r.oneCross*1.8,
      r.oneCross+' crossings at 1 cycle, '+r.twoCross+' at 2');
    S.ok('random is repeatable, not different every press',r.randomRepeatable);
    S.ok('  values are stored to the same precision as painting',r.rounded);

    /* the point of a shape control is hearing it while you move it — applying only on a button
       press means every experiment costs a round trip */
    S.head('the shape is live, not on-apply');
    r=await page.evaluate(()=>{
      const arr=song.patterns[0].auto['0.p.cutoff'];
      arr.fill(.5);
      autoSel=null;
      Object.assign(ASHAPE,{shape:'ramp',cycles:1,phase:0,lo:0,hi:1});
      laneShapeDlg('0.p.cutoff',10,10);
      const onOpen=arr.slice(0,4);                    /* drawn immediately, before any click */
      /* move the SLIDER — the handler reads its value, so poking ASHAPE directly proves nothing */
      const c=document.getElementById('shC');
      c.value=4;c.dispatchEvent(new Event('input'));
      const afterDrag=arr.slice(0,4);
      const painted=[...document.querySelectorAll('.alane .astep i')].length>0;
      document.getElementById('shRst').click();
      const afterReset=arr.slice(0,4);
      document.getElementById('shApply').click();
      return{onOpen,afterDrag,afterReset,painted,closed:ctxMenu.hidden};
    });
    S.ok('opening it already shows the shape',r.onOpen[0]!==.5||r.onOpen[3]!==.5,
      'lane reads '+r.onOpen.join(', '));
    S.ok('  moving a knob redraws immediately',r.afterDrag.join()!==r.onOpen.join(),
      r.onOpen.join(',')+' → '+r.afterDrag.join(','));
    S.ok('  the lane itself is repainted',r.painted);
    S.ok('  Reset restores what was there before it opened',r.afterReset.every(v=>v===.5),
      r.afterReset.join(', '));
    S.ok('  and Apply just closes it',r.closed);

    S.head('it respects a selected range');
    r=await page.evaluate(() => {
      const arr=song.patterns[0].auto['0.p.cutoff'];
      arr.fill(.5);
      autoSel={k:'0.p.cutoff',a:8,b:15};
      Object.assign(ASHAPE,{shape:'ramp',cycles:1,phase:0,lo:0,hi:1});
      laneShapeDlg('0.p.cutoff',10,10);
      document.getElementById('shApply').click();
      const out={before:arr.slice(0,8),inside:arr.slice(8,16),after:arr.slice(16,24)};
      autoSel=null;
      return out;
    });
    S.ok('rows outside the selection are untouched',
      r.before.every(v=>v===.5)&&r.after.every(v=>v===.5),
      'before '+r.before.join(',')+' | after '+r.after.join(','));
    S.ok('  and the selection got the shape',r.inside[0]<r.inside[7],
      'inside '+r.inside.join(','));

    S.head('level scaling');
    r=await page.evaluate(()=>{
      const arr=song.patterns[0].auto['0.p.cutoff'];
      const set=v=>{for(let i=0;i<arr.length;i++)arr[i]=v[i%v.length]};
      const runScale=(o)=>{
        set([.2,.8]);
        Object.assign(ASCALE,{mult:1,off:0,center:.5,tension:0},o);
        laneScaleDlg('0.p.cutoff',10,10);
        ['scM','scC','scO','scT'].forEach(id=>document.getElementById(id).dispatchEvent(new Event('input')));
        const before=arr.slice(0,4);
        document.getElementById('scApply').click();
        return before;
      };
      const flat=runScale({mult:0});
      const wide=runScale({mult:2});
      const up=runScale({off:.2});
      const reset=(()=>{
        set([.2,.8]);
        Object.assign(ASCALE,{mult:1,off:0,center:.5,tension:0});
        laneScaleDlg('0.p.cutoff',10,10);
        ASCALE.mult=2;document.getElementById('scM').dispatchEvent(new Event('input'));
        const changed=arr.slice(0,2);
        document.getElementById('scRst').click();
        const back=arr.slice(0,2);
        document.getElementById('scApply').click();
        return{changed,back};
      })();
      return{flat,wide,up,reset};
    });
    S.ok('multiply 0 collapses everything to the centre',r.flat.every(v=>Math.abs(v-.5)<.02),
      r.flat.join(', '));
    S.ok('  multiply 2 pushes it apart',Math.abs(r.wide[0]-r.wide[1])>.5,
      r.wide.slice(0,2).join(' / '));
    S.ok('  offset shifts the whole thing',Math.abs(r.up[0]-.4)<.03,
      '0.2 + 0.2 offset → '+r.up[0]);
    S.ok('  nothing escapes 0..1',r.wide.every(v=>v>=0&&v<=1),r.wide.join(', '));
    S.ok('Reset puts the lane back',r.reset.back[0]===.2&&r.reset.back[1]===.8,
      'changed to '+r.reset.changed.join('/')+', reset to '+r.reset.back.join('/'));

    S.head('hotkeys');
    await seed();
    await page.evaluate(()=>{ctxMenu.hidden=true;autoSel=null;document.body.focus()});
    await page.keyboard.down('Alt');await page.keyboard.press('KeyJ');await page.keyboard.up('Alt');
    r=await page.evaluate(()=>({open:!ctxMenu.hidden,shape:!!document.getElementById('shApply')}));
    S.ck('Alt+J opens the shape tool',[r.open,r.shape],[true,true]);
    await page.evaluate(()=>{ctxMenu.hidden=true});
    await page.keyboard.down('Alt');await page.keyboard.press('KeyU');await page.keyboard.up('Alt');
    r=await page.evaluate(()=>({open:!ctxMenu.hidden,scale:!!document.getElementById('scApply')}));
    S.ck('Alt+U opens the scale tool',[r.open,r.scale],[true,true]);
    await page.evaluate(()=>{ctxMenu.hidden=true});

    r=await page.evaluate(()=>{
      /* it should open the panel for you rather than doing nothing */
      if(!autoPan.hidden)autoToggle();
      const wasHidden=autoPan.hidden;
      laneTool('shape');
      const out={wasHidden,nowOpen:!autoPan.hidden,dialog:!ctxMenu.hidden};
      ctxMenu.hidden=true;
      /* and with no automation at all it should say so, not fail silently */
      song.patterns[0].auto={};renderAuto();
      laneTool('shape');
      out.noLanes={dialog:!ctxMenu.hidden,said:/no automation/i.test($id('status').textContent)};
      return out;
    });
    S.ck('it opens the lane panel if it is closed',[r.wasHidden,r.nowOpen,r.dialog],[true,true,true]);
    S.ok('  and with no lanes it tells you why',!r.noLanes.dialog&&r.noLanes.said);

    r=await page.evaluate(()=>{
      const src=document.documentElement.innerHTML;
      const bad=[];
      src.split('\n').forEach(l=>{
        if(/^\s*\/\*/.test(l))return;
        if(!/(^|[^!])e\.altKey/.test(l))return;
        [...l.matchAll(/e\.code==='Key([FEVSTHB])'/g)].forEach(m=>bad.push(m[1]));
      });
      return[...new Set(bad)];
    });
    S.ck('neither collides with a Firefox menu',r,[]);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
