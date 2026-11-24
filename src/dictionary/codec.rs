use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::{Dictionary, DictionaryId};

pub const MAGIC: [u8; 8] = *b"\x89DCTLOG\n";
// v1 использовал u16-длину строки, v2 использует u32.
pub const FORMAT_VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 14; // magic (8) + version (2) + records count (4)
pub const RECORD_HEADER_SIZE: usize = 8; // id (4) + length (4)

/// Ошибка кодирования словаря в бинарный формат.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Записей больше, чем помещается в `u32` в заголовке.
    TooManyRecords { count: usize },
    /// Строка не помещается в `u32`-длину записи.
    ValueTooLong { id: u32, len: usize },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count } => {
                write!(f, "в словаре {count} записей, формат допускает не более {}", u32::MAX)
            }
            Self::ValueTooLong { id, len } => {
                write!(f, "строка id={id} длиной {len} байт не помещается в u32")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Ошибка сохранения словаря в файл.
#[derive(Debug)]
pub enum SaveError {
    Encode(EncodeError),
    Io(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => write!(f, "не удалось закодировать словарь: {error}"),
            Self::Io(error) => write!(f, "не удалось записать словарь: {error}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<EncodeError> for SaveError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Сериализует словарь в бинарный буфер.
///
/// Формат: заголовок `magic | version | records count`, затем записи
/// `id (u32) | length (u32) | utf-8 bytes`. Все числа Little-Endian.
pub fn encode<Id: DictionaryId>(dict: &Dictionary<Id>) -> Result<Bytes, EncodeError> {
    let count = u32::try_from(dict.by_id.len())
        .map_err(|_| EncodeError::TooManyRecords { count: dict.by_id.len() })?;

    let mut buf = BytesMut::new();
    buf.put_slice(&MAGIC);
    buf.put_u16_le(FORMAT_VERSION);
    buf.put_u32_le(count);

    for (index, value) in dict.by_id.iter().enumerate() {
        // index меньше count, который уже проверен на вхождение в u32.
        let id = index as u32;
        let value = value.as_bytes();
        let len =
            u32::try_from(value.len()).map_err(|_| EncodeError::ValueTooLong { id, len: value.len() })?;

        buf.put_u32_le(id);
        buf.put_u32_le(len);
        buf.put_slice(value);
    }

    Ok(buf.freeze())
}

/// Сохраняет словарь в файл.
pub fn save<Id: DictionaryId, P: AsRef<Path>>(
    dict: &Dictionary<Id>,
    path: P,
) -> Result<(), SaveError> {
    let bytes = encode(dict)?;
    fs::write(path, bytes)?;
    Ok(())
}

/// Ошибка декодирования словаря из бинарного формата.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Буфер короче заголовка.
    TooShort { len: usize, min: usize },
    /// Magic не совпадает с форматом словаря.
    InvalidMagic { found: [u8; 8] },
    /// Версия формата не поддерживается.
    UnsupportedVersion { found: u16, expected: u16 },
    /// Запись или её строка обрываются раньше, чем заявлено в длине.
    UnexpectedEof,
    /// Идентификаторы должны идти подряд начиная с нуля.
    InvalidId { expected: u32, actual: u32 },
    /// Байты записи не являются корректным UTF-8.
    InvalidUtf8 { id: u32 },
    /// Одна и та же строка встретилась под разными id.
    DuplicateValue { first_id: u32, id: u32 },
    /// После последней записи остались лишние байты.
    TrailingBytes { count: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { len, min } => {
                write!(f, "файл слишком короткий: {len} байт, нужно минимум {min}")
            }
            Self::InvalidMagic { found } => write!(f, "неверный magic: {found:02X?}"),
            Self::UnsupportedVersion { found, expected } => {
                write!(f, "неподдерживаемая версия {found}, ожидалась {expected}")
            }
            Self::UnexpectedEof => f.write_str("неожиданный конец файла"),
            Self::InvalidId { expected, actual } => {
                write!(f, "ожидался id {expected}, получен {actual}")
            }
            Self::InvalidUtf8 { id } => write!(f, "запись id={id} не является корректным UTF-8"),
            Self::DuplicateValue { first_id, id } => {
                write!(f, "строка с id={id} уже встречалась под id={first_id}")
            }
            Self::TrailingBytes { count } => {
                write!(f, "после последней записи осталось {count} лишних байт")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Ошибка загрузки словаря из файла.
#[derive(Debug)]
pub enum LoadError {
    Decode(DecodeError),
    Io(io::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "не удалось декодировать словарь: {error}"),
            Self::Io(error) => write!(f, "не удалось прочитать словарь: {error}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<DecodeError> for LoadError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

impl From<io::Error> for LoadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Разбирает словарь из бинарного буфера.
///
/// Проверяет заголовок, версию, последовательность id, границы записей,
/// корректность UTF-8 и отсутствие лишних байт в конце.
pub fn decode<Id: DictionaryId>(bytes: &[u8]) -> Result<Dictionary<Id>, DecodeError> {
    let mut cursor = bytes;

    let mut magic = [0u8; 8];
    if cursor.len() < HEADER_SIZE {
        return Err(DecodeError::TooShort {
            len: cursor.len(),
            min: HEADER_SIZE,
        });
    }
    cursor.copy_to_slice(&mut magic);
    if magic != MAGIC {
        return Err(DecodeError::InvalidMagic { found: magic });
    }

    let version = cursor.get_u16_le();
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion {
            found: version,
            expected: FORMAT_VERSION,
        });
    }

    let count = cursor.get_u32_le();
    let mut by_id = Vec::new();
    let mut by_value = HashMap::new();

    for expected_id in 0..count {
        // id (4) + length (4), без байтов строки.
        if cursor.len() < RECORD_HEADER_SIZE {
            return Err(DecodeError::UnexpectedEof);
        }

        let id = cursor.get_u32_le();
        let len = cursor.get_u32_le() as usize;

        if id != expected_id {
            return Err(DecodeError::InvalidId {
                expected: expected_id,
                actual: id,
            });
        }

        if cursor.len() < len {
            return Err(DecodeError::UnexpectedEof);
        }

        let (chunk, rest) = cursor.split_at(len);
        cursor = rest;

        let text = std::str::from_utf8(chunk)
            .map_err(|_| DecodeError::InvalidUtf8 { id })?
            .to_owned();

        let id = Id::from_index(expected_id);
        if let Some(previous) = by_value.insert(text.clone(), id) {
            return Err(DecodeError::DuplicateValue {
                first_id: previous.index(),
                id: expected_id,
            });
        }
        by_id.push(text);
    }

    if !cursor.is_empty() {
        return Err(DecodeError::TrailingBytes {
            count: cursor.len(),
        });
    }

    Ok(Dictionary { by_value, by_id })
}

/// Загружает словарь из файла.
pub fn load<Id: DictionaryId, P: AsRef<Path>>(path: P) -> Result<Dictionary<Id>, LoadError> {
    let bytes = fs::read(path)?;
    let dict = decode(&bytes)?;
    Ok(dict)
}
