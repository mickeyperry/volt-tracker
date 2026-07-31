/* Shared bits for the VOLT test suites.
   Everything here is deliberately dependency-light: the syntax check needs nothing at all, and the
   browser suites need only puppeteer-core plus a Chrome you already have installed. */
const fs=require('fs'),path=require('path');

const ROOT=path.join(__dirname,'..');
const FILE=process.env.VOLT_FILE||'beta.html';           // VOLT_FILE=volt.html to test the stable one
const PAGE='file:///'+path.join(ROOT,FILE).replace(/\\/g,'/');

/* ---- Chrome ---- */
const CHROME_CANDIDATES=[
  process.env.CHROME_PATH,
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
  '/usr/bin/google-chrome','/usr/bin/chromium','/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
].filter(Boolean);
function chromePath(){
  for(const p of CHROME_CANDIDATES)if(p&&fs.existsSync(p))return p;
  return null;
}
function puppeteer(){
  try{return require('puppeteer-core')}catch(e){
    try{return require(path.join(ROOT,'node_modules','puppeteer-core'))}catch(e2){return null}
  }
}

/* ---- tiny assertion helpers: every suite prints the same shape ---- */
function suite(name){
  const S={name,fails:0,checks:0};
  S.ck=(what,got,want)=>{
    S.checks++;
    const ok=JSON.stringify(got)===JSON.stringify(want);
    if(!ok)S.fails++;
    console.log('   '+(ok?'ok  ':'FAIL')+'  '+what.padEnd(46)+' '+JSON.stringify(got)+(ok?'':'   want '+JSON.stringify(want)));
  };
  S.ok=(what,cond,note)=>{
    S.checks++;
    if(!cond)S.fails++;
    console.log('   '+(cond?'ok  ':'FAIL')+'  '+what.padEnd(46)+' '+(note==null?'':note));
  };
  S.note=t=>console.log('        '+t);
  S.head=t=>console.log('\n  -- '+t);
  return S;
}

/* ---- browser boot: returns {browser,page,errs} with console/page errors collected ---- */
async function open(opts){
  const pp=puppeteer(),chrome=chromePath();
  if(!pp||!chrome)throw new Error('no browser');
  const browser=await pp.launch({executablePath:chrome,headless:'shell',
    args:['--no-sandbox','--autoplay-policy=no-user-gesture-required','--allow-file-access-from-files','--mute-audio']});
  const page=await browser.newPage();
  await page.setViewport({width:(opts&&opts.w)||1400,height:(opts&&opts.h)||800});
  const errs=[];
  page.on('pageerror',e=>errs.push('PAGEERROR: '+e.message));
  page.on('console',m=>{if(m.type()==='error')errs.push('CONSOLE: '+m.text())});
  await page.goto(PAGE,{waitUntil:'load'});
  await page.waitForFunction('typeof song!=="undefined" && !!song && !!song.patterns',{timeout:20000});
  return{browser,page,errs};
}

/* ---- render the song through VOLT's own offline path and find where notes actually start ----
   This is how the timing claims get proved instead of asserted: we listen to the output. */
const INSTALL_RENDER=()=>{
  const orig=URL.createObjectURL.bind(URL);
  URL.createObjectURL=b=>{window.__wavBlob=b;return orig(b)};
  HTMLAnchorElement.prototype.click=function(){};
  window.__onsets=async()=>{
    window.__wavBlob=null;
    await renderWav();
    const dv=new DataView(await window.__wavBlob.arrayBuffer());
    let off=12,fmt={},data=null;
    while(off<dv.byteLength-8){
      const id=String.fromCharCode(dv.getUint8(off),dv.getUint8(off+1),dv.getUint8(off+2),dv.getUint8(off+3));
      const sz=dv.getUint32(off+4,true);
      if(id==='fmt '){fmt.ch=dv.getUint16(off+10,true);fmt.sr=dv.getUint32(off+12,true);fmt.bits=dv.getUint16(off+22,true)}
      if(id==='data'){data={off:off+8,sz};break}
      off+=8+sz+(sz&1);
    }
    const bytes=fmt.bits/8,frames=Math.floor(data.sz/(bytes*fmt.ch)),win=Math.round(fmt.sr*.004),env=[];
    for(let i=0;i+win<frames;i+=win){
      let s=0;
      for(let j=0;j<win;j++){const p=data.off+(i+j)*bytes*fmt.ch;s+=Math.pow(dv.getInt16(p,true)/32768,2)}
      env.push(Math.sqrt(s/win));
    }
    const peak=Math.max(...env),on=[];let last=-99;
    for(let i=2;i<env.length;i++){
      const rise=env[i]-Math.max(env[i-1],env[i-2]);
      if(env[i]>peak*.16&&rise>peak*.09&&(i-last)>3){on.push(+(i*win/fmt.sr).toFixed(4));last=i}
    }
    return on;
  };
};

module.exports={ROOT,FILE,PAGE,suite,open,chromePath,puppeteer,INSTALL_RENDER};
