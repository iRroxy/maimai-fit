use crate::catalog::types::{casefold, CatalogError, Diagnostics};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToonCell {
    String(String),
    Bool(bool),
    Null,
}

pub fn decode_toon_cells(line: &str) -> Result<Vec<ToonCell>, CatalogError> {
    let mut cells = Vec::new();
    let bytes = line.as_bytes();
    let mut pos = 0;
    let len = bytes.len();

    while pos < len {
        // Skip leading spaces and tabs
        while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }

        if pos < len && bytes[pos] == b'"' {
            // Quoted string
            let _start = pos;
            pos += 1;
            let mut decoded = String::new();
            let mut closed = false;

            while pos < len {
                let b = bytes[pos];
                if b == b'"' {
                    closed = true;
                    pos += 1;
                    break;
                } else if b == b'\\' {
                    pos += 1;
                    if pos >= len {
                        return Err(CatalogError("Malformed quoted TOON scalar.".into()));
                    }
                    let esc = bytes[pos];
                    match esc {
                        b'"' => decoded.push('"'),
                        b'\\' => decoded.push('\\'),
                        b'n' => decoded.push('\n'),
                        b'r' => decoded.push('\r'),
                        b't' => decoded.push('\t'),
                        b'u' => {
                            if pos + 4 >= len {
                                return Err(CatalogError("Malformed quoted TOON scalar.".into()));
                            }
                            let hex_str = &line[pos + 1..pos + 5];
                            let code = u16::from_str_radix(hex_str, 16)
                                .map_err(|_| CatalogError("Malformed quoted TOON scalar.".into()))?;
                            if (0xD800..=0xDFFF).contains(&code) {
                                return Err(CatalogError(
                                    "TOON strings cannot contain surrogate codepoints.".into(),
                                ));
                            }
                            let ch = char::from_u32(code as u32)
                                .ok_or_else(|| CatalogError("Malformed quoted TOON scalar.".into()))?;
                            decoded.push(ch);
                            pos += 4;
                        }
                        _ => {
                            return Err(CatalogError("Unsupported TOON string escape.".into()));
                        }
                    }
                    pos += 1;
                } else {
                    // Regular char, utf8 decoded
                    let rest = &line[pos..];
                    let ch = rest.chars().next().unwrap();
                    decoded.push(ch);
                    pos += ch.len_utf8();
                }
            }

            if !closed {
                return Err(CatalogError("Malformed quoted TOON scalar.".into()));
            }

            // Check trailing content after quoted scalar before comma
            while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                pos += 1;
            }
            if pos < len && bytes[pos] != b',' {
                return Err(CatalogError(
                    "Unexpected content after a quoted TOON scalar.".into(),
                ));
            }
            cells.push(ToonCell::String(decoded));
        } else {
            // Unquoted token up to comma
            let end = line[pos..].find(',').map(|i| pos + i).unwrap_or(len);
            let token = line[pos..end].trim_matches(|c| c == ' ' || c == '\t');
            let cell = match token {
                "null" => ToonCell::Null,
                "true" => ToonCell::Bool(true),
                "false" => ToonCell::Bool(false),
                _ => ToonCell::String(token.to_string()),
            };
            cells.push(cell);
            pos = end;
        }

        if pos < len {
            // consume comma
            pos += 1;
            if pos == len {
                cells.push(ToonCell::String(String::new()));
            }
        }
    }

    Ok(cells)
}

pub struct RawFoodRow {
    pub name: String,
    pub kcal: Value,
    pub protein_g: Value,
    pub sodium_mg: Value,
}

pub fn parse_mcd_toon_rows(
    document: &Value,
    diagnostics: &mut Diagnostics,
) -> Result<Vec<RawFoodRow>, CatalogError> {
    if document.get("success") != Some(&Value::Bool(true)) {
        return Err(CatalogError(
            "Nutrition API payload was not successful; no rows imported.".into(),
        ));
    }
    match document.get("code") {
        Some(Value::Number(n)) if n.as_i64() == Some(200) => {}
        _ => {
            return Err(CatalogError(
                "Nutrition API payload was not successful; no rows imported.".into(),
            ));
        }
    }

    let data_str = document
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatalogError("Nutrition API data must be an observed TOON table string.".into()))?;

    let lines: Vec<&str> = data_str.trim().lines().collect();
    if lines.is_empty() {
        return Err(CatalogError("Unsupported nutrition TOON header.".into()));
    }

    let header_re = Regex::new(r"^\[(0|[1-9][0-9]{0,8})\]\{([^{}]+)\}:$").unwrap();
    let caps = header_re
        .captures(lines[0])
        .ok_or_else(|| CatalogError("Unsupported nutrition TOON header.".into()))?;

    let count: usize = caps[1]
        .parse()
        .map_err(|_| CatalogError("Unsupported nutrition TOON header.".into()))?;

    let fields: Vec<&str> = caps[2].split(',').collect();
    let known_fields: HashSet<&str> = [
        "productName",
        "nutritionDescription",
        "energyKj",
        "energyKcal",
        "protein",
        "fat",
        "carbohydrate",
        "sodium",
        "calcium",
    ]
    .iter()
    .copied()
    .collect();

    let field_set: HashSet<&str> = fields.iter().copied().collect();
    if fields.len() != known_fields.len() || field_set != known_fields {
        return Err(CatalogError(
            "Nutrition TOON fields differ from the verified official schema.".into(),
        ));
    }

    if lines.len() - 1 != count {
        return Err(CatalogError(
            "Nutrition TOON declared row count does not match its data.".into(),
        ));
    }

    let mut rows = Vec::new();
    let mut identities: HashMap<String, Vec<ToonCell>> = HashMap::new();

    for line in &lines[1..] {
        if !line.starts_with("  ") || line.starts_with("   ") {
            return Err(CatalogError(
                "Nutrition TOON row must have the observed two-space indentation.".into(),
            ));
        }
        let values = decode_toon_cells(&line[2..])?;
        if values.len() != fields.len() {
            return Err(CatalogError(
                "Nutrition TOON row width does not match its header.".into(),
            ));
        }

        let mut row_map: HashMap<&str, ToonCell> = HashMap::new();
        for (f, val) in fields.iter().zip(values.iter()) {
            row_map.insert(*f, val.clone());
        }

        let name = match row_map.get("productName") {
            Some(ToonCell::String(s)) if !s.trim().is_empty() => s.clone(),
            _ => return Err(CatalogError("Nutrition TOON row has no serving name.".into())),
        };

        let identity = casefold(name.trim());
        if let Some(existing_vals) = identities.get(&identity) {
            if existing_vals != &values {
                return Err(CatalogError(
                    "Conflicting nutrition TOON rows share a serving name.".into(),
                ));
            }
            continue;
        }

        identities.insert(identity, values);

        let cell_to_value = |cell: Option<&ToonCell>| -> Value {
            match cell {
                Some(ToonCell::String(s)) => Value::String(s.clone()),
                Some(ToonCell::Bool(b)) => Value::Bool(*b),
                Some(ToonCell::Null) | None => Value::Null,
            }
        };

        rows.push(RawFoodRow {
            name,
            kcal: cell_to_value(row_map.get("energyKcal")),
            protein_g: cell_to_value(row_map.get("protein")),
            sodium_mg: cell_to_value(row_map.get("sodium")),
        });
    }

    diagnostics.format = Some("mcd-toon".to_string());
    diagnostics.input_rows = Some(count);
    diagnostics.duplicate_rows_removed = Some(count - rows.len());

    Ok(rows)
}
