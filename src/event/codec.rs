use super::model::{self, BIN_SIZE};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    InvalidOutcome(u8),
}

pub fn encode(event: &model::Event) -> Bytes {
    let mut write_buf = BytesMut::with_capacity(BIN_SIZE);
    write_buf.put_i64_le(event.timestamp);
    write_buf.put_u16_le(event.event_code);
    write_buf.put_u8(event.outcome as u8);
    write_buf.put_u16_le(event.location_id);
    write_buf.put_u16_le(event.operation_id);
    write_buf.put_u32_le(event.actor_id);
    write_buf.put_u32_le(event.target_id);
    write_buf.put_u32_le(event.payload_id);
    write_buf.freeze()
}

pub fn decode(bytes: &mut Bytes) -> Result<Option<model::Event>, DecodeError> {
    if bytes.len() < BIN_SIZE {
        return Ok(None);
    }

    let mut cursor = &bytes[..BIN_SIZE];

    let timestamp = cursor.get_i64_le();
    let event_code = cursor.get_u16_le();
    let outcome = match cursor.get_u8() {
        0 => model::Outcome::Unknown,
        1 => model::Outcome::Success,
        2 => model::Outcome::Failure,
        invalid => return Err(DecodeError::InvalidOutcome(invalid)),
    };
    let location_id = cursor.get_u16_le();
    let operation_id = cursor.get_u16_le();
    let actor_id = cursor.get_u32_le();
    let target_id = cursor.get_u32_le();
    let payload_id = cursor.get_u32_le();

    bytes.advance(BIN_SIZE);

    Ok(Some(model::Event {
        timestamp,
        event_code,
        outcome,
        location_id,
        operation_id,
        actor_id,
        target_id,
        payload_id,
    }))
}

