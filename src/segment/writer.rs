use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use bytes::{BufMut, BytesMut};
use crate::event::{codec, model::{Event, BIN_SIZE}};
use super::header::{SegmentHeader, SEGMENT_HEADER_SIZE, BlockHeader, BLOCK_HEADER_SIZE};

pub struct SegmentWriter{
    writer: BufWriter<File>,
    max_block_records: usize,
    payload_buf: BytesMut,
    records_in_block: u32,
    min_timestamp: i64,
    max_timestamp: i64,
    bytes_written: u64,
}

impl SegmentWriter {
    pub fn create<P: AsRef<Path>>(
        path: P,
        segment_id: u32,
        created_at: i64,
        max_block_records: usize,
    ) -> io::Result<Self> {

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        let mut writer = BufWriter::new(file);
        let header = SegmentHeader::new(segment_id,created_at);
        writer.write_all(&header.encode())?;
        writer.flush()?;

        Ok(Self {
            writer,
            max_block_records,
            payload_buf: BytesMut::with_capacity(max_block_records * BIN_SIZE),
            records_in_block: 0,
            min_timestamp: i64::MAX,
            max_timestamp: i64::MIN,
            bytes_written: SEGMENT_HEADER_SIZE as u64,
        })
    }
    
    pub fn append(&mut self, event: &Event) -> io::Result<()> {
        let encoded = codec::encode(event);
        self.payload_buf.put_slice(&encoded);

        if self.records_in_block == 0 {
            self.min_timestamp = event.timestamp;
            self.max_timestamp = event.timestamp;
        } else {
            self.min_timestamp = self.min_timestamp.min(event.timestamp);
            self.max_timestamp = self.max_timestamp.max(event.timestamp);
        }

        self.records_in_block += 1;

        if self.records_in_block as usize >= self.max_block_records {
            self.flush_block()?;
        }

        Ok(())
    }

    pub fn flush_block(&mut self) -> io::Result<()> {
        if self.records_in_block == 0 {
            return Ok(());
        }

        let block_size = self.payload_buf.len() as u32;
        let checksum = crc32fast::hash(&self.payload_buf);

        let block_header = BlockHeader {
            records_count: self.records_in_block,
            block_size,
            checksum,
            min_timestamp: self.min_timestamp,
            max_timestamp: self.max_timestamp,
        };

        self.writer.write_all(&block_header.encode())?;
        self.writer.write_all(&self.payload_buf)?;
        self.writer.flush()?;

        self.bytes_written += (BLOCK_HEADER_SIZE as u64) + (block_size as u64);

        self.payload_buf.clear();
        self.records_in_block = 0;
        self.min_timestamp = i64::MAX;
        self.max_timestamp = i64::MIN;

        Ok(())
    }

    pub fn finish(mut self) -> io::Result<u64> {
        self.flush_block()?;
        self.writer.get_ref().sync_all()?;
        Ok(self.bytes_written)
    }

}
