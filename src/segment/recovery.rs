use super::header::{
    BlockHeader, SegmentHeader, BLOCK_HEADER_SIZE, FORMAT_VERSION, MAGIC, SEGMENT_HEADER_SIZE,
};
use super::reader::{SegmentError, MAX_BLOCK_SIZE};
use crate::event::{codec, model::BIN_SIZE};
use bytes::Bytes;
use std::fs::OpenOptions;
use std::io::{BufReader, ErrorKind, Read};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryReport {
    /// Смещение, до которого файл был усечён (совпадает с итоговым размером файла)
    pub valid_bytes: u64,
    /// Сколько байтов повреждённого хвоста было удалено
    pub truncated_bytes: u64,
    /// Количество уцелевших блоков
    pub valid_blocks: u64,
    /// Количество уцелевших событий
    pub valid_events: u64,
    /// Причина остановки (None, если файл был полностью исправен)
    pub corruption_reason: Option<String>,
}

/// Проверяет сегмент и при обнаружении повреждённого или недописанного хвоста
/// обрезает файл до границы последнего валидного блока.
pub fn recover_segment<P: AsRef<Path>>(path: P) -> Result<RecoveryReport, SegmentError> {
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let original_len = file.metadata()?.len();
    let mut reader = BufReader::new(&file);

    // 1. Проверяем SegmentHeader (если повреждён он — восстановить сегмент невозможно)
    let mut seg_header_buf = [0u8; SEGMENT_HEADER_SIZE];
    match reader.read_exact(&mut seg_header_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
            return Err(SegmentError::TruncatedSegmentHeader);
        }
        Err(e) => return Err(SegmentError::Io(e)),
    }

    let seg_header =
        SegmentHeader::decode(&seg_header_buf).ok_or(SegmentError::TruncatedSegmentHeader)?;

    if seg_header.magic != MAGIC {
        return Err(SegmentError::InvalidMagic(seg_header.magic));
    }
    if seg_header.compute_checksum() != seg_header.header_checksum {
        return Err(SegmentError::HeaderChecksumMismatch { offset: 0 });
    }
    if seg_header.version != FORMAT_VERSION {
        return Err(SegmentError::UnsupportedVersion(seg_header.version));
    }
    if seg_header.record_size != BIN_SIZE as u32 {
        return Err(SegmentError::IncompatibleRecordSize(seg_header.record_size));
    }

    let mut last_valid_offset = SEGMENT_HEADER_SIZE as u64;
    let mut valid_blocks = 0u64;
    let mut valid_events = 0u64;
    let mut corruption_reason = None;

    // 2. Сканируем блоки до EOF или до первого сбоя
    loop {
        match verify_next_block(&mut reader, last_valid_offset) {
            Ok(Some(records_in_block)) => {
                let block_total_len = (BLOCK_HEADER_SIZE as u64)
                    + (records_in_block as u64) * (BIN_SIZE as u64);
                last_valid_offset += block_total_len;
                valid_blocks += 1;
                valid_events += records_in_block as u64;
            }
            Ok(None) => {
                // Штатный конец файла на границе блока
                break;
            }
            Err(SegmentError::Io(e)) => {
                // Системная ошибка чтения диска — не обрезаем файл, возвращаем ошибку
                return Err(SegmentError::Io(e));
            }
            Err(corruption_err) => {
                // Обнаружено повреждение данных или обрыв записи — фиксируем причину и прерываем цикл
                corruption_reason = Some(corruption_err.to_string());
                break;
            }
        }
    }

    // 3. Если хвост файла повреждён — усекаем его до last_valid_offset
    let truncated_bytes = original_len.saturating_sub(last_valid_offset);
    if truncated_bytes > 0 {
        drop(reader); // Освобождаем ссылку на file перед изменением размера
        file.set_len(last_valid_offset)?;
        file.sync_all()?;
    }

    Ok(RecoveryReport {
        valid_bytes: last_valid_offset,
        truncated_bytes,
        valid_blocks,
        valid_events,
        corruption_reason,
    })
}

/// Читает и проверяет один блок, возвращая количество записей в нём (или None при чистом EOF).
fn verify_next_block<R: Read>(
    reader: &mut R,
    offset: u64,
) -> Result<Option<u32>, SegmentError> {
    let mut header_buf = [0u8; BLOCK_HEADER_SIZE];
    let mut filled = 0;

    while filled < BLOCK_HEADER_SIZE {
        match reader.read(&mut header_buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(None), // Чистый EOF
            Ok(0) => return Err(SegmentError::TruncatedBlockHeader { offset }),
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(SegmentError::Io(e)),
        }
    }

    let block_header = BlockHeader::decode(&header_buf)
        .ok_or(SegmentError::TruncatedBlockHeader { offset })?;

    if block_header.compute_checksum() != block_header.header_checksum {
        return Err(SegmentError::HeaderChecksumMismatch { offset });
    }

    if block_header.records_count == 0 {
        return Err(SegmentError::EmptyBlock { offset });
    }

    if block_header.block_size > MAX_BLOCK_SIZE {
        return Err(SegmentError::BlockTooLarge(block_header.block_size));
    }

    let expected_size = block_header
        .records_count
        .checked_mul(BIN_SIZE as u32)
        .ok_or(SegmentError::BlockTooLarge(u32::MAX))?;

    if block_header.block_size != expected_size {
        return Err(SegmentError::InvalidBlockSize {
            expected: expected_size,
            actual: block_header.block_size,
        });
    }

    let mut payload_buf = vec![0u8; block_header.block_size as usize];
    match reader.read_exact(&mut payload_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
            return Err(SegmentError::TruncatedBlockPayload { offset });
        }
        Err(e) => return Err(SegmentError::Io(e)),
    }

    let actual_crc = crc32fast::hash(&payload_buf);
    if actual_crc != block_header.checksum {
        return Err(SegmentError::ChecksumMismatch {
            offset,
            expected: block_header.checksum,
            actual: actual_crc,
        });
    }

    // Проверяем, что каждое событие внутри блока декодируется без ошибок
    let mut cursor = Bytes::from(payload_buf);
    for _ in 0..block_header.records_count {
        let event = codec::decode(&mut cursor)?
            .ok_or(SegmentError::TruncatedBlockPayload { offset })?;

        if event.timestamp < block_header.min_timestamp
            || event.timestamp > block_header.max_timestamp
        {
            return Err(SegmentError::TimestampOutOfBounds);
        }
    }

    Ok(Some(block_header.records_count))
}

