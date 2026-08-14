//! One reese note, no mixer, no cut — at 44.1k and 48k. Which sample rate explodes?
use std::collections::HashMap;
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn run(sr: f32, res: f32, lfo_amt: f32) {
    let frames = (2.0 * sr) as usize;
    let mut ctx = OfflineAudioContext::new(2, frames, sr);
    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);
    let job = Job {
        instr: "reese".into(), note: 14.0, vel: 0.8,
        params: params(&[("voices",1.84),("detune",50.0),("cutoff",147.0),("res",res),
                         ("lfoRate",0.35),("lfoAmt",lfo_amt),("drive",0.14),("sub",0.0),("rel",0.09)]),
        dur: 0.5, sr, t: 0.05, rev: false, strip: None, mix: None,
    };
    let v = trig(&c, &job).expect("builds");
    v.connect(&ctx.destination());
    let buf = ctx.start_rendering_sync();
    let d = buf.get_channel_data(0);
    let peak = d.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!("sr {sr:>7}  res {res}  lfoAmt {lfo_amt}  peak {peak:e}");
}

fn main() {
    run(44100.0, 0.0, 0.35);
    run(48000.0, 0.0, 0.35);
    run(48000.0, 2.0, 0.35);
    run(48000.0, 0.0, 0.0);
}
