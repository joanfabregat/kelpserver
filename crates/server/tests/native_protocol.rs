use kelpserver::native::{
    protocol::*,
    stream::{self, Request, validate_size},
};
use std::io::{self, Cursor, Read};

struct Fragmented(Cursor<Vec<u8>>);
impl Read for Fragmented {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let limit = bytes.len().min(1);
        self.0.read(&mut bytes[..limit])
    }
}

#[test]
fn framed_messages_survive_byte_fragmentation_without_consuming_next_packet() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &ClientMessage::Stream(Request::Input {
            pane: 1,
            bytes: b"a\x02z\x1b[31m".to_vec(),
        }),
    )
    .unwrap();
    write_message(&mut bytes, &ClientMessage::Diagnostics).unwrap();
    let mut reader = Fragmented(Cursor::new(bytes));
    assert!(matches!(
        read_message::<ClientMessage>(&mut reader).unwrap(),
        Some(ClientMessage::Stream(Request::Input { pane: 1, bytes })) if bytes == b"a\x02z\x1b[31m"
    ));
    assert!(matches!(
        read_message::<ClientMessage>(&mut reader).unwrap(),
        Some(ClientMessage::Diagnostics)
    ));
    assert!(
        read_message::<ClientMessage>(&mut reader)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_envelope_keeps_its_wire_form() {
    let hello = ClientMessage::Stream(Request::Hello {
        version: stream::VERSION,
    });
    assert_eq!(
        serde_json::to_value(&hello).unwrap(),
        serde_json::json!({"Stream": {"type": "hello", "version": 1}})
    );
    assert_eq!(
        serde_json::to_value(ClientMessage::Diagnostics).unwrap(),
        serde_json::json!("Diagnostics")
    );
    assert_eq!(
        serde_json::to_value(ServerMessage::Error("x".into())).unwrap(),
        serde_json::json!({"Error": "x"})
    );
    // Records of the removed composed protocol do not decode.
    for legacy in [
        serde_json::json!({"Attach": {"version": 58, "cols": 80, "rows": 24, "takeover": false}}),
        serde_json::json!("Ping"),
        serde_json::json!({"Command": "Shutdown"}),
    ] {
        assert!(serde_json::from_value::<ClientMessage>(legacy).is_err());
    }
}

#[test]
fn malformed_partial_and_oversized_messages_fail_before_allocation_or_dispatch() {
    for bytes in [
        vec![0],
        vec![0, 0, 0, 0],
        u32::MAX.to_be_bytes().to_vec(),
        vec![0, 0, 0, 5, b'{'],
        vec![0, 0, 0, 1, b'!'],
    ] {
        assert!(read_message::<ClientMessage>(&mut Cursor::new(bytes)).is_err());
    }
    for (cols, rows) in [(0, 0), (1, 10), (2, 0), (401, 160), (400, 161)] {
        assert!(validate_size(cols, rows).is_err());
    }
    for (cols, rows) in [(2, 1), (120, 40), (400, 160)] {
        assert!(validate_size(cols, rows).is_ok());
    }
}
