use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

#[derive(Default)]
pub struct CGroups {
    pub v1: HashMap<String, OsString>,
    pub v2: Option<OsString>,
}

impl CGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.v1.get(name).map(|s| s.as_os_str())
    }

    pub fn v2(&self) -> Option<&OsStr> {
        self.v2.as_ref().map(|s| s.as_os_str())
    }
}
