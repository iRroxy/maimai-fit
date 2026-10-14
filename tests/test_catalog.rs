use maimai::catalog::{
    load_catalog, normalize_tool_result, validate_catalog, Diagnostics,
};
use serde_json::json;
use std::fs;

fn make_catalog(foods: Option<serde_json::Value>) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "source": { "kind": "import" },
        "foods": foods.unwrap_or_else(|| json!([
            { "name": " Food (one serving) ", "kcal": "12.50", "protein_g": "1.25" }
        ]))
    })
}

fn make_tool_result(texts: &[&str]) -> serde_json::Value {
    let content: Vec<serde_json::Value> = texts
        .iter()
        .map(|t| json!({ "type": "text", "text": t }))
        .collect();
    json!({ "content": content })
}

#[test]
fn test_decimal_preservation_and_unknown_sodium() {
    let value = "123456789012345678901234567890.123456789";
    let input = make_catalog(Some(json!([
        { "name": " A ", "kcal": value, "protein_g": 0 }
    ])));
    let result = validate_catalog(&input).unwrap();
    assert_eq!(result.foods.len(), 1);
    assert_eq!(result.foods[0].name, "A");
    assert_eq!(result.foods[0].kcal, value);
    assert_eq!(result.foods[0].protein_g, "0");
    assert_eq!(result.foods[0].sodium_mg, None);
}

#[test]
fn test_missing_is_not_zero_and_invalid_numbers_rejected() {
    let invalid_values = vec![
        serde_json::Value::Null,
        json!(true),
        json!(false),
        json!(""),
        json!("NaN"),
        json!("Infinity"),
        json!(-1),
        json!("2 mg"),
        json!("1,000"),
    ];

    for value in invalid_values {
        let input = make_catalog(Some(json!([
            { "name": "A", "kcal": value, "protein_g": 1 }
        ])));
        assert!(
            validate_catalog(&input).is_err(),
            "Expected failure for value: {value:?}"
        );
    }

    // Missing protein_g
    let missing_protein = make_catalog(Some(json!([
        { "name": "A", "kcal": 1 }
    ])));
    assert!(validate_catalog(&missing_protein).is_err());
}

#[test]
fn test_duplicate_serving_names_are_not_merged() {
    let pairs = [
        vec![" A ", "a"],
        vec!["Straße", "STRASSE"],
    ];

    for names in pairs {
        let input = make_catalog(Some(json!([
            { "name": names[0], "kcal": 1, "protein_g": 0 },
            { "name": names[1], "kcal": 1, "protein_g": 0 }
        ])));
        assert!(
            validate_catalog(&input).is_err(),
            "Expected duplicate rejection for names: {names:?}"
        );
    }

    let distinct = make_catalog(Some(json!([
        { "name": "A (small)", "kcal": 1, "protein_g": 0 },
        { "name": "A (large)", "kcal": 1, "protein_g": 0 }
    ])));
    let res = validate_catalog(&distinct).unwrap();
    assert_eq!(res.foods.len(), 2);
}

#[test]
fn test_source_and_version_are_required() {
    let invalid_versions = vec![
        json!(true),
        json!(1.0),
        json!("1"),
        json!(2),
        serde_json::Value::Null,
    ];

    for version in invalid_versions {
        let mut data = make_catalog(None);
        data["schema_version"] = version.clone();
        assert!(
            validate_catalog(&data).is_err(),
            "Expected failure for version: {version:?}"
        );
    }

    let invalid_sources = vec![
        json!({}),
        json!({ "kind": "real" }),
        json!({ "kind": "mcp" }),
        json!({ "kind": "mcp", "tool": "list-nutrition-foods", "fetched_at": "2026-10-14T10:00:00" }),
    ];

    for source in invalid_sources {
        let mut data = make_catalog(None);
        data["source"] = source.clone();
        assert!(
            validate_catalog(&data).is_err(),
            "Expected failure for source: {source:?}"
        );
    }
}

#[test]
fn test_load_preserves_json_decimal_and_rejects_duplicate_properties() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("catalog.json");

    let valid_content = r#"{"schema_version":1,"source":{"kind":"import"},"foods":[{"name":"A","kcal":1.12345678901234567890123456789,"protein_g":0}]}"#;
    fs::write(&file_path, valid_content).unwrap();

    let loaded = load_catalog(&file_path).unwrap();
    assert_eq!(loaded.foods[0].kcal, "1.12345678901234567890123456789");

    let duplicate_content = r#"{"schema_version":1,"schema_version":1}"#;
    fs::write(&file_path, duplicate_content).unwrap();
    assert!(load_catalog(&file_path).is_err());
}

#[test]
fn test_structured_aliases_and_authoritative_structured_content() {
    let result = json!({
        "structuredContent": {
            "foods": [
                { "foodName": "A", "energy_kcal": "100.5", "protein_g": "2", "sodium_mg": null }
            ]
        },
        "content": [{ "type": "text", "text": "unrelated presentation" }]
    });

    let mut diagnostics = Diagnostics::default();
    let foods = normalize_tool_result(&result, &mut diagnostics).unwrap();
    assert_eq!(foods.len(), 1);
    assert_eq!(foods[0].name, "A");
    assert_eq!(foods[0].kcal, "100.5");
    assert_eq!(foods[0].protein_g, "2");
    assert_eq!(foods[0].sodium_mg, None);
}

#[test]
fn test_multiple_json_blocks_are_all_validated() {
    let b1 = json!({ "foods": [{ "name": "A", "kcal": 2, "protein_g": 1 }] }).to_string();
    let b2 = json!({ "foods": [{ "name": "B", "kcal": 3, "protein_g": 2 }] }).to_string();
    let tool_res = make_tool_result(&[&b1, &b2]);

    let mut diagnostics = Diagnostics::default();
    let foods = normalize_tool_result(&tool_res, &mut diagnostics).unwrap();
    let names: Vec<String> = foods.into_iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["A", "B"]);

    let b3_bad = json!({ "foods": [{ "name": "B", "kcal": 3 }] }).to_string();
    let tool_res_bad = make_tool_result(&[&b1, &b3_bad]);
    assert!(normalize_tool_result(&tool_res_bad, &mut diagnostics).is_err());
}

#[test]
fn test_explicit_markdown_units_and_kj_conversion() {
    let table = "| 餐品名称 | 能量（kJ） | 蛋白质(g) | 钠(mg) |\n| --- | ---: | --- | --- |\n| A (一份) | 418.4 kJ | 2.50 g | — |\n| B (一份) | 836.8 | 4 | 10.25 mg |";
    let tool_res = make_tool_result(&[table]);

    let mut diagnostics = Diagnostics::default();
    let foods = normalize_tool_result(&tool_res, &mut diagnostics).unwrap();
    assert_eq!(foods[0].kcal, "100");
    assert_eq!(foods[0].sodium_mg, None);
    assert_eq!(foods[1].sodium_mg, Some("10.25".to_string()));
}

#[test]
fn test_markdown_missing_sodium_is_unknown_not_zero() {
    let table = "| 名称 | 热量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 0 |";
    let tool_res = make_tool_result(&[table]);

    let mut diagnostics = Diagnostics::default();
    let foods = normalize_tool_result(&tool_res, &mut diagnostics).unwrap();
    assert_eq!(foods[0].sodium_mg, None);
}

#[test]
fn test_unknown_and_incomplete_units_are_rejected() {
    let cases = [
        ("能量", "100"),
        ("能量(kcal)", "100 kJ"),
        ("能量(kcal)", ""),
    ];

    for (header, cell) in cases {
        let text = format!("| 名称 | {header} | 蛋白质(g) |\n| --- | --- | --- |\n| A | {cell} | 1 |");
        let tool_res = make_tool_result(&[&text]);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&tool_res, &mut diagnostics).is_err(),
            "Expected failure for {header}: {cell}"
        );
    }

    let extras = [
        json!({ "sodium": 100 }),
        json!({ "钠(g)": 1 }),
        json!({ "protein": 3 }),
        json!({ "calories": 5 }),
    ];

    for extra in extras {
        let mut row = json!({ "name": "A", "kcal": 1, "protein_g": 0 });
        for (k, v) in extra.as_object().unwrap() {
            row.as_object_mut().unwrap().insert(k.clone(), v.clone());
        }
        let result = json!({ "structuredContent": { "foods": [row] } });
        let mut diagnostics = Diagnostics::default();
        assert!(normalize_tool_result(&result, &mut diagnostics).is_err());
    }
}

#[test]
fn test_no_partial_import_for_malformed_table_rows() {
    let invalid_tails = ["| B | 2 |", "| B | 2 | unknown |", "| a | 2 | 1 |"];

    for tail in invalid_tails {
        let text = format!("| 名称 | 能量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 1 |\n{tail}");
        let tool_res = make_tool_result(&[&text]);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&tool_res, &mut diagnostics).is_err(),
            "Expected failure for tail: {tail}"
        );
    }
}

#[test]
fn test_per_100g_metadata_is_rejected() {
    let bases = ["per 100g", "每100克", "每 100 ml", "/100g"];

    for basis in bases {
        let text = format!("{basis}\n| 名称 | 能量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 1 |");
        let tool_res = make_tool_result(&[&text]);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&tool_res, &mut diagnostics).is_err(),
            "Expected failure for basis: {basis}"
        );
    }

    let mut data = make_catalog(None);
    data["basis"] = json!("per 100g");
    assert!(validate_catalog(&data).is_err());
}

#[test]
fn test_converted_per_serving_values_can_record_original_basis() {
    let mut data = make_catalog(Some(json!([
        { "name": "一份", "kcal": "500", "protein_g": "30" }
    ])));
    data["source"]["note"] = json!("源数据每100g，按明确份重200g转换为每份");
    let result = validate_catalog(&data).unwrap();
    assert_eq!(result.foods[0].kcal, "500");
}

#[test]
fn test_errors_and_unknown_formats_do_not_fabricate_foods() {
    let bad_results = vec![
        json!({ "isError": true, "content": [] }),
        json!({ "content": [] }),
        make_tool_result(&["No supported foods"]),
        json!({ "structuredContent": { "unexpected": [] } }),
        make_tool_result(&[r#"{"foods": []}"#]),
        make_tool_result(&["{broken}"]),
    ];

    for res in bad_results {
        let mut diagnostics = Diagnostics::default();
        assert!(normalize_tool_result(&res, &mut diagnostics).is_err());
    }
}
