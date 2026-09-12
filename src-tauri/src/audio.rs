//! Playback engine.
//!
//! rodio's device sink owns a cpal stream, which is `!Send`, so it cannot live
//! in Tauri's managed state. One thread owns both buses for the life of the
//! process and takes commands over a channel; everything else talks to it
//! through the `Sender`.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::stream::{DeviceSinkBuilder, MixerDeviceSink};
use rodio::{Decoder, Player, Source};

/// Rodio shifts pitch by resampling, so playback rate *is* pitch. Under ~0.1 a
/// short clip crawls for minutes; over ~6 it is an inaudible click. Config is
/// user-editable JSON, so clamp here rather than trusting it.
pub const MIN_SPEED: f32 = 0.1;
pub const MAX_SPEED: f32 = 6.0;

pub enum Cmd {
    /// Names are matched as case-insensitive substrings; empty means "OS
    /// default" for `out` and "no monitoring" for `monitor`.
    Devices { out: String, monitor: String },
    Play {
        path: String,
        speed: f32,
        volume: f32,
        /// Seconds of the clip to throw away up front, for files that open with
        /// silence. Cut from the decoded source, so it is measured in the file's
        /// own time and does not drift when the pitch walks.
        offset: f32,
        /// Auditioning a clip in the UI must not go out to the call.
        monitor_only: bool,
    },
    Stop,
}

struct Bus {
    /// Dropping this closes the stream, so it is held even though it is unused.
    _sink: MixerDeviceSink,
    player: Player,
}

fn open(name: &str) -> Option<Bus> {
    let host = rodio::cpal::default_host();
    let device = if name.is_empty() {
        host.default_output_device()?
    } else {
        let want = name.to_lowercase();
        host.output_devices()
            .ok()?
            .find(|d| device_name(d).is_some_and(|n| n.to_lowercase().contains(&want)))?
    };
    let sink = DeviceSinkBuilder::from_device(device)
        .ok()?
        .open_sink_or_fallback()
        .ok()?;
    let player = Player::connect_new(sink.mixer());
    Some(Bus { _sink: sink, player })
}

/// rodio deprecated `name()` in favour of a structured description; only the
/// display name is of any use for matching a cable by substring.
fn device_name(d: &rodio::cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_string())
}

pub fn output_devices() -> Vec<String> {
    rodio::cpal::default_host()
        .output_devices()
        .map(|list| list.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

pub fn spawn() -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    std::thread::spawn(move || {
        let mut out: Option<Bus> = None;
        let mut monitor: Option<Bus> = None;
        // Clips are small and there are tens of them, so holding every one that
        // has been played keeps the hotkey path off the disk entirely.
        let mut cache: HashMap<String, Vec<u8>> = HashMap::new();

        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Devices { out: o, monitor: m } => {
                    out = open(&o);
                    monitor = if m.is_empty() { None } else { open(&m) };
                    if out.is_none() {
                        eprintln!("output device {o:?} not found");
                    }
                }
                Cmd::Stop => {
                    for bus in [out.as_ref(), monitor.as_ref()].into_iter().flatten() {
                        bus.player.clear();
                    }
                }
                Cmd::Play { path, speed, volume, offset, monitor_only } => {
                    let bytes = match cache.get(&path) {
                        Some(b) => b,
                        None => match std::fs::read(&path) {
                            Ok(b) => cache.entry(path.clone()).or_insert(b),
                            Err(e) => {
                                eprintln!("{path}: {e}");
                                continue;
                            }
                        },
                    };
                    let targets: [Option<&Bus>; 2] = if monitor_only {
                        [monitor.as_ref(), None]
                    } else {
                        [out.as_ref(), monitor.as_ref()]
                    };
                    for bus in targets.into_iter().flatten() {
                        let decoded = match Decoder::new(Cursor::new(bytes.clone())) {
                            Ok(src) => src,
                            Err(e) => {
                                eprintln!("{path}: {e}");
                                break;
                            }
                        };
                        // Cutting the previous clip is the whole point: a
                        // soundboard that layers is just noise.
                        bus.player.clear();
                        bus.player.set_speed(speed.clamp(MIN_SPEED, MAX_SPEED));
                        bus.player.set_volume(volume);
                        if offset > 0.0 {
                            bus.player
                                .append(decoded.skip_duration(Duration::from_secs_f32(offset)));
                        } else {
                            bus.player.append(decoded);
                        }
                        // `clear` leaves the player paused.
                        bus.player.play();
                    }
                }
            }
        }
    });
    tx
}
