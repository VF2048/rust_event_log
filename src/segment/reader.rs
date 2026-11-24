use super::header::{
    BLOCK_HEADER_SIZE, BlockHeader, FORMAT_VERSION, MAGIC, SEGMENT_HEADER_SIZE, SegmentHeader, EventRef
};
use crate::event::{
    codec::{self, DecodeError},
    model::{BIN_SIZE, Event},
};
use bytes::Bytes;
use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;

/// Защита от чрезмерного выделения памяти при повреждении поля block_size (например, лимит 64 МБ)
pub const MAX_BLOCK_SIZE: u32 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum SegmentError {
    Io(io::Error),
    InvalidMagic([u8; 8]),
    UnsupportedVersion(u16),
    IncompatibleRecordSize(u32),
    TruncatedSegmentHeader,
    TruncatedBlockHeader {
        offset: u64,
    },
    TruncatedBlockPayload {
        offset: u64,
    },
    InvalidBlockSize {
        expected: u32,
        actual: u32,
    },
    BlockTooLarge(u32),
    ChecksumMismatch {
        offset: u64,
        expected: u32,
        actual: u32,
    },
    EventDecode(DecodeError),
    TimestampOutOfBounds,
    SegmentIdMismatch {
        expected: u32,
        actual: u32,
    },
    RecordIndexOutOfBounds {
        index: u32,
        count: u32,
    },
    HeaderChecksumMismatch {
        offset: u64,
    },
    EmptyBlock {
        offset: u64,
    },
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Ошибка ввода-вывода: {}", e),
            Self::InvalidMagic(m) => write!(f, "Неверная сигнатура сегмента: {:?}", m),
            Self::UnsupportedVersion(v) => write!(f, "Неподдерживаемая версия формата: {}", v),
            Self::IncompatibleRecordSize(s) => write!(f, "Несовместимый размер записи: {}", s),
            Self::TruncatedSegmentHeader => write!(f, "Файл обрезан внутри заголовка сегмента"),
            Self::TruncatedBlockHeader { offset } => {
                write!(f, "Обрезан заголовок блока на смещении {}", offset)
            }
            Self::TruncatedBlockPayload { offset } => {
                write!(f, "Обрезано тело блока на смещении {}", offset)
            }
            Self::InvalidBlockSize { expected, actual } => {
                write!(
                    f,
                    "Несоответствие размера блока: ожидалось {}, получено {}",
                    expected, actual
                )
            }
            Self::BlockTooLarge(size) => {
                write!(f, "Размер блока {} превышает допустимый лимит", size)
            }
            Self::ChecksumMismatch {
                offset,
                expected,
                actual,
            } => write!(
                f,
                "Ошибка CRC32 в блоке на смещении {}: ожидалось {:#X}, получено {:#X}",
                offset, expected, actual
            ),
            Self::EventDecode(e) => write!(f, "Ошибка декодирования события: {:?}", e),
            Self::TimestampOutOfBounds => write!(
                f,
                "Временная метка события выходит за границы заголовка блока"
            ),
                        Self::SegmentIdMismatch { expected, actual } => write!(
                f,
                "Несоответствие ID сегмента: ожидался {}, получен {}",
                expected, actual
            ),
            Self::RecordIndexOutOfBounds { index, count } => write!(
                f,
                "Индекс записи {} выходит за пределы блока (всего записей: {})",
                index, count
            ),
            Self::HeaderChecksumMismatch { offset } => {
                write!(f, "Неверная контрольная сумма заголовка на смещении {}", offset)
            },
            Self::EmptyBlock { offset } => {
                write!(f, "Обнаружен пустой блок (0 записей) на смещении {}", offset)
            },
        }
    }
}

impl std::error::Error for SegmentError {}

impl From<io::Error> for SegmentError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<DecodeError> for SegmentError {
    fn from(err: DecodeError) -> Self {
        Self::EventDecode(err)
    }
}

pub struct SegmentReader {
    reader: BufReader<File>,
    header: SegmentHeader,
    current_block_events: VecDeque<Event>,
    current_offset: u64,
}

impl SegmentReader {
    /// Открывает сегмент и валидирует SegmentHeader
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, SegmentError> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut header_buf = [0u8; SEGMENT_HEADER_SIZE];
        match reader.read_exact(&mut header_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(SegmentError::TruncatedSegmentHeader);
            }
            Err(e) => return Err(SegmentError::Io(e)),
        }

        let header =
            SegmentHeader::decode(&header_buf).ok_or(SegmentError::TruncatedSegmentHeader)?;

            if header.header_checksum != header.compute_checksum() {
                return Err(SegmentError::HeaderChecksumMismatch { offset: 0 });
            }
        if header.magic != MAGIC {
            return Err(SegmentError::InvalidMagic(header.magic));
        }
        if header.version != FORMAT_VERSION {
            return Err(SegmentError::UnsupportedVersion(header.version));
        }
        if header.record_size != BIN_SIZE as u32 {
            return Err(SegmentError::IncompatibleRecordSize(header.record_size));
        }

        Ok(Self {
            reader,
            header,
            current_block_events: VecDeque::new(),
            current_offset: SEGMENT_HEADER_SIZE as u64,
        })
    }

    pub fn header(&self) -> &SegmentHeader {
        &self.header
    }

    /// Возвращает текущую позицию чтения в файле (на границе последнего прочитанного блока)
    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }

    /// Читает следующее событие из сегмента. Возвращает Ok(None) при штатном конце файла.
    pub fn next_event(&mut self) -> Result<Option<Event>, SegmentError> {
        while self.current_block_events.is_empty() {
            if !self.load_next_block()? {
                return Ok(None);
            }
        }
        Ok(self.current_block_events.pop_front())
    }

    fn load_next_block(&mut self) -> Result<bool, SegmentError> {
        let block_offset = self.current_offset;
        let mut header_buf = [0u8; BLOCK_HEADER_SIZE];

        // Проверяем, начался ли новый блок или файл закончился ровно на границе
        if !read_exact_or_eof(&mut self.reader, &mut header_buf, block_offset)? {
            return Ok(false);
        }

        let block_header =
            BlockHeader::decode(&header_buf).ok_or(SegmentError::TruncatedBlockHeader {
                offset: block_offset,
            })?;

        if block_header.header_checksum != block_header.compute_checksum() {
            return Err(SegmentError::HeaderChecksumMismatch {
                offset: block_offset,
            });
        }
        if block_header.records_count == 0 {
            return Err(SegmentError::EmptyBlock {
                offset: block_offset, // или event_ref.block_offset в read_at
            });
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
        
        if block_header.block_size > MAX_BLOCK_SIZE {
            return Err(SegmentError::BlockTooLarge(block_header.block_size));
        }

        let mut payload_buf = vec![0u8; block_header.block_size as usize];
        match self.reader.read_exact(&mut payload_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(SegmentError::TruncatedBlockPayload {
                    offset: block_offset,
                });
            }
            Err(e) => return Err(SegmentError::Io(e)),
        }

        let actual_crc = crc32fast::hash(&payload_buf);
        if actual_crc != block_header.checksum {
            return Err(SegmentError::ChecksumMismatch {
                offset: block_offset,
                expected: block_header.checksum,
                actual: actual_crc,
            });
        }

        let mut cursor = Bytes::from(payload_buf);
        self.current_block_events
            .reserve(block_header.records_count as usize);

        for _ in 0..block_header.records_count {
            let event = codec::decode(&mut cursor)?.ok_or(SegmentError::TruncatedBlockPayload {
                offset: block_offset,
            })?;

            if event.timestamp < block_header.min_timestamp
                || event.timestamp > block_header.max_timestamp
            {
                return Err(SegmentError::TimestampOutOfBounds);
            }

            self.current_block_events.push_back(event);
        }

        self.current_offset += (BLOCK_HEADER_SIZE as u64) + (block_header.block_size as u64);
        Ok(true)
    }

    pub fn read_at(&mut self, event_ref: EventRef) -> Result<Event, SegmentError> {
        // 1. Проверяем, что ссылка относится к текущему сегменту
        if event_ref.segment_id != self.header.segment_id {
            return Err(SegmentError::SegmentIdMismatch {
                expected: self.header.segment_id,
                actual: event_ref.segment_id,
            });
        }

        // 2. Перемещаем курсор файла на начало нужного блока
        // (seek на BufReader автоматически сбрасывает его внутренний буфер)
        self.reader.seek(SeekFrom::Start(event_ref.block_offset))?;
        self.current_block_events.clear();

        // 3. Читаем и валидируем BlockHeader (32 байт)
        let mut header_buf = [0u8; BLOCK_HEADER_SIZE];
        match self.reader.read_exact(&mut header_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(SegmentError::TruncatedBlockHeader {
                    offset: event_ref.block_offset,
                });
            }
            Err(e) => return Err(SegmentError::Io(e)),
        }

        let block_header =
            BlockHeader::decode(&header_buf).ok_or(SegmentError::TruncatedBlockHeader {
                offset: event_ref.block_offset,
            })?;

        if block_header.header_checksum != block_header.compute_checksum() {
            return Err(SegmentError::HeaderChecksumMismatch {
                offset: event_ref.block_offset,
            });
        }
        if block_header.records_count == 0 {
            return Err(SegmentError::EmptyBlock {
                offset: event_ref.block_offset, // или event_ref.block_offset в read_at
            });
        }

        // 4. Проверяем, что запрашиваемый индекс существует в этом блоке
        if event_ref.record_index >= block_header.records_count {
            return Err(SegmentError::RecordIndexOutOfBounds {
                index: event_ref.record_index,
                count: block_header.records_count,
            });
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
        
        if block_header.block_size > MAX_BLOCK_SIZE {
            return Err(SegmentError::BlockTooLarge(block_header.block_size));
        }

        // 5. Читаем полезные данные блока и сверяем CRC32
        let mut payload_buf = vec![0u8; block_header.block_size as usize];
        match self.reader.read_exact(&mut payload_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(SegmentError::TruncatedBlockPayload {
                    offset: event_ref.block_offset,
                });
            }
            Err(e) => return Err(SegmentError::Io(e)),
        }

        let actual_crc = crc32fast::hash(&payload_buf);
        if actual_crc != block_header.checksum {
            return Err(SegmentError::ChecksumMismatch {
                offset: event_ref.block_offset,
                expected: block_header.checksum,
                actual: actual_crc,
            });
        }

        // 6. Вырезаем только нужное событие по смещению без декодирования всего блока
        let record_offset = (event_ref.record_index as usize) * BIN_SIZE;
        let mut record_bytes =
            Bytes::copy_from_slice(&payload_buf[record_offset..record_offset + BIN_SIZE]);

        let event =
            codec::decode(&mut record_bytes)?.ok_or(SegmentError::TruncatedBlockPayload {
                offset: event_ref.block_offset,
            })?;

        if event.timestamp < block_header.min_timestamp
            || event.timestamp > block_header.max_timestamp
        {
            return Err(SegmentError::TimestampOutOfBounds);
        }

        // Обновляем current_offset на конец прочитанного блока,
        // чтобы при желании можно было продолжить последовательное чтение дальше
        self.current_offset =
            event_ref.block_offset + (BLOCK_HEADER_SIZE as u64) + (block_header.block_size as u64);

        Ok(event)
    }
}

impl Iterator for SegmentReader {
    type Item = Result<Event, SegmentError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_event().transpose()
    }
}

/// Читает `buf.len()` байтов.
/// Возвращает `Ok(false)`, если поток закончился сразу (прочитано 0 байтов).
/// Возвращает `TruncatedBlockHeader`, если прочитана только часть заголовка.
fn read_exact_or_eof<R: Read>(
    reader: &mut R,
    mut buf: &mut [u8],
    offset: u64,
) -> Result<bool, SegmentError> {
    let total_len = buf.len();
    while !buf.is_empty() {
        match reader.read(buf) {
            Ok(0) if buf.len() == total_len => return Ok(false),
            Ok(0) => return Err(SegmentError::TruncatedBlockHeader { offset }),
            Ok(n) => buf = &mut buf[n..],
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(SegmentError::Io(e)),
        }
    }
    Ok(true)
}
