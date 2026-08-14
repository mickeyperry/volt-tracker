//! No VOLT code at all: one sine, one gain, the exact cut() call sequence. Who explodes?
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};

fn run(label: &str, cancel: bool, target: bool, stop: bool) {
    let mut ctx = OfflineAudioContext::new(1, 88200, 44100.0);
    let mut o = ctx.create_oscillator();
    let g = ctx.create_gain();
    o.connect(&g);
    g.connect(&ctx.destination());
    g.gain().set_value_at_time(0.0, 0.05);
    g.gain().linear_ramp_to_value_at_time(0.8, 0.056);
    o.start_at(0.05);
    if cancel { g.gain().cancel_scheduled_values(0.5); }
    if target { g.gain().set_target_at_time(0.0, 0.5, 0.0025); }
    if stop { o.stop_at(0.56); }
    let buf = ctx.start_rendering_sync();
    let d = buf.get_channel_data(0);
    let peak = d.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    // where does it first exceed 1?
    let first_over = d.iter().position(|s| s.abs() > 1.0).map(|i| i as f32 / 44100.0);
    println!("{label:<38} peak {peak:e}   first>1.0 at {first_over:?}");
}

fn main() {
    run("stop only", false, false, true);
    run("setTarget only", false, true, false);
    run("cancel only", true, false, false);
    run("cancel + setTarget", true, true, false);
    run("cancel + setTarget + stop (=cut)", true, true, true);
}
