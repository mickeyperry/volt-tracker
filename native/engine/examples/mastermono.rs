//! Mickey's exact setup: a MONO plugin on the MASTER insert, and a channel panned hard right.
//! Before the downmix fix the right side vanished entirely; this proves it survives.
use std::collections::HashMap;
use std::path::Path;
use volt_engine::mixer::{MixState, Mixer, TrackSet};
use volt_engine::vst::VstNode;
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn run(pan: f32, want: &str) {
    const SR: f32 = 48000.0;
    let mut ctx = OfflineAudioContext::new(2, (SR as usize)/2, SR);
    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);
    let mut mixer = Mixer::new(&ctx);
    mixer.ensure(&ctx, 1);
    let mut ts = TrackSet::default();
    ts.vol = 0.8; ts.flt_f = 1.0; ts.pan = pan;
    mixer.apply(&ctx, &MixState{tracks:vec![ts],master:0.9,delay_time:0.25,..Default::default()});

    let (tx,_rx) = std::sync::mpsc::channel();
    let mut _held=None;
    for path in volt_vsthost::scan(Path::new(volt_vsthost::VST3_DIR)) {
        let f=path.file_name().map(|f|f.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !f.contains("waveshell") && !f.contains(want) { continue }
        let Ok(m)=volt_vsthost::Module::open(&path) else { continue };
        let Some(info)=m.classes().into_iter().find(|i|i.category=="Audio Module Class"
            && !i.is_instrument() && i.name.to_lowercase().contains(want)) else { continue };
        let p=m.instantiate(&info,SR as f64,512).unwrap();
        let (node,_e)=VstNode::from_plugin(&ctx,p,tx.clone()).unwrap();
        mixer.set_master_plugin_chain(&[node.node()]);   // THE MASTER insert
        _held=Some(node); break;
    }
    mixer.master.connect(&ctx.destination());
    let params: HashMap<String,f32> = [("harm",0.15f32),("rel",0.06)].iter().map(|(k,v)|(k.to_string(),*v)).collect();
    let v = trig(&c,&Job{instr:"sub".into(),note:36.0,vel:0.8,params,dur:0.4,sr:SR,t:0.02,
                         rev:false,strip:None,mix:None}).unwrap();
    v.connect(mixer.input(0).unwrap());
    let buf = ctx.start_rendering_sync();
    let rms=|d:&[f32]| (d.iter().map(|s|(*s as f64)*(*s as f64)).sum::<f64>()/d.len() as f64).sqrt();
    let (l,r)=(rms(buf.get_channel_data(0)),rms(buf.get_channel_data(1)));
    println!("master-mono-plugin  pan {pan:>5}   L {l:.4}  R {r:.4}   {}",
             if l+r < 1e-5 {"*** SILENT — the mix was thrown away ***"} else {"audible"});
}

fn main(){
    let want = std::env::args().nth(1).unwrap_or_else(||"l2 mono".into()).to_lowercase();
    for pan in [-1.0f32, 0.0, 1.0] { run(pan, &want); }
    volt_vsthost::hard_exit(0);
}
