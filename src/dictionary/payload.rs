use super::{Dictionary, DictionaryId};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadId(pub u32);

impl DictionaryId for PayloadId {
    fn from_index(index: u32) -> Self {
        Self(index)
    }

    fn index(self) -> u32 {
        self.0
    }
}

pub type PayloadDictionary = Dictionary<PayloadId>;