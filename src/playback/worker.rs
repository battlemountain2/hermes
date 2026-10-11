use std::{
    io::{self, BufRead, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as app;
use gstreamer_video as video;

use super::wire::{Command, Event, write_event};

pub(crate) fn report_error(message: &str) {
    let message = message.chars().take(500).collect();
    let _sent = write_event(&mut io::stdout().lock(), &Event::Error { message }, &[]);
}

pub(crate) fn run() -> Result<(), String> {
    gst::init().map_err(|error| error.to_string())?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let epoch = Arc::new(AtomicU64::new(0));
    let preroll_epoch = Arc::new(AtomicU64::new(0));
    let video_bin = gst::parse::bin_from_description(
        "videoflip video-direction=auto ! videoconvert ! videoscale ! videorate drop-only=true max-rate=30 ! video/x-raw,format=RGBA,width=[1,1280],height=[1,720],framerate=[0/1,30/1],pixel-aspect-ratio=1/1 ! appsink name=frames sync=true max-buffers=2 drop=true enable-last-sample=false", true)
        .map_err(|error| error.to_string())?;
    let frames = video_bin
        .by_name("frames")
        .unwrap()
        .downcast::<app::AppSink>()
        .unwrap();
    let video_output = output.clone();
    let video_epoch = epoch.clone();
    let video_preroll_epoch = preroll_epoch.clone();
    frames.set_callbacks(
        app::AppSinkCallbacks::builder()
            .new_preroll(move |sink| {
                let result = send_frame(
                    sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?,
                    &video_output,
                    &video_epoch,
                );
                if result.is_ok() {
                    video_preroll_epoch
                        .store(video_epoch.load(Ordering::Acquire), Ordering::Release);
                }
                result
            })
            .new_sample({
                let output = output.clone();
                let epoch = epoch.clone();
                move |sink| {
                    send_frame(
                        sink.pull_sample().map_err(|_| gst::FlowError::Eos)?,
                        &output,
                        &epoch,
                    )
                }
            })
            .build(),
    );
    let audio_bin = gst::parse::bin_from_description(
        "audioconvert ! audioresample ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! appsink name=pcm sync=true max-buffers=4 enable-last-sample=false", true)
        .map_err(|error| error.to_string())?;
    let audio = audio_bin
        .by_name("pcm")
        .unwrap()
        .downcast::<app::AppSink>()
        .unwrap();
    let audio_output = output.clone();
    let audio_epoch = epoch.clone();
    let audio_preroll_epoch = preroll_epoch.clone();
    let preroll_audio_epoch = epoch.clone();
    audio.set_callbacks(
        app::AppSinkCallbacks::builder()
            .new_preroll(move |sink| {
                let _sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                audio_preroll_epoch.store(
                    preroll_audio_epoch.load(Ordering::Acquire),
                    Ordering::Release,
                );
                Ok(gst::FlowSuccess::Ok)
            })
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let event = Event::Audio {
                    epoch: audio_epoch.load(Ordering::Acquire),
                };
                write_event(&mut *audio_output.lock().unwrap(), &event, map.as_slice())
                    .map_err(|_| gst::FlowError::Error)?;
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    let player = gst::ElementFactory::make("playbin")
        .property("uri", "file:///input")
        .property("video-sink", &video_bin)
        .property("audio-sink", &audio_bin)
        .property("buffer-size", 4 * 1024 * 1024_i32)
        .build()
        .map_err(|error| error.to_string())?;
    player.set_property_from_str("flags", "video+audio");
    player.connect("deep-element-added", false, |values| {
        if let Ok(element) = values[2].get::<gst::Element>()
            && element.find_property("max-threads").is_some()
        {
            element.set_property_from_str("max-threads", "2");
        }
        None
    });
    let bus = player.bus().ok_or("Playback bus missing")?;
    let (send, receive) = mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let mut line = Vec::new();
            let result = (&mut input).take(4097).read_until(b'\n', &mut line);
            if !matches!(result, Ok(1..=4096)) || !line.ends_with(b"\n") {
                break;
            }
            let Ok(command) = serde_json::from_slice::<Command>(&line) else {
                break;
            };
            if send.send(command).is_err() {
                return;
            }
        }
        let _sent = send.send(Command::Stop);
    });
    player
        .set_state(gst::State::Paused)
        .map_err(|error| error.to_string())?;
    let mut prepared = false;
    let mut pending_seek = None;
    let mut playing = false;
    let mut ended = false;
    let result = (|| {
        loop {
            while let Ok(command) = receive.try_recv() {
                match command {
                    Command::Play => {
                        if pending_seek.is_none() {
                            player
                                .set_state(gst::State::Playing)
                                .map_err(|error| error.to_string())?;
                        }
                        playing = true;
                    }
                    Command::Pause => {
                        player
                            .set_state(gst::State::Paused)
                            .map_err(|error| error.to_string())?;
                        playing = false;
                    }
                    Command::Seek {
                        epoch: next,
                        timestamp,
                    } => {
                        if ended {
                            player
                                .set_state(gst::State::Ready)
                                .map_err(|error| error.to_string())?;
                            ended = false;
                        }
                        player
                            .set_state(gst::State::Paused)
                            .map_err(|error| error.to_string())?;
                        let (result, state, _) = player.state(gst::ClockTime::from_seconds(2));
                        result.map_err(|error| error.to_string())?;
                        if state != gst::State::Paused {
                            return Err("Decoder could not pause for seeking".into());
                        }
                        while let Some(message) = bus.pop() {
                            if let gst::MessageView::Error(error) = message.view() {
                                return Err(error.error().to_string());
                            }
                        }
                        epoch.store(next, Ordering::Release);
                        let success = timestamp >= 0
                            && player
                                .seek_simple(
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                    gst::ClockTime::from_useconds(timestamp.max(0) as u64),
                                )
                                .is_ok();
                        if success {
                            pending_seek = Some(next);
                        } else {
                            write_event(
                                &mut *output.lock().unwrap(),
                                &Event::Seeked {
                                    epoch: next,
                                    success: false,
                                },
                                &[],
                            )
                            .map_err(|error| error.to_string())?;
                        }
                    }
                    Command::Stop => return Ok(()),
                }
            }
            for message in bus.iter_timed(gst::ClockTime::from_mseconds(20)).take(32) {
                match message.view() {
                    gst::MessageView::Error(error) => {
                        return Err(format!(
                            "{}: {}",
                            error.error(),
                            error.debug().as_deref().unwrap_or("Decoder error")
                        ));
                    }
                    gst::MessageView::Eos(_) => {
                        ended = true;
                        playing = false;
                        if let Some(next) = pending_seek.take() {
                            write_event(
                                &mut *output.lock().unwrap(),
                                &Event::Seeked {
                                    epoch: next,
                                    success: true,
                                },
                                &[],
                            )
                            .map_err(|error| error.to_string())?;
                        }
                        write_event(
                            &mut *output.lock().unwrap(),
                            &Event::Ended {
                                epoch: epoch.load(Ordering::Acquire),
                            },
                            &[],
                        )
                        .map_err(|error| error.to_string())?;
                    }
                    gst::MessageView::AsyncDone(_) => {
                        if !prepared {
                            let duration = player
                                .query_duration::<gst::ClockTime>()
                                .map_or(0, |time| time.useconds().min(i64::MAX as u64) as i64);
                            let mut seek = gst::query::Seeking::new(gst::Format::Time);
                            let seekable = player.query(&mut seek) && seek.result().0;
                            write_event(
                                &mut *output.lock().unwrap(),
                                &Event::Prepared {
                                    duration,
                                    audio: player.property::<i32>("n-audio") > 0,
                                    video: player.property::<i32>("n-video") > 0,
                                    seekable,
                                },
                                &[],
                            )
                            .map_err(|error| error.to_string())?;
                            prepared = true;
                        }
                        if let Some(epoch) = pending_seek.take() {
                            if playing {
                                player
                                    .set_state(gst::State::Playing)
                                    .map_err(|error| error.to_string())?;
                            }
                            write_event(
                                &mut *output.lock().unwrap(),
                                &Event::Seeked {
                                    epoch,
                                    success: true,
                                },
                                &[],
                            )
                            .map_err(|error| error.to_string())?;
                            if let Some(position) = player.query_position::<gst::ClockTime>() {
                                write_event(
                                    &mut *output.lock().unwrap(),
                                    &Event::Position {
                                        epoch,
                                        timestamp: position.useconds().min(i64::MAX as u64) as i64,
                                    },
                                    &[],
                                )
                                .map_err(|error| error.to_string())?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(next) = pending_seek
                && preroll_epoch.load(Ordering::Acquire) == next
            {
                write_event(
                    &mut *output.lock().unwrap(),
                    &Event::Seeked {
                        epoch: next,
                        success: true,
                    },
                    &[],
                )
                .map_err(|error| error.to_string())?;
                if let Some(position) = player.query_position::<gst::ClockTime>() {
                    write_event(
                        &mut *output.lock().unwrap(),
                        &Event::Position {
                            epoch: next,
                            timestamp: position.useconds().min(i64::MAX as u64) as i64,
                        },
                        &[],
                    )
                    .map_err(|error| error.to_string())?;
                }
                pending_seek = None;
                if playing {
                    player
                        .set_state(gst::State::Playing)
                        .map_err(|error| error.to_string())?;
                }
            }
            if prepared
                && playing
                && let Some(position) = player.query_position::<gst::ClockTime>()
            {
                write_event(
                    &mut *output.lock().unwrap(),
                    &Event::Position {
                        epoch: epoch.load(Ordering::Acquire),
                        timestamp: position.useconds().min(i64::MAX as u64) as i64,
                    },
                    &[],
                )
                .map_err(|error| error.to_string())?;
            }
        }
    })();
    let _stopped = player.set_state(gst::State::Null);
    if let Err(message) = &result {
        let bounded: String = message.chars().take(500).collect();
        let _sent = write_event(
            &mut *output.lock().unwrap(),
            &Event::Error { message: bounded },
            &[],
        );
    }
    result
}

fn send_frame(
    sample: gst::Sample,
    output: &Mutex<io::Stdout>,
    epoch: &AtomicU64,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let info = video::VideoInfo::from_caps(sample.caps().ok_or(gst::FlowError::Error)?)
        .map_err(|_| gst::FlowError::Error)?;
    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
    let stride = usize::try_from(info.stride()[0]).map_err(|_| gst::FlowError::Error)?;
    let row = info.width() as usize * 4;
    if info.width() > 1280 || info.height() > 720 || stride < row {
        return Err(gst::FlowError::Error);
    }
    let mut packed = Vec::with_capacity(row * info.height() as usize);
    for index in 0..info.height() as usize {
        let start = info.offset()[0] + index * stride;
        packed.extend_from_slice(
            map.as_slice()
                .get(start..start + row)
                .ok_or(gst::FlowError::Error)?,
        );
    }
    write_event(
        &mut *output.lock().unwrap(),
        &Event::Frame {
            epoch: epoch.load(Ordering::Acquire),
            timestamp: buffer
                .pts()
                .map_or(0, |time| time.useconds().min(i64::MAX as u64) as i64),
            width: info.width(),
            height: info.height(),
        },
        &packed,
    )
    .map_err(|_| gst::FlowError::Error)?;
    Ok(gst::FlowSuccess::Ok)
}
