use super::*;
use crate::playback::wire::read_event;

#[test]
fn large_audio_buffers_keep_every_sample_in_bounded_packets() {
    for length in [8, MAX_AUDIO, 147_440, MAX_AUDIO * 3 + 8] {
        let source: Vec<_> = (0..length).map(|index| (index % 251) as u8).collect();
        let mut transport = Vec::new();
        write_audio(&mut transport, 7, &source)
            .expect("expected test value from write_audio(&mut transport, 7, &source)");
        let mut reader = transport.as_slice();
        let mut received = Vec::new();
        let mut packets = 0;
        while !reader.is_empty() {
            let (event, data) =
                read_event(&mut reader).expect("expected test value from read_event(&mut reader)");
            assert!(matches!(event, Event::Audio { epoch: 7 }));
            assert!(!data.is_empty() && data.len() <= MAX_AUDIO);
            assert!(data.len().is_multiple_of(8));
            received.extend_from_slice(&data);
            packets += 1;
        }
        assert_eq!(received, source);
        assert_eq!(packets, length.div_ceil(MAX_AUDIO));
    }
}

#[test]
fn empty_audio_is_skipped_and_misaligned_audio_never_enters_transport() {
    let mut transport = Vec::new();
    write_audio(&mut transport, 2, &[])
        .expect("expected test value from write_audio(&mut transport, 2, &[])");
    assert!(transport.is_empty());
    for length in [1, 4, MAX_AUDIO + 1] {
        assert!(write_audio(&mut transport, 2, &vec![0; length]).is_err());
        assert!(transport.is_empty());
    }
}

#[test]
fn audio_transport_failure_stops_packetization() {
    let mut storage = [];
    let mut writer = io::Cursor::new(storage.as_mut_slice());
    assert!(write_audio(&mut writer, 3, &[0; 8]).is_err());
}

#[test]
#[ignore = "Private GStreamer plugins; generated large decoded PCM sample"]
fn generated_large_decoded_pcm_buffer_is_packetized_without_sample_loss() {
    gst::init().expect("expected test value from gst::init()");
    let pipeline = gst::parse::launch(
        "audiotestsrc num-buffers=1 samplesperbuffer=20000 wave=sine ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! appsink name=pcm",
    ).expect("expected test value from gst::parse::launch( \"audiotestsrc num-buffers=1 samplesperbuffer=20000 wave=sine ! audio/x-raw,format=F32LE,rate=48000,channels...").downcast::<gst::Pipeline>().expect("expected test value from gst::parse::launch( \"audiotestsrc num-buffers=1 samplesperbuffer=20000 wave=sine ! audio/x-raw,format=F32LE,rate=48000,channels...");
    let sink = pipeline
        .by_name("pcm")
        .expect("expected test value from pipeline .by_name(\"pcm\")")
        .downcast::<app::AppSink>()
        .expect("expected test value from pipeline .by_name(\"pcm\") .unwrap() .downcast::<app::AppSink>()");
    pipeline
        .set_state(gst::State::Paused)
        .expect("expected test value from pipeline.set_state(gst::State::Paused)");
    let sample = sink
        .try_pull_preroll(gst::ClockTime::from_seconds(5))
        .expect("expected test value from sink .try_pull_preroll(gst::ClockTime::from_seconds(5))");
    let source = sample
        .buffer()
        .expect("expected test value from sample.buffer()")
        .map_readable()
        .expect("expected test value from sample.buffer().unwrap().map_readable()");
    assert_eq!(source.len(), 160_000);
    let mut packetized = Vec::new();
    write_audio(&mut packetized, 9, source.as_slice())
        .expect("expected test value from write_audio(&mut packetized, 9, source.as_slice())");
    let mut reader = packetized.as_slice();
    let mut received = Vec::new();
    while !reader.is_empty() {
        let (event, data) =
            read_event(&mut reader).expect("expected test value from read_event(&mut reader)");
        assert!(matches!(event, Event::Audio { epoch: 9 }));
        received.extend_from_slice(&data);
    }
    assert_eq!(received, source.as_slice());
    pipeline
        .set_state(gst::State::Null)
        .expect("expected test value from pipeline.set_state(gst::State::Null)");
}
