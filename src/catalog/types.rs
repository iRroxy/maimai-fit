use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CatalogError {}

impl From<&str> for CatalogError {
    fn from(s: &str) -> Self {
        CatalogError(s.to_string())
    }
}

impl From<String> for CatalogError {
    fn from(s: String) -> Self {
        CatalogError(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Mcp,
    Demo,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoodItem {
    pub name: String,
    pub kcal: String,
    pub protein_g: String,
    pub sodium_mg: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogDocument {
    pub schema_version: i64,
    pub source: Source,
    pub foods: Vec<FoodItem>,
}

#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    pub format: Option<String>,
    pub input_rows: Option<usize>,
    pub duplicate_rows_removed: Option<usize>,
}

pub fn casefold(s: &str) -> String {
    s.to_uppercase().to_lowercase()
}

pub fn normalize_key(value: &str) -> String {
    let replaced = value.replace('（', "(").replace('）', ")");
    let stripped: String = replaced.chars().filter(|c| !c.is_whitespace()).collect();
    casefold(&stripped)
}
