/* The `.volt` package — a song and its samples in one file.
 *
 * A song used to travel as JSON with every sample inlined as base64: correct, but a third bigger
 * than the audio it carries and one enormous line of text. A package is a zip — `song.json` plus
 * `samples/…` — written with no library, because VOLT takes no runtime dependencies.
 *
 * What has to hold, in order of how much it would hurt to get wrong:
 *   1. a sample survives the round trip BYTE FOR BYTE (it is somebody's audio),
 *   2. an old plain-JSON song still opens — "don't break old songs" is the house rule,
 *   3. the file really is a zip, so Explorer, 7-Zip and anything else can open it,
 *   4. it is smaller than the JSON it replaces.
 */
const{suite,open}=require('./lib');

module.exports=async function(){
  const S=suite('.volt package');
  const{browser,page,errs}=await open();
  try{
    S.head('a song with samples packs and comes back unchanged');
    let r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      /* a "sample" of known bytes — every value 0..255, so a single mangled byte shows up */
      const raw=new Uint8Array(256);for(let i=0;i<256;i++)raw[i]=i;
      const b64=abToB64(raw.buffer);
      song.name='pack test';
      song.instruments=[
        Object.assign(mkInst('sampler','Zap'),{sampleB64:b64,sampleName:'zap.wav'}),
        mkInst('kick','K',{pitch:50,punch:.6,decay:.4,drive:.3}),
      ];
      song.patterns[curPat].data[0][0]={n:60,i:0,v:64,c:null,x:0};
      song.patterns[curPat].data[1][4]={n:48,i:1,v:64,c:null,x:0};

      const bytes=await packSong();
      const asJson=serialize(true).length;
      /* it must BE a zip: PK\x03\x04 */
      const magic=[...bytes.slice(0,4)].join(',');
      const names=[...(await zipUnpack(bytes)).keys()].sort();

      const back=await songFromFile(bytes);
      const gotB64=back.instruments[0].sampleB64;
      const gotRaw=new Uint8Array(b64ToAb(gotB64));
      let same=gotRaw.length===raw.length;
      if(same)for(let i=0;i<raw.length;i++)if(gotRaw[i]!==raw[i]){same=false;break}

      return{
        magic,names,packed:bytes.length,asJson,
        sampleIntact:same,
        noPackLeftover:back.instruments[0].pack===undefined,
        name:back.name,
        note:back.patterns[0].data[0][0].n,
        kick:back.patterns[0].data[1][4].n,
        insts:back.instruments.length,
        secondHasNoSample:back.instruments[1].sampleB64===undefined,
      };
    });
    S.ck('it is a real zip',r.magic,'80,75,3,4');
    S.ck('  holding the song, the sample and a readme',r.names,
      ['README.txt','samples/00_zap.wav','song.json']);
    S.ok('the sample survives byte for byte',r.sampleIntact);
    S.ok('  and the packing detail is gone from the song',r.noPackLeftover);
    S.ck('the song comes back whole',[r.name,r.note,r.kick,r.insts],['pack test',60,48,2]);
    S.ok('  an instrument with no sample stays that way',r.secondHasNoSample);
    S.ok('the package is smaller than the JSON it replaces',r.packed<r.asJson,
      r.packed+' vs '+r.asJson+' bytes');

    S.head('an old plain-JSON song still opens — the rule that matters most');
    r=await page.evaluate(async()=>{
      await loadSong(blankSong());
      song.name='old school';
      const raw=new Uint8Array([1,2,3,4,5,6,7,8]);
      song.instruments=[Object.assign(mkInst('sampler','Old'),
        {sampleB64:abToB64(raw.buffer),sampleName:'old.wav'})];
      song.patterns[curPat].data[0][2]={n:55,i:0,v:32,c:null,x:0};
      /* exactly what the old export wrote: JSON text, samples inline */
      const json=serialize(true);
      const bytes=new TextEncoder().encode(json);
      const back=await songFromFile(bytes);
      return{name:back.name,note:back.patterns[0].data[0][2].n,
             b64:back.instruments[0].sampleB64,name0:back.instruments[0].sampleName};
    });
    S.ck('it loads by the same door',[r.name,r.note,r.name0],['old school',55,'old.wav']);
    S.ok('  with its sample still attached',!!r.b64);

    S.head('a damaged file is refused, not half-loaded');
    r=await page.evaluate(async()=>{
      const junk=new Uint8Array([0x50,0x4b,0x03,0x04,9,9,9,9,9,9]);   /* claims to be a zip */
      let msg='';
      try{await songFromFile(junk)}catch(e){msg=(e&&e.message)||String(e)}
      let msg2='';
      try{await songFromFile(new TextEncoder().encode('{not json'))}catch(e){msg2='refused'}
      return{msg,msg2};
    });
    S.ok('a truncated package says so',/package|zip/i.test(r.msg),r.msg);
    S.ck('  and broken JSON is still an error, not a blank song',r.msg2,'refused');

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
