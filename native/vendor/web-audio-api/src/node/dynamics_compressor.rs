use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::{AtomicF32, RENDER_QUANTUM_SIZE};

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation};

// Converting a value 𝑣 in decibels to linear gain unit means returning 10𝑣/20.
fn db_to_lin(val: f32) -> f32 {
    (10.0_f32).powf(val / 20.)
}

// Converting a value 𝑣 in linear gain unit to decibel means executing the following steps:
// If 𝑣 is equal to zero, return -1000.
// Else, return 20log10𝑣.
fn lin_to_db(val: f32) -> f32 {
    if val == 0. {
        -1000.
    } else {
        20. * val.log10() // 20 * log10(val);
    }
}

/// Options for constructing a [`DynamicsCompressorNode`]
// https://webaudio.github.io/web-audio-api/#DynamicsCompressorOptions
// dictionary DynamicsCompressorOptions : AudioNodeOptions {
//   float attack = 0.003;
//   float knee = 30;
//   float ratio = 12;
//   float release = 0.25;
//   float threshold = -24;
// };
#[derive(Clone, Debug)]
pub struct DynamicsCompressorOptions {
    pub attack: f32,
    pub knee: f32,
    pub ratio: f32,
    pub release: f32,
    pub threshold: f32,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for DynamicsCompressorOptions {
    fn default() -> Self {
        Self {
            attack: 0.003,   // seconds
            knee: 30.,       // dB
            ratio: 12.,      // unit less
            release: 0.25,   // seconds
            threshold: -24., // dB
            audio_node_options: AudioNodeOptions {
                channel_count: 2,
                channel_count_mode: ChannelCountMode::ClampedMax,
                channel_interpretation: ChannelInterpretation::Speakers,
            },
        }
    }
}

/// Assert that the channel count is valid for the DynamicsCompressorNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcount-constraints>
///
/// # Panics
///
/// This function panics if given count is greater than 2
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count(count: usize) {
    assert!(
        count <= 2,
        "NotSupportedError - DynamicsCompressorNode channel count cannot be greater than two"
    );
}

/// Assert that the channel count is valid for the DynamicsCompressorNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcountmode-constraints>
///
/// # Panics
///
/// This function panics if given count mode is [`ChannelCountMode::Max`]
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count_mode(mode: ChannelCountMode) {
    assert_ne!(
        mode,
        ChannelCountMode::Max,
        "NotSupportedError - DynamicsCompressorNode channel count mode cannot be set to max"
    );
}

/// `DynamicsCompressorNode` provides a compression effect.
///
/// It lowers the volume of the loudest parts of the signal and raises the volume
/// of the softest parts. Overall, a louder, richer, and fuller sound can be achieved.
/// It is especially important in games and musical applications where large numbers
/// of individual sounds are played simultaneous to control the overall signal level
/// and help avoid clipping (distorting) the audio output to the speakers.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/DynamicsCompressorNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#DynamicsCompressorNode>
/// - see also: [`BaseAudioContext::create_dynamics_compressor`]
///
/// # Usage
///
/// ```no_run
/// use std::fs::File;
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// // create an `AudioContext`
/// let context = AudioContext::default();
/// // load and decode a soundfile into an audio buffer
/// let file = File::open("samples/sample.wav").unwrap();
/// let buffer = context.decode_audio_data_sync(file).unwrap();
///
/// // create compressor and connect to destination
/// let compressor = context.create_dynamics_compressor();
/// compressor.connect(&context.destination());
///
/// // pipe the audio source in the compressor
/// let mut src = context.create_buffer_source();
/// src.connect(&compressor);
/// src.set_buffer(buffer.clone());
/// src.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example compressor`
///
#[derive(Debug)]
pub struct DynamicsCompressorNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    attack: AudioParam,
    knee: AudioParam,
    ratio: AudioParam,
    release: AudioParam,
    threshold: AudioParam,
    reduction: Arc<AtomicF32>,
}

impl AudioNode for DynamicsCompressorNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn number_of_inputs(&self) -> usize {
        1
    }

    fn number_of_outputs(&self) -> usize {
        1
    }

    // see <https://webaudio.github.io/web-audio-api/#audionode-channelcount-constraints>
    fn set_channel_count(&self, count: usize) {
        assert_valid_channel_count(count);
        self.channel_config.set_count(count, self.registration());
    }

    // see <https://webaudio.github.io/web-audio-api/#audionode-channelcountmode-constraints>
    fn set_channel_count_mode(&self, mode: ChannelCountMode) {
        assert_valid_channel_count_mode(mode);
        self.channel_config
            .set_count_mode(mode, self.registration());
    }
}

impl DynamicsCompressorNode {
    pub fn new<C: BaseAudioContext>(context: &C, options: DynamicsCompressorOptions) -> Self {
        context.base().register(move |registration| {
            assert_valid_channel_count(options.audio_node_options.channel_count);
            assert_valid_channel_count_mode(options.audio_node_options.channel_count_mode);

            // attack, knee, ratio, release and threshold have automation rate constraints
            // https://webaudio.github.io/web-audio-api/#audioparam-automation-rate-constraints
            let attack_param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: 0.,
                max_value: 1.,
                default_value: 0.003,
                automation_rate: crate::param::AutomationRate::K,
            };
            let (mut attack_param, attack_proc) =
                context.create_audio_param(attack_param_opts, &registration);
            attack_param.set_automation_rate_constrained(true);
            attack_param.set_value(options.attack);

            let knee_param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: 0.,
                max_value: 40.,
                default_value: 30.,
                automation_rate: crate::param::AutomationRate::K,
            };
            let (mut knee_param, knee_proc) =
                context.create_audio_param(knee_param_opts, &registration);
            knee_param.set_automation_rate_constrained(true);
            knee_param.set_value(options.knee);

            let ratio_param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: 1.,
                max_value: 20.,
                default_value: 12.,
                automation_rate: crate::param::AutomationRate::K,
            };
            let (mut ratio_param, ratio_proc) =
                context.create_audio_param(ratio_param_opts, &registration);
            ratio_param.set_automation_rate_constrained(true);
            ratio_param.set_value(options.ratio);

            let release_param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: 0.,
                max_value: 1.,
                default_value: 0.25,
                automation_rate: crate::param::AutomationRate::K,
            };
            let (mut release_param, release_proc) =
                context.create_audio_param(release_param_opts, &registration);
            release_param.set_automation_rate_constrained(true);
            release_param.set_value(options.release);

            let threshold_param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: -100.,
                max_value: 0.,
                default_value: -24.,
                automation_rate: crate::param::AutomationRate::K,
            };
            let (mut threshold_param, threshold_proc) =
                context.create_audio_param(threshold_param_opts, &registration);
            threshold_param.set_automation_rate_constrained(true);
            threshold_param.set_value(options.threshold);

            let reduction = Arc::new(AtomicF32::new(0.));

            // VOLT FORK: the 6 ms lookahead is a SAMPLE delay, not a block delay.
            //
            // Upstream kept a ring of whole render quanta — ceil(sr * 0.006 / 128) + 1 buffers —
            // which at 44.1 kHz delays by three blocks, 384 samples, 8.7 ms. Browsers delay by
            // exactly floor(sr * 0.006): measured in Chrome by rendering an impulse through a
            // compressor, 264 frames at 44.1 kHz, 288 at 48 kHz, 576 at 96 kHz.
            //
            // 120 samples is 2.7 ms. It is inaudible on its own and it matters twice over here:
            // VOLT puts a compressor on every channel AND a limiter on the master, so the error
            // compounds down the mix and shifts the whole mix against anything that skips it (the
            // metronome is delayed by the measured mix latency to stay in time — see MET.lat in
            // beta2.html), and it makes a sample-exact null test against the browser impossible,
            // which is the discipline the entire native port is graded by.
            let delay_frames = ((context.sample_rate() * 0.006) as usize).max(1);

            let render = DynamicsCompressorRenderer {
                attack: attack_proc,
                knee: knee_proc,
                ratio: ratio_proc,
                release: release_proc,
                threshold: threshold_proc,
                reduction: Arc::clone(&reduction),
                delay_frames,
                delay_lines: Vec::new(),
                delay_index: 0,
                prev_detector_value: 0.,
            };

            let node = DynamicsCompressorNode {
                registration,
                channel_config: options.audio_node_options.into(),
                attack: attack_param,
                knee: knee_param,
                ratio: ratio_param,
                release: release_param,
                threshold: threshold_param,
                reduction,
            };

            (node, Box::new(render))
        })
    }

    pub fn attack(&self) -> &AudioParam {
        &self.attack
    }

    pub fn knee(&self) -> &AudioParam {
        &self.knee
    }

    pub fn ratio(&self) -> &AudioParam {
        &self.ratio
    }

    pub fn release(&self) -> &AudioParam {
        &self.release
    }

    pub fn threshold(&self) -> &AudioParam {
        &self.threshold
    }

    pub fn reduction(&self) -> f32 {
        self.reduction.load(Ordering::Relaxed)
    }
}

struct DynamicsCompressorRenderer {
    attack: AudioParamId,
    knee: AudioParamId,
    ratio: AudioParamId,
    release: AudioParamId,
    threshold: AudioParamId,
    reduction: Arc<AtomicF32>,
    /// VOLT FORK: the lookahead, in samples — floor(sample_rate * 0.006), as browsers do it.
    delay_frames: usize,
    /// VOLT FORK: one delay line per channel, allocated on the first block that carries audio and
    /// reallocated only if the channel count changes. Was a ring of whole `AudioRenderQuantum`s,
    /// which is what forced the delay to be a multiple of 128 samples.
    delay_lines: Vec<Vec<f32>>,
    /// Read/write cursor shared by every channel's line.
    delay_index: usize,
    prev_detector_value: f32,
}

// https://webaudio.github.io/web-audio-api/#DynamicsCompressorOptions-processing
// see also https://www.eecs.qmul.ac.uk/~josh/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf
// follow Fig. 7 (c) diagram in paper
impl AudioProcessor for DynamicsCompressorRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = inputs[0].clone();
        let output = &mut outputs[0];
        let sample_rate = scope.sample_rate;

        // VOLT FORK: one delay line per channel, `delay_frames` long, zero-filled — the silence a
        // fresh compressor emits for its first 6 ms. Reallocated only when the channel count
        // changes, which for a fixed graph is once.
        let channels = input.channels().len();
        if self.delay_lines.len() != channels {
            self.delay_lines = vec![vec![0.; self.delay_frames]; channels];
            self.delay_index = 0;
        }

        // setup values for compression curve
        // https://webaudio.github.io/web-audio-api/#compression-curve
        let threshold = params.get(&self.threshold)[0];
        let knee = params.get(&self.knee)[0];
        let ratio = params.get(&self.ratio)[0];
        // @note: if knee != 0. we shadow threshold to match definitions of knee
        //   and threshold given in https://www.eecs.qmul.ac.uk/~josh/documents/2012/
        //   where knee is centered around threshold.
        //   We can thus reuse their formula for the gain computer stage.
        // yG =
        //     xG                                      if 2(xG − T) < −W
        //     xG + (1/R − 1)(xG − T + W/2)^2 / (2W)   if 2|(xG − T)| ≤ W
        //     T + (xG − T)/R                          if 2(xG − T) > W
        // This is weird, and probably wrong because `knee` and `threshold` are not
        // independent, but matches the spec.
        let threshold = if knee > 0. {
            threshold + knee / 2.
        } else {
            threshold
        };
        let half_knee = knee / 2.;
        // pre-compute for this block the constant part of the formula of the knee
        let knee_partial = (1. / ratio - 1.) / (2. * knee);

        // compute time constants for attack and release - eq. (7) in paper
        let attack = params.get(&self.attack)[0];
        let release = params.get(&self.release)[0];
        let attack_tau = (-1. / (attack * sample_rate)).exp();
        let release_tau = (-1. / (release * sample_rate)).exp();

        // Computing the makeup gain means executing the following steps:
        // - Let full range gain be the value returned by applying the compression curve to the value 1.0.
        // - Let full range makeup gain be the inverse of full range gain.
        // - Return the result of taking the 0.6 power of full range makeup gain.
        // @note: this should be confirmed / simplified, maybe could do all this in dB
        // seems coherent with chrome implementation
        let full_range_gain = threshold + (-threshold / ratio);
        let full_range_makeup = 1. / db_to_lin(full_range_gain);
        let makeup_gain = lin_to_db(full_range_makeup.powf(0.6));

        let mut prev_detector_value = self.prev_detector_value;

        let mut reduction_gain = 0.; // dB
        let mut reduction_gains = [0.; 128]; // lin
        let mut detector_values = [0.; 128]; // lin

        for i in 0..RENDER_QUANTUM_SIZE {
            // pick highest value for this index across all input channels
            // @tbc - this seems to be what is done in chrome
            let mut max = f32::MIN;

            for channel in input.channels().iter() {
                let sample = channel[i].abs();
                if sample > max {
                    max = sample;
                }
            }

            // pick absolute value and convert to dB domain
            // var xG in paper
            let sample_db = lin_to_db(max);

            // Gain Computer stage
            // ------------------------------------------------
            // var yG - eq. 4 in paper
            // if knee == 0. (hard knee), the `else if` branch is bypassed
            let sample_attenuated = if sample_db <= threshold - half_knee {
                sample_db
            } else if sample_db <= threshold + half_knee {
                sample_db + (sample_db - threshold + half_knee).powi(2) * knee_partial
            } else {
                threshold + (sample_db - threshold) / ratio
            };
            // variable xL in paper
            let sample_attenuation = sample_db - sample_attenuated;

            // Level Detector stage
            // ------------------------------------------------
            // Branching peak detector - eq. 16 in paper - var yL
            // attack branch
            let detector_value = if sample_attenuation > prev_detector_value {
                attack_tau * prev_detector_value + (1. - attack_tau) * sample_attenuation
            // release branch
            } else {
                release_tau * prev_detector_value + (1. - release_tau) * sample_attenuation
            };

            detector_values[i] = detector_value;
            // cdB = -yL + make up gain
            reduction_gain = -detector_value + makeup_gain;
            // convert to lin now, so we just to multiply samples later
            reduction_gains[i] = db_to_lin(reduction_gain);
            // update prev_detector_value for next sample
            prev_detector_value = detector_value;
        }

        // update prev_detector_value for next block
        self.prev_detector_value = prev_detector_value;
        // update reduction shared w/ main thread
        self.reduction.store(reduction_gain, Ordering::Relaxed);

        // VOLT FORK: push this block through the sample delay lines, taking the delayed signal out
        // as we go. The gain curve above was computed from the CURRENT block and is applied to the
        // signal from 6 ms ago — that is what makes it a lookahead compressor rather than a
        // feed-forward one, and it is unchanged; only the length of the look is.
        *output = input.clone();
        let mut end_index = self.delay_index;
        let mut audible = false;
        for (channel, line) in output.channels_mut().iter_mut().zip(self.delay_lines.iter_mut()) {
            let mut idx = self.delay_index;
            for s in channel.iter_mut() {
                let delayed = line[idx];
                line[idx] = *s;
                *s = delayed;
                if delayed != 0. {
                    audible = true;
                }
                idx += 1;
                if idx == line.len() {
                    idx = 0;
                }
            }
            end_index = idx;
        }
        self.delay_index = end_index;

        // Nothing has come out of the delay line yet (or nothing is left in it): no compression to
        // apply, and the node can report its tail time as over.
        if !audible {
            output.make_silent(); // truncate to 1 channel if needed
            return false;
        }

        output.channels_mut().iter_mut().for_each(|channel| {
            channel
                .iter_mut()
                .zip(reduction_gains.iter())
                .for_each(|(o, g)| *o *= g);
        });

        true
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::OfflineAudioContext;
    use crate::node::AudioScheduledSourceNode;

    use super::*;

    #[test]
    fn test_constructor_default() {
        let context = OfflineAudioContext::new(1, 1, 44_100.);
        let compressor = DynamicsCompressorNode::new(&context, Default::default());

        assert_float_eq!(compressor.attack().value(), 0.003, abs <= 0.);
        assert_float_eq!(compressor.knee().value(), 30., abs <= 0.);
        assert_float_eq!(compressor.ratio().value(), 12., abs <= 0.);
        assert_float_eq!(compressor.release().value(), 0.25, abs <= 0.);
        assert_float_eq!(compressor.threshold().value(), -24., abs <= 0.);
    }

    #[test]
    fn test_constructor_non_default() {
        let context = OfflineAudioContext::new(1, 1, 44_100.);
        let compressor = DynamicsCompressorNode::new(
            &context,
            DynamicsCompressorOptions {
                attack: 0.5,
                knee: 12.,
                ratio: 1.,
                release: 0.75,
                threshold: -60.,
                ..DynamicsCompressorOptions::default()
            },
        );

        assert_float_eq!(compressor.attack().value(), 0.5, abs <= 0.);
        assert_float_eq!(compressor.knee().value(), 12., abs <= 0.);
        assert_float_eq!(compressor.ratio().value(), 1., abs <= 0.);
        assert_float_eq!(compressor.release().value(), 0.75, abs <= 0.);
        assert_float_eq!(compressor.threshold().value(), -60., abs <= 0.);
    }

    /// VOLT FORK: the delay is now sample-accurate, so this test asserts the number browsers
    /// actually produce instead of the next block boundary after it. Measured in Chrome by
    /// rendering an impulse through a compressor: first non-zero sample at 264 (44.1 kHz), 288
    /// (48 kHz), 576 (96 kHz) — i.e. floor(sample_rate * 0.006), which is what this now checks.
    #[test]
    fn test_inner_delay() {
        let sample_rate = 44_100.;
        let compressor_delay = 0.006;
        // index of the first non zero sample: the lookahead, in samples
        let non_zero_index = (compressor_delay * sample_rate) as usize;
        assert_eq!(non_zero_index, 264);

        let mut context = OfflineAudioContext::new(1, 128 * 8, sample_rate);

        let compressor = DynamicsCompressorNode::new(&context, Default::default());
        compressor.connect(&context.destination());

        let mut buffer = context.create_buffer(1, 128 * 5, sample_rate);
        let signal = [1.; 128 * 5];
        buffer.copy_to_channel(&signal, 0);

        let mut src = context.create_buffer_source();
        src.set_buffer(buffer);
        src.connect(&compressor);
        src.start();

        let res = context.start_rendering_sync();
        let chan = res.channel_data(0).as_slice();

        // this is the delay
        assert_float_eq!(
            chan[0..non_zero_index],
            vec![0.; non_zero_index][..],
            abs_all <= 0.
        );

        // as some compression is applied, we just check the signal itself is non zero.
        // VOLT FORK: bounded by the length of the source plus the delay. With the old block-rounded
        // delay of 384 the delayed signal happened to run exactly to the end of the render, so the
        // test could read to the end; a shorter, correct delay lets the source finish first and the
        // silence after it is not a failure.
        for sample in chan.iter().take(128 * 5 + non_zero_index).skip(non_zero_index) {
            assert!(*sample != 0.);
        }
    }

    #[test]
    fn test_db_to_lin() {
        assert_float_eq!(db_to_lin(0.), 1., abs <= 0.);
        assert_float_eq!(db_to_lin(-20.), 0.1, abs <= 1e-8);
        assert_float_eq!(db_to_lin(-40.), 0.01, abs <= 1e-8);
        assert_float_eq!(db_to_lin(-60.), 0.001, abs <= 1e-8);
    }

    #[test]
    fn test_lin_to_db() {
        assert_float_eq!(lin_to_db(1.), 0., abs <= 0.);
        assert_float_eq!(lin_to_db(0.1), -20., abs <= 0.);
        assert_float_eq!(lin_to_db(0.01), -40., abs <= 0.);
        assert_float_eq!(lin_to_db(0.001), -60., abs <= 0.);
        // special case
        assert_float_eq!(lin_to_db(0.), -1000., abs <= 0.);
    }

    // @note: keep this, is useful to grab some internal value to be plotted
    // #[test]
    // fn test_attenuated_values() {
    //     // threshold: -40.
    //     // knee: 0.
    //     // ratio: 12.
    //     let sample_rate = 1_000.;
    //     let mut context = OfflineAudioContext::new(1, 128, sample_rate);

    //     let compressor = DynamicsCompressorNode::new(&context, Default::default());
    //     compressor.knee().set_value(0.);
    //     compressor.threshold().set_value(-30.);
    //     compressor.attack().set_value(0.05);
    //     compressor.release().set_value(0.1);
    //     compressor.connect(&context.destination());

    //     let mut buffer = context.create_buffer(1, 128 * 8, sample_rate);
    //     let mut signal = [0.; 128 * 8];

    //     for (i, s) in signal.iter_mut().enumerate() {
    //         *s = if i < 300 { 1. } else { 0.3 };
    //     }

    //     // println!("{:?}", signal);
    //     buffer.copy_to_channel(&signal, 0);

    //     let mut src = context.create_buffer_source();
    //     src.set_buffer(buffer);
    //     src.connect(&compressor);
    //     src.start();

    //     let _res = context.start_rendering_sync();
    // }
}
