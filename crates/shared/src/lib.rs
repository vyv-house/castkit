use bytes::{Bytes, BytesMut};

pub const FRAME_KEYFRAME: u8 = 0x01;
pub const FRAME_DELTA: u8 = 0x02;
pub const FRAME_META: u8 = 0x03;
pub const FRAME_FORCE_KEYFRAME: u8 = 0x04;

pub const HEADER_SIZE: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub frame_type: u8,
    pub timestamp_ms: u32,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub pts: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomInfo {
    pub id: String,
    pub width: u16,
    pub height: u16,
    pub watchers: usize,
    pub active: bool,
}

pub fn encode_frame(header: &FrameHeader, payload: &[u8]) -> Bytes {
    let mut buf = BytesMut::with_capacity(HEADER_SIZE + payload.len());
    buf.extend_from_slice(&[header.frame_type]);
    buf.extend_from_slice(&header.timestamp_ms.to_be_bytes());
    buf.extend_from_slice(&header.width.to_be_bytes());
    buf.extend_from_slice(&header.height.to_be_bytes());
    buf.extend_from_slice(payload);
    buf.freeze()
}

pub fn decode_frame(data: &[u8]) -> Option<(FrameHeader, &[u8])> {
    if data.len() < HEADER_SIZE {
        return None;
    }

    let header = FrameHeader {
        frame_type: data[0],
        timestamp_ms: u32::from_be_bytes(data[1..5].try_into().ok()?),
        width: u16::from_be_bytes(data[5..7].try_into().ok()?),
        height: u16::from_be_bytes(data[7..9].try_into().ok()?),
    };

    Some((header, &data[HEADER_SIZE..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_frame() {
        let header = FrameHeader {
            frame_type: FRAME_KEYFRAME,
            timestamp_ms: 123_456,
            width: 1920,
            height: 1080,
        };
        let payload = b"hello screen-share";

        let encoded = encode_frame(&header, payload);
        let (decoded_header, decoded_payload) = decode_frame(&encoded).expect("frame should decode");

        assert_eq!(header, decoded_header);
        assert_eq!(payload, decoded_payload);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let header = FrameHeader {
            frame_type: FRAME_META,
            timestamp_ms: 0,
            width: 1,
            height: 1,
        };

        let encoded = encode_frame(&header, &[]);
        let (decoded_header, decoded_payload) = decode_frame(&encoded).expect("frame should decode");

        assert_eq!(header, decoded_header);
        assert!(decoded_payload.is_empty());
    }

    #[test]
    fn supports_keyframe_and_delta_types() {
        let keyframe = FrameHeader {
            frame_type: FRAME_KEYFRAME,
            timestamp_ms: 1,
            width: 10,
            height: 20,
        };
        let delta = FrameHeader {
            frame_type: FRAME_DELTA,
            timestamp_ms: 2,
            width: 10,
            height: 20,
        };

        let keyframe_encoded = encode_frame(&keyframe, b"a");
        let delta_encoded = encode_frame(&delta, b"b");

        let (decoded_keyframe, _) = decode_frame(&keyframe_encoded).expect("keyframe should decode");
        let (decoded_delta, _) = decode_frame(&delta_encoded).expect("delta should decode");

        assert_eq!(decoded_keyframe.frame_type, FRAME_KEYFRAME);
        assert_eq!(decoded_delta.frame_type, FRAME_DELTA);
    }

    #[test]
    fn decode_too_short_returns_none() {
        assert!(decode_frame(&[]).is_none());
        assert!(decode_frame(&[0; HEADER_SIZE - 1]).is_none());
    }
}
