/* Dropping audio in from OUTSIDE VOLT — Explorer, Everything, any app that hands over files.
   Two things have to hold, and the second is the one that broke in the desktop build:

     1. a file dropped on a grid cell becomes an instrument AND writes a note in that cell,
     2. …even when the drop event does not arrive at the grid at all.

   (2) is not hypothetical. An OS drag is synthesised by the webview, not by the page, and in
   WebView2 the dragovers tracked the pointer correctly — the cell lit up all the way down — and
   then the drop landed on the document instead. The grid's own handler never ran, so the sample
   was added to the instrument list and no note was ever written: "it highlited the location in
   the main window but didnt put the note, only add the sample to the panel of insts"
   (2026-08-15). The fix is that BOTH handlers place notes, and both work out where the drop
   landed from the pointer or from the cell the last dragover lit up — never from e.target alone.

   The wav below is real (44-byte header + 8-bit PCM) because decodeAudioData has to succeed:
   a file that fails to decode is skipped, which would make this suite pass while proving
   nothing. */
const{suite,open}=require('./lib');

const MKWAV=`(name)=>{
  const n=800,hdr=44,buf=new ArrayBuffer(hdr+n),v=new DataView(buf);
  const wr=(o,s)=>{for(let i=0;i<s.length;i++)v.setUint8(o+i,s.charCodeAt(i))};
  wr(0,'RIFF');v.setUint32(4,36+n,true);wr(8,'WAVEfmt ');v.setUint32(16,16,true);
  v.setUint16(20,1,true);v.setUint16(22,1,true);v.setUint32(24,8000,true);
  v.setUint32(28,8000,true);v.setUint16(32,1,true);v.setUint16(34,8,true);
  wr(36,'data');v.setUint32(40,n,true);
  for(let i=0;i<n;i++)v.setUint8(hdr+i,128+Math.round(60*Math.sin(i/8)));
  return new File([buf],name,{type:'audio/wav'});
}`;

module.exports=async function(){
  const S=suite('drop / samples from outside');
  const{browser,page,errs}=await open();
  try{
    S.head('a sample dropped on a grid cell lands as a note');
    let r=await page.evaluate(async(mkSrc)=>{
      const mkWav=eval(mkSrc);
      await loadSong(blankSong());
      audio();
      const before=song.instruments.length;
      /* row 4 of track 1, chosen so neither coordinate is zero and a wrong one is obvious */
      const row=grows.children[4],cell=row.querySelector('.tcell');
      const dt=new DataTransfer();dt.items.add(mkWav('zap.wav'));
      const ev=t=>new DragEvent(t,{dataTransfer:dt,bubbles:true,cancelable:true});
      cell.dispatchEvent(ev('dragover'));
      const lit=!!document.querySelector('.dropCell');
      cell.dispatchEvent(ev('drop'));
      await new Promise(r=>setTimeout(r,600));
      const c=song.patterns[curPat].data[+cell.dataset.t][4];
      return{lit,added:song.instruments.length-before,note:c&&c.n,
             instOfNote:c&&c.i,isSampler:!!(c&&song.instruments[c.i]&&song.instruments[c.i].type==='sampler'),
             hasBuffer:!!(c&&song.instruments[c.i]&&song.instruments[c.i].buffer),
             cleared:!document.querySelector('.dropCell')};
    },MKWAV);
    S.ok('the cell lights up while the file is over it',r.lit);
    S.ck('  one instrument is added',r.added,1);
    S.ck('  and a C-4 lands in that exact cell',r.note,48);
    S.ok('  pointing at the new sampler, decoded',r.isSampler&&r.hasBuffer,
      'sampler:'+r.isSampler+' buffer:'+r.hasBuffer);
    S.ok('  and the highlight is cleared afterwards',r.cleared);

    S.head('the same drop delivered to the DOCUMENT still writes the note (WebView2)');
    r=await page.evaluate(async(mkSrc)=>{
      const mkWav=eval(mkSrc);
      await loadSong(blankSong());
      audio();
      const before=song.instruments.length;
      const row=grows.children[6],cell=row.querySelector('.tcell');
      const dt=new DataTransfer();dt.items.add(mkWav('boom.wav'));
      /* the pointer really is over the cell — the dragovers say so and light it up… */
      const b=cell.getBoundingClientRect();
      const x=Math.round(b.left+b.width/2),y=Math.round(b.top+b.height/2);
      cell.dispatchEvent(new DragEvent('dragover',{dataTransfer:dt,bubbles:true,cancelable:true,clientX:x,clientY:y}));
      const lit=!!document.querySelector('.dropCell');
      /* …and then the drop arrives bare, on the document, with no coordinates at all — which is
         exactly the shape of the failure in the desktop build */
      document.dispatchEvent(new DragEvent('drop',{dataTransfer:dt,bubbles:true,cancelable:true}));
      await new Promise(r=>setTimeout(r,600));
      const c=song.patterns[curPat].data[+cell.dataset.t][6];
      return{lit,added:song.instruments.length-before,note:c&&c.n,
             named:c!=null&&!!song.instruments[c.i]&&song.instruments[c.i].name==='boom'};
    },MKWAV);
    S.ok('the cell was lit by the dragover',r.lit);
    S.ck('  the instrument is still added exactly once',r.added,1);
    S.ck('  and the note lands in the lit cell, not nowhere',r.note,48);
    S.ok('  from the file that was dropped',r.named);

    S.head('a drop that misses the grid adds the instrument and no note');
    r=await page.evaluate(async(mkSrc)=>{
      const mkWav=eval(mkSrc);
      await loadSong(blankSong());
      audio();
      const before=song.instruments.length;
      const pat=song.patterns[curPat];
      const notesBefore=pat.data.reduce((n,tr)=>n+tr.filter(Boolean).length,0);
      const dt=new DataTransfer();dt.items.add(mkWav('stray.wav'));
      /* over the grid first (so a highlight exists), then away and dropped on the toolbar —
         the stale highlight must NOT be taken as "put a note here" */
      const cell=grows.children[2].querySelector('.tcell');
      const b=cell.getBoundingClientRect();
      cell.dispatchEvent(new DragEvent('dragover',{dataTransfer:dt,bubbles:true,cancelable:true,
        clientX:Math.round(b.left+b.width/2),clientY:Math.round(b.top+b.height/2)}));
      const top=document.getElementById('top'),tb=top.getBoundingClientRect();
      const tx=Math.round(tb.left+tb.width/2),ty=Math.round(tb.top+tb.height/2);
      top.dispatchEvent(new DragEvent('dragover',{dataTransfer:dt,bubbles:true,cancelable:true,clientX:tx,clientY:ty}));
      const clearedOnLeave=!document.querySelector('.dropCell');
      top.dispatchEvent(new DragEvent('drop',{dataTransfer:dt,bubbles:true,cancelable:true,clientX:tx,clientY:ty}));
      await new Promise(r=>setTimeout(r,600));
      const notesAfter=song.patterns[curPat].data.reduce((n,tr)=>n+tr.filter(Boolean).length,0);
      return{clearedOnLeave,added:song.instruments.length-before,wrote:notesAfter-notesBefore};
    },MKWAV);
    S.ok('leaving the grid clears the highlight',r.clearedOnLeave);
    S.ck('  the sample is still added',r.added,1);
    S.ck('  and nothing is written to the pattern',r.wrote,0);

    S.ok('no console errors',errs.length===0,errs.join(' | '));
  }finally{await browser.close()}
  return S;
};
