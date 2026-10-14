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

#[derive(Clone)]
struct DecNutrients {
    kcal: BigDecimal,
    protein: BigDecimal,
    sodium: Option<BigDecimal>,
}

impl DecNutrients {
    fn adding(&self, other: &Self) -> Self {
        Self {
            kcal: &self.kcal + &other.kcal,
            protein: &self.protein + &other.protein,
            sodium: match (&self.sodium, &other.sodium) {
                (Some(left), Some(right)) => Some(left + right),
                _ => None,
            },
        }
    }
}

struct Candidate<'a> {
    totals: DecNutrients,
    distance: BigDecimal,
    added_count: usize,
    names: [&'a str; 3],
    extras: [usize; 2],
}

impl Candidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .cmp(&other.distance)
            .then_with(|| self.added_count.cmp(&other.added_count))
            .then_with(|| other.totals.protein.cmp(&self.totals.protein))
            .then_with(|| self.names.cmp(&other.names))
    }
}

pub fn solve(
    catalog: &CatalogDocument,
    constraints: &Constraints,
) -> Result<PlanningResult, PlanningError> {
    constraints.validate()?;

    let identities: Vec<String> = catalog.foods.iter().map(|f| casefold(&f.name)).collect();
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for (index, identity) in identities.iter().enumerate() {
        by_name.insert(identity.as_str(), index);
    }

    let excluded: Vec<String> = constraints
        .exclude
        .iter()
        .map(|t| casefold(t.trim()))
        .collect();

    let mut mandatory = [0; 3];
    for (position, name) in constraints.keep.iter().enumerate() {
        let identity = casefold(name.trim());
        let index = *by_name.get(identity.as_str()).ok_or_else(|| {
            PlanningError(format!(
                "找不到要保留的餐品：{name}；请使用目录中的精确份量名称"
            ))
        })?;
        if excluded.iter().any(|term| identities[index].contains(term)) {
            return Err(PlanningError(format!(
                "保留与排除条件冲突：{}",
                catalog.foods[index].name
            )));
        }
        mandatory[position] = index;
    }
    let mandatory = &mandatory[..constraints.keep.len()];

    let available: Vec<usize> = identities
        .iter()
        .enumerate()
        .filter(|(_, identity)| {
            !mandatory.iter().any(|&index| &identities[index] == *identity)
                && !excluded.iter().any(|term| identity.contains(term))
        })
        .map(|(index, _)| index)
        .collect();

    // Nutrients and borrowed names share the catalog's indices.
    let numeric: Vec<DecNutrients> = catalog
        .foods
        .iter()
        .map(|food| DecNutrients {
            kcal: BigDecimal::from_str(&food.kcal).unwrap(),
            protein: BigDecimal::from_str(&food.protein_g).unwrap(),
            sodium: food
                .sodium_mg
                .as_ref()
                .map(|s| BigDecimal::from_str(s).unwrap()),
        })
        .collect();
    let names: Vec<&str> = catalog.foods.iter().map(|food| food.name.as_str()).collect();
    let mut mandatory_totals = DecNutrients {
        kcal: BigDecimal::from(0),
        protein: BigDecimal::from(0),
        sodium: Some(BigDecimal::from(0)),
    };
    for &index in mandatory {
        mandatory_totals = mandatory_totals.adding(&numeric[index]);
    }

    let two = BigDecimal::from(2);
    let midpoint = (&constraints.min_kcal + &constraints.max_kcal) / two;

    let mut best: Vec<Candidate<'_>> = Vec::with_capacity(constraints.top);
    let mut feasible_count = 0;
    let mut examined = 0;
    let mut unknown_sodium_rejections = 0;

    let max_added = constraints.max_items - mandatory.len();
    let mut consider = |totals: DecNutrients, extras: [usize; 2], added_count: usize| {
        examined += 1;
        if totals.kcal < constraints.min_kcal
            || totals.kcal > constraints.max_kcal
            || totals.protein < constraints.min_protein
        {
            return;
        }
        if let Some(max_s) = &constraints.max_sodium {
            match &totals.sodium {
                None => {
                    unknown_sodium_rejections += 1;
                    return;
                }
                Some(sodium) if sodium > max_s => return,
                _ => {}
            }
        }
        feasible_count += 1;

        let distance = if totals.kcal >= midpoint {
            &totals.kcal - &midpoint
        } else {
            &midpoint - &totals.kcal
        };
        let mut selected_names = [""; 3];
        for (position, &index) in mandatory.iter().chain(extras[..added_count].iter()).enumerate() {
            selected_names[position] = names[index];
        }
        selected_names[..mandatory.len() + added_count].sort_unstable();
        let candidate = Candidate {
            totals,
            distance,
            added_count,
            names: selected_names,
            extras,
        };

        // Insert after equal ranks to retain the enumeration's stable tie order.
        let position = best.partition_point(|item| item.cmp(&candidate) != Ordering::Greater);
        if position < constraints.top {
            if best.len() == constraints.top {
                best.pop();
            }
            best.insert(position, candidate);
        }
    };

    consider(mandatory_totals.clone(), [0; 2], 0);
    if max_added >= 1 {
        for &index in &available {
            consider(mandatory_totals.adding(&numeric[index]), [index, 0], 1);
        }
    }
    if max_added == 2 {
        for (position, &first) in available[..available.len().saturating_sub(1)].iter().enumerate() {
            let first_totals = mandatory_totals.adding(&numeric[first]);
            for &second in &available[position + 1..] {
                consider(first_totals.adding(&numeric[second]), [first, second], 2);
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
        plans: best
            .into_iter()
            .map(|candidate| PlanItem {
                foods: mandatory
                    .iter()
                    .chain(candidate.extras[..candidate.added_count].iter())
                    .map(|&index| catalog.foods[index].clone())
                    .collect(),
                totals: PlanTotals {
                    kcal: candidate.totals.kcal.to_string(),
                    protein_g: candidate.totals.protein.to_string(),
                    sodium_mg: candidate.totals.sodium.map(|s| s.to_string()),
                },
                added_items: candidate.added_count,
                target_distance_kcal: candidate.distance.to_string(),
            })
            .collect(),
        search: SearchStats {
            examined,
            feasible: feasible_count,
            unknown_sodium_rejections,
        },
        ranking: "热量最接近区间中点 → 新增餐品更少 → 蛋白质更高 → 餐品名确定性排序".into(),
        notice: "营养信息不是门店实时菜单；价格、供应、过敏原未核验。仅供参考，不构成医疗或专业营养建议。".into(),
    })
}
