//! Does a STEREO signal stay stereo through a plugin insert? L and R carry different tones.
use std::path::Path;
use volt_engine::mixer::{MixState, Mixer, TrackSet};
use volt_engine::vst::VstNode;
use volt_engine::noise_buffer;
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::{AudioNode, AudioScheduledSourceNode, ChannelMergerNode};

fn run(with_plugin: bool) {
    const SR: f32 = 48000.0;
    let mut ctx = OfflineAudioContext::new(2, (SR as usize)/2, SR);
    let _noise = noise_buffer(&ctx);
    let mut mixer = Mixer::new(&ctx);
    mixer.ensure(&ctx, 1);
    let mut ts = TrackSet::default();
    ts.vol = 0.8; ts.flt_f = 1.0; ts.pan = 0.0;
    mixer.apply(&ctx, &MixState{tracks:vec![ts],master:0.9,delay_time:0.25,..Default::default()});

    let (tx,_rx) = std::sync::mpsc::channel();
    let mut _held=None;
    if with_plugin {
        for path in volt_vsthost::scan(Path::new(volt_vsthost::VST3_DIR)) {
            let f=path.file_name().map(|f|f.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !f.contains("apqualizr") { continue }
            let m=volt_vsthost::Module::open(&path).unwrap();
            let info=m.classes().into_iter().find(|i|i.category=="Audio Module Class"&&!i.is_instrument()).unwrap();
            let p=m.instantiate(&info,SR as f64,512).unwrap();
            let (node,_e)=VstNode::from_plugin(&ctx,p,tx.clone()).unwrap();
            mixer.set_plugin_chain(0,&[node.node()]);
            _held=Some(node); break;
        }
    }
    mixer.master.connect(&ctx.destination());

    // hard-stereo source: 200 Hz on the LEFT only, 900 Hz on the RIGHT only
    let merger: ChannelMergerNode = ctx.create_channel_merger(2);
    let mut ol = ctx.create_oscillator(); ol.frequency().set_value(200.0);
    let mut orr = ctx.create_oscillator(); orr.frequency().set_value(900.0);
    ol.connect_from_output_to_input(&merger, 0, 0);
    orr.connect_from_output_to_input(&merger, 0, 1);
    ol.start(); orr.start();
    merger.connect(mixer.input(0).unwrap());

    let buf = ctx.start_rendering_sync();
    let rms=|d:&[f32]| (d.iter().map(|s|(*s as f64)*(*s as f64)).sum::<f64>()/d.len() as f64).sqrt();
    let (l,r)=(rms(buf.get_channel_data(0)),rms(buf.get_channel_data(1)));
    // correlation: +1 means the two ears are identical (mono), 0 means independent (stereo)
    let (a,b)=(buf.get_channel_data(0),buf.get_channel_data(1));
    let n=a.len().min(b.len());
    let mut num=0.0f64; let (mut da,mut db)=(0.0f64,0.0f64);
    for i in 0..n { num+=a[i] as f64*b[i] as f64; da+=(a[i] as f64).powi(2); db+=(b[i] as f64).powi(2); }
    let corr = if da>0.0&&db>0.0 { num/(da.sqrt()*db.sqrt()) } else { 0.0 };
    println!("plugin {:<5}  L {l:.4}  R {r:.4}  correlation {corr:+.3}  -> {}",
             with_plugin, if corr.abs()>0.95 {"MONO (stereo lost)"} else {"stereo preserved"});
}

fn main(){ run(false); run(true); volt_vsthost::hard_exit(0); }
