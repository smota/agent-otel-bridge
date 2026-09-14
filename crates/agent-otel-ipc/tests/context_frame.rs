use agent_otel_ipc::frame::{
    decode_context_payload, decode_header, encode_context_payload, encode_frame, MsgType,
    WireHeader, HEADER_LEN, MAX_CONTEXT_LEN,
};

#[test]
fn legacy_and_context_frames_roundtrip() {
    let json = br#"{"event":"hook"}"#;
    let legacy = encode_frame(MsgType::HookPayload, json);
    let mut header = [0; HEADER_LEN];
    header.copy_from_slice(&legacy[..HEADER_LEN]);
    assert_eq!(
        decode_header(&header).unwrap(),
        (MsgType::HookPayload, json.len() as u32)
    );
    assert_eq!(&legacy[HEADER_LEN..], json);

    let context_header = WireHeader::new(0x1234, 7);
    let body = encode_context_payload(context_header, Some("workspace"), json);
    let (decoded_header, context, decoded_json) = decode_context_payload(&body).unwrap();
    assert_eq!(decoded_header, context_header);
    assert_eq!(context, Some("workspace"));
    assert_eq!(decoded_json, json);
}

#[test]
fn message_type_mapping_includes_context_and_rejects_unknown() {
    assert_eq!(MsgType::from_u8(0x04), MsgType::HookPayloadWithContext);
    assert_eq!(MsgType::from_u8(0x42), MsgType::Unknown);
}

#[test]
fn malformed_envelopes_are_rejected() {
    let header = WireHeader::new(1, 2);
    let body = encode_context_payload(header, Some("ctx"), b"{}");
    assert!(decode_context_payload(&body[..2]).is_err());

    let mut bad_version = body.clone();
    bad_version[WireHeader::LEN] = 2;
    assert!(decode_context_payload(&bad_version).is_err());

    let mut truncated_context = body.clone();
    truncated_context.truncate(WireHeader::LEN + 1 + 2 + 2);
    assert!(decode_context_payload(&truncated_context).is_err());

    let mut oversized = body;
    oversized[WireHeader::LEN + 1..WireHeader::LEN + 3]
        .copy_from_slice(&((MAX_CONTEXT_LEN as u16) + 1).to_le_bytes());
    assert!(decode_context_payload(&oversized).is_err());
}

#[test]
fn empty_oversized_and_invalid_utf8_contexts_preserve_json() {
    let header = WireHeader::new(1, 2);
    let json = [0, 1, b'{', b'}'];

    for context in [None, Some("")] {
        let body = encode_context_payload(header, context, &json);
        let (_, decoded_context, decoded_json) = decode_context_payload(&body).unwrap();
        assert_eq!(decoded_context, None);
        assert_eq!(decoded_json, json);
    }

    let oversized = "x".repeat(MAX_CONTEXT_LEN + 1);
    let body = encode_context_payload(header, Some(&oversized), &json);
    let (_, decoded_context, decoded_json) = decode_context_payload(&body).unwrap();
    assert_eq!(decoded_context, None);
    assert_eq!(decoded_json, json);

    let mut invalid = encode_context_payload(header, Some("ok"), &json);
    invalid[WireHeader::LEN + 1..WireHeader::LEN + 3].copy_from_slice(&3u16.to_le_bytes());
    invalid.splice(WireHeader::LEN + 3..WireHeader::LEN + 5, [0xff, 0xfe, 0xfd]);
    let (_, decoded_context, decoded_json) = decode_context_payload(&invalid).unwrap();
    assert_eq!(decoded_context, None);
    assert_eq!(decoded_json, json);
}
