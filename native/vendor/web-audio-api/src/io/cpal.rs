//! Audio IO management API
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Error as CpalError, ErrorKind as CpalErrorKind, OutputCallbackInfo, SampleFormat,
    Stream, StreamConfig, SupportedBufferSize,
};
use crossbeam_channel::Receiver;

use super::{
    AudioBackendError, AudioBackendErrorKind, AudioBackendManager, BackendResult, RenderThreadInit,
};

use crate::buffer::AudioBuffer;
use crate::context::AudioContextLatencyCategory;
use crate::context::AudioContextOptions;
use crate::io::microphone::MicrophoneRender;
use crate::media_devices::{MediaDeviceInfo, MediaDeviceInfoKind};
use crate::render::RenderThread;
use crate::stats::AudioStats;
use crate::{AtomicF64, MAX_CHANNELS};

// ---- VOLT FORK: host selection is plural ------------------------------------------------------
//
// Upstream picks exactly ONE host and every device then comes from it. That is why enabling
// `cpal-asio` appeared to do nothing on Windows: the ASIO backend was compiled in, but `get_host`
// special-cased only PipeWire and JACK before falling through to `default_host()`, which on
// Windows is always WASAPI. Nothing ever asked for the ASIO host.
//
// Making ASIO win instead — a third `#[cfg]` arm, which is where this fork started — is worse than
// the bug. It hides every WASAPI device the moment any ASIO driver is installed. For a tracker
// that is unacceptable: on this machine ASIO4ALL gives 5.8 ms while the interface's own ASIO
// driver gives 17.3 ms, *worse* than the same interface through WASAPI at 12 ms. Which is best is
// per-device and per-driver, so the only correct answer is to show them all and let the user pick.
//
// So: `all_hosts()` replaces `get_host()`, enumeration walks every host, and each device id is
// salted with its host name so the same interface appearing under both APIs stays two entries.
// See native/README.md for the measurements.

/// Every audio API this build can actually reach, best-effort first.
///
/// Order matters only for `default_host_for_output` below — it is the preference used when the
/// caller names no device at all. Anything that fails to initialise is skipped rather than
/// propagated: one broken driver must not take the whole device list with it.
fn all_hosts() -> Vec<(&'static str, cpal::Host)> {
    let mut out: Vec<(&'static str, cpal::Host)> = Vec::new();

    for id in cpal::available_hosts() {
        // VOLT FORK: a vendor ASIO driver can ACCESS-VIOLATE inside ASIOInit while being loaded
        // to answer enumeration (measured 2026-08-13: WER 0xc0000005 at volt_desktop!ASIOInit,
        // twice, app dead both times). Loading drivers is unavoidable to list them — what is
        // avoidable is walking into the same minefield on every listing after one has gone off.
        // The host app sets VOLT_SKIP_ASIO when its enumeration journal says the last listing
        // died in a driver; with it set the ASIO host is not even loaded.
        #[cfg(all(feature = "cpal-asio", target_os = "windows"))]
        {
            if id == cpal::HostId::Asio && std::env::var_os("VOLT_SKIP_ASIO").is_some() {
                log::warn!("ASIO host skipped (VOLT_SKIP_ASIO set after a driver crash)");
                continue;
            }
        }
        // `host_from_id` is what actually loads the backend, and for ASIO that means loading
        // vendor driver DLLs — entirely capable of failing or being absent.
        let Ok(host) = cpal::host_from_id(id) else {
            log::warn!("audio host {id:?} is unavailable, skipping");
            continue;
        };
        out.push((host_label(id), host));
    }

    if out.is_empty() {
        out.push(("Default", cpal::default_host()));
    }
    out
}

/// Short, stable name for a host, used both in the device label the user sees and in the hash
/// that makes device ids unique. Stable is the important word: change these and every saved
/// device preference stops resolving.
fn host_label(id: cpal::HostId) -> &'static str {
    // Each variant only EXISTS when cpal was built with the backend that defines it — `HostId::Asio`
    // is absent entirely from a build without `cpal-asio`, so these arms are gated on the feature
    // and not merely on the platform. Getting that wrong is a compile error, not a runtime one,
    // which is the good outcome: a plain `cargo build` caught it immediately.
    match id {
        #[cfg(all(feature = "cpal-asio", target_os = "windows"))]
        cpal::HostId::Asio => "ASIO",
        #[cfg(target_os = "windows")]
        cpal::HostId::Wasapi => "WASAPI",
        #[cfg(feature = "cpal-jack")]
        cpal::HostId::Jack => "JACK",
        #[cfg(target_os = "linux")]
        cpal::HostId::Alsa => "ALSA",
        #[cfg(all(feature = "cpal-pipewire", target_os = "linux"))]
        cpal::HostId::PipeWire => "PipeWire",
        #[allow(unreachable_patterns)]
        _ => "Audio",
    }
}

/// The host to use when no `sink_id` was given.
///
/// Upstream's preference order is kept exactly — PipeWire, then JACK, then the platform default —
/// because it is about which server is *running*, not about latency. ASIO is deliberately NOT
/// promoted here: picking a random ASIO driver for someone who asked for "the default" would take
/// exclusive control of an interface they did not name. Opting in to ASIO means naming a device.
fn default_host() -> cpal::Host {
    #[cfg(all(feature = "cpal-pipewire", target_os = "linux"))]
    {
        if let Some(host) = preferred_host(cpal::HostId::PipeWire, "PipeWire") {
            return host;
        }
    }

    #[cfg(feature = "cpal-jack")]
    {
        if let Some(host) = preferred_host(cpal::HostId::Jack, "JACK") {
            return host;
        }
    }

    cpal::default_host()
}

fn get_host() -> BackendResult<cpal::Host> {
    Ok(default_host())
}

#[cfg(any(
    feature = "cpal-jack",
    all(feature = "cpal-pipewire", target_os = "linux")
))]
fn preferred_host(host_id: cpal::HostId, host_name: &str) -> Option<cpal::Host> {
    let host_id = cpal::available_hosts()
        .into_iter()
        .find(|id| *id == host_id)?;

    let host = match cpal::host_from_id(host_id) {
        Ok(host) => host,
        Err(err) => {
            log::warn!("{host_name} host is unavailable, falling back to default host: {err}");
            return None;
        }
    };

    match host.devices() {
        Ok(devices) => {
            if devices.count() > 0 {
                Some(host)
            } else {
                log::warn!("No {host_name} devices found, falling back to default host");
                None
            }
        }
        Err(err) => {
            log::warn!(
                "Could not enumerate {host_name} devices, falling back to default host: {err}"
            );
            None
        }
    }
}

fn map_cpal_backend_error(
    kind: AudioBackendErrorKind,
    operation: &'static str,
    err: impl std::fmt::Display,
) -> AudioBackendError {
    AudioBackendError::new(kind, "cpal", operation, err.to_string())
}

fn map_cpal_error(operation: &'static str, err: CpalError) -> AudioBackendError {
    let kind = match err.kind() {
        CpalErrorKind::DeviceNotAvailable => AudioBackendErrorKind::DeviceUnavailable,
        CpalErrorKind::UnsupportedConfig | CpalErrorKind::UnsupportedOperation => {
            AudioBackendErrorKind::NotSupported
        }
        CpalErrorKind::InvalidInput => AudioBackendErrorKind::InvalidArgument,
        CpalErrorKind::DeviceBusy
        | CpalErrorKind::DeviceChanged
        | CpalErrorKind::HostUnavailable
        | CpalErrorKind::PermissionDenied
        | CpalErrorKind::RealtimeDenied
        | CpalErrorKind::ResourceExhausted
        | CpalErrorKind::StreamInvalidated
        | CpalErrorKind::Xrun
        | CpalErrorKind::BackendError
        | CpalErrorKind::Other
        | _ => AudioBackendErrorKind::BackendSpecific,
    };
    map_cpal_backend_error(kind, operation, err)
}

// VOLT FORK: searches every host, and hands back the host the device was found on.
//
// The host is not incidental — a `Device` only works with streams built through the host that
// produced it, so returning the device alone would leave the caller opening an ASIO device on the
// WASAPI host. That is the bug this signature exists to make impossible.
//
// And since 2026-08-12 it searches AS LITTLE as it can, because on Windows searching is not free:
// cpal's ASIO `Devices` iterator LOADS every driver it walks past ("briefly loading each driver
// to capture its metadata" — cpal 0.18, host/asio/device.rs), and a loading ASIO driver is
// allowed to put up a tray icon, a splash, a control panel. Resolving a saved WASAPI device used
// to parade every ASIO driver on the machine across the taskbar at startup. So:
//   * the id now names its host in clear text (`WASAPI#…`) — hosts that do not match are never
//     opened at all;
//   * within the host, matching is by NAME (see `stable_device_id`), so no per-device config
//     query is needed, and the walk returns at the first hit rather than collecting the list.
// An ASIO-hosted id still loads the drivers that come before it in registry order — that is
// cpal's iterator, and only deeper surgery could avoid it — but the common cases (WASAPI id, or
// the target being an early driver) now load nothing, or one.
fn cpal_device_for_id(
    kind: MediaDeviceInfoKind,
    device_id: &str,
) -> BackendResult<Option<(cpal::Host, Device)>> {
    let host_hint = device_id.split_once('#').map(|(h, _)| h.to_string());
    for (host_name, host) in all_hosts() {
        if let Some(hint) = host_hint.as_deref() {
            if hint != host_name {
                continue;
            }
        }
        let devices: Vec<Device> = match kind {
            MediaDeviceInfoKind::AudioInput => match host.input_devices() {
                Ok(d) => d.collect(),
                // A host that cannot enumerate is a dead end, not a fatal error — keep looking in
                // the others rather than failing the whole lookup.
                Err(e) => {
                    log::warn!("{host_name}: cannot enumerate inputs: {e}");
                    continue;
                }
            },
            MediaDeviceInfoKind::AudioOutput => match host.output_devices() {
                Ok(d) => d.collect(),
                Err(e) => {
                    log::warn!("{host_name}: cannot enumerate outputs: {e}");
                    continue;
                }
            },
            MediaDeviceInfoKind::VideoInput => Vec::new(),
        };
        let mut seen = Vec::<String>::new();

        for device in devices {
            let stable_id = cpal_stable_device_id(host_name, &device, kind, &seen)?;
            if stable_id == device_id {
                return Ok(Some((host, device)));
            }
            seen.push(stable_id);
        }
    }

    Ok(None)
}

// VOLT FORK: does this output id resolve to a device, right now?
//
// The cheap existence check `eng_open` needs before spending fifteen seconds on a driver: with
// the host carried in the id, a saved device that is gone (or an id from the pre-prefix format)
// answers `false` quickly instead of `build_output` silently falling back to the default device —
// which for an app that promised to reopen *the interface the user chose* is the wrong kind of
// helpful.
pub(crate) fn output_device_available(device_id: &str) -> bool {
    matches!(
        cpal_device_for_id(MediaDeviceInfoKind::AudioOutput, device_id),
        Ok(Some(_))
    )
}

// VOLT FORK: no longer called by the id lookup (ids stopped hashing the channel count, see
// `stable_device_id`); kept because it is upstream's and the diff should stay small.
#[allow(dead_code)]
fn cpal_device_channels(device: &Device, kind: MediaDeviceInfoKind) -> Option<u16> {
    Some(match kind {
        MediaDeviceInfoKind::AudioInput => device.default_input_config().ok()?.channels(),
        MediaDeviceInfoKind::AudioOutput => device.default_output_config().ok()?.channels(),
        MediaDeviceInfoKind::VideoInput => return None,
    })
}

// VOLT FORK: takes the host name instead of hardcoding "cpal", so the same interface seen through
// two APIs gets two ids. Without this, "Line Out (nio)" under ASIO and under WASAPI collide and
// whichever host is walked second silently loses its entry.
fn cpal_stable_device_id(
    host_name: &str,
    device: &Device,
    kind: MediaDeviceInfoKind,
    seen: &[String],
) -> BackendResult<String> {
    let name = device
        .description()
        .map_err(|e| map_cpal_error("device_name", e))?
        .to_string();

    Ok(stable_device_id(host_name, kind, name, seen))
}

// VOLT FORK, changed again 2026-08-12: the id is `{host}#{hash}`, and the hash covers host, kind,
// name and collision index — NOT the channel count any more. Two reasons, both about what an id
// is for. The clear-text host is what lets `cpal_device_for_id` skip whole hosts without opening
// them (see there). And the channel count is gone because computing it needs the device's config —
// which for ASIO means loading the driver — and because a channel count is not identity: Windows
// re-profiling the speakers from stereo to 5.1 used to change the hash and orphan the saved
// device. Name + host + index is what a human means by "that device".
//
// Saved ids from the older format no longer resolve; VOLT's UI falls back to matching by label
// (unchanged) and saves the new id, so the cost is one full enumeration on the first launch after
// this change.
fn stable_device_id(
    host: &str,
    kind: MediaDeviceInfoKind,
    friendly_name: String,
    seen: &[String],
) -> String {
    let mut index = 0;
    loop {
        let hash = crate::media_devices::DeviceId::as_string(
            kind,
            host.to_string(),
            friendly_name.clone(),
            0,
            index,
        );
        let device_id = format!("{host}#{hash}");

        if !seen.iter().any(|id| id == &device_id) {
            return device_id;
        }

        index += 1;
    }
}

/// Audio backend using the `cpal` library
#[derive(Clone)]
#[allow(unused)]
pub(crate) struct CpalBackend {
    stream: Arc<Mutex<Option<Stream>>>,
    output_latency: Arc<AtomicF64>,
    sample_rate: f32,
    number_of_channels: usize,
    sink_id: String,
}

impl AudioBackendManager for CpalBackend {
    fn build_output(
        options: AudioContextOptions,
        render_thread_init: RenderThreadInit,
    ) -> BackendResult<Self>
    where
        Self: Sized,
    {
        let RenderThreadInit {
            state,
            startup_pending,
            frames_played,
            stats,
            ctrl_msg_recv,
            event_send,
        } = render_thread_init;

        // VOLT FORK: the host now comes FROM the chosen device, not the other way round. A cpal
        // Device is only usable with streams built through the host that produced it, so binding
        // the host first and then looking the device up inside it is what confined everything to
        // WASAPI. A named ASIO device brings its ASIO host with it.
        let (host, device) = if options.sink_id.is_empty() {
            let host = default_host();
            let device = host.default_output_device().ok_or_else(|| {
                AudioBackendError::new(
                    AudioBackendErrorKind::DeviceUnavailable,
                    "cpal",
                    "default_output_device",
                    "No output device available",
                )
            })?;
            (host, device)
        } else {
            match cpal_device_for_id(MediaDeviceInfoKind::AudioOutput, &options.sink_id)? {
                Some(pair) => pair,
                // Falling back to the default is upstream's behaviour and worth keeping: a saved
                // device that has been unplugged should still let the app open and make sound.
                None => {
                    let host = default_host();
                    let device = host.default_output_device().ok_or_else(|| {
                        AudioBackendError::new(
                            AudioBackendErrorKind::DeviceUnavailable,
                            "cpal",
                            "select_output_device",
                            "No output device available",
                        )
                    })?;
                    (host, device)
                }
            }
        };

        log::info!("Audio Output Host: cpal {:?}", host.id());

        log::info!(
            "Output device: {:?}",
            device
                .description()
                .map_err(|e| map_cpal_error("output_device_name", e))?
                .to_string()
        );

        let default_device_config = device
            .default_output_config()
            .map_err(|e| map_cpal_error("default_output_config", e))?;

        // we grab the largest number of channels provided by the soundcard
        // clamped to MAX_CHANNELS, this value cannot be changed by the user
        let number_of_channels = usize::from(default_device_config.channels()).min(MAX_CHANNELS);

        // override default device configuration with the options provided by
        // the user when creating the `AudioContext`
        let mut preferred_config: StreamConfig = default_device_config.into();
        // make sure the number of channels is clamped to MAX_CHANNELS
        preferred_config.channels = number_of_channels as u16;

        // set specific sample rate if requested
        if let Some(sample_rate) = options.sample_rate {
            crate::assert_valid_sample_rate(sample_rate);
            preferred_config.sample_rate = sample_rate as u32;
        }

        // always try to set a decent buffer size
        let buffer_size = super::buffer_size_for_latency_category(
            options.latency_hint,
            preferred_config.sample_rate as f32,
        ) as u32;

        let clamped_buffer_size: u32 = match default_device_config.buffer_size() {
            SupportedBufferSize::Unknown => buffer_size,
            SupportedBufferSize::Range { min, max } => buffer_size.clamp(*min, *max),
        };

        preferred_config.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);

        // On android detected range for the buffer size seems to be too big, use default buffer size instead
        // See https://github.com/orottier/web-audio-api-rs/issues/515
        if cfg!(target_os = "android") {
            if let AudioContextLatencyCategory::Balanced
            | AudioContextLatencyCategory::Interactive = options.latency_hint
            {
                preferred_config.buffer_size = cpal::BufferSize::Default;
            }
        }

        // report the picked sample rate to the render thread, i.e. if the requested
        // sample rate is not supported by the hardware, it will fallback to the
        // default device sample rate
        let mut sample_rate = preferred_config.sample_rate as f32;

        // shared atomic to report output latency to the control thread
        let output_latency = Arc::new(AtomicF64::new(0.));

        let mut renderer = RenderThread::new(
            sample_rate,
            preferred_config.channels as usize,
            ctrl_msg_recv.clone(),
            Arc::clone(&state),
            Arc::clone(&frames_played),
            stats.clone(),
            event_send.clone(),
        );
        renderer.set_startup_pending(Arc::clone(&startup_pending));
        renderer.spawn_garbage_collector_thread();

        log::debug!(
            "Attempt output stream with preferred config: {:?}",
            preferred_config
        );

        let spawned = spawn_output_stream(
            &device,
            default_device_config.sample_format(),
            preferred_config,
            renderer,
            Arc::clone(&output_latency),
            stats.clone(),
        );

        let stream = match spawned {
            Ok(stream) => {
                log::debug!("Output stream set up successfully");
                stream
            }
            Err(e) => {
                log::warn!("Output stream build failed with preferred config: {}", e);

                let mut supported_config: StreamConfig = default_device_config.into();
                // make sure number of channels is clamped to MAX_CHANNELS
                supported_config.channels = number_of_channels as u16;
                // fallback to device default sample rate
                sample_rate = supported_config.sample_rate as f32;

                log::debug!(
                    "Attempt output stream with fallback config: {:?}",
                    supported_config
                );

                let mut renderer = RenderThread::new(
                    sample_rate,
                    supported_config.channels as usize,
                    ctrl_msg_recv,
                    state,
                    frames_played,
                    stats.clone(),
                    event_send,
                );
                renderer.set_startup_pending(startup_pending);
                renderer.spawn_garbage_collector_thread();

                let spawned = spawn_output_stream(
                    &device,
                    default_device_config.sample_format(),
                    supported_config,
                    renderer,
                    Arc::clone(&output_latency),
                    stats.clone(),
                );

                spawned.map_err(|e| map_cpal_error("build_fallback_output_stream", e))?
            }
        };

        // Required because some hosts don't play the stream automatically
        stream
            .play()
            .map_err(|e| map_cpal_error("play_output_stream", e))?;

        Ok(CpalBackend {
            stream: Arc::new(Mutex::new(Some(stream))),
            output_latency,
            sample_rate,
            number_of_channels,
            sink_id: options.sink_id,
        })
    }

    fn build_input(
        options: AudioContextOptions,
        number_of_channels: Option<u32>,
    ) -> BackendResult<(Self, Receiver<AudioBuffer>)>
    where
        Self: Sized,
    {
        // VOLT FORK: same as build_output — the device carries its own host.
        let (host, device) = if options.sink_id.is_empty() {
            let host = default_host();
            let device = host.default_input_device().ok_or_else(|| {
                AudioBackendError::new(
                    AudioBackendErrorKind::DeviceUnavailable,
                    "cpal",
                    "default_input_device",
                    "No input device available",
                )
            })?;
            (host, device)
        } else {
            match cpal_device_for_id(MediaDeviceInfoKind::AudioInput, &options.sink_id)? {
                Some(pair) => pair,
                None => {
                    let host = default_host();
                    let device = host.default_input_device().ok_or_else(|| {
                        AudioBackendError::new(
                            AudioBackendErrorKind::DeviceUnavailable,
                            "cpal",
                            "select_input_device",
                            "No input device available",
                        )
                    })?;
                    (host, device)
                }
            }
        };

        log::info!("Audio Input Host: cpal {:?}", host.id());

        log::info!(
            "Input device: {:?}",
            device
                .description()
                .map_err(|e| map_cpal_error("input_device_name", e))?
                .to_string()
        );

        let supported = device
            .default_input_config()
            .map_err(|e| map_cpal_error("default_input_config", e))?;

        // clone the config, we may need to fall back on it later
        let mut preferred: StreamConfig = supported.into();

        if let Some(number_of_channels) = number_of_channels {
            // do not ask for more channels than available
            preferred.channels = preferred.channels.min(number_of_channels as u16);
        }

        // make sure we don't exceed MAX_CHANNELS
        preferred.channels = preferred.channels.min(MAX_CHANNELS as u16);

        // set specific sample rate if requested
        if let Some(sample_rate) = options.sample_rate {
            crate::assert_valid_sample_rate(sample_rate);
            preferred.sample_rate = sample_rate as u32;
        }

        // always try to set a decent buffer size
        let buffer_size = super::buffer_size_for_latency_category(
            options.latency_hint,
            preferred.sample_rate as f32,
        ) as u32;

        let clamped_buffer_size: u32 = match supported.buffer_size() {
            SupportedBufferSize::Unknown => buffer_size,
            SupportedBufferSize::Range { min, max } => buffer_size.clamp(*min, *max),
        };

        preferred.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);
        let mut sample_rate = preferred.sample_rate as f32;
        let mut number_of_channels = preferred.channels as usize;

        let smoothing = 3; // todo, use buffering to smooth frame drops
        let (sender, mut receiver) = crossbeam_channel::bounded(smoothing);
        let renderer = MicrophoneRender::new(number_of_channels, sample_rate, sender);

        log::debug!(
            "Attempt input stream with preferred config: {:?}",
            preferred
        );

        let spawned = spawn_input_stream(&device, supported.sample_format(), preferred, renderer);

        // the required block size preferred config may not be supported, in that
        // case, fallback the supported config
        let stream = match spawned {
            Ok(stream) => {
                log::debug!("Input stream set up successfully");
                stream
            }
            Err(e) => {
                log::warn!("Input stream build failed with preferred config: {}", e);

                let supported_config: StreamConfig = supported.into();
                if usize::from(supported_config.channels) > MAX_CHANNELS {
                    return Err(AudioBackendError::new(
                        AudioBackendErrorKind::NotSupported,
                        "cpal",
                        "build_input_stream",
                        format!(
                            "Input device requires {} channels, exceeding the supported maximum of {MAX_CHANNELS}; building a stream with {MAX_CHANNELS} channels failed: {e}",
                            supported_config.channels
                        ),
                    ));
                }

                // fallback to device default sample rate and channel count
                number_of_channels = usize::from(supported_config.channels);
                sample_rate = supported_config.sample_rate as f32;

                log::debug!(
                    "Attempt input stream with fallback config: {:?}",
                    supported_config
                );

                // setup a new comms channel
                let (sender, receiver2) = crossbeam_channel::bounded(smoothing);
                receiver = receiver2; // overwrite earlier

                let renderer = MicrophoneRender::new(number_of_channels, sample_rate, sender);

                let spawned = spawn_input_stream(
                    &device,
                    supported.sample_format(),
                    supported_config,
                    renderer,
                );
                spawned.map_err(|e| map_cpal_error("build_fallback_input_stream", e))?
            }
        };

        // Required because some hosts don't play the stream automatically
        stream
            .play()
            .map_err(|e| map_cpal_error("play_input_stream", e))?;

        let backend = CpalBackend {
            stream: Arc::new(Mutex::new(Some(stream))),
            output_latency: Arc::new(AtomicF64::new(0.)),
            sample_rate,
            number_of_channels,
            sink_id: options.sink_id,
        };

        Ok((backend, receiver))
    }

    fn resume(&self) -> BackendResult<bool> {
        if let Some(s) = self.stream.lock().unwrap().as_ref() {
            s.play()
                .map(|_| true)
                .map_err(|e| map_cpal_error("resume", e))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn suspend(&self) -> BackendResult<bool> {
        if let Some(s) = self.stream.lock().unwrap().as_ref() {
            s.pause()
                .map(|_| true)
                .map_err(|e| map_cpal_error("suspend", e))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn close(&self) -> BackendResult<()> {
        self.stream.lock().unwrap().take(); // will Drop
        Ok(())
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    fn number_of_channels(&self) -> usize {
        self.number_of_channels
    }

    fn output_latency(&self) -> BackendResult<f64> {
        Ok(self.output_latency.load(Ordering::Relaxed))
    }

    fn sink_id(&self) -> &str {
        self.sink_id.as_str()
    }

    fn enumerate_devices_sync() -> BackendResult<Vec<MediaDeviceInfo>>
    where
        Self: Sized,
    {
        // VOLT FORK: walk every host, not just one. See the note above `all_hosts`.
        // cf. https://github.com/orottier/web-audio-api-rs/issues/356
        let mut list = Vec::<MediaDeviceInfo>::new();

        for (host_name, host) in all_hosts() {
            // A host that refuses to enumerate is skipped, not fatal. On Windows an ASIO driver
            // for an interface that is currently unplugged does exactly this, and it must not be
            // able to empty the list for every other device on the machine.
            let inputs: Vec<_> = match host.input_devices() {
                Ok(d) => d
                    .filter_map(|d| {
                        let n = d.default_input_config().ok()?.channels();
                        Some((d, MediaDeviceInfoKind::AudioInput, n))
                    })
                    .collect(),
                Err(e) => {
                    log::warn!("{host_name}: cannot enumerate inputs: {e}");
                    Vec::new()
                }
            };
            let outputs: Vec<_> = match host.output_devices() {
                Ok(d) => d
                    .filter_map(|d| {
                        let n = d.default_output_config().ok()?.channels();
                        Some((d, MediaDeviceInfoKind::AudioOutput, n))
                    })
                    .collect(),
                Err(e) => {
                    log::warn!("{host_name}: cannot enumerate outputs: {e}");
                    Vec::new()
                }
            };

            for (device, kind, _num_channels) in inputs.into_iter().chain(outputs) {
                let name = device
                    .description()
                    .map_err(|e| map_cpal_error("device_name", e))?
                    .to_string();

                // One id scheme for enumeration and lookup, or a listed device could never be
                // reopened: `stable_device_id` is the single place that builds one.
                let seen: Vec<String> =
                    list.iter().map(|d| d.device_id().to_string()).collect();
                let device_id = stable_device_id(host_name, kind, name.clone(), &seen);
                // Label carries the API, because "Line Out (nio 2|4)" twice in a dropdown
                // is useless and the choice between them is the entire point of the fork.
                list.push(MediaDeviceInfo::new(
                    device_id,
                    None,
                    kind,
                    format!("{name} [{host_name}]"),
                ));
            }
        }

        Ok(list)
    }
}

fn latency_in_seconds(infos: &OutputCallbackInfo) -> f64 {
    let timestamp = infos.timestamp();
    let delta = timestamp.playback.duration_since(timestamp.callback);
    delta.as_secs() as f64 + delta.subsec_nanos() as f64 * 1e-9
}

/// Creates an output stream
///
/// # Arguments:
///
/// * `device` - the output audio device on which the stream is created
/// * `sample_format` - audio sample format of the stream
/// * `config` - stream configuration
/// * `render` - the render thread which process the audio data
fn spawn_output_stream(
    device: &Device,
    sample_format: SampleFormat,
    config: StreamConfig,
    mut render: RenderThread,
    output_latency: Arc<AtomicF64>,
    stats: AudioStats,
) -> Result<Stream, CpalError> {
    let err_fn = |err| log::error!("an error occurred on the output audio stream: {}", err);

    match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            config,
            move |d: &mut [f32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::F64 => device.build_output_stream(
            config,
            move |d: &mut [f64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U8 => device.build_output_stream(
            config,
            move |d: &mut [u8], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_output_stream(
            config,
            move |d: &mut [u16], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U32 => device.build_output_stream(
            config,
            move |d: &mut [u32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U64 => device.build_output_stream(
            config,
            move |d: &mut [u64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I8 => device.build_output_stream(
            config,
            move |d: &mut [i8], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            config,
            move |d: &mut [i16], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I32 => device.build_output_stream(
            config,
            move |d: &mut [i32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I64 => device.build_output_stream(
            config,
            move |d: &mut [i64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        _ => Err(CpalErrorKind::UnsupportedConfig.into()),
    }
}

/// Creates an input stream
///
/// # Arguments:
///
/// * `device` - the input audio device on which the stream is created
/// * `sample_format` - audio sample format of the stream
/// * `config` - stream configuration
/// * `render` - the render thread which process the audio data
fn spawn_input_stream(
    device: &Device,
    sample_format: SampleFormat,
    config: StreamConfig,
    render: MicrophoneRender,
) -> Result<Stream, CpalError> {
    let err_fn = |err| log::error!("an error occurred on the input audio stream: {}", err);

    match sample_format {
        SampleFormat::F32 => {
            device.build_input_stream(config, move |d: &[f32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::F64 => {
            device.build_input_stream(config, move |d: &[f64], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U8 => {
            device.build_input_stream(config, move |d: &[u8], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U16 => {
            device.build_input_stream(config, move |d: &[u16], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U32 => {
            device.build_input_stream(config, move |d: &[u32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U64 => {
            device.build_input_stream(config, move |d: &[u64], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I8 => {
            device.build_input_stream(config, move |d: &[i8], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I16 => {
            device.build_input_stream(config, move |d: &[i16], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I32 => {
            device.build_input_stream(config, move |d: &[i32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I64 => {
            device.build_input_stream(config, move |d: &[i64], _c| render.render(d), err_fn, None)
        }
        _ => Err(CpalErrorKind::UnsupportedConfig.into()),
    }
}
