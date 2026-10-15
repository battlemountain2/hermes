use super::*;
use std::{fs::File, os::unix::net::UnixStream};

#[test]
fn explicit_formats_survive_the_actual_descriptor_wire_protocol() {
    let input = File::open("/dev/null").expect("input descriptor");
    let output = File::open("/dev/null").expect("output descriptor");
    let (sender, receiver) = UnixStream::pair().expect("private wire socket");
    for operation in [
        Operation::PreviewModel,
        Operation::PreviewThreeMf,
        Operation::PreviewEpubCover,
        Operation::PreviewCbzCover,
        Operation::PreviewCbrCover,
    ] {
        send(&sender, &input, &output, operation).expect("typed wire request");
        let (received, input_fd, output_fd) = receive(&receiver)
            .expect("typed wire response")
            .expect("descriptor request");
        assert_eq!(received, operation);
        assert_eq!(
            File::from(input_fd)
                .metadata()
                .expect("received input")
                .len(),
            0
        );
        assert_eq!(
            File::from(output_fd)
                .metadata()
                .expect("received output")
                .len(),
            0
        );
    }
    assert!(Operation::parse(26).is_err());
    assert!(Operation::parse(u8::MAX).is_err());
}
