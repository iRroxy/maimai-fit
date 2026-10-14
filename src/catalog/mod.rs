pub mod markdown;
pub mod strict_json;
pub mod toon;
pub mod types;

pub use markdown::parse_markdown_rows;
pub use strict_json::parse_strict_json;
pub use toon::{parse_mcd_toon_rows, RawFoodRow};
pub use types::{
    casefold, normalize_key, CatalogDocument, CatalogError, Diagnostics, FoodItem, Source,
    SourceKind,
};

use bigdecimal::BigDecimal;
use chrono::DateTime;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

static NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$").unwrap());

static PER_100_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:per\s*100\s*(?:grams?|g|millilit(?:er|re)s?|ml)\b|每\s*(?:100|一百)\s*(?:公克|克|毫升|g|ml)|/\s*100\s*(?:grams?|g|ml)\b)").unwrap()
});

static UNKNOWN_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["", "-", "—", "未知", "unknown", "n/a", "na", "null"]
        .iter()
        .copied()
        .collect()
});

static NAMES_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["name", "foodname", "名称", "食品名称", "餐品名称"]
        .iter()
        .copied()
        .collect()
});

static ENERGY_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    [
        ("kcal", "kcal"),
        ("energy_kcal", "kcal"),
        ("energy_kj", "kJ"),
        ("能量(kcal)", "kcal"),
        ("热量(kcal)", "kcal"),
        ("能量(kj)", "kJ"),
        ("热量(kj)", "kJ"),
        ("energy(kcal)", "kcal"),
        ("energy(kj)", "kJ"),
    ]
    .iter()
    .copied()
    .collect()
});

static PROTEIN_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["protein_g", "蛋白质(g)", "protein(g)"]
        .iter()
        .copied()
        .collect()
});

static SODIUM_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["sodium_mg", "钠(mg)", "sodium(mg)"]
        .iter()
        .copied()
        .collect()
});

pub fn parse_decimal(value: &Value, field: &str) -> Result<BigDecimal, CatalogError> {
    if value.is_boolean() {
        return Err(CatalogError(format!(
            "{field} must be an explicit finite nonnegative number."
        )));
    }

    let text = match value {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => {
            return Err(CatalogError(format!(
                "{field} must be an explicit finite nonnegative number."
            )));
        }
    };

    if text.len() > 128 || !NUMBER_RE.is_match(&text) {
        return Err(CatalogError(format!(
            "{field} must be numeric without unknown or inferred units."
        )));
    }

    let number = BigDecimal::from_str(&text).map_err(|_| {
        CatalogError(format!("{field} is not a decimal number."))
    })?;

    if number < BigDecimal::from(0) {
        return Err(CatalogError(format!(
            "{field} must be finite, nonnegative and within supported decimal precision."
        )));
    }

    Ok(number)
}

pub fn format_decimal(d: &BigDecimal) -> String {
    use bigdecimal::Zero;
    if d.is_zero() {
        "0".to_string()
    } else {
        d.to_string()
    }
}

pub fn measure(value: &Value, unit: &str, field: &str) -> Result<Option<String>, CatalogError> {
    let mut current_val = value.clone();
    if let Value::String(s) = value {
        let text = s.trim();
        if field == "sodium_mg" && UNKNOWN_SET.contains(casefold(text).as_str()) {
            return Ok(None);
        }
        let unit_re = Regex::new(r"(?i)^(.+?)\s*(kcal|kj|mg|g)$").unwrap();
        if let Some(caps) = unit_re.captures(text) {
            let matched_unit = &caps[2];
            if casefold(matched_unit) != casefold(unit) {
                return Err(CatalogError(format!(
                    "{field} cell unit conflicts with its explicit header/key unit."
                )));
            }
            current_val = Value::String(caps[1].trim().to_string());
        }
    }

    if current_val.is_null() && field == "sodium_mg" {
        return Ok(None);
    }

    let d = parse_decimal(&current_val, field)?;
    if unit == "kJ" {
        let divisor = BigDecimal::from_str("4.184").unwrap();
        let kcal = d / divisor;
        Ok(Some(format_decimal(&kcal)))
    } else {
        Ok(Some(format_decimal(&d)))
    }
}

pub fn alias_row(row: &HashMap<String, Value>) -> Result<RawFoodRow, CatalogError> {
    let mut mapped_name: Option<String> = None;
    let mut mapped_kcal: Option<Value> = None;
    let mut mapped_protein: Option<Value> = None;
    let mut mapped_sodium: Option<Value> = None;

    for (orig_key, val) in row {
        let key = normalize_key(orig_key);
        let field: &str;
        let unit: Option<&str>;

        if NAMES_SET.contains(key.as_str()) {
            field = "name";
            unit = None;
        } else if let Some(&u) = ENERGY_MAP.get(key.as_str()) {
            field = "kcal";
            unit = Some(u);
        } else if PROTEIN_SET.contains(key.as_str()) {
            field = "protein_g";
            unit = Some("g");
        } else if SODIUM_SET.contains(key.as_str()) {
            field = "sodium_mg";
            unit = Some("mg");
        } else if ["sodium", "钠", "protein", "蛋白", "energy", "calori", "能量", "热量"]
            .iter()
            .any(|w| key.contains(w))
        {
            return Err(CatalogError(
                "Unsupported nutrient field or unit; use explicit kcal (or kJ), protein_g and sodium_mg keys.".into(),
            ));
        } else {
            continue;
        }

        match field {
            "name" => {
                if mapped_name.is_some() {
                    return Err(CatalogError(
                        "Nutrition row contains ambiguous duplicate nutrient/name aliases.".into(),
                    ));
                }
                let name_str = match val {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(CatalogError(
                            "Each nutrition food must be an object with named fields.".into(),
                        ));
                    }
                };
                mapped_name = Some(name_str);
            }
            "kcal" => {
                if mapped_kcal.is_some() {
                    return Err(CatalogError(
                        "Nutrition row contains ambiguous duplicate nutrient/name aliases.".into(),
                    ));
                }
                let m = measure(val, unit.unwrap(), "kcal")?
                    .ok_or_else(|| CatalogError("kcal requires an explicit value.".into()))?;
                mapped_kcal = Some(Value::String(m));
            }
            "protein_g" => {
                if mapped_protein.is_some() {
                    return Err(CatalogError(
                        "Nutrition row contains ambiguous duplicate nutrient/name aliases.".into(),
                    ));
                }
                let m = measure(val, unit.unwrap(), "protein_g")?
                    .ok_or_else(|| CatalogError("protein_g requires an explicit value.".into()))?;
                mapped_protein = Some(Value::String(m));
            }
            "sodium_mg" => {
                if mapped_sodium.is_some() {
                    return Err(CatalogError(
                        "Nutrition row contains ambiguous duplicate nutrient/name aliases.".into(),
                    ));
                }
                let m = measure(val, unit.unwrap(), "sodium_mg")?;
                mapped_sodium = Some(match m {
                    Some(s) => Value::String(s),
                    None => Value::Null,
                });
            }
            _ => unreachable!(),
        }
    }

    let name = mapped_name.ok_or_else(|| {
        CatalogError("Food row requires a serving-specific name.".into())
    })?;
    let kcal = mapped_kcal.ok_or_else(|| {
        CatalogError("Food row requires kcal and protein_g; missing values are not zero.".into())
    })?;
    let protein_g = mapped_protein.ok_or_else(|| {
        CatalogError("Food row requires kcal and protein_g; missing values are not zero.".into())
    })?;
    let sodium_mg = mapped_sodium.unwrap_or(Value::Null);

    Ok(RawFoodRow {
        name,
        kcal,
        protein_g,
        sodium_mg,
    })
}

pub fn check_basis(val: &Value) -> Result<(), CatalogError> {
    let s = val.to_string();
    if PER_100_RE.is_match(&s) {
        return Err(CatalogError(
            "Per-100g/per-100ml data cannot be used as per-serving nutrition; provide explicit per-serving data.".into(),
        ));
    }
    Ok(())
}

fn check_basis_str(text: &str) -> Result<(), CatalogError> {
    if PER_100_RE.is_match(text) {
        return Err(CatalogError(
            "Per-100g/per-100ml data cannot be used as per-serving nutrition; provide explicit per-serving data.".into(),
        ));
    }
    Ok(())
}

fn parse_food_rows(raw_rows: Vec<RawFoodRow>) -> Result<Vec<FoodItem>, CatalogError> {
    if raw_rows.is_empty() {
        return Err(CatalogError(
            "Catalog foods must be a nonempty array; no rows were imported.".into(),
        ));
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();

    for (index, row) in raw_rows.into_iter().enumerate() {
        let row_idx = index + 1;
        let name = row.name.trim().to_string();
        if name.is_empty() {
            return Err(CatalogError(format!(
                "Food row {row_idx} requires a serving-specific name."
            )));
        }
        let identity = casefold(&name);
        if !seen.insert(identity) {
            return Err(CatalogError(format!(
                "Duplicate serving name at food row {row_idx}; use distinct explicit serving names."
            )));
        }

        if row.kcal.is_null() || row.protein_g.is_null() {
            return Err(CatalogError(format!(
                "Food row {row_idx} requires kcal and protein_g; missing values are not zero."
            )));
        }

        let kcal_d = parse_decimal(&row.kcal, "kcal")?;
        let protein_d = parse_decimal(&row.protein_g, "protein_g")?;
        let sodium_val = if row.sodium_mg.is_null() {
            None
        } else {
            let sodium_d = parse_decimal(&row.sodium_mg, "sodium_mg")?;
            Some(format_decimal(&sodium_d))
        };

        result.push(FoodItem {
            name,
            kcal: format_decimal(&kcal_d),
            protein_g: format_decimal(&protein_d),
            sodium_mg: sodium_val,
        });
    }

    Ok(result)
}

pub fn json_rows(document: &Value, diagnostics: &mut Diagnostics) -> Result<Vec<RawFoodRow>, CatalogError> {
    check_basis(document)?;
    if document.is_object() && document.get("data").is_some() {
        return parse_mcd_toon_rows(document, diagnostics);
    }

    let rows_val = if let Some(arr) = document.as_array() {
        arr
    } else if let Some(foods) = document.get("foods").and_then(|v| v.as_array()) {
        foods
    } else {
        return Err(CatalogError(
            "Unsupported nutrition JSON: expected a foods array with explicit unit-bearing keys; save --raw-out and adapt the actual schema.".into(),
        ));
    };

    if rows_val.is_empty() {
        return Err(CatalogError("Nutrition foods must be a nonempty array.".into()));
    }

    let mut result = Vec::new();
    for row_val in rows_val {
        let row_obj = row_val.as_object().ok_or_else(|| {
            CatalogError("Each nutrition food must be an object with named fields.".into())
        })?;
        let mut row_dict = HashMap::new();
        for (k, v) in row_obj {
            row_dict.insert(k.clone(), v.clone());
        }
        result.push(alias_row(&row_dict)?);
    }

    Ok(result)
}

pub fn validate_catalog(document: &Value) -> Result<CatalogDocument, CatalogError> {
    let obj = document
        .as_object()
        .ok_or_else(|| CatalogError("Catalog must be a JSON object.".into()))?;

    // Check schema_version: must be integer 1
    match obj.get("schema_version") {
        Some(Value::Number(n)) if n.as_i64() == Some(1) => {}
        _ => return Err(CatalogError("Catalog schema_version must be integer 1.".into())),
    }

    // Check source
    let source_obj = obj
        .get("source")
        .and_then(|v| v.as_object())
        .ok_or_else(|| CatalogError("Catalog source.kind must be mcp, demo or import.".into()))?;

    let kind_str = source_obj
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatalogError("Catalog source.kind must be mcp, demo or import.".into()))?;

    let kind = match kind_str {
        "mcp" => SourceKind::Mcp,
        "demo" => SourceKind::Demo,
        "import" => SourceKind::Import,
        _ => return Err(CatalogError("Catalog source.kind must be mcp, demo or import.".into())),
    };

    let mut tool = None;
    let mut fetched_at = None;
    if kind == SourceKind::Mcp {
        let t = source_obj
            .get("tool")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatalogError("MCP catalog source.tool must be list-nutrition-foods.".into()))?;
        if t != "list-nutrition-foods" {
            return Err(CatalogError("MCP catalog source.tool must be list-nutrition-foods.".into()));
        }
        tool = Some(t.to_string());

        let ts = source_obj
            .get("fetched_at")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatalogError("MCP catalog requires an ISO8601 fetched_at timestamp.".into()))?;
        if !ts.contains('T') {
            return Err(CatalogError("MCP catalog requires an ISO8601 fetched_at timestamp.".into()));
        }
        DateTime::parse_from_rfc3339(ts)
            .map_err(|_| CatalogError("MCP catalog requires an ISO8601 fetched_at timestamp.".into()))?;
        fetched_at = Some(ts.to_string());
    }

    let note = match source_obj.get("note") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(CatalogError("Catalog source.note must be text.".into())),
        None => None,
    };

    // Basis check on document excluding "source"
    let mut doc_no_source = obj.clone();
    doc_no_source.remove("source");
    check_basis(&Value::Object(doc_no_source))?;

    let foods_val = obj
        .get("foods")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CatalogError("Catalog foods must be a nonempty array; no rows were imported.".into()))?;

    if foods_val.is_empty() {
        return Err(CatalogError("Catalog foods must be a nonempty array; no rows were imported.".into()));
    }

    let mut raw_rows = Vec::new();
    for (i, row_val) in foods_val.iter().enumerate() {
        let row_obj = row_val.as_object().ok_or_else(|| {
            CatalogError(format!("Food row {} must be an object.", i + 1))
        })?;
        let name = row_obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatalogError(format!("Food row {} requires a serving-specific name.", i + 1)))?
            .to_string();

        let kcal = row_obj.get("kcal").ok_or_else(|| {
            CatalogError(format!("Food row {} requires kcal and protein_g; missing values are not zero.", i + 1))
        })?.clone();

        let protein_g = row_obj.get("protein_g").ok_or_else(|| {
            CatalogError(format!("Food row {} requires kcal and protein_g; missing values are not zero.", i + 1))
        })?.clone();

        let sodium_mg = row_obj.get("sodium_mg").cloned().unwrap_or(Value::Null);

        raw_rows.push(RawFoodRow {
            name,
            kcal,
            protein_g,
            sodium_mg,
        });
    }

    let foods = parse_food_rows(raw_rows)?;

    Ok(CatalogDocument {
        schema_version: 1,
        source: Source {
            kind,
            tool,
            fetched_at,
            note,
        },
        foods,
    })
}

pub fn load_catalog(path: &Path) -> Result<CatalogDocument, CatalogError> {
    let content = fs::read_to_string(path).map_err(|_| {
        CatalogError("Cannot load catalog: provide a readable UTF-8 JSON file without duplicate properties or nonfinite numbers.".into())
    })?;
    let doc = parse_strict_json(&content).map_err(|_| {
        CatalogError("Cannot load catalog: provide a readable UTF-8 JSON file without duplicate properties or nonfinite numbers.".into())
    })?;
    validate_catalog(&doc)
}

pub fn normalize_tool_result(
    result: &Value,
    diagnostics: &mut Diagnostics,
) -> Result<Vec<FoodItem>, CatalogError> {
    if !result.is_object() {
        return Err(CatalogError("Nutrition tool result must be an object.".into()));
    }
    if result.get("isError") == Some(&Value::Bool(true)) {
        return Err(CatalogError(
            "Nutrition tool returned an error; no food data was imported.".into(),
        ));
    }

    if let Some(structured) = result.get("structuredContent") {
        let raw = json_rows(structured, diagnostics)?;
        return parse_food_rows(raw);
    }

    let blocks = result
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CatalogError("Nutrition tool has no supported content; save --raw-out and inspect the actual schema.".into())
        })?;

    let mut raw_rows = Vec::new();
    for block in blocks {
        if block.get("type") != Some(&Value::String("text".to_string())) {
            continue;
        }
        let text = block
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CatalogError("Nutrition text block does not contain text.".into()))?
            .trim();

        let unquoted = if text.starts_with("```") {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() >= 3 && lines.last().map(|l| l.trim()) == Some("```") {
                lines[1..lines.len() - 1].join("\n").trim().to_string()
            } else {
                text.to_string()
            }
        } else {
            text.to_string()
        };

        let trimmed_unquoted = unquoted.trim();
        if trimmed_unquoted.starts_with('{') || trimmed_unquoted.starts_with('[') {
            let doc = parse_strict_json(trimmed_unquoted).map_err(|_| {
                CatalogError("Nutrition tool returned malformed JSON; save --raw-out and inspect the actual schema.".into())
            })?;
            raw_rows.extend(json_rows(&doc, diagnostics)?);
        } else {
            check_basis_str(trimmed_unquoted)?;
            if let Some(table_rows) = parse_markdown_rows(trimmed_unquoted)? {
                raw_rows.extend(table_rows);
            }
        }
    }

    if raw_rows.is_empty() {
        return Err(CatalogError(
            "Unsupported nutrition format: need a foods JSON array or explicit-unit Markdown table; save --raw-out and adapt the actual schema.".into(),
        ));
    }

    parse_food_rows(raw_rows)
}
