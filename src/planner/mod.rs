use crate::catalog::types::{casefold, CatalogDocument, FoodItem, Source};
use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningError(pub String);

impl fmt::Display for PlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PlanningError {}

impl From<&str> for PlanningError {
    fn from(s: &str) -> Self {
        PlanningError(s.to_string())
    }
}

impl From<String> for PlanningError {
    fn from(s: String) -> Self {
        PlanningError(s)
    }
}

pub fn parse_number(value: &str, label: &str) -> Result<BigDecimal, PlanningError> {
    let trimmed = value.trim();
    if trimmed.len() > 128 {
        return Err(PlanningError(format!("{label}必须是非负有限数值（最多 128 字符）")));
    }
    let d = BigDecimal::from_str(trimmed)
        .map_err(|_| PlanningError(format!("{label}必须是非负有限数值")))?;
    if d < BigDecimal::from(0) {
        return Err(PlanningError(format!(
            "{label}必须是非负有限数值，十进制指数不超过 ±1000"
        )));
    }
    Ok(d)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraints {
    pub keep: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub min_kcal: BigDecimal,
    pub max_kcal: BigDecimal,
    pub min_protein: BigDecimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sodium: Option<BigDecimal>,
    pub max_items: usize,
    pub top: usize,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            keep: Vec::new(),
            exclude: Vec::new(),
            min_kcal: BigDecimal::from_str("450").unwrap(),
            max_kcal: BigDecimal::from_str("650").unwrap(),
            min_protein: BigDecimal::from_str("30").unwrap(),
            max_sodium: None,
            max_items: 3,
            top: 3,
        }
    }
}

impl Constraints {
    pub fn validate(&self) -> Result<(), PlanningError> {
        let zero = BigDecimal::from(0);
        if self.min_kcal < zero {
            return Err(PlanningError("最低热量必须是非负有限数值，十进制指数不超过 ±1000".into()));
        }
        if self.max_kcal < zero {
            return Err(PlanningError("最高热量必须是非负有限数值，十进制指数不超过 ±1000".into()));
        }
        if self.min_protein < zero {
            return Err(PlanningError("最低蛋白质必须是非负有限数值，十进制指数不超过 ±1000".into()));
        }
        if let Some(ref s) = self.max_sodium {
            if s < &zero {
                return Err(PlanningError("最高钠必须是非负有限数值，十进制指数不超过 ±1000".into()));
            }
        }

        if self.min_kcal > self.max_kcal {
            return Err(PlanningError("最低热量不能大于最高热量".into()));
        }
        if self.keep.is_empty() || self.keep.iter().any(|n| n.trim().is_empty()) {
            return Err(PlanningError(
                "至少指定一份要保留的餐品（--keep 精确名称）".into(),
            ));
        }

        let mut seen = HashSet::new();
        for name in &self.keep {
            let id = casefold(name.trim());
            if !seen.insert(id) {
                return Err(PlanningError(
                    "要保留的餐品不能重复；每种餐品只计算一份".into(),
                ));
            }
        }

        if !(1..=3).contains(&self.max_items) {
            return Err(PlanningError("max-items 必须在 1–3 之间".into()));
        }
        if self.keep.len() > self.max_items {
            return Err(PlanningError("保留餐品数量超过 max-items".into()));
        }
        if !(1..=10).contains(&self.top) {
            return Err(PlanningError("top 必须在 1–10 之间".into()));
        }
        if self.exclude.iter().any(|term| term.trim().is_empty()) {
            return Err(PlanningError("排除关键词不能为空".into()));
        }

        Ok(())
    }

    pub fn as_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "keep": self.keep,
            "exclude": self.exclude,
            "min_kcal": self.min_kcal.to_string(),
            "max_kcal": self.max_kcal.to_string(),
            "min_protein_g": self.min_protein.to_string(),
            "max_sodium_mg": self.max_sodium.as_ref().map(|s| s.to_string()),
            "max_items": self.max_items,
            "top": self.top,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTotals {
    pub kcal: String,
    pub protein_g: String,
    pub sodium_mg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub foods: Vec<FoodItem>,
    pub totals: PlanTotals,
    pub added_items: usize,
    pub target_distance_kcal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStats {
    pub examined: usize,
    pub feasible: usize,
    pub unknown_sodium_rejections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningResult {
    pub status: String,
    pub source: Source,
    pub constraints: serde_json::Value,
    pub plans: Vec<PlanItem>,
    pub search: SearchStats,
    pub ranking: String,
    pub notice: String,
}

fn combinations<'a, T>(slice: &'a [T], k: usize) -> Vec<Vec<&'a T>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    fn backtrack<'a, T>(
        slice: &'a [T],
        start: usize,
        k: usize,
        current: &mut Vec<&'a T>,
        result: &mut Vec<Vec<&'a T>>,
    ) {
        if current.len() == k {
            result.push(current.clone());
            return;
        }
        for i in start..slice.len() {
            current.push(&slice[i]);
            backtrack(slice, i + 1, k, current, result);
            current.pop();
        }
    }
    backtrack(slice, 0, k, &mut current, &mut result);
    result
}

struct RankKey {
    distance: BigDecimal,
    added_count: usize,
    protein: BigDecimal,
    names: Vec<String>,
}

impl PartialEq for RankKey {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
            && self.added_count == other.added_count
            && self.protein == other.protein
            && self.names == other.names
    }
}

impl Eq for RankKey {}

impl PartialOrd for RankKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.distance.cmp(&other.distance) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.added_count.cmp(&other.added_count) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // Higher protein comes first (reverse comparison)
        match other.protein.cmp(&self.protein) {
            Ordering::Equal => {}
            ord => return ord,
        }
        self.names.cmp(&other.names)
    }
}

pub fn solve(
    catalog: &CatalogDocument,
    constraints: &Constraints,
) -> Result<PlanningResult, PlanningError> {
    constraints.validate()?;

    let mut by_name: HashMap<String, &FoodItem> = HashMap::new();
    for food in &catalog.foods {
        by_name.insert(casefold(&food.name), food);
    }

    let excluded: Vec<String> = constraints
        .exclude
        .iter()
        .map(|t| casefold(t.trim()))
        .collect();

    let mut mandatory = Vec::new();
    for name in &constraints.keep {
        let identity = casefold(name.trim());
        let food = by_name.get(&identity).ok_or_else(|| {
            PlanningError(format!(
                "找不到要保留的餐品：{name}；请使用目录中的精确份量名称"
            ))
        })?;
        let food_name_cf = casefold(&food.name);
        if excluded.iter().any(|term| food_name_cf.contains(term)) {
            return Err(PlanningError(format!("保留与排除条件冲突：{}", food.name)));
        }
        mandatory.push(*food);
    }

    let mandatory_names: HashSet<String> = mandatory.iter().map(|f| casefold(&f.name)).collect();

    let available: Vec<&FoodItem> = catalog
        .foods
        .iter()
        .filter(|f| {
            let cf = casefold(&f.name);
            !mandatory_names.contains(&cf) && !excluded.iter().any(|term| cf.contains(term))
        })
        .collect();

    // Parse food nutrients into BigDecimals
    struct DecNutrients {
        kcal: BigDecimal,
        protein: BigDecimal,
        sodium: Option<BigDecimal>,
    }

    let mut numeric: HashMap<String, DecNutrients> = HashMap::new();
    for food in &catalog.foods {
        let kcal = BigDecimal::from_str(&food.kcal).unwrap();
        let protein = BigDecimal::from_str(&food.protein_g).unwrap();
        let sodium = food
            .sodium_mg
            .as_ref()
            .map(|s| BigDecimal::from_str(s).unwrap());
        numeric.insert(
            food.name.clone(),
            DecNutrients {
                kcal,
                protein,
                sodium,
            },
        );
    }

    let two = BigDecimal::from(2);
    let midpoint = (&constraints.min_kcal + &constraints.max_kcal) / two;

    let mut best: Vec<(RankKey, PlanItem)> = Vec::new();
    let mut feasible_count = 0;
    let mut examined = 0;
    let mut unknown_sodium_rejections = 0;

    let max_added = constraints.max_items - mandatory.len();

    for added_count in 0..=max_added {
        for extras in combinations(&available, added_count) {
            examined += 1;
            let mut selected = mandatory.clone();
            for &extra in &extras {
                selected.push(extra);
            }

            let mut kcal = BigDecimal::from(0);
            let mut protein = BigDecimal::from(0);
            let mut has_unknown_sodium = false;
            let mut sodium_sum = BigDecimal::from(0);

            for food in &selected {
                let n = &numeric[&food.name];
                kcal += &n.kcal;
                protein += &n.protein;
                if let Some(ref s) = n.sodium {
                    sodium_sum += s;
                } else {
                    has_unknown_sodium = true;
                }
            }

            if kcal < constraints.min_kcal || kcal > constraints.max_kcal || protein < constraints.min_protein {
                continue;
            }

            let sodium = if has_unknown_sodium {
                None
            } else {
                Some(sodium_sum)
            };

            if let Some(ref max_s) = constraints.max_sodium {
                if sodium.is_none() {
                    unknown_sodium_rejections += 1;
                    continue;
                }
                if sodium.as_ref().unwrap() > max_s {
                    continue;
                }
            }

            feasible_count += 1;

            let distance = if kcal >= midpoint {
                &kcal - &midpoint
            } else {
                &midpoint - &kcal
            };

            let mut names: Vec<String> = selected.iter().map(|f| f.name.clone()).collect();
            names.sort();

            let rank = RankKey {
                distance: distance.clone(),
                added_count,
                protein: protein.clone(),
                names,
            };

            let candidate = PlanItem {
                foods: selected.into_iter().cloned().collect(),
                totals: PlanTotals {
                    kcal: kcal.to_string(),
                    protein_g: protein.to_string(),
                    sodium_mg: sodium.map(|s| s.to_string()),
                },
                added_items: added_count,
                target_distance_kcal: distance.to_string(),
            };

            best.push((rank, candidate));
            best.sort_by(|a, b| a.0.cmp(&b.0));
            if best.len() > constraints.top {
                best.pop();
            }
        }
    }

    let status = if best.is_empty() {
        "infeasible"
    } else {
        "ok"
    };

    Ok(PlanningResult {
        status: status.to_string(),
        source: catalog.source.clone(),
        constraints: constraints.as_dict(),
        plans: best.into_iter().map(|(_, item)| item).collect(),
        search: SearchStats {
            examined,
            feasible: feasible_count,
            unknown_sodium_rejections,
        },
        ranking: "热量最接近区间中点 → 新增餐品更少 → 蛋白质更高 → 餐品名确定性排序".into(),
        notice: "营养信息不是门店实时菜单；价格、供应、过敏原未核验。仅供参考，不构成医疗或专业营养建议。".into(),
    })
}
