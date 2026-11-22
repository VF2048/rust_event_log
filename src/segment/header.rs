use bytes::{BufMut, Bytes, BytesMut};
use crate::event::model::BIN_SIZE;

pub const SEGMENT_HEADER_SIZE: usize = 26;

pub struct SegmentHeader {
    magic: [u8; 8],
    version: u16,
    segment_id: u32,
    created_at: i64,
    record_size: u32,
}

impl SegmentHeader {
    pub fn new(segment_id: u32, created_at: i64) -> Self {
        Self{
            magic: MAGIC,
            version: FORMAT_VERSION,
            segment_id,
            created_at,
            record_size: BIN_SIZE as u32,
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(SEGMENT_HEADER_SIZE);
        buf.put_slice(&self.magic);
        buf.put_u16_le(self.version);
        buf.put_u32_le(self.segment_id);
        buf.put_i64_le(self.created_at);
        buf.put_u32_le(self.record_size);
        buf.freeze()
    }
}

pub const BLOCK_HEADER_SIZE: usize = 28;

pub struct BlockHeader {
    pub records_count: u32,
    pub block_size: u32,
    pub checksum: u32,
    pub min_timestamp: i64,
    pub max_timestamp: i64,
}

impl BlockHeader {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(BLOCK_HEADER_SIZE);
        buf.put_u32_le(self.records_count);
        buf.put_u32_le(self.block_size);
        buf.put_u32_le(self.checksum);
        buf.put_i64_le(self.min_timestamp);
        buf.put_i64_le(self.max_timestamp);
        buf.freeze()
    }
}

pub const MAGIC: [u8; 8] = *b"\x89EVTLOG\n";
pub const FORMAT_VERSION: u16 = 1;
