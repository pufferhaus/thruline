use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactStore {
    pub files: HashMap<String, PathBuf>,
    pub refs:  HashMap<String, String>,
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_file(&mut self, key: impl Into<String>, path: impl Into<PathBuf>) {
        self.files.insert(key.into(), path.into());
    }

    pub fn set_ref(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.refs.insert(key.into(), value.into());
    }

    pub fn get_file(&self, key: &str) -> Option<&PathBuf> {
        self.files.get(key)
    }

    pub fn get_ref(&self, key: &str) -> Option<&str> {
        self.refs.get(key).map(|s| s.as_str())
    }

    pub fn file_keys(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }

    pub fn ref_keys(&self) -> impl Iterator<Item = &String> {
        self.refs.keys()
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in &self.files {
            map.insert(k.clone(), serde_json::Value::String(v.to_string_lossy().into_owned()));
        }
        for (k, v) in &self.refs {
            map.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        serde_json::Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_get_ref() {
        let mut store = ArtifactStore::new();
        store.set_ref("interview.verdict", "approved");
        assert_eq!(store.get_ref("interview.verdict"), Some("approved"));
        assert_eq!(store.get_ref("other"), None);
    }

    #[test]
    fn test_set_get_file() {
        let mut store = ArtifactStore::new();
        store.set_file("interview.spec", "/tmp/spec.md");
        assert_eq!(
            store.get_file("interview.spec"),
            Some(&PathBuf::from("/tmp/spec.md"))
        );
    }

    #[test]
    fn test_to_json_contains_both_types() {
        let mut store = ArtifactStore::new();
        store.set_ref("stage.v", "ok");
        store.set_file("stage.f", "/tmp/f.md");
        let j = store.to_json();
        assert_eq!(j["stage.v"], "ok");
        assert_eq!(j["stage.f"], "/tmp/f.md");
    }

    #[test]
    fn test_default_is_empty() {
        let store = ArtifactStore::default();
        assert_eq!(store.get_ref("any"), None);
        assert_eq!(store.get_file("any"), None);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut store = ArtifactStore::new();
        store.set_ref("a.b", "value");
        store.set_file("c.d", "/some/path");
        let json = serde_json::to_string(&store).unwrap();
        let back: ArtifactStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get_ref("a.b"), Some("value"));
        assert_eq!(back.get_file("c.d"), Some(&PathBuf::from("/some/path")));
    }
}
