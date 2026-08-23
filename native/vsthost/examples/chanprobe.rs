//! Reproduce "the kick comes out LEFT only at first" offline: load the exact plugin a channel's
//! chain uses, apply the SESSION'S saved patch for that slot, feed it dual-mono kick bursts, and
//! print the LEFT and RIGHT output levels over time. If the right channel starts silent and
//! creeps in, the plugin+patch is convicted; if both channels track from block one, the
//! left-only lives elsewhere (device path, strip, webview copy).
//!
//! `cargo run --example chanprobe -- <patches.pat> <key like f0.0>`
//!
//! Offline and windowless — safe to run while VOLT is open.

use std::path::Path;

fn take(rest: &[u8], p: &mut usize) -> Option<Vec<u8>> {
    let n = u32::from_le_bytes(rest.get(*p..*p + 4)?.try_into().ok()?) as usize;
    *p += 4;
    let v = rest.get(*p..*p + n)?.to_vec();
    *p += n;
    Some(v)
}

fn main() {
    let file = std::env::args().nth(1).unwrap_or_default();
    let want_key = std::env::args().nth(2).unwrap_or_default();
    if file.is_empty() || want_key.is_empty() {
        eprintln!("usage: chanprobe <patches.pat> <key e.g. f0.0>");
        volt_vsthost::hard_exit(2);
    }
    let data = std::fs::read(&file).expect("read patches file");
    let rest = data.strip_prefix(b"VOLTPAT1".as_slice()).expect("not a VOLTPAT1 file");
    let mut p = 0usize;
    let mut found: Option<(String, Vec<u8>)> = None;
    while p < rest.len() {
        let (Some(k), Some(i), Some(b)) = (take(rest, &mut p), take(rest, &mut p), take(rest, &mut p)) else { break };
        let (Ok(k), Ok(i)) = (String::from_utf8(k), String::from_utf8(i)) else { continue };
        if k == want_key {
            println!("key {k}: ident {i}, {} bytes of state", b.len());
            found = Some((i, b));
        }
    }
    let Some((ident, blob)) = found else {
        println!("key {want_key} not in {file}");
        volt_vsthost::hard_exit(1);
        unreachable!()
    };
    let (path, index) = ident.rsplit_once('|').expect("ident is path|index");
    let index: usize = index.parse().unwrap_or(0);

    let module = volt_vsthost::Module::open(Path::new(path)).expect("open plugin");
    let classes = module.classes();
    let effects: Vec<_> = classes.iter().filter(|c| c.category.contains("Audio Module")).collect();
    let c = effects.get(index).or_else(|| effects.first()).expect("no audio class");
    println!("{} — {} ({})", c.name, path, c.vendor);
    let sr = 48000.0f32;
    let mut plug = module.instantiate(c, sr as f64, 512).expect("instantiate");
    println!("loaded: {} in, {} out, latency {}", plug.in_channels, plug.out_channels, plug.latency);
    match plug.set_state(&blob) {
        Ok(()) => println!("patch applied ({} bytes)", blob.len()),
        Err(e) => println!("patch REFUSED: {e}"),
    }

    // 8 s of dual-mono kick bursts: 55 Hz decaying sine, 120 ms long, every 344 ms (~174 bpm rows)
    let frames = 256usize;
    let events = volt_vsthost::HostEvents::new();
    let nch = plug.out_channels.max(2);
    let mut out: Vec<Vec<f32>> = (0..nch).map(|_| vec![0.0; frames]).collect();
    let period = (sr * 0.344) as usize;
    let blocks = (sr as usize * 8) / frames;
    let win = (sr * 0.25) as usize; // report every 250 ms
    let (mut le, mut re, mut n) = (0f64, 0f64, 0usize);
    println!("time    L rms     R rms");
    for b in 0..blocks {
        let sig: Vec<f32> = (0..frames)
            .map(|i| {
                let t = (b * frames + i) % period;
                let ts = t as f32 / sr;
                if ts < 0.12 {
                    0.8 * (2.0 * std::f32::consts::PI * 55.0 * ts).sin() * (-ts * 30.0).exp()
                } else {
                    0.0
                }
            })
            .collect();
        let input: Vec<&[f32]> = vec![&sig, &sig];
        if let Err(e) = plug.process_io(&events, &input, &mut out, frames) {
            println!("process FAILED at block {b}: {e}");
            break;
        }
        for i in 0..frames {
            le += (out[0][i] as f64) * (out[0][i] as f64);
            re += (out[1.min(nch - 1)][i] as f64) * (out[1.min(nch - 1)][i] as f64);
            n += 1;
            if n >= win {
                let t = (b * frames + i) as f32 / sr;
                println!("{t:5.2}s  {:8.5}  {:8.5}", (le / n as f64).sqrt(), (re / n as f64).sqrt());
                le = 0.0;
                re = 0.0;
                n = 0;
            }
        }
    }
    volt_vsthost::hard_exit(0);
}
