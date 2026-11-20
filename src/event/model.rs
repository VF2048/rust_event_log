#[derive(Debug, PartialEq)]
pub struct Event {
    pub timestamp: i64,
    pub event_code: u16,
    pub outcome: Outcome,
    pub location_id: u16,
    pub operation_id: u16,
    pub actor_id: u32,
    pub target_id: u32,
    pub payload_id: u32,
}

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum Outcome {
    Unknown = 0,
    Success = 1,
    Failure = 2,
}

pub const BIN_SIZE: usize = 27;
