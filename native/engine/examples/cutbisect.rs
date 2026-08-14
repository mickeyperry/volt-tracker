//! Which half of retriggering detonates: the CUT, the DROP, or the overlap?
use std::collections::HashMap;
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn params() -> HashMap<String, f32> {
    [("voices",1.84f32),("detune",50.0),("cutoff",147.0),("res",0.0),
     ("lfoRate",0.35),("lfoAmt",0.35),("drive",0.14),("sub",0.0),("rel",0.09)]
        .iter().map(|(k,v)|(k.to_string(),*v)).collect()
}

fn job(t: f64) -> Job {
    Job { instr:"reese".into(), note:14.0, vel:0.8, params:params(),
          dur:0.4, sr:48000.0, t, rev:false, strip:None, mix:None }
}

fn peak(d: &[f32]) -> f32 { d.iter().fold(0.0f32,|m,s|m.max(s.abs())) }

fn main() {
    // (a) one note, cut at 0.5, voice kept alive to the end
    {
        let mut ctx = OfflineAudioContext::new(2, 96000, 48000.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut v = trig(&c, &job(0.05)).unwrap();
        v.connect(&ctx.destination());
        v.cut(0.5);
        let buf = ctx.start_rendering_sync();
        println!("cut, voice kept:      peak {:e}", peak(buf.get_channel_data(0)));
    }
    // (b) one note, cut at 0.5, voice DROPPED immediately
    {
        let mut ctx = OfflineAudioContext::new(2, 96000, 48000.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut v = trig(&c, &job(0.05)).unwrap();
        v.connect(&ctx.destination());
        v.cut(0.5);
        drop(v);
        let buf = ctx.start_rendering_sync();
        println!("cut, voice dropped:   peak {:e}", peak(buf.get_channel_data(0)));
    }
    // (c) two overlapping notes, no cut, both kept
    {
        let mut ctx = OfflineAudioContext::new(2, 96000, 48000.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let v1 = trig(&c, &job(0.05)).unwrap();
        v1.connect(&ctx.destination());
        let v2 = trig(&c, &job(0.7)).unwrap();
        v2.connect(&ctx.destination());
        let buf = ctx.start_rendering_sync();
        println!("two notes, no cut:    peak {:e}", peak(buf.get_channel_data(0)));
    }
    // (d) the engine's exact dance: cut, retire, drop at quiet_by
    {
        let mut ctx = OfflineAudioContext::new(2, 96000, 48000.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut v1 = trig(&c, &job(0.05)).unwrap();
        v1.connect(&ctx.destination());
        v1.cut(0.7);
        let _q = v1.quiet_by(0.7);
        drop(v1); // the engine would hold it 120 ms; offline the drop lands "now" either way
        let v2 = trig(&c, &job(0.7)).unwrap();
        v2.connect(&ctx.destination());
        let buf = ctx.start_rendering_sync();
        println!("cut+drop+retrigger:   peak {:e}", peak(buf.get_channel_data(0)));
    }
}
