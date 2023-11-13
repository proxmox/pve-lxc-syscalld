//! uid/gid mapping helpers

pub struct IdMap(Vec<IdMapEntry>);

pub struct IdMapEntry {
    pub ns: u64,
    pub host: u64,
    pub range: u64,
}

impl IdMap {
    pub fn new(entries: Vec<IdMapEntry>) -> Self {
        Self(entries)
    }

    pub fn map_into(&self, id: u64) -> Option<u64> {
        for entry in self.0.iter() {
            if entry.host <= id && entry.host + entry.range > id {
                return Some(entry.ns + id - entry.host);
            }
        }

        None
    }

    pub fn map_from(&self, id: u64) -> Option<u64> {
        for entry in self.0.iter() {
            if entry.ns <= id && entry.ns + entry.range > id {
                return Some(entry.host + id - entry.ns);
            }
        }

        None
    }
}
