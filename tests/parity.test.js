/* Parity — does the Rust engine make the same sound as the browser?
   =================================================================
   This is the suite the whole native port hangs off. Every instrument in native/engine is a
   transliteration of the matching case in beta2.html's trig(), and the only honest way to know a
   transliteration worked is to render the same note through both engines and subtract one from
   the other. What's left is the error, in dB. Nothing here is judged by ear.

   Both sides render the same bare graph — voice → destination — with no channel strip, no master
   FX and no compressor, so a wrong instrument shows up on its own instead of hiding underneath
   two compressors and a limiter.

   Needs the Rust binary:
     cd native/engine && cargo build --release
   If it isn't built the suite skips rather than fails, so `node tests/run.js` still works on a
   machine with no Rust. */
const fs=require('fs'),path=require('path'),{execFileSync}=require('child_process');
const {ROOT,suite,open}=require('./lib.js');

const BIN=path.join(ROOT,'native','engine','target','release','render.exe');
const TMP=path.join(ROOT,'tests','.parity');

/* ---- QUANTUM: why every note starts at 2048 samples and not at a round 0.05 s ----------------
   Chrome reads an oscillator's frequency automation from the start of the 128-sample render
   quantum, not from the sample the note actually starts on. Start a note 29 samples into a
   quantum and its first 29 samples come out at the *previous* frequency, after which Chrome
   catches up — measured, samples 2205-2234 of probe-linfreq. It is inaudible (0.7 ms at the very
   top of a note) but it shifts the phase of everything that follows, so a sample-exact null test
   against a mid-quantum note can never pass however correct the port is. Started on a boundary,
   the same probe nulls at -63.5 dB.

   So: the parity suite schedules on quantum boundaries, and that is a statement about the
   measurement, not about VOLT. Real notes land wherever the row lands, and this quirk rides along
   with them in the browser exactly as it always has.

   ---- SETTARGET: the other one -----------------------------------------------------------------
   Chrome also begins a setTargetAtTime decay one render block EARLY. For the whole 128-sample
   quantum leading up to the event it multiplies the parameter by exp(-(T0 - blockStart) / tc),
   where the spec says the value should simply hold. Measured on probe-kickenv-aligned: a flat
   1.154% dip across samples 2176-2303, and exp(-128/(0.25 x 44100)) = 0.98846 predicts it to five
   digits. Worth 0.1 dB for 2.9 ms, once per envelope — inaudible, and the single biggest term in
   what's left of the drums' residue.

   ---- what this means for the bar ---------------------------------------------------------------
   Sample-exact parity with Chrome is not achievable, because two of Chrome's own AudioParam
   behaviours don't follow the spec and Firefox has a different set again. There is no single
   "the browser" to null against. So the bar is RMS of the residue, which is energy-weighted and
   doesn't let one sample at a note edge condemn a correct port; peak is printed beside it as
   information. Everything the spec does pin down — filters, buffers, the seeded noise, gains
   under steady automation — is expected to be near-exact, and measures between -85 and -990 dB.

   Nothing here is loosened to make a port pass. Where a difference is real and understood it is
   listed as a `known` gap with the number it currently measures, so it stays visible and cannot
   silently get worse.                                                                          */
const T=2048/44100;

/* The notes under test. Params are the instrument defaults from ITYPES, spelled out here rather
   than read from the page: if a default ever changes, this suite should keep testing the case it
   was written against until someone deliberately updates it. */
const JOBS=[
  /* ---- instruments ---------------------------------------------------------------------------
     The residue on the drums is dominated by SETTARGET below, not by their DSP: the kick's body
     nulls at better than -100 dB and only its first 12 ms carry any error at all. */
  {instr:'kick', note:48, vel:.8, dur:1.5, sr:44100, t:T, maxRms:-55,
   why:'attack only; body nulls below -100 dB. See SETTARGET',
   params:{pitch:50,punch:.6,decay:.4,drive:.3,fcut:18000,fres:0}},
  {instr:'kick', note:36, vel:1,  dur:1.5, sr:44100, t:T,   /* drive 0 takes the other branch */
   params:{pitch:50,punch:1,decay:.8,drive:0,fcut:18000,fres:0}},
  {instr:'snare',note:48, vel:.9, dur:1.2, sr:44100, t:T,
   why:'body is a triangle — carries part of the TRIANGLE gap',
   params:{tone:190,snap:.7,decay:.16,fcut:18000,fres:0}},
  {instr:'hat',  note:48, vel:.7, dur:.8,  sr:44100, t:T,
   params:{tone:7500,decay:.06,fcut:18000,fres:0}},
  {instr:'clap', note:48, vel:.8, dur:1.2, sr:44100, t:T,
   params:{tone:1150,decay:.22,fcut:18000,fres:0}},
  {instr:'sub',  note:36, vel:.8, dur:1.0, sr:44100, t:T,
   params:{harm:.15,rel:.06,fcut:18000,fres:0}},
  /* reese — Mickey's actual dnb5 preset, fractional voices and all. The saws ride the known
     band-limited-table gap (see WAVEFORMS below), so the bar is where the saw probes sit, not
     the exact-null the filters manage; what this job is FOR is everything else — voice count,
     the detune spread, the LFO'd cutoff, drive, the sub layer, levels. */
  {instr:'reese',note:24, vel:.8, dur:1.5, sr:44100, t:T, maxRms:-25,
   why:'saws are band-limited tables, not Chrome\'s crossfade — see WAVEFORMS',
   params:{voices:1.867,detune:49.98,cutoff:147.28,res:0,lfoRate:.35,lfoAmt:.35,
           drive:.576,sub:.027,rel:.09,fcut:18000,fres:0}},
  /* Mickey's ACTUAL reese, at the notes he actually writes. The dnb5 job above plays note 24;
     his dnb8 bassline sits on D-1/F-1/G-1 — notes 14 to 19, a fifth to an octave LOWER — and
     that is where he hears distortion on every note with the channel's plugin bypassed
     (2026-08-14, annotated screenshot: "dist" on each note, "ok" in the gaps). Low notes are
     exactly where the band-limited saw tables are furthest from Chrome's (see WAVEFORMS and the
     known reese gap in ROADMAP), so these jobs measure the thing he is listening to instead of
     a note nobody plays. Params are read off his instrument panel. */
  {instr:'reese',note:14, vel:.8, dur:1.2, sr:44100, t:T, maxRms:-20,
   why:'his D-1 — the lowest note in the bassline',
   params:{voices:1.84,detune:50,cutoff:147,res:0,lfoRate:.35,lfoAmt:.35,
           drive:.14,sub:0,rel:.09,fcut:18000,fres:0}},
  {instr:'reese',note:19, vel:.8, dur:1.2, sr:44100, t:T, maxRms:-20,
   why:'his G-1',
   params:{voices:1.84,detune:50,cutoff:147,res:0,lfoRate:.35,lfoAmt:.35,
           drive:.14,sub:0,rel:.09,fcut:18000,fres:0}},

  /* ---- the 2026-08-14 ports: the one-engine push (see ROADMAP P3) -------------------------
     All four are saw-based, so they inherit the WAVEFORMS gap exactly as reese does; the bars
     sit where the saw probes sit. What these jobs prove is everything else: envelopes, filter
     snaps, the drive shapers, the psybass sub layer, supersaw's detune fan and pan spread. */
  {instr:'supersaw',note:60,vel:.8,dur:1.5,sr:44100,t:T,maxRms:-25,
   why:'7 band-limited-table saws — the WAVEFORMS gap times the stack. Measured -27.2 dB rms, level +0.34 dB (2026-08-14)',
   params:{detune:28,spread:.85,hp:160,cutoff:7000,att:.01,rel:.25,fcut:18000,fres:0}},
  {instr:'psybass',note:31,vel:.9,dur:1.0,sr:44100,t:T,maxRms:-45,
   why:'the shaper flattens the saw-table gap. Measured -53.7 dB rms, level exact (2026-08-14)',
   params:{cutoff:420,envAmt:.5,res:2,decay:.16,drive:.5,sub:.6,fcut:18000,fres:0}},
  {instr:'pluck',note:60,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-25,
   why:'two detuned saws — WAVEFORMS gap. Measured -28.2 dB rms, level +0.32 dB (2026-08-14)',
   params:{cutoff:800,envAmt:.6,decay:.3,res:3,fcut:18000,fres:0}},
  {instr:'acid303',note:36,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-35,
   why:'hot resonant filter then the shaper. Measured -44.6 dB rms, level exact (2026-08-14)',
   params:{wave:0,cutoff:700,res:14,envAmt:.65,decay:.2,drive:.4,rel:.05,fcut:18000,fres:0}},
  {instr:'acid303',note:36,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-35,   /* the square branch */
   why:'square wave branch. Measured -41.8 dB rms, level exact (2026-08-14)',
   params:{wave:1,cutoff:700,res:14,envAmt:.65,decay:.2,drive:.4,rel:.05,fcut:18000,fres:0}},
  {instr:'mutant',note:36,vel:.8,dur:1.5,sr:44100,t:T,maxRms:-20,
   why:'FM through a wavefolder multiplies the saw+square table gap. Measured -27.1 dB rms, level -0.06 dB (2026-08-14)',
   params:{fm:.4,ratio:2,fold:.5,sub:.5,cutoff:600,res:10,lfoRate:1,lfoAmt:.5,lfoShape:0,rel:.1,fcut:18000,fres:0}},
  /* glitch: all four source modes, because they are four separate code paths sharing only the
     grain loop. Deterministic by SEED — same note+vel scatters the same way in both engines,
     which is what makes a null test of a "random" instrument possible at all. A wrong rnd()
     call order fails these instantly, by design. */
  {instr:'glitch',note:48,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-40,
   why:'src 0, noise ticks — seeded noise is shared, so this is mostly the grain envelopes',
   params:{src:0,pitch:700,sweep:-.55,hits:6,gap:.05,decay:.05,fall:.3,chaos:.5,accel:0,crush:0,cutoff:2400,res:8,rel:.05,fcut:18000,fres:0}},
  /* srcs 1-3 run with chaos 0, accel 0 and a gap that is an exact number of 128-sample quanta
     (2560/44100), so every grain lands on a quantum boundary. NOT a softening of the bar — the
     QUANTUM note at the top of this file: these modes re-tune an oscillator at every grain, and
     a mid-quantum retune starts Chrome's phase a fraction of a block away from ours. Jittered
     grains therefore cannot null however correct the port is (measured: level matches to
     0.07 dB while phase decorrelates to ~0 dB residue). The chaotic path — seed, rnd order,
     jittered grain envelopes — is proven by the src 0 job above, whose noise source has no
     oscillator phase to lose. */
  {instr:'glitch',note:48,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-40,
   why:'src 1, FM metal — grains on quantum boundaries, see the note above. Measured -74.3 dB rms (2026-08-14)',
   params:{src:1,pitch:700,sweep:.3,hits:5,gap:2560/44100,decay:.06,fall:.2,chaos:0,accel:0,crush:0,cutoff:3000,res:6,rel:.05,fcut:18000,fres:0}},
  {instr:'glitch',note:48,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-20,
   why:'src 2, pulse — square+saw sweeps CROSS octave-table buckets, where Chrome crossfades and we switch (see WAVEFORMS). Measured -22.5 dB rms, level +0.03 dB (2026-08-14)',
   params:{src:2,pitch:400,sweep:.5,hits:5,gap:2560/44100,decay:.05,fall:.25,chaos:0,accel:0,crush:.3,cutoff:2000,res:14,rel:.05,fcut:18000,fres:0}},
  {instr:'glitch',note:48,vel:.8,dur:1.2,sr:44100,t:T,maxRms:-40,
   why:'src 3, sine zaps through the crusher. Measured -47.7 dB rms, level exact (2026-08-14)',
   params:{src:3,pitch:900,sweep:.6,hits:7,gap:2560/44100,decay:.04,fall:.3,chaos:0,accel:0,crush:.5,cutoff:5000,res:4,rel:.05,fcut:18000,fres:0}},

  /* ---- probes: one primitive each, with a twin in native/engine's trig() ----------------------
     These exist so a failure names the node that differs rather than the instrument containing it.
     Every one of them earned its place by catching something. */
  /* ---- sampler ----------------------------------------------------------------------------
     Both engines build the same seeded-noise buffer rather than loading a file, so what is being
     compared is the playback, not two WAV decoders. Three cases, because the ways this breaks are
     independent: at the root note nothing is resampled, above it the playback rate is exercised,
     and with a trim region the start offset and the fade-out are — and `dur/rt` is only wrong when
     BOTH a rate and a fade are in play, which is what the third one is for. */
  {instr:'sampler',note:48,vel:.8,dur:1.0,sr:44100,t:T,maxRms:-60,
   params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:1,fin:0,fout:0,fcut:18000,fres:0}},
  {instr:'sampler',note:60,vel:.8,dur:1.0,sr:44100,t:T,maxRms:-55,   /* an octave up: rate = 2 */
   params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:1,fin:0,fout:0,fcut:18000,fres:0}},
  {instr:'sampler',note:55,vel:.9,dur:1.0,sr:44100,t:T,maxRms:-55,   /* trimmed, faded, resampled */
   params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:.8,fin:.005,fout:.05,
           fcut:18000,fres:0,s0:.25,s1:.85,off:.1}},

  /* Bxx — reversed, and reversed WITH a trim region, which is where it goes wrong: the region has
     to mirror with the audio or you hear the wrong part of the sample played backwards. */
  {instr:'sampler',note:48,vel:.8,dur:1.0,sr:44100,t:T,rev:true,maxRms:-60,
   params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:1,fin:0,fout:0,fcut:18000,fres:0}},
  {instr:'sampler',note:48,vel:.8,dur:1.0,sr:44100,t:T,rev:true,maxRms:-60,
   params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:1,fin:0,fout:0,fcut:18000,fres:0,
           s0:.2,s1:.7}},

  /* sliced — a chopped break. Note 50 picks slice 2 of 4; note 99 must clamp to the last one
     rather than reading off the end, which is the failure that would crash a live set. */
  {instr:'sliced',note:50,vel:.9,dur:1.0,sr:44100,t:T,maxRms:-60,
   params:{sampleSeed:0xB0B,sampleLen:22050,rate:1,rel:.04,fcut:18000,fres:0,
           sliceBase:48,sliceCount:4,
           slice0s:0,slice0e:.25,slice1s:.25,slice1e:.5,slice2s:.5,slice2e:.75,slice3s:.75,slice3e:1}},
  /* An out-of-range note (99, i.e. slice index 51 against a 4-slice kit) is NOT tested here, and
     that is a deliberate omission with an open bug behind it — see ROADMAP. Rust clamps to the
     last slice and its render is byte-identical to asking for that slice directly; the browser
     produces something that matches neither the last slice nor its neighbour. Rust looks right
     and the browser looks wrong, which is exactly the case where a null test would be asserting
     the wrong thing. */

  {instr:'probe-bandpass',note:0,vel:1,dur:1.0,sr:44100,t:T,params:{}},   /* biquad: exact */
  {instr:'probe-expgain', note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},   /* exp ramp: exact */
  {instr:'probe-expramp', note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},
  {instr:'probe-linfreq', note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},
  {instr:'probe-noiseoffset',note:0,vel:1,dur:.6,sr:44100,t:T,params:{}}, /* off-grid start: exact */
  {instr:'probe-fasttarget', note:0,vel:1,dur:.3,sr:44100,t:T,params:{}}, /* 4 ms decay: bit-identical */
  {instr:'probe-shaper',     note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},/* 2x oversample: exact */
  /* A plain stop nulls at -92 dB, so stopping is not the problem. It was the shaper AT a stop:
     the oversampler's tail. Kept as the control that says so. */
  {instr:'probe-stopedge',note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},
  {instr:'probe-kicknoise',  note:0,vel:1,dur:.3,sr:44100,t:T,maxRms:-50,
   why:'see SETTARGET',params:{}},
  {instr:'probe-kicksine',   note:0,vel:1,dur:1.5,sr:44100,t:T,params:{}},
  {instr:'probe-kickenv',    note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   why:'see SETTARGET'},

  /* ---- the three known deviations, each pinned to a number so it cannot quietly get worse ---- */
  /* WAVEFORMS. Saw and square go through band-limited tables built from the spec's Fourier
     series (see harmonics() in the engine); the triangle deliberately does not, because the
     crate's built-in beats a table at both ends of the keyboard. Every number below is what it
     currently measures — they are pinned so a change to the oscillator can't quietly regress one
     while improving another, which is exactly what happened twice while getting here. */
  {instr:'probe-triangle',note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'triangle @190 Hz — built-in, beats a table here',known_at:-64},
  {instr:'probe-triangle-hi',note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'triangle @1760 Hz — built-in',known_at:-36},
  {instr:'probe-saw',       note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'saw @110 Hz — band-limited table (was -15 dB)',known_at:-34},
  {instr:'probe-square',    note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'square @110 Hz — band-limited table (was -15 dB)',known_at:-36},
  {instr:'probe-square-hi', note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'square @1760 Hz — band-limited table (was -15 dB)',known_at:-42},
  /* The worst case left. Chrome crossfades between neighbouring octave tables and we pick one,
     so the top harmonics differ; it is the least audible part of the spectrum and browsers
     disagree there anyway. Closing it means replicating Chrome's crossfade. */
  {instr:'probe-saw-hi',    note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'saw @1760 Hz — worst case, Chrome crossfades octave tables',known_at:-20},
  {instr:'probe-sine-hi',   note:0,vel:1,dur:1.2,sr:44100,t:T,params:{}},  /* control: sine is exact */
  {instr:'probe-kickenv-aligned',note:0,vel:1,dur:1.2,sr:44100,t:T,params:{},
   known:'Chrome starts a setTarget decay one block early (SETTARGET)',known_at:-54},

  /* ---- THE COMPRESSOR, and the reason the strip below does not null ---------------------------
     Blink's DynamicsCompressor is not the spec's. The spec describes a gain computer and a
     branching peak detector (the crate implements exactly that); Chrome ships an older algorithm
     with a numerically-solved knee and its own detector, and the two disagree in a way no amount of
     care in the port can fix — starting with the fact that CHROME DOES NOT PASS RATIO 1:1 THROUGH.
     At the settings a channel gets with its compressor knob at zero, Chrome's gain wanders between
     0.93 and 1.00 with the envelope where any textbook compressor would sit at exactly 1.

     Two jobs: the knob at zero, which every track has by default and every mix therefore contains,
     and the knob up, which is what a limiter looks like and is much worse. Closing this means
     porting Blink's kernel into the vendored crate — see ROADMAP. */
  {instr:'probe-comp',tag:'probe-comp-unity',note:0,vel:1,dur:.8,sr:44100,t:T,
   params:{th:0,knee:6,ratio:1,amp:.35},
   known:'Chrome is not unity at ratio 1:1 — its own algorithm, not the spec\'s',known_at:-26},
  {instr:'probe-comp',tag:'probe-comp-hit',note:0,vel:1,dur:.8,sr:44100,t:T,
   params:{th:-20,knee:6,ratio:8,amp:.6},
   known:'Chrome vs the spec algorithm, compressing hard',known_at:-8},
  /* THE SAFETY LIMITER, SLAMMED — the exact settings the master limiter runs (both engines:
     -3 dB, knee 2, ratio 12, 1.5 ms attack, 80 ms release) with a signal hot enough to keep it
     in constant deep reduction, which is what a dnb mix at masterVol .9 does to it. This is the
     one compressor configuration EVERY native mix passes through and the one that had never
     been nulled under load — added 2026-08-14 while hunting "the snare fucks it up with
     distortions like crazy" on the all-native path. */
  {instr:'probe-comp',tag:'probe-comp-slam',note:0,vel:1,dur:.8,sr:44100,t:T,
   params:{th:-3,knee:2,ratio:12,att:.0015,relc:.08,amp:.9},
   known:'Chrome vs the spec algorithm at the safety limiter\'s own settings, in constant deep GR',known_at:-8},

  /* A sweep deliberately started 29 samples INTO a quantum — the case QUANTUM describes. It is
     expected to null badly, and is kept precisely so that the day someone "fixes" the oscillator
     to chase this number, the comment is right here to stop them. */
  {instr:'probe-linfreq', tag:'probe-linfreq-midquantum', expectFail:true,
   why:'Chrome reads frequency automation from the block start, see QUANTUM',
   note:0,vel:1,dur:1.2,sr:44100,t:(2048+29)/44100,params:{}},

  /* ---- the channel strip -----------------------------------------------------------------------
     Everything above renders voice → destination with nothing in the way, which is the right way to
     grade an instrument and the wrong way to grade a mixer. These jobs put a `strip` on the job
     instead: the browser routes the note through the REAL ensureChans() + applyMix() — not a
     re-implementation living in this file — and Rust routes it through mixer.rs. What is subtracted
     is the strip.

     The source is the seeded-noise sampler in all of them, on purpose. Noise correlates with
     nothing but itself, so a filter coefficient that is a hair off, or one sample of compressor
     lookahead too few, sends the residue up immediately; a tone would forgive both. And the sampler
     itself nulls at -118 dB, so what shows up here is the strip and not the source.                */
  ...stripJobs(),
];

/* One track's mixer settings, spelled out. Same reasoning as the instrument params above: if
   mkTs()'s defaults ever change, these jobs should keep testing the strip they were written
   against until someone deliberately updates them. */
function TS(o){return Object.assign(
  {vol:.85,pan:0,dly:0,rvb:0,duck:0,duckSrc:-1,duckRel:.16,mute:false,solo:false,name:'',
   eq:{l:0,m:0,h:0},cmp:0,drv:0,fltF:1,fltQ:0},o)}

function stripJobs(){
  /* The sampler fixture, identical to the sampler jobs above but at velocity 0.35 rather than 0.8,
     and that is not tidiness — it is the difference between measuring the strip and measuring the
     WAV writer. The strip's own default filter setting (fltF 1 → an 18 kHz lowpass, Q 0 dB → linear
     1) sits close enough to Nyquist that its resonance boosts PEAKS by about 1.8x even though it
     cuts total energy. At velocity 0.8 the browser's render peaks at 1.22, the 16-bit file clamps
     it, and both engines are then being compared through a clipper. */
  const src={instr:'sampler',note:48,vel:.35,dur:1.0,sr:44100,t:T,
    params:{sampleSeed:0xB0B,sampleLen:22050,root:48,rel:.04,amp:1,fin:0,fout:0,fcut:18000,fres:0}};
  const j=(tag,strip,extra)=>Object.assign({},src,{tag,strip:TS(strip)},extra||{});
  /* A master job: a strip job that also carries `mix`, which is what makes both engines build the
     whole mixer rather than one channel. `delayTime` defaults to the 128 bpm the delay jobs use;
     it only matters when something is being sent to the delay bus. */
  const m=(tag,strip,mix,extra)=>Object.assign({},src,{tag,strip:TS(strip),from:.25,
    mix:Object.assign({master:.75,delayTime:60/128*.75},mix)},extra||{});
  return[
    /* Flat, and the most informative of the lot: every node is in circuit — three EQ bands at 0 dB,
       the filter wide open, the shaper with a null curve, the compressor at ratio 1 — so this one
       says whether the *path* matches before any control is touched. It is also the only job that
       measures the compressor's 6 ms of lookahead, which the spec pins and both engines must spend
       whether or not anything is being compressed. */
    j('strip-flat',{},{from:.25}),
    /* One band at a time would need three jobs to say what one says: a shelf, a peak and a shelf
       are three different biquad coefficient sets, and a wrong one lands in its own part of the
       spectrum. */
    j('strip-eq',{eq:{l:6,m:-4.5,h:3}},{from:.25}),
    /* The lowpass with resonance — fltHz's exponential curve AND Q x 20 in one shot. Q is where
       this would break: a factor of 20 is easy to drop, and at 0 it is invisible. */
    j('strip-filter',{fltF:.35,fltQ:.5},{from:.25}),
    /* Equal-power pan law. A wrong law is a fraction of a dB — inaudible, and 20 dB of residue. */
    j('strip-pan',{pan:-.6,vol:.5},{from:.25}),
    /* The waveshaper, at 2x oversample, with a curve rebuilt from drive_curve(). probe-shaper
       nulls this node on a 110 Hz sine; on NOISE it does not, and the difference is the
       oversampler rather than the curve. Chrome up-samples with a fixed FIR, the crate uses an FFT
       resampler (rubato), and two anti-imaging filters that are not the same filter disagree
       exactly where the material has energy near Nyquist — which a sine has none of and noise is
       made of. Worth knowing before drive is trusted on a hi-hat; harmless on the bass it is
       usually reached for. Closing it means porting Chrome's up/downsampler, and it is a smaller
       job than the compressor below. */
    j('strip-drive',{drv:.5},{vel:.12,
      known:'2x oversampler differs on broadband material — Chrome FIR vs FFT resampler',
      known_at:-21}),
    /* The compressor actually compressing, which is the one node in the strip that is not a
       formula: a DynamicsCompressor is a whole algorithm, and Chrome's is Chrome's. The spec pins
       its 6 ms lookahead and nothing else, so if these two disagree this is where it shows. */
    j('strip-comp',{cmp:.8},
      {known:'Chrome\'s compressor algorithm is not the spec\'s — see probe-comp-hit',known_at:-8}),
    /* And the whole strip working at once, which is the case a real mix is made of. */
    j('strip-all',{vol:.7,pan:.35,eq:{l:4,m:-3,h:2.5},fltF:.6,fltQ:.3,drv:.25},{from:.25,vel:.12}),

    /* ---- and everything AFTER the strip ------------------------------------------------------
       These add `mix`, which makes both sides build the whole mixer instead of one channel: the
       browser calls the real buildGraph(), Rust builds mixer::Mixer. What is being subtracted is
       the master fader, the safety limiter, and — in their own jobs — the two send buses.

       **None of these can null, and the reason is one node.** The master chain ends in a safety
       limiter, the limiter is a DynamicsCompressor, and Chrome's DynamicsCompressor is not the
       spec's algorithm — the finding A2 in ROADMAP records and `probe-comp-unity` pins. So every
       one of these carries a documented gap. What each of them then asserts is the thing that
       actually matters for a mixer and that a null test cannot say: **the mix comes out at the
       same level.** Measured steady ratio between the two engines, everywhere the limiter is not
       reacting: 0.978, or -0.19 dB. Inaudible, bounded, and now pinned.                          */
    /* The master path with the sends at zero: fader, insert (straight through), limiter idling.
       Says whether the PATH is right before any of it is pushed. */
    m('master-flat',{},{master:.75},
      {known:'the master limiter is a DynamicsCompressor — Chrome\'s is not the spec\'s (A2)',
       known_at:-30,levelWithin:.5}),
    /* The master fader somewhere other than its default, because 0.75 is easy to hard-code by
       accident on one side and never notice. The level check is what catches that: a fader applied
       twice, or not at all, is 7 dB out, not 0.2. */
    m('master-vol',{},{master:.4},
      {known:'as master-flat — the limiter',known_at:-30,levelWithin:.5}),
    /* The ping-pong delay bus: a full send, and the tempo that sets the tap time. Feedback at 0.42
       keeps the tail audible for seconds, so a wrong delay time or a swapped L/R would show up as
       a gross error rather than a subtle one — and it does not. What IS visible is the limiter
       working harder: the repeats push peaks past its -3 dB threshold, so this job runs it in its
       compressing regime and the gap widens accordingly (measured envelope ratio holds 0.978 away
       from the repeats and swings to 1.19 exactly on them). */
    m('master-delay',{dly:.9},{master:.75,delayTime:60/128*.75},
      {known:'the limiter compressing on the delay repeats — same node, harder worked',
       known_at:-9,levelWithin:.5}),
    /* The reverb bus — and the reason the impulse had to be seeded. This job could not come close
       unless both engines generate the SAME 1.9 s of noise from one seed with the same decay,
       which is a strict test of `seed_rnd` matching `seedRnd` over 167,580 samples. That it lands
       at the same -30 dB as master-flat, no worse, is the evidence the convolver and its impulse
       agree — the limiter's gap is all that is left. */
    m('master-reverb',{rvb:.9},{master:.75},
      {known:'as master-flat — the limiter; the reverb itself adds nothing',
       known_at:-30,levelWithin:.5}),
    /* The safety limiter actually catching peaks, hard. The master-bus twin of `strip-comp`, and
       the widest gap of the set — which is exactly what it should be. */
    m('master-limit',{vol:1},{master:1},
      {vel:.9,known:'the limiter is a DynamicsCompressor — Chrome\'s algorithm is not the spec\'s, '
                   +'same gap as strip-comp',known_at:-8,levelWithin:2}),
  ];
}

/* ---- WAV in, samples out. Both engines write the same 16-bit format (bufToWav / wav.rs). ---- */
function readWav(file){
  const b=fs.readFileSync(file),dv=new DataView(b.buffer,b.byteOffset,b.byteLength);
  let off=12,ch=0,sr=0,bits=0,data=null;
  while(off<dv.byteLength-8){
    const id=String.fromCharCode(dv.getUint8(off),dv.getUint8(off+1),dv.getUint8(off+2),dv.getUint8(off+3));
    const sz=dv.getUint32(off+4,true);
    if(id==='fmt '){ch=dv.getUint16(off+10,true);sr=dv.getUint32(off+12,true);bits=dv.getUint16(off+22,true)}
    if(id==='data'){data={off:off+8,sz};break}
    off+=8+sz+(sz&1);
  }
  const n=Math.floor(data.sz/2),s=new Int16Array(n);
  for(let i=0;i<n;i++)s[i]=dv.getInt16(data.off+i*2,true);
  return{ch,sr,bits,s};
}

/* A null test. Subtract, and report how far the residue sits below the reference's own peak —
   which is the number that says "you would not hear this", not a raw sample count. */
function nullTest(ref,got,fromSec){
  const n=Math.min(ref.s.length,got.s.length);
  /* `fromSec` skips the head of the render. It exists for exactly one reason and must never be
     used for any other: Chrome's compressor spends its first ~90 ms recovering from a start-up
     gain dip that the spec's algorithm does not have (see probe-comp-unity). Measuring from after
     that says whether the REST of the strip is right, which is a different and answerable
     question, and the answer turns out to be -87 dB. The whole-file number is still asserted
     alongside it, so nothing is being hidden by skipping. */
  const i0=fromSec?Math.floor(fromSec*ref.sr)*2:0;
  let peakR=0,peakD=0,sumD=0,sumR=0,firstBad=-1;
  for(let i=i0;i<n;i++){
    const r=ref.s[i]/32768,g=got.s[i]/32768,d=r-g;
    if(Math.abs(r)>peakR)peakR=Math.abs(r);
    if(Math.abs(d)>peakD){peakD=Math.abs(d);if(firstBad<0&&Math.abs(d)>.01)firstBad=i}
    sumD+=d*d;sumR+=r*r;
  }
  const dB=x=>x<=0?-999:20*Math.log10(x);
  /* Each side's own level, not just the difference. A residue tells you the two disagree; the two
     levels tell you HOW — a pure gain error shows as a constant ratio, a timing or algorithm
     difference shows as two near-identical levels that still will not cancel. Cheap to compute and
     it is the first thing wanted whenever a null fails. */
  let sumG=0,peakG=0;
  for(let i=i0;i<n;i++){const g=got.s[i]/32768;sumG+=g*g;if(Math.abs(g)>peakG)peakG=Math.abs(g)}
  const rmsR=Math.sqrt(sumR/n),rmsG=Math.sqrt(sumG/n);
  return{
    n, lenMatch:ref.s.length===got.s.length,
    peakRef:peakR,
    peakGot:peakG,
    peakDiffDb:+(dB(peakD)-dB(peakR)).toFixed(1),
    rmsDiffDb:+(dB(Math.sqrt(sumD/n))-dB(rmsR)).toFixed(1),
    refRmsDb:+dB(rmsR).toFixed(2),
    gotRmsDb:+dB(rmsG).toFixed(2),
    levelDb:+(dB(rmsG)-dB(rmsR)).toFixed(2),
    firstBadSec:firstBad<0?null:+(firstBad/2/ref.sr).toFixed(4),
  };
}

/* Installed in the page: the browser's half of the same bare render.
   It swaps the engine onto an OfflineAudioContext exactly as renderWav does, but builds only a
   single gain as the "channel strip" so trig()'s output is all that reaches the destination. */
const INSTALL_BARE=()=>{
  window.__bare=async job=>{
    /* Everything buildGraph() writes to. A master job calls the REAL buildGraph(), so every global
       it sets has to be put back or the next test in this page renders through a dead context —
       the same trap that made `metOut` fatal in renderWav (see CLAUDE.md). If you add a node to
       buildGraph(), add it here and to the restore line at the bottom. */
    const S={AC,master,chans,noiseBuf,dlyIn,dlyL,dlyR,rvbIn,masterAn,masterAnL,masterAnR,MCH,masterComp,
             spkOut,metOut,loudHP,loudAn,
             trackset:song.trackset,masterVol:song.masterVol,bpm:song.bpm};
    try{
      AC=new OfflineAudioContext(2,Math.ceil(job.dur*job.sr),job.sr);
      /* the same seeded noise buildGraph() makes — without it every noise instrument diverges */
      noiseBuf=AC.createBuffer(1,AC.sampleRate*2,AC.sampleRate);
      const nd=noiseBuf.getChannelData(0),nr=seedRnd(0x5EED17);
      for(let i=0;i<nd.length;i++)nd[i]=nr()*2-1;
      let bus;
      if(job.strip&&job.mix){
        /* A MASTER job builds the REAL graph: buildGraph() itself, not a stand-in — the ping-pong
           delay bus, the reverb bus with its seeded impulse, the master fader, its insert slot and
           the safety limiter, all wired as the app wires them. Then one channel strip on top.
           This is the only job shape where the output goes through `spkOut`, because that is the
           node buildGraph() ends on. */
        buildGraph();
        chans=[];ensureChans(1);
        song.trackset=[job.strip];song.masterVol=job.mix.master;
        song.bpm=60/(job.mix.delayTime/0.75);   /* setDelayTime()'s formula, run backwards */
        applyMix();setDelayTime();
        spkOut.connect(AC.destination);
        bus=chans[0].in;
      }else if(job.strip){
        /* A strip job builds the REAL channel strip: ensureChans() and applyMix() are called as
           they are in the app, not copied into this file. A second implementation of the thing
           under test, living in the test, is how a port comes out "passing" and wrong.
           The buses ensureChans wires into are stubbed as plain gains — the sends are measured in
           their own right once the delay and reverb buses are ported (stage B); here they are dead
           ends, exactly as they are on the Rust side, so the two graphs load the strip identically.
           master is a gain of 1: an exact multiply, which is why Rust can connect straight to the
           destination and still be comparing the same signal. */
        master=AC.createGain();
        dlyIn=AC.createGain();rvbIn=AC.createGain();
        chans=[];ensureChans(1);
        master.connect(AC.destination);
        song.trackset=[job.strip];song.masterVol=1;
        applyMix();
        bus=chans[0].in;
      }else{
        bus=AC.createGain();bus.connect(AC.destination);
        chans=[{in:bus}];
      }
      /* the probes bypass trig() on both sides — they are single primitives, not instruments */
      if(job.instr.startsWith('probe-')){
        const g=AC.createGain();g.connect(bus);
        if(job.instr==='probe-expramp'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=200;
          o.frequency.setValueAtTime(2000,job.t);
          o.frequency.exponentialRampToValueAtTime(200,job.t+.3);
          o.connect(g);g.gain.value=.5;o.start(job.t);o.stop(job.t+.9);
        }else if(/^probe-(triangle|saw|square|sine)(-hi)?$/.test(job.instr)){
          const hi=job.instr.endsWith('-hi');
          const type=/triangle/.test(job.instr)?'triangle':/saw/.test(job.instr)?'sawtooth'
                    :/square/.test(job.instr)?'square':'sine';
          const f=hi?1760:(type==='triangle'?190:110);
          const o=AC.createOscillator();o.type=type;o.frequency.value=f;
          o.connect(g);g.gain.value=.5;o.start(job.t);o.stop(job.t+.9);
        }else if(job.instr==='probe-expgain'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=300;
          o.connect(g);g.gain.setValueAtTime(1,job.t);
          g.gain.exponentialRampToValueAtTime(.01,job.t+.3);
          o.start(job.t);o.stop(job.t+.9);
        }else if(job.instr==='probe-linfreq'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=200;
          o.frequency.setValueAtTime(2000,job.t);
          o.frequency.linearRampToValueAtTime(200,job.t+.3);
          o.connect(g);g.gain.value=.5;o.start(job.t);o.stop(job.t+.9);
        }else if(job.instr==='probe-noiseoffset'){
          const ns=mkNoise(job.t+.011,.2);
          ns.connect(g);g.gain.value=.8;
        }else if(job.instr==='probe-fasttarget'){
          const ns=mkNoise(job.t,.012);const ng=AC.createGain();
          ng.gain.setValueAtTime(.48,job.t);
          ng.gain.setTargetAtTime(0,job.t+.001,.004);
          ns.connect(ng);ng.connect(g);g.gain.value=1;
        }else if(job.instr==='probe-kicknoise'){
          const ns=mkNoise(job.t,.012);
          const nf=AC.createBiquadFilter();nf.type='highpass';nf.frequency.value=3500;
          const ng=AC.createGain();
          ng.gain.setValueAtTime(.6,job.t);ng.gain.setTargetAtTime(0,job.t+.001,.004);
          ns.connect(nf);nf.connect(ng);ng.connect(g);
          g.gain.setValueAtTime(0,job.t);g.gain.linearRampToValueAtTime(1,job.t+.002);
          g.gain.setTargetAtTime(0,job.t+.004,.25);
        }else if(job.instr==='probe-kicksine'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=25;
          o.frequency.setValueAtTime(187.5,job.t);
          o.frequency.exponentialRampToValueAtTime(25,job.t+.065);
          o.connect(g);
          g.gain.setValueAtTime(0,job.t);g.gain.linearRampToValueAtTime(1,job.t+.002);
          g.gain.setTargetAtTime(0,job.t+.004,.25);
          o.start(job.t);o.stop(job.t+3.35);
        }else if(job.instr==='probe-kickenv'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=300;
          o.connect(g);
          g.gain.setValueAtTime(0,job.t);g.gain.linearRampToValueAtTime(1,job.t+.002);
          g.gain.setTargetAtTime(0,job.t+.004,.25);
          o.start(job.t);o.stop(job.t+1);
        }else if(job.instr==='probe-kickenv-aligned'){
          const sp=1/44100;
          const o=AC.createOscillator();o.type='sine';o.frequency.value=300;
          o.connect(g);
          g.gain.setValueAtTime(0,job.t);g.gain.linearRampToValueAtTime(1,job.t+88*sp);
          g.gain.setTargetAtTime(0,job.t+256*sp,.25);
          o.start(job.t);o.stop(job.t+1);
        }else if(job.instr==='probe-shaper'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=110;
          const sh=mkShaper(.3),pre=AC.createGain();pre.gain.value=.9;
          o.connect(pre);pre.connect(sh);sh.connect(g);g.gain.value=.7;
          o.start(job.t);o.stop(job.t+2);   /* past the render — this probe is about the shaper */
        }else if(job.instr==='probe-stopedge'){
          const o=AC.createOscillator();o.type='sine';o.frequency.value=110;
          o.connect(g);g.gain.value=.7;
          o.start(job.t);o.stop(job.t+1);
        }else if(job.instr==='probe-comp'){
          const ns=mkNoise(job.t,.5);
          const c=AC.createDynamicsCompressor();
          c.threshold.value=job.params.th==null?0:job.params.th;
          c.knee.value=job.params.knee==null?6:job.params.knee;
          c.ratio.value=job.params.ratio==null?1:job.params.ratio;
          c.attack.value=job.params.att==null?.003:job.params.att;
          c.release.value=job.params.relc==null?.12:job.params.relc;
          ns.connect(c);c.connect(g);g.gain.value=job.params.amp==null?.35:job.params.amp;
        }else if(job.instr==='probe-bandpass'){
          const ns=mkNoise(job.t,.5);
          const bp=AC.createBiquadFilter();bp.type='bandpass';
          bp.frequency.value=1150;bp.Q.value=1.4;
          ns.connect(bp);bp.connect(g);g.gain.value=.9;
        }
      }else{
        const inst={type:job.instr,params:job.params};
        /* The sampler needs actual audio, and a WAV would only test that the two decoders agree —
           which is not the thing under test. Both sides generate the same buffer from the same
           seeded PRNG instead (see fixture_sample in native/engine/src/bin/render.rs). Noise, not
           a tone: a tone is periodic, so a playback-rate or offset error lands on a similar-looking
           waveform and hides; noise correlates with nothing but itself, so one sample of
           misalignment sends the null test to roughly 0 dB. */
        if(job.params&&job.params.sampleSeed!=null&&job.params.sampleLen!=null){
          const n=job.params.sampleLen|0;
          const b=AC.createBuffer(1,n,AC.sampleRate),d=b.getChannelData(0);
          const r=seedRnd(job.params.sampleSeed>>>0);
          for(let i=0;i<n;i++)d[i]=r()*2-1;
          inst.buffer=b;
          /* trim/pad live on the instrument in the browser, but travel as params to Rust */
          if(job.params.s0!=null)inst.s0=job.params.s0;
          if(job.params.s1!=null)inst.s1=job.params.s1;
          if(job.params.pad!=null)inst.pad=job.params.pad;
          /* slices travel as flat params (see the sliced arm in lib.rs); rebuild the array the
             browser's trig() expects */
          if(job.params.sliceCount){
            inst.sliceBase=job.params.sliceBase;
            inst.slices=[];
            for(let k=0;k<job.params.sliceCount;k++)
              inst.slices.push({s0:job.params['slice'+k+'s'],s1:job.params['slice'+k+'e']});
          }
        }
        trig(inst,job.note,job.t,0,job.vel,job.params&&job.params.off||0,null,!!job.rev,null);
      }
      const rendered=await AC.startRendering();
      const ab=bufToWav(rendered,0),u=new Uint8Array(ab);
      let s='';for(let i=0;i<u.length;i+=8192)s+=String.fromCharCode.apply(null,u.subarray(i,i+8192));
      return btoa(s);
    }finally{
      AC=S.AC;master=S.master;chans=S.chans;noiseBuf=S.noiseBuf;
      dlyIn=S.dlyIn;dlyL=S.dlyL;dlyR=S.dlyR;rvbIn=S.rvbIn;
      masterAn=S.masterAn;masterAnL=S.masterAnL;masterAnR=S.masterAnR;MCH=S.MCH;masterComp=S.masterComp;
      spkOut=S.spkOut;metOut=S.metOut;loudHP=S.loudHP;loudAn=S.loudAn;
      song.trackset=S.trackset;song.masterVol=S.masterVol;song.bpm=S.bpm;
    }
  };
};

module.exports=async function(){
  const knowns=[];
  const S=suite('parity');

  /* No Rust on this machine is not a failure — the browser build is still the product. */
  if(!fs.existsSync(BIN)){
    S.note('native/engine not built — skipping.  cd native/engine && cargo build --release');
    return S;
  }
  fs.mkdirSync(TMP,{recursive:true});

  let browser;
  try{
    const o=await open();browser=o.browser;const page=o.page;
    await page.evaluate(INSTALL_BARE);

    for(const job of JOBS){
      const tag=job.tag||(job.instr+'-'+job.note+(job.params.drive===0?'-nodrive':''));
      const jf=path.join(TMP,tag+'.json'),rf=path.join(TMP,tag+'-rust.wav'),bf=path.join(TMP,tag+'-web.wav');
      fs.writeFileSync(jf,JSON.stringify(job));

      const b64=await page.evaluate(j=>window.__bare(j),job);
      fs.writeFileSync(bf,Buffer.from(b64,'base64'));
      execFileSync(BIN,[jf,rf]);

      const ref=readWav(bf),got=readWav(rf),r=nullTest(ref,got,job.from);
      S.head(tag);
      S.ok('same length',r.lenMatch,ref.s.length+' vs '+got.s.length);
      S.ok('browser render is not silent',r.peakRef>.01,'peak '+r.peakRef.toFixed(3));
      /* **Neither render may be INSANE, either** — a blanket sanity bar under every job in this
         suite, checked before the null test says anything.
         Added 2026-08-15, the day a sign flip in the vendored crate's `setTargetAtTime` blew the
         gain to f32::MAX on every cut note (see CLAUDE.md). Two days of hunting, and the reason
         nothing here caught it is that a null test only ever asks "are these two the SAME?" — it
         has no opinion about a number being absurd, and no job asserted an upper bound at all.
         Anything above 4.0 is not a mix decision, it is a broken engine: VOLT's own limiter
         cannot exceed ~1.0, and even a deliberately hot stack of saws sits near 2. Costs nothing,
         and would have named the bug the day the reese was ported. */
      const SANE=4.0;
      S.ok('  neither render is insane (no runaway gain)',
           r.peakRef<SANE&&r.peakGot<SANE,
           'browser peak '+r.peakRef.toFixed(3)+', rust peak '+r.peakGot.toFixed(3));
      /* A job with `from` is graded twice: once over everything, reported as the known start-up
         gap it is, and once from `from` onward, where it is expected to be exact like anything
         else. Two numbers, neither of them hidden. */
      if(job.from){
        const all=nullTest(ref,got,0);
        knowns.push(tag+': Chrome\'s compressor dips ~2 dB for its first 90 ms — whole render '
                    +all.rmsDiffDb+' dB rms');
        /* Whichever bound is LOOSER. This number always includes the start-up dip, so it is always
           worse than the from-`from` one — holding a job to a documented gap that happens to be
           tighter than -20 would fail it for being better than expected. */
        const startLim=Math.max(job.known_at!=null?job.known_at:-20,-20);
        S.ok('start-up gap no worse than '+startLim+' dB rms',all.rmsDiffDb<=startLim,
             all.rmsDiffDb+' dB rms over the whole render — Chrome\'s compressor start-up, see probe-comp-unity');
      }
      /* -60 dB is the bar: a residue that far under the sound itself is inaudible, and it leaves
         room for the last-bit differences two independent biquad/oversampler implementations are
         entitled to. Anything worse is a real porting bug, not arithmetic. */
      const fig=r.rmsDiffDb+' dB rms / '+r.peakDiffDb+' dB peak'
               +(r.rmsDiffDb>-60?'  [browser '+r.refRmsDb+' dB, rust '+r.gotRmsDb
                                 +' dB, level '+(r.levelDb>0?'+':'')+r.levelDb+' dB]':'');
      if(job.expectFail){
        S.ok('diverges, as documented',r.peakDiffDb>-60,r.peakDiffDb+' dB peak — '+job.why);
      }else if(job.known){
        /* A real, understood difference. Reported every run so it stays visible, and pinned to a
           number so it can't quietly get worse — but it doesn't fail the suite, because failing on
           something already written down teaches nobody anything. */
        knowns.push(tag+': '+job.known);
        S.ok('known gap, no worse than '+job.known_at+' dB',r.rmsDiffDb<=job.known_at,fig+'  — '+job.known);
        /* And the check that actually matters for a MIXER. A residue says the two engines are not
           bit-identical; this says the mix comes out at the same LOUDNESS, which is the thing a
           null test cannot express and the thing anyone would notice. A compressor algorithm that
           differs by a fifth of a dB is inaudible; one that differs by three would be a broken
           master bus wearing a documented excuse. */
        if(job.levelWithin!=null){
          S.ok('  and the mix comes out within '+job.levelWithin+' dB of the browser',
               Math.abs(r.levelDb)<=job.levelWithin,
               (r.levelDb>0?'+':'')+r.levelDb+' dB');
        }
      }else{
        const lim=job.maxRms!=null?job.maxRms:-60;
        S.ok('null test ≤ '+lim+' dB rms'+(job.from?' (from '+job.from+'s)':''),r.rmsDiffDb<=lim,fig
             +(job.why?'  — '+job.why:''));
      }
    }
  }catch(e){
    S.ok('suite ran',false,e.message);
  }finally{if(browser)await browser.close()}

  if(knowns.length){
    S.head('known gaps — measured and documented, not regressions');
    knowns.forEach(k=>S.note('· '+k));
  }
  return S;
};
