#!/usr/bin/env node
/* VOLT test runner —  node tests/run.js  [name ...]
   Runs the syntax check always (no dependencies). The browser suites need puppeteer-core and a
   Chrome; if either is missing they're skipped loudly rather than failing the run, so the syntax
   check still guards every commit on any machine.

     node tests/run.js                 everything
     node tests/run.js arp lpb         just those
     VOLT_FILE=volt.html node tests/run.js    test the stable file instead of beta
     CHROME_PATH=... node tests/run.js        if Chrome lives somewhere unusual                */
const{FILE,chromePath,puppeteer}=require('./lib');

const SUITES={
  syntax:require('./syntax.test'),
  lpb:require('./lpb.test'),
  arp:require('./arp.test'),
  store:require('./store.test'),
  duck:require('./duck.test'),
  vault:require('./vault.test'),
  home:require('./home.test'),
  swing:require('./swing.test'),
  spectrum:require('./spectrum.test'),
  sections:require('./sections.test'),
  disable:require('./disable.test'),
  bounce:require('./bounce.test'),
  roll:require('./roll.test'),
  glitch:require('./glitch.test'),
  master:require('./master.test'),
  metsync:require('./metsync.test'),
  autolane:require('./autolane.test'),
  ui:require('./ui.test'),
  perf:require('./perf.test')
};

(async()=>{
  const want=process.argv.slice(2).filter(a=>a[0]!=='-');
  const names=want.length?want.filter(n=>SUITES[n]):Object.keys(SUITES);
  if(want.length&&names.length!==want.length)
    console.log('unknown suite(s): '+want.filter(n=>!SUITES[n]).join(', ')+' — have: '+Object.keys(SUITES).join(', '));

  const browserOk=!!(puppeteer()&&chromePath());
  console.log('VOLT tests · '+FILE+(browserOk?'':'  (browser suites will be SKIPPED)'));
  if(!browserOk){
    console.log('  no headless browser: install one with  npm i -D puppeteer-core');
    console.log('  and make sure Chrome is installed (or set CHROME_PATH).');
  }

  let fails=0,checks=0,ran=0,skipped=[];
  for(const name of names){
    if(name!=='syntax'&&!browserOk){skipped.push(name);continue}
    console.log('\n['+name+']');
    try{
      const S=await SUITES[name]();
      fails+=S.fails;checks+=S.checks;ran++;
    }catch(e){
      fails++;
      console.log('   FAIL  suite crashed: '+(e&&e.message));
      if(e&&e.stack)console.log(e.stack.split('\n').slice(1,4).join('\n'));
    }
  }
  console.log('\n────────────────────────────────');
  console.log(ran+' suite(s), '+checks+' checks, '+fails+' failing'+(skipped.length?'  ·  skipped: '+skipped.join(', '):''));
  process.exit(fails?1:0);
})();
