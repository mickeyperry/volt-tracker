/* The cheapest, most important test there is: does the file still parse?
   No browser, no dependencies — this is the one that must never be skipped, because a syntax error
   published to GitHub Pages means a blank page for everyone. */
const fs=require('fs'),path=require('path');
const{ROOT,suite}=require('./lib');

module.exports=async function(){
  const S=suite('syntax');
  /* every published file, not just the one under test — a syntax error in any of them is a blank
     page for whoever opens that URL */
  for(const file of ['volt.html','beta.html','beta2.html']){
    const p=path.join(ROOT,file);
    if(!fs.existsSync(p)){S.note(file+' — not present, skipped');continue}
    const html=fs.readFileSync(p,'utf8');
    const blocks=[...html.matchAll(/<script(?![^>]*src)([^>]*)>([\s\S]*?)<\/script>/g)];
    S.ok(file+': has an inline script',blocks.length>0,blocks.length+' block(s)');
    let bad=0,checked=0;
    blocks.forEach((b,i)=>{
      const attrs=b[1]||'',body=b[2];
      if(/type\s*=\s*["'](?!text\/javascript|module)/i.test(attrs)){ /* baked-in song JSON etc. */
        try{JSON.parse(body)}catch(e){bad++;S.note(file+' block '+i+' (data): bad JSON — '+e.message)}
        return;
      }
      checked++;
      try{new Function(body)}catch(e){bad++;S.note(file+' block '+i+': '+e.message)}
    });
    S.ok(file+': every script parses',bad===0,checked+' script block(s) checked');
    /* things that must exist for the app to boot at all */
    ['id="status"','id="gridwrap"','id="grows"','id="lpbSel"'].forEach(needle=>{
      if(file==='volt.html'&&needle==='id="lpbSel"')return; /* stable may lag behind beta */
      S.ok(file+': contains '+needle,html.indexOf(needle)>=0);
    });
  }
  return S;
};
