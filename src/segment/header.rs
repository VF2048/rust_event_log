
use crate::event::model::BIN_SIZE;
use bytes::{Buf, BufMut, Bytes, BytesMut};

pub const MAGIC: [u8; 8] = *b"\x89EVTLOG\n";
pub const FORMAT_VERSION: u16 = 1;
pub const SEGMENT_HEADER_SIZE: usize = 30; // 26 + 4 (CRC32)
pub const BLOCK_HEADER_SIZE: usize = 32;   // 28 + 4 (CRC32)

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentHeader {
    pub magic: [u8; 8],
    pub version: u16,
    pub segment_id: u32,
    pub created_at: i64,
    pub record_size: u32,
    pub header_checksum: u32,
}

impl SegmentHeader {
    pub fn new(segment_id: u32, created_at: i64) -> Self {
        let mut header = Self {
            magic: MAGIC,
            version: FORMAT_VERSION,
            segment_id,
            created_at,
            record_size: BIN_SIZE as u32,
            header_checksum: 0,
        };
        header.header_checksum = header.compute_checksum();
        header
    }

    pub fn compute_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&self.magic);
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&self.segment_id.to_le_bytes());
        hasher.update(&self.created_at.to_le_bytes());
        hasher.update(&self.record_size.to_le_bytes());
        hasher.finalize()
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(SEGMENT_HEADER_SIZE);
        buf.put_slice(&self.magic);
        buf.put_u16_le(self.version);
        buf.put_u32_le(self.segment_id);
        buf.put_i64_le(self.created_at);
        buf.put_u32_le(self.record_size);
        buf.put_u32_le(self.header_checksum);
        buf.freeze()
    }

    pub fn decode(mut bytes: &[u8]) -> Option<Self> {
        if bytes.len() < SEGMENT_HEADER_SIZE {
            return None;
        }
        let mut magic = [0u8; 8];
        bytes.copy_to_slice(&mut magic);
        let version = bytes.get_u16_le();
        let segment_id = bytes.get_u32_le();
        let created_at = bytes.get_i64_le();
        let record_size = bytes.get_u32_le();
        let header_checksum = bytes.get_u32_le();

        Some(Self {
            magic,
            version,
            segment_id,
            created_at,
            record_size,
            header_checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockHeader {
    pub records_count: u32,
    pub block_size: u32,
    pub checksum: u32, // CRC32 полезных данных (payload)
    pub min_timestamp: i64,
    pub max_timestamp: i64,
    pub header_checksum: u32, // CRC32 самого заголовка блока
}

impl BlockHeader {
    pub fn new(
        records_count: u32,
        block_size: u32,
        checksum: u32,
        min_timestamp: i64,
        max_timestamp: i64,
    ) -> Self {
        let mut header = Self {
            records_count,
            block_size,
            checksum,
            min_timestamp,
            max_timestamp,
            header_checksum: 0,
        };
        header.header_checksum = header.compute_checksum();
        header
    }

    pub fn compute_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&self.records_count.to_le_bytes());
        hasher.update(&self.block_size.to_le_bytes());
        hasher.update(&self.checksum.to_le_bytes());
        hasher.update(&self.min_timestamp.to_le_bytes());
        hasher.update(&self.max_timestamp.to_le_bytes());
        hasher.finalize()
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(BLOCK_HEADER_SIZE);
        buf.put_u32_le(self.records_count);
        buf.put_u32_le(self.block_size);
        buf.put_u32_le(self.checksum);
        buf.put_i64_le(self.min_timestamp);
        buf.put_i64_le(self.max_timestamp);
        buf.put_u32_le(self.header_checksum);
        buf.freeze()
    }

    pub fn decode(mut bytes: &[u8]) -> Option<Self> {
        if bytes.len() < BLOCK_HEADER_SIZE {
            return None;
        }
        let records_count = bytes.get_u32_le();
        let block_size = bytes.get_u32_le();
        let checksum = bytes.get_u32_le();
        let min_timestamp = bytes.get_i64_le();
        let max_timestamp = bytes.get_i64_le();
        let header_checksum = bytes.get_u32_le();

        Some(Self {
            records_count,
            block_size,
            checksum,
            min_timestamp,
            max_timestamp,
            header_checksum,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct EventRef {
    pub segment_id: u32,
    pub block_offset: u64,
    pub record_index: u32,
}
