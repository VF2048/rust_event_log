use super::{Dictionary, DictionaryId};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

impl DictionaryId for EntityId {
    fn from_index(index: u32) -> Self {
        Self(index)
    }

    fn index(self) -> u32 {
        self.0
    }
}

pub type EntityDictionary = Dictionary<EntityId>;