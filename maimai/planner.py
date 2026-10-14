"""Exact, preference-preserving search over distinct single-serving foods."""
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation, localcontext
from itertools import combinations

from .catalog import validate_catalog


class PlanningError(ValueError):
    pass


def number(value, label):
    if isinstance(value, bool) or len(str(value)) > 128:
        raise PlanningError(f"{label}必须是非负有限数值（最多 128 字符）")
    try:
        result = Decimal(str(value))
    except (InvalidOperation, ValueError):
        raise PlanningError(f"{label}必须是非负有限数值") from None
    if not result.is_finite() or result < 0 or abs(result.as_tuple().exponent) > 1000:
        raise PlanningError(f"{label}必须是非负有限数值，十进制指数不超过 ±1000")
    return result


@dataclass(frozen=True)
class Constraints:
    keep: tuple[str, ...]
    exclude: tuple[str, ...] = ()
    min_kcal: Decimal = Decimal("450")
    max_kcal: Decimal = Decimal("650")
    min_protein: Decimal = Decimal("30")
    max_sodium: Decimal | None = None
    max_items: int = 3
    top: int = 3

    def __post_init__(self):
        for label in ("min_kcal", "max_kcal", "min_protein"):
            object.__setattr__(self, label, number(getattr(self, label), label))
        if self.max_sodium is not None:
            object.__setattr__(self, "max_sodium", number(self.max_sodium, "max_sodium"))

    def validate(self):
        for label in ("min_kcal", "max_kcal", "min_protein"):
            number(getattr(self, label), label)
        if self.max_sodium is not None:
            number(self.max_sodium, "max_sodium")
        if self.min_kcal > self.max_kcal:
            raise PlanningError("最低热量不能大于最高热量")
        if not self.keep or any(not isinstance(name, str) or not name.strip() for name in self.keep):
            raise PlanningError("至少指定一份要保留的餐品（--keep 精确名称）")
        if len({name.strip().casefold() for name in self.keep}) != len(self.keep):
            raise PlanningError("要保留的餐品不能重复；每种餐品只计算一份")
        if type(self.max_items) is not int or not 1 <= self.max_items <= 3:
            raise PlanningError("max-items 必须在 1–3 之间")
        if len(self.keep) > self.max_items:
            raise PlanningError("保留餐品数量超过 max-items")
        if type(self.top) is not int or not 1 <= self.top <= 10:
            raise PlanningError("top 必须在 1–10 之间")
        if any(not isinstance(term, str) or not term.strip() for term in self.exclude):
            raise PlanningError("排除关键词不能为空")

    def as_dict(self):
        return {
            "keep": list(self.keep), "exclude": list(self.exclude),
            "min_kcal": str(self.min_kcal), "max_kcal": str(self.max_kcal),
            "min_protein_g": str(self.min_protein),
            "max_sodium_mg": None if self.max_sodium is None else str(self.max_sodium),
            "max_items": self.max_items, "top": self.top,
        }


def solve(document, constraints):
    """Never relax hard constraints; return explicit infeasible results."""
    constraints.validate()
    catalog = validate_catalog(document)
    numeric = {
        food["name"]: (Decimal(food["kcal"]), Decimal(food["protein_g"]),
                       None if food.get("sodium_mg") is None else Decimal(food["sodium_mg"]))
        for food in catalog["foods"]
    }
    values = [value for nutrients in numeric.values() for value in nutrients if value is not None]
    values.extend((constraints.min_kcal, constraints.max_kcal, constraints.min_protein))
    if constraints.max_sodium is not None:
        values.append(constraints.max_sodium)
    # Preserve all input digits in sums, differences and the half-step midpoint.
    precision = max(value.adjusted() for value in values) - min(value.as_tuple().exponent for value in values) + 4
    with localcontext() as context:
        context.prec = max(28, precision)
        return _search(catalog, constraints, numeric)


def _search(catalog, constraints, numeric):
    foods = catalog["foods"]
    by_name = {food["name"].casefold(): food for food in foods}
    mandatory = []
    excluded = tuple(term.strip().casefold() for term in constraints.exclude)
    for name in constraints.keep:
        food = by_name.get(name.strip().casefold())
        if food is None:
            raise PlanningError(f"找不到要保留的餐品：{name}；请使用目录中的精确份量名称")
        if any(term in food["name"].casefold() for term in excluded):
            raise PlanningError(f"保留与排除条件冲突：{food['name']}")
        mandatory.append(food)
    mandatory_names = {food["name"].casefold() for food in mandatory}
    available = [food for food in foods if food["name"].casefold() not in mandatory_names
                 and not any(term in food["name"].casefold() for term in excluded)]
    midpoint = (constraints.min_kcal + constraints.max_kcal) / 2
    best = []
    feasible_count = 0
    examined = 0
    unknown_sodium_rejections = 0
    for added_count in range(constraints.max_items - len(mandatory) + 1):
        for extras in combinations(available, added_count):
            examined += 1
            selected = (*mandatory, *extras)
            values = [numeric[food["name"]] for food in selected]
            kcal = sum((value[0] for value in values), Decimal(0))
            protein = sum((value[1] for value in values), Decimal(0))
            if not constraints.min_kcal <= kcal <= constraints.max_kcal or protein < constraints.min_protein:
                continue
            sodium = None if any(value[2] is None for value in values) else sum((value[2] for value in values), Decimal(0))
            if constraints.max_sodium is not None:
                if sodium is None:
                    unknown_sodium_rejections += 1
                    continue
                if sodium > constraints.max_sodium:
                    continue
            feasible_count += 1
            names = tuple(sorted(food["name"] for food in selected))
            # The rank is deliberately explainable, not a nutrition/health score.
            rank = (abs(kcal - midpoint), added_count, -protein, names)
            candidate = {
                "foods": list(selected),
                "totals": {"kcal": str(kcal), "protein_g": str(protein),
                           "sodium_mg": None if sodium is None else str(sodium)},
                "added_items": added_count, "target_distance_kcal": str(abs(kcal - midpoint)),
            }
            best.append((rank, candidate))
            best.sort(key=lambda entry: entry[0])
            if len(best) > constraints.top:
                best.pop()
    return {
        "status": "ok" if best else "infeasible",
        "source": catalog["source"], "constraints": constraints.as_dict(),
        "plans": [candidate for _, candidate in best],
        "search": {"examined": examined, "feasible": feasible_count,
                   "unknown_sodium_rejections": unknown_sodium_rejections},
        "ranking": "热量最接近区间中点 → 新增餐品更少 → 蛋白质更高 → 餐品名确定性排序",
        "notice": "营养信息不是门店实时菜单；价格、供应、过敏原未核验。仅供参考，不构成医疗或专业营养建议。",
    }
