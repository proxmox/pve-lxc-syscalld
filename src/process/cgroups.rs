use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

#[derive(Default)]
pub struct CGroups {
    pub v1: Option<HashMap<String, OsString>>,
    pub v2: Option<OsString>,
}

impl CGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.v1
            .as_ref()
            .and_then(|v1| v1.get(name).map(|s| s.as_os_str()))
    }

    pub fn v2(&self) -> Option<&OsStr> {
        self.v2.as_deref()
    }

    pub fn has_v1(&self) -> bool {
        self.v1.is_some()
    }
}
