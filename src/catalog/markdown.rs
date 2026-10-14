use crate::catalog::toon::RawFoodRow;
use crate::catalog::types::{normalize_key, CatalogError};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_matches('|');
    trimmed.split('|').map(|c| c.trim().to_string()).collect()
}

pub fn parse_markdown_rows(text: &str) -> Result<Option<Vec<RawFoodRow>>, CatalogError> {
    let lines: Vec<&str> = text.lines().collect();
    let divider_re = Regex::new(r"^:?-{3,}:?$").unwrap();
    let mut rows = Vec::new();
    let mut found = false;
    let mut index = 0;

    let names_set: HashSet<&str> = ["name", "foodname", "名称", "食品名称", "餐品名称"]
        .iter()
        .copied()
        .collect();
    let energy_map: HashMap<&str, &str> = [
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
    .collect();
    let protein_set: HashSet<&str> = ["protein_g", "蛋白质(g)", "protein(g)"]
        .iter()
        .copied()
        .collect();

    while index + 1 < lines.len() {
        if !lines[index].contains('|') || !lines[index + 1].contains('|') {
            index += 1;
            continue;
        }

        let headers = cells(lines[index]);
        let divider = cells(lines[index + 1]);

        if divider.is_empty() || !divider.iter().all(|c| divider_re.is_match(c)) {
            index += 1;
            continue;
        }

        found = true;
        let keys: Vec<String> = headers.iter().map(|h| normalize_key(h)).collect();
        let key_set: HashSet<String> = keys.iter().cloned().collect();

        if divider.len() != headers.len() || key_set.len() != keys.len() {
            return Err(CatalogError(
                "Malformed or duplicate Markdown nutrition table headers.".into(),
            ));
        }

        let has_name = keys.iter().any(|k| names_set.contains(k.as_str()));
        let has_energy = keys.iter().any(|k| energy_map.contains_key(k.as_str()));
        let has_protein = keys.iter().any(|k| protein_set.contains(k.as_str()));

        if !has_name || !has_energy || !has_protein {
            return Err(CatalogError(
                "Nutrition table needs a name, explicit energy(kcal/kJ) and protein(g) headers; unlabeled units cannot be guessed.".into(),
            ));
        }

        index += 2;
        let mut count = 0;

        while index < lines.len() && lines[index].contains('|') && !lines[index].trim().is_empty() {
            let row_cells = cells(lines[index]);
            if row_cells.len() != headers.len() {
                return Err(CatalogError(
                    "Nutrition table row has a different number of cells than its header.".into(),
                ));
            }

            let mut row_dict: HashMap<String, Value> = HashMap::new();
            for (h, c) in headers.iter().zip(row_cells.into_iter()) {
                row_dict.insert(h.clone(), Value::String(c));
            }

            let mapped = crate::catalog::alias_row(&row_dict)?;
            rows.push(mapped);
            count += 1;
            index += 1;
        }

        if count == 0 {
            return Err(CatalogError(
                "Nutrition Markdown table contains no food rows.".into(),
            ));
        }
    }

    if found {
        Ok(Some(rows))
    } else {
        Ok(None)
    }
}
