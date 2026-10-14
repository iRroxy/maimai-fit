use bigdecimal::BigDecimal;
use maimai::catalog::{CatalogDocument, FoodItem, Source, SourceKind};
use maimai::planner::{solve, Constraints};
use maimai::report::render_html;
use std::collections::HashSet;
use std::str::FromStr;

fn food(name: &str, kcal: &str, protein: &str, sodium: Option<&str>) -> FoodItem {
    FoodItem {
        name: name.to_string(),
        kcal: kcal.to_string(),
        protein_g: protein.to_string(),
        sodium_mg: sodium.map(|s| s.to_string()),
    }
}

fn make_catalog(foods: Vec<FoodItem>) -> CatalogDocument {
    CatalogDocument {
        schema_version: 1,
        source: Source {
            kind: SourceKind::Demo,
            tool: None,
            fetched_at: None,
            note: Some("合成测试数据".into()),
        },
        foods,
    }
}

fn default_targets() -> Constraints {
    Constraints {
        keep: vec!["保留品".into()],
        exclude: Vec::new(),
        min_kcal: BigDecimal::from_str("450").unwrap(),
        max_kcal: BigDecimal::from_str("650").unwrap(),
        min_protein: BigDecimal::from_str("30").unwrap(),
        max_sodium: None,
        max_items: 3,
        top: 3,
    }
}

#[test]
fn test_retains_anchor_and_every_returned_plan_obeys_all_caps() {
    let cat = make_catalog(vec![
        food("保留品", "410", "25", Some("650")),
        food("配菜", "90", "5", Some("120")),
        food("高蛋白配菜", "130", "12", Some("240")),
        food("高盐配菜", "110", "18", Some("2000")),
    ]);

    let mut targets = default_targets();
    targets.max_sodium = Some(BigDecimal::from_str("1000").unwrap());
    targets.top = 10;

    let result = solve(&cat, &targets).unwrap();
    assert_eq!(result.status, "ok");

    let expected_names: Vec<HashSet<&str>> = vec![
        ["保留品", "高蛋白配菜"].iter().copied().collect(),
        ["保留品", "配菜"].iter().copied().collect(),
    ];

    let actual_names: Vec<HashSet<&str>> = result
        .plans
        .iter()
        .map(|p| p.foods.iter().map(|f| f.name.as_str()).collect())
        .collect();

    assert_eq!(actual_names, expected_names);

    for plan in &result.plans {
        let kcal = BigDecimal::from_str(&plan.totals.kcal).unwrap();
        let protein = BigDecimal::from_str(&plan.totals.protein_g).unwrap();
        let sodium = BigDecimal::from_str(plan.totals.sodium_mg.as_ref().unwrap()).unwrap();

        assert!(kcal >= BigDecimal::from(450));
        assert!(kcal <= BigDecimal::from(650));
        assert!(protein >= BigDecimal::from(30));
        assert!(sodium <= BigDecimal::from(1000));
    }
}

#[test]
fn test_inclusive_decimal_limits_without_rounding_leak() {
    let exact = solve(
        &make_catalog(vec![food("保留品", "650", "30", Some("100"))]),
        &default_targets(),
    )
    .unwrap();
    assert_eq!(exact.status, "ok");

    let excess = solve(
        &make_catalog(vec![food("保留品", "650.0000001", "30", Some("100"))]),
        &default_targets(),
    )
    .unwrap();
    assert_eq!(excess.status, "infeasible");

    let insufficient = solve(
        &make_catalog(vec![food("保留品", "500", "29.9999999", Some("100"))]),
        &default_targets(),
    )
    .unwrap();
    assert_eq!(insufficient.status, "infeasible");

    let mut targets = default_targets();
    targets.max_kcal = BigDecimal::from_str("500").unwrap();
    let high_precision = solve(
        &make_catalog(vec![food(
            "保留品",
            "500.00000000000000000000000000001",
            "30",
            Some("100"),
        )]),
        &targets,
    )
    .unwrap();
    assert_eq!(high_precision.status, "infeasible");
}

#[test]
fn test_missing_sodium_is_not_zero_and_cannot_satisfy_cap() {
    let cat = make_catalog(vec![food("保留品", "500", "35", None)]);
    let unconstrained = solve(&cat, &default_targets()).unwrap();
    assert_eq!(unconstrained.plans[0].totals.sodium_mg, None);

    let mut constrained_targets = default_targets();
    constrained_targets.max_sodium = Some(BigDecimal::from_str("1000").unwrap());
    let constrained = solve(&cat, &constrained_targets).unwrap();
    assert_eq!(constrained.status, "infeasible");
    assert_eq!(constrained.search.unknown_sodium_rejections, 1);
}

#[test]
fn test_infeasible_never_substitutes_favorite() {
    let cat = make_catalog(vec![
        food("保留品", "900", "40", Some("100")),
        food("很合适的另一个主食", "500", "35", Some("100")),
    ]);
    let result = solve(&cat, &default_targets()).unwrap();
    assert_eq!(result.status, "infeasible");
    assert!(result.plans.is_empty());
}

#[test]
fn test_exclude_matches_casefold_and_conflicting_keep_is_error() {
    let cat = make_catalog(vec![
        food("保留品", "410", "25", Some("100")),
        food("HOT配菜", "100", "10", Some("100")),
        food("普通配菜", "120", "12", Some("100")),
    ]);

    let mut targets = default_targets();
    targets.exclude = vec!["hot".into()];
    let result = solve(&cat, &targets).unwrap();
    let plan_names: Vec<String> = result.plans[0].foods.iter().map(|f| f.name.clone()).collect();
    assert_eq!(plan_names, vec!["保留品", "普通配菜"]);

    let mut conflict_targets = default_targets();
    conflict_targets.exclude = vec!["保留".into()];
    assert!(solve(&cat, &conflict_targets).is_err());
}

#[test]
fn test_multiple_favorites_retained_once_and_item_limit_respected() {
    let cat = make_catalog(vec![
        food("保留品", "300", "20", Some("100")),
        food("第二保留品", "200", "12", Some("100")),
        food("配菜", "50", "3", Some("100")),
    ]);

    let mut targets = default_targets();
    targets.keep = vec!["保留品".into(), "第二保留品".into()];
    targets.max_items = 2;

    let result = solve(&cat, &targets).unwrap();
    let plan_names: Vec<String> = result.plans[0].foods.iter().map(|f| f.name.clone()).collect();
    assert_eq!(plan_names, vec!["保留品", "第二保留品"]);
    assert_eq!(result.search.examined, 1);

    // Duplicate keep
    let mut dup_targets = default_targets();
    dup_targets.keep = vec!["保留品".into(), "保留品".into()];
    assert!(solve(&cat, &dup_targets).is_err());

    // Non-existent keep
    let mut missing_targets = default_targets();
    missing_targets.keep = vec!["不存在".into()];
    assert!(solve(&cat, &missing_targets).is_err());
}

#[test]
fn test_rank_prefers_fewer_changes_after_equal_calorie_distance() {
    let cat = make_catalog(vec![
        food("保留品", "400", "20", Some("100")),
        food("一个配菜", "150", "10", Some("100")),
        food("半份配菜A", "70", "10", Some("100")),
        food("半份配菜B", "80", "12", Some("100")),
    ]);

    let result = solve(&cat, &default_targets()).unwrap();
    let plan_names: Vec<String> = result.plans[0].foods.iter().map(|f| f.name.clone()).collect();
    assert_eq!(plan_names, vec!["保留品", "一个配菜"]);
}

#[test]
fn test_invalid_targets_rejected() {
    let cat = make_catalog(vec![food("保留品", "500", "35", Some("100"))]);

    let mut t1 = default_targets();
    t1.min_kcal = BigDecimal::from(-1);
    assert!(solve(&cat, &t1).is_err());

    let mut t2 = default_targets();
    t2.min_kcal = BigDecimal::from(700);
    t2.max_kcal = BigDecimal::from(650);
    assert!(solve(&cat, &t2).is_err());

    let mut t3 = default_targets();
    t3.max_items = 4;
    assert!(solve(&cat, &t3).is_err());

    let mut t4 = default_targets();
    t4.top = 0;
    assert!(solve(&cat, &t4).is_err());
}

#[test]
fn test_report_escapes_untrusted_food_and_provenance() {
    let name = r#"<img src=x onerror="alert(1)">"#;
    let mut cat = make_catalog(vec![food(name, "500", "35", Some("100"))]);
    cat.source.note = Some(r#"</p><script>alert(2)</script>"#.into());

    let mut targets = default_targets();
    targets.keep = vec![name.into()];

    let result = solve(&cat, &targets).unwrap();
    let html = render_html(&result);

    assert!(!html.contains("<img src=x"));
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;img"));
    assert!(html.contains("&lt;script&gt;"));
}
