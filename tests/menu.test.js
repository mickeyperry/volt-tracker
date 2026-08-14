/* The menu bar.
 *
 * VOLT has always been a palette-and-hotkeys program — fast once you know it, a locked door if you
 * don't. The menu bar exists so someone who has used a DAW before can find Preferences and the
 * audio device without being told.
 *
 * Every entry is an EXISTING command referenced by name, which buys one guarantee worth testing
 * above all others: **the menu cannot drift from the commands**. Rename a command and the entry
 * shows up greyed and labelled "(missing)" instead of quietly doing nothing — and this suite fails
 * with the name, which is the point.
 */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('menu bar');
  const{browser,page,errs}=await open();
  try{
    S.head('every entry points at a command that exists');
    let r=await page.evaluate(()=>{
      const named=[];
      MENUS.forEach(([t,es])=>es.forEach(e=>{if(e!=='-'&&e[0]!=='#')named.push([t,e])}));
      return{
        total:named.length,
        missing:named.filter(([t,e])=>!COMMANDS.some(c=>c.n===e)).map(([t,e])=>t+' ▸ '+e),
        /* the DOM is the real proof — a name could match and still not render */
        dead:[...document.querySelectorAll('#menubar .mitem.off')].map(e=>e.textContent),
        menus:[...document.querySelectorAll('#menubar .mtop')].map(e=>e.firstChild.textContent),
      };
    });
    S.ok('there are entries to check',r.total>=40,r.total+' of them');
    S.ck('  none of them names a command that is gone',r.missing,[]);
    S.ck('  and nothing renders as dead in the DOM',r.dead,[]);
    S.ok('the menus a DAW user looks for are there',
      ['File','Edit','View','Transport','Audio','Preferences'].every(m=>r.menus.indexOf(m)>=0),
      r.menus.join(' | '));

    S.head('desktop-only commands stay out of the browser build');
    r=await page.evaluate(()=>{
      const desk=COMMANDS.filter(c=>c.desk).map(c=>c.n.replace(/^(Native engine|Song folder): ?/,''));
      const shown=[...document.querySelectorAll('#menubar .mitem')].map(e=>e.firstChild.textContent);
      return{leaked:shown.filter(s=>desk.indexOf(s)>=0),tauri:TAURI};
    });
    S.ck('TAURI is null here, so none of them should show',r.tauri,null);
    S.ck('  and none does',r.leaked,[]);

    S.head('opening, closing, and not eating the keyboard');
    r=await page.evaluate(()=>{
      const top=[...document.querySelectorAll('#menubar .mtop')].find(e=>e.firstChild.textContent==='View');
      const fire=(el,t)=>el.dispatchEvent(new MouseEvent(t,{bubbles:true,cancelable:true}));
      fire(top,'mousedown');
      const opened=top.classList.contains('on');
      /* sliding across opens the next one — what makes it feel like a menu bar */
      const edit=[...document.querySelectorAll('#menubar .mtop')].find(e=>e.firstChild.textContent==='Edit');
      edit.dispatchEvent(new MouseEvent('mouseenter',{bubbles:false}));
      const slid=edit.classList.contains('on')&&!top.classList.contains('on');
      document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
      const closed=!document.querySelector('#menubar .mtop.on');
      return{opened,slid,closed};
    });
    S.ok('a click opens a menu',r.opened===true);
    S.ok('  sliding across opens the next one',r.slid===true);
    S.ok('  Escape closes it',r.closed===true);

    /* The bar sits above the grid and must not become a way to lose the keyboard. VOLT's hotkeys
       die the moment focus lands somewhere unexpected, so the mousedown handler preventDefaults. */
    r=await page.evaluate(()=>{
      const top=document.querySelector('#menubar .mtop');
      const ev=new MouseEvent('mousedown',{bubbles:true,cancelable:true});
      top.dispatchEvent(ev);
      const prevented=ev.defaultPrevented;
      menuClose();
      return{prevented,focus:document.activeElement===document.body||document.activeElement===null};
    });
    S.ok('clicking the bar does not steal focus from the grid',r.prevented===true);

    S.head('a menu item actually runs its command');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      const before=!mixer.hidden;
      const items=[...document.querySelectorAll('#menubar .mitem')];
      const it=items.find(e=>e.firstChild.textContent==='Toggle mixer');
      it.click();
      const after=!mixer.hidden;
      if(after!==before)it.click();                 /* put it back */
      return{before,after,closed:!document.querySelector('#menubar .mtop.on')};
    });
    S.ok('clicking "Toggle mixer" toggles the mixer',r.after!==r.before,r.before+' → '+r.after);
    S.ok('  and the menu closes behind it',r.closed===true);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
