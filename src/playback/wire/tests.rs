use super::{Event, MAX_FRAME, read_event, write_event};

#[test]
fn frame_round_trip() {
    let mut packet = Vec::new();
    write_event(
        &mut packet,
        &Event::Frame {
            epoch: 3,
            timestamp: 9_000_000,
            width: 2,
            height: 1,
        },
        &[40; 8],
    )
    .unwrap();
    let (event, data) = read_event(&mut packet.as_slice()).unwrap();
    assert!(matches!(
        event,
        Event::Frame {
            epoch: 3,
            timestamp: 9_000_000,
            width: 2,
            height: 1
        }
    ));
    assert_eq!(data, [40; 8]);
}

#[test]
fn rejects_oversized_packets_before_allocating() {
    let mut bytes = Vec::from(12_u32.to_le_bytes());
    bytes.extend_from_slice(&((MAX_FRAME + 1) as u32).to_le_bytes());
    assert!(read_event(&mut bytes.as_slice()).is_err());
}

#[test]
fn rejects_malformed_frames_audio_and_timestamps() {
    for event in [
        Event::Frame {
            epoch: 0,
            timestamp: 0,
            width: 1281,
            height: 1,
        },
        Event::Frame {
            epoch: 0,
            timestamp: -1,
            width: 1,
            height: 1,
        },
        Event::Frame {
            epoch: 0,
            timestamp: 0,
            width: 0,
            height: 1,
        },
        Event::Audio { epoch: 0 },
    ] {
        assert!(write_event(&mut Vec::new(), &event, &[0; 4]).is_err());
    }
    assert!(
        write_event(
            &mut Vec::new(),
            &Event::Position {
                epoch: 0,
                timestamp: -1
            },
            &[]
        )
        .is_err()
    );
    assert!(
        write_event(
            &mut Vec::new(),
            &Event::Prepared {
                duration: -1,
                audio: true,
                video: true,
                seekable: true
            },
            &[]
        )
        .is_err()
    );
}

#[test]
fn rejects_truncated_packet_and_unexpected_payload() {
    let mut packet = Vec::new();
    write_event(
        &mut packet,
        &Event::Position {
            epoch: 0,
            timestamp: 1,
        },
        &[],
    )
    .unwrap();
    packet.pop();
    assert!(read_event(&mut packet.as_slice()).is_err());
    assert!(write_event(&mut Vec::new(), &Event::Ended { epoch: 0 }, &[0]).is_err());
}
