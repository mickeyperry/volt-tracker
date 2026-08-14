//! cut() explodes a reese. Which ingredient: instrument, sample rate, or a specific node?
use std::collections::HashMap;
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn p(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn run(label: &str, instr: &str, sr: f32, params: HashMap<String, f32>) {
    let frames = (2.0 * sr) as usize;
    let mut ctx = OfflineAudioContext::new(2, frames, sr);
    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);
    let job = Job { instr: instr.into(), note: 14.0, vel: 0.8, params,
                    dur: 0.4, sr, t: 0.05, rev: false, strip: None, mix: None };
    let mut v = trig(&c, &job).unwrap();
    v.connect(&ctx.destination());
    v.cut(0.5);
    let buf = ctx.start_rendering_sync();
    let d = buf.get_channel_data(0);
    let peak = d.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!("{label:<34} peak {peak:e}");
}

fn main() {
    let reese = || p(&[("voices",1.84),("detune",50.0),("cutoff",147.0),("res",0.0),
                       ("lfoRate",0.35),("lfoAmt",0.35),("drive",0.14),("sub",0.0),("rel",0.09)]);
    let sub = || p(&[("harm",0.15),("rel",0.06)]);
    run("sub 44100 + cut", "sub", 44100.0, sub());
    run("sub 48000 + cut", "sub", 48000.0, sub());
    run("reese 44100 + cut", "reese", 44100.0, reese());
    run("reese 48000 + cut", "reese", 48000.0, reese());
    // reese with the LFO throttled to nothing — is the wobble the detonator?
    let mut quiet = reese(); quiet.insert("lfoAmt".into(), 0.0);
    run("reese 48000 + cut, lfoAmt 0", "reese", 48000.0, quiet);
    // reese with no drive shaper in the path
    let mut nodrv = reese(); nodrv.insert("drive".into(), 0.0);
    run("reese 48000 + cut, drive 0", "reese", 48000.0, nodrv);
    // psybass and supersaw for breadth — both were ported yesterday
    run("psybass 48000 + cut", "psybass", 48000.0,
        p(&[("cutoff",420.0),("envAmt",0.5),("res",2.0),("decay",0.16),("drive",0.5),("sub",0.6)]));
    run("supersaw 48000 + cut", "supersaw", 48000.0,
        p(&[("detune",28.0),("spread",0.85),("hp",160.0),("cutoff",7000.0),("att",0.01),("rel",0.25)]));
}
