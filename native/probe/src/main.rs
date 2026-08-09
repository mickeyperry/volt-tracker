//! VOLT latency probe.
//!
//! One question: how low can this machine's output latency actually go through a native audio
//! API? VOLT runs on Web Audio today, which on Windows means the OS default path and roughly
//! 20-40 ms. Whether it is worth porting the engine to Rust depends entirely on what the native
//! path measures HERE, on this interface — not on what is true in general.
//!
//! It reports the buffer size the device will accept and the latency that implies, then plays a
//! short click train so the figure can be sanity-checked by ear.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    println!("VOLT latency probe\n");

    // Every audio API the build can see. On Windows that is WASAPI, plus ASIO when compiled with
    // the `asio` feature and an SDK present — ASIO is the one that gets into single digits.
    let hosts: Vec<_> = cpal::available_hosts();
    println!("audio APIs available: {:?}\n", hosts);

    for host_id in hosts {
        let host = match cpal::host_from_id(host_id) {
            Ok(h) => h,
            Err(e) => {
                println!("{:?}: unavailable ({e})", host_id);
                continue;
            }
        };
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                println!("{:?}: no output device", host_id);
                continue;
            }
        };
        let name = device.name().unwrap_or_else(|_| "<unnamed>".into());
        println!("{:?} -> {}", host_id, name);

        let cfg = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                println!("   no usable config ({e})\n");
                continue;
            }
        };
        let sr = cfg.sample_rate().0 as f32;
        println!("   default: {} Hz, {} ch, {:?}", sr, cfg.channels(), cfg.sample_format());

        // What buffer sizes will it accept? The smallest one is the latency floor.
        match cfg.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                println!(
                    "   buffer range: {}..{} frames  ({:.2} ms .. {:.1} ms)",
                    min,
                    max,
                    *min as f32 / sr * 1000.0,
                    *max as f32 / sr * 1000.0
                );
            }
            cpal::SupportedBufferSize::Unknown => {
                println!("   buffer range: not reported by this API");
            }
        }

        // Now actually open a stream at a small buffer and see what we really get. Asking is not
        // the same as being given.
        for want in [64u32, 128, 256, 512] {
            let mut config: cpal::StreamConfig = cfg.clone().into();
            config.buffer_size = cpal::BufferSize::Fixed(want);
            let seen = Arc::new(AtomicUsize::new(0));
            let seen_cb = seen.clone();
            let ch = config.channels as usize;
            let mut phase = 0f32;
            let res = device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    seen_cb.store(data.len() / ch, Ordering::Relaxed);
                    // a quiet click train, so the number can be checked by ear
                    for frame in data.chunks_mut(ch) {
                        phase += 1.0;
                        let v = if (phase as usize) % 22050 < 60 { 0.15 } else { 0.0 };
                        for s in frame.iter_mut() {
                            *s = v;
                        }
                    }
                },
                |e| eprintln!("   stream error: {e}"),
                None,
            );
            match res {
                Ok(stream) => {
                    if stream.play().is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        let got = seen.load(Ordering::Relaxed);
                        if got > 0 {
                            println!(
                                "   asked {:>4} frames -> got {:>4} = {:.2} ms per callback",
                                want,
                                got,
                                got as f32 / sr * 1000.0
                            );
                        }
                    }
                }
                Err(_) => println!("   asked {:>4} frames -> refused", want),
            }
        }
        println!();
    }

    println!("For comparison, VOLT on Web Audio today reports its own figure in the browser:");
    println!("  AudioContext.baseLatency + outputLatency  (typically 20-40 ms on Windows)");
}
