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
fn test_top_truncation_preserves_protein_then_name_ranking_and_full_statistics() {
    let cat = make_catalog(vec![
        food("保留品", "400", "20", Some("100")),
        food("Z配菜", "150", "10", Some("20")),
        food("A配菜", "150", "10", Some("20")),
        food("高蛋白配菜", "150", "15", Some("20")),
    ]);
    let mut targets = default_targets();
    targets.max_items = 2;
    targets.top = 2;

    let result = solve(&cat, &targets).unwrap();
    let names: Vec<Vec<&str>> = result
        .plans
        .iter()
        .map(|plan| plan.foods.iter().map(|item| item.name.as_str()).collect())
        .collect();
    assert_eq!(names, vec![vec!["保留品", "高蛋白配菜"], vec!["保留品", "A配菜"]]);
    assert_eq!(result.search.examined, 4);
    assert_eq!(result.search.feasible, 3);
    assert_eq!(result.search.unknown_sodium_rejections, 0);

    targets.top = 10;
    let all = solve(&cat, &targets).unwrap();
    assert_eq!(all.plans.len(), 3);
    assert_eq!(
        serde_json::to_value(&result.plans).unwrap(),
        serde_json::to_value(&all.plans[..2]).unwrap()
    );
    assert_eq!(all.search.examined, result.search.examined);
    assert_eq!(all.search.feasible, result.search.feasible);
}

#[test]
fn test_pair_name_ties_use_sorted_names_without_reordering_output_foods() {
    let cat = make_catalog(vec![
        food("M", "400", "20", Some("100")),
        food("Z", "100", "10", Some("10")),
        food("A", "100", "10", Some("10")),
        food("Y", "100", "10", Some("10")),
        food("B", "100", "10", Some("10")),
    ]);
    let mut targets = default_targets();
    targets.keep = vec!["M".into()];
    targets.min_kcal = BigDecimal::from(550);

    let result = solve(&cat, &targets).unwrap();
    let names: Vec<Vec<&str>> = result
        .plans
        .iter()
        .map(|plan| plan.foods.iter().map(|item| item.name.as_str()).collect())
        .collect();
    assert_eq!(
        names,
        vec![vec!["M", "A", "B"], vec!["M", "A", "Y"], vec!["M", "Z", "A"]]
    );
    assert_eq!(result.search.examined, 11);
    assert_eq!(result.search.feasible, 6);
    assert_eq!(result.search.unknown_sodium_rejections, 0);
    for plan in result.plans {
        assert_eq!(plan.added_items, 2);
        assert_eq!(plan.target_distance_kcal, "0");
        assert_eq!(plan.totals.kcal, "600");
        assert_eq!(plan.totals.protein_g, "40");
        assert_eq!(plan.totals.sodium_mg.as_deref(), Some("120"));
    }
}

#[test]
fn test_multiple_keeps_preserve_request_order_and_exact_decimal_totals() {
    let cat = make_catalog(vec![
        food("A", "200.1", "10.05", Some("100.1")),
        food("Z", "300.2", "20.10", Some("200.2")),
        food("C", "40.3", "1.25", Some("30.3")),
        food("B", "40.3", "1.25", Some("20.3")),
    ]);
    let mut targets = default_targets();
    targets.keep = vec!["Z".into(), "A".into()];
    targets.min_kcal = BigDecimal::from_str("500.3").unwrap();
    targets.max_kcal = BigDecimal::from_str("580.9").unwrap();

    let result = solve(&cat, &targets).unwrap();
    let names: Vec<Vec<&str>> = result
        .plans
        .iter()
        .map(|plan| plan.foods.iter().map(|item| item.name.as_str()).collect())
        .collect();
    assert_eq!(names, vec![vec!["Z", "A", "B"], vec!["Z", "A", "C"], vec!["Z", "A"]]);
    assert_eq!(result.search.examined, 3);
    assert_eq!(result.search.feasible, 3);
    assert_eq!(result.plans[0].totals.kcal, "540.6");
    assert_eq!(result.plans[0].totals.protein_g, "31.40");
    assert_eq!(result.plans[0].totals.sodium_mg.as_deref(), Some("320.6"));
    assert_eq!(result.plans[2].totals.kcal, "500.3");
    assert_eq!(result.plans[2].totals.protein_g, "30.15");
    assert_eq!(result.plans[2].totals.sodium_mg.as_deref(), Some("300.3"));
    assert_eq!(result.plans[2].added_items, 0);
}

#[test]
fn test_unknown_sodium_statistics_count_only_nutritionally_eligible_candidates() {
    let cat = make_catalog(vec![
        food("保留品", "400", "25", Some("100")),
        food("已知钠", "100", "5", Some("50")),
        food("未知钠", "100", "5", None),
        food("蛋白不足的未知钠", "100", "0", None),
        food("钠超标", "100", "5", Some("1000")),
        food("排除的未知钠", "100", "5", None),
    ]);
    let mut targets = default_targets();
    targets.exclude = vec!["排除".into()];
    targets.max_items = 2;
    targets.max_sodium = Some(BigDecimal::from(200));

    let result = solve(&cat, &targets).unwrap();
    assert_eq!(result.plans.len(), 1);
    assert_eq!(result.plans[0].foods[1].name, "已知钠");
    assert_eq!(result.search.examined, 5);
    assert_eq!(result.search.feasible, 1);
    assert_eq!(result.search.unknown_sodium_rejections, 1);

    targets.max_sodium = None;
    let unconstrained = solve(&cat, &targets).unwrap();
    assert_eq!(unconstrained.search.examined, 5);
    assert_eq!(unconstrained.search.feasible, 3);
    assert_eq!(unconstrained.search.unknown_sodium_rejections, 0);
    let unknown = unconstrained
        .plans
        .iter()
        .find(|plan| plan.foods[1].name == "未知钠")
        .unwrap();
    assert_eq!(unknown.totals.sodium_mg, None);
}

#[test]
fn test_unknown_mandatory_sodium_propagates_through_all_combination_sizes() {
    let cat = make_catalog(vec![
        food("保留品", "500", "35", None),
        food("A", "50", "1", Some("100")),
        food("B", "60", "1", Some("100")),
    ]);
    let mut targets = default_targets();
    targets.top = 10;

    let result = solve(&cat, &targets).unwrap();
    assert_eq!(result.plans.len(), 4);
    assert_eq!(result.search.examined, 4);
    assert_eq!(result.search.feasible, 4);
    assert!(result.plans.iter().all(|plan| plan.totals.sodium_mg.is_none()));

    targets.max_sodium = Some(BigDecimal::from(1000));
    let constrained = solve(&cat, &targets).unwrap();
    assert_eq!(constrained.status, "infeasible");
    assert!(constrained.plans.is_empty());
    assert_eq!(constrained.search.examined, 4);
    assert_eq!(constrained.search.feasible, 0);
    assert_eq!(constrained.search.unknown_sodium_rejections, 4);
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
