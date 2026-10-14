use maimai::catalog::{normalize_tool_result, Diagnostics};
use serde_json::json;

const FIELDS: &str = "productName,nutritionDescription,energyKj,energyKcal,protein,fat,carbohydrate,sodium,calcium";

fn make_payload(rows: &[&str], declared: Option<usize>, extra: Option<serde_json::Map<String, serde_json::Value>>) -> serde_json::Value {
    let count = declared.unwrap_or(rows.len());
    let mut data = format!("[{count}]{{{FIELDS}}}:\n");
    for row in rows {
        data.push_str("  ");
        data.push_str(row);
        data.push('\n');
    }

    let mut doc = json!({
        "success": true,
        "code": 200,
        "data": data.trim_end(),
    });

    if let Some(map) = extra {
        for (k, v) in map {
            doc.as_object_mut().unwrap().insert(k, v);
        }
    }

    json!({ "structuredContent": doc })
}

#[test]
fn test_explicit_official_schema_and_quoted_serving_name() {
    let name = r#"示例,含"引号"与\符号（一份）"#;
    let name_quoted = serde_json::to_string(name).unwrap();
    let row = format!("{name_quoted},null,2092,500,30,10,40,null,20");

    let mut diagnostics = Diagnostics::default();
    let payload = make_payload(&[&row], None, None);
    let rows = normalize_tool_result(&payload, &mut diagnostics).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, name);
    assert_eq!(rows[0].kcal, "500");
    assert_eq!(rows[0].protein_g, "30");
    assert_eq!(rows[0].sodium_mg, None);
}

#[test]
fn test_identical_source_entries_merge_with_quality_diagnostics() {
    let row = "示例（一份）,null,2092,500,30,10,40,100,20";
    let payload = make_payload(&[row, row], None, None);

    let mut diagnostics = Diagnostics::default();
    let rows = normalize_tool_result(&payload, &mut diagnostics).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "示例（一份）");
    assert_eq!(rows[0].kcal, "500");
    assert_eq!(rows[0].protein_g, "30");
    assert_eq!(rows[0].sodium_mg, Some("100".to_string()));

    assert_eq!(diagnostics.input_rows, Some(2));
    assert_eq!(diagnostics.duplicate_rows_removed, Some(1));
}

#[test]
fn test_conflicting_same_serving_is_not_silently_selected() {
    let row = "示例（一份）,null,2092,500,30,10,40,100,20";
    let others = [
        row.replace(",500,", ",600,"),
        row.replace(",10,40,", ",11,40,"),
    ];

    for other in others {
        let payload = make_payload(&[row, &other], None, None);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&payload, &mut diagnostics).is_err(),
            "Expected conflicting rows to fail"
        );
    }
}

#[test]
fn test_truncated_or_extra_rows_fail_declared_count() {
    let row = "示例（一份）,null,2092,500,30,10,40,100,20";

    for count in [0, 2] {
        let payload = make_payload(&[row], Some(count), None);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&payload, &mut diagnostics).is_err(),
            "Expected count mismatch failure for declared={count}"
        );
    }
}

#[test]
fn test_wrong_fields_width_and_numeric_values_fail_closed() {
    let valid = "示例（一份）,null,2092,500,30,10,40,100,20";
    let mut truncated = valid.to_string();
    if let Some(idx) = truncated.rfind(',') {
        truncated.truncate(idx);
    }

    let bad_rows = [
        truncated,
        valid.replace(",30,", ",true,"),
        valid.replace(",500,", ",null,"),
    ];

    for bad in bad_rows {
        let payload = make_payload(&[&bad], None, None);
        let mut diagnostics = Diagnostics::default();
        assert!(
            normalize_tool_result(&payload, &mut diagnostics).is_err(),
            "Expected failure for row: {bad}"
        );
    }

    let payload_changed = json!({
        "structuredContent": {
            "success": true,
            "code": 200,
            "data": format!("[1]{{{FIELDS}}}:\n  {valid}").replace("energyKcal", "energy"),
        }
    });
    let mut diagnostics = Diagnostics::default();
    assert!(normalize_tool_result(&payload_changed, &mut diagnostics).is_err());
}

#[test]
fn test_business_error_is_not_treated_as_nutrition_success() {
    let row = "示例（一份）,null,2092,500,30,10,40,100,20";

    let mut map_err = serde_json::Map::new();
    map_err.insert("success".into(), json!(false));
    let p1 = make_payload(&[row], None, Some(map_err));
    let mut diag = Diagnostics::default();
    assert!(normalize_tool_result(&p1, &mut diag).is_err());

    let mut map_code = serde_json::Map::new();
    map_code.insert("code".into(), json!(500));
    let p2 = make_payload(&[row], None, Some(map_code));
    assert!(normalize_tool_result(&p2, &mut diag).is_err());
}

#[test]
fn test_bad_quotes_and_row_indentation_are_rejected() {
    let bad_quotes = [
        r#""示例"extra,null,2092,500,30,10,40,100,20"#,
        r#""unterminated,null,2092,500,30,10,40,100,20"#,
    ];

    for bad in bad_quotes {
        let payload = make_payload(&[bad], None, None);
        let mut diag = Diagnostics::default();
        assert!(
            normalize_tool_result(&payload, &mut diag).is_err(),
            "Expected failure for bad quote: {bad}"
        );
    }

    let valid = "示例（一份）,null,2092,500,30,10,40,100,20";
    let bad_indent = json!({
        "structuredContent": {
            "success": true,
            "code": 200,
            "data": format!("[1]{{{FIELDS}}}:\n    {valid}"),
        }
    });
    let mut diag = Diagnostics::default();
    assert!(normalize_tool_result(&bad_indent, &mut diag).is_err());
}
