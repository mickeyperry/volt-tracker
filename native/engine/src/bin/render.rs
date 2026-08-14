//! Offline render of a single note, for the parity harness.
//!
//!   render <job.json> <out.wav>
//!
//! The graph is deliberately bare: voice → destination, with no channel strip, no master chain
//! and no compressor. If a ported instrument is wrong, that difference should show up on its own
//! rather than hiding under two compressors and a limiter. The browser side of the harness builds
//! the same bare graph.

use volt_engine::{noise_buffer, seed_rnd, trig, wav, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};

/// The fixture that makes the sampler testable.
///
/// A null test needs both engines to hold *the same* audio, and shipping a WAV into the harness
/// would only prove the two decoders agree — which is not the thing under test, and which they
/// need not do bit for bit. So neither side loads a file: both generate the same buffer from the
/// same seeded PRNG, the one already proven bit-identical across the two engines by the noise
/// probe. `sampleSeed` and `sampleLen` in the job turn it on, and `tests/parity.test.js` builds
/// the identical buffer in JavaScript.
///
/// A tone would have been friendlier to read on a scope and useless here: it is periodic, so a
/// playback-rate or start-offset error lands on a similar-looking waveform and the residue stays
/// small. Noise correlates with nothing but itself — one sample of misalignment and the null test
/// goes to roughly 0 dB, which is exactly the sensitivity this needs.
fn fixture_sample<C: BaseAudioContext>(ctx: &C, seed: u32, len: usize, sr: f32) -> web_audio_api::AudioBuffer {
    let mut buf = ctx.create_buffer(1, len, sr);
    let mut rnd = seed_rnd(seed);
    let data: Vec<f32> = (0..len).map(|_| (rnd() * 2.0 - 1.0) as f32).collect();
    buf.copy_to_channel(&data, 0);
    buf
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: render <job.json> <out.wav>");
        std::process::exit(2);
    }
    let src = std::fs::read_to_string(&args[1]).expect("cannot read job");
    let job: Job = serde_json::from_str(&src).expect("job is not valid JSON");

    let len = (job.dur * job.sr as f64).ceil() as usize;
    let mut ctx = OfflineAudioContext::new(2, len, job.sr);
    let noise = noise_buffer(&ctx);

    // `sampleSeed` / `sampleLen` are parameters rather than fields so the job schema does not have
    // to change; a job without them behaves exactly as before.
    let smp = match (job.params.get("sampleSeed"), job.params.get("sampleLen")) {
        (Some(&seed), Some(&len)) if len >= 1.0 => {
            Some(fixture_sample(&ctx, seed as u32, len as usize, job.sr))
        }
        _ => None,
    };
    let c = Ctx::new(&ctx, &noise);
    let c = match smp.as_ref() {
        Some(b) => c.with_sample(b),
        None => c,
    };

    let voice = match trig(&c, &job) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(3);
        }
    };

    // With `strip` in the job the note is routed through a channel strip instead of straight at the
    // destination, and the thing being measured is the mixer rather than the instrument. The strip's
    // output goes direct to the destination: the browser side puts a master gain of exactly 1.0
    // there, which is an exact multiply and so changes nothing either side.
    //
    // `strip` is held past the connect on purpose — dropping a Channel drops its nodes, and a
    // dropped node is a hole in the graph, not a tidy-up.
    // Three shapes, and which one is used says what is being measured:
    //   * no strip           — voice straight at the destination. The instrument.
    //   * strip, no mix      — one channel strip, output direct. The strip.
    //   * strip AND mix      — the whole mixer: strip → sends → master → limiter. The mix bus.
    //
    // Held past the connect on purpose in every case — dropping a Channel or a Mixer drops its
    // nodes, and a dropped node is a hole in the graph, not a tidy-up.
    let mut strip = None;
    let mut full = None;
    match (job.strip, job.mix) {
        (Some(ts), Some(mj)) => {
            let mut m = volt_engine::mixer::Mixer::new(&ctx);
            m.apply(
                &ctx,
                &volt_engine::mixer::MixState {
                    tracks: vec![ts],
                    master: mj.master,
                    delay_time: mj.delay_time,
                    ..Default::default()
                },
            );
            if let Some(input) = m.input(0) {
                voice.connect(input);
            }
            m.master.connect(&ctx.destination());
            full = Some(m);
        }
        (Some(ts), None) => {
            // The browser side puts a master gain of exactly 1.0 here, which is an exact multiply
            // and so changes nothing either side.
            let mut ch = volt_engine::mixer::Channel::new(&ctx);
            ch.apply(&ts);
            voice.connect(&ch.input);
            ch.connect(&ctx.destination());
            strip = Some(ch);
        }
        _ => voice.connect(&ctx.destination()),
    }
    let _keep = (&strip, &full);

    let rendered = ctx.start_rendering_sync();
    std::fs::write(&args[2], wav::encode(&rendered)).expect("cannot write wav");
}
