pub mod codec;
pub mod entity;
pub mod payload;

use std::collections::HashMap;
use std::fmt;

/// Идентификатор записи словаря. Значение id совпадает с индексом в `by_id`.
pub trait DictionaryId: Copy {
    fn from_index(index: u32) -> Self;
    fn index(self) -> u32;
}

/// Ошибки словаря.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictionaryError {
    /// Свободные идентификаторы закончились.
    IdSpaceExhausted,
}

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdSpaceExhausted => f.write_str("пространство идентификаторов словаря исчерпано"),
        }
    }
}

impl std::error::Error for DictionaryError {}

pub struct Dictionary<Id> {
    by_value: HashMap<String, Id>,
    by_id: Vec<String>,
}

impl<Id> Dictionary<Id> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<Id> Default for Dictionary<Id> {
    fn default() -> Self {
        Self {
            by_value: HashMap::new(),
            by_id: Vec::new(),
        }
    }
}

impl<Id: DictionaryId> Dictionary<Id> {
    /// Возвращает id существующей строки или добавляет новую.
    ///
    /// Ошибка возвращается, если в пространстве id не осталось свободных значений.
    pub fn get_or_insert(&mut self, value: &str) -> Result<Id, DictionaryError> {
        if let Some(&id) = self.by_value.get(value) {
            return Ok(id);
        }

        // Проверяем ёмкость до изменения словаря, чтобы при ошибке он остался прежним.
        let index = u32::try_from(self.by_id.len())
            .map_err(|_| DictionaryError::IdSpaceExhausted)?;
        let id = Id::from_index(index);

        // Строка хранится дважды: как ключ поиска и как значение по индексу.
        let owned = value.to_owned();
        self.by_value.insert(owned.clone(), id);
        self.by_id.push(owned);

        Ok(id)
    }

    /// Возвращает строку по id или `None`, если такого id нет.
    pub fn get(&self, id: Id) -> Option<&str> {
        self.by_id.get(id.index() as usize).map(String::as_str)
    }
}