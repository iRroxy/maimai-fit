import unittest
from decimal import Decimal

from maimai.planner import Constraints, PlanningError, solve
from maimai.report import render_html


def food(name, kcal, protein, sodium=100):
    return {"name": name, "kcal": str(kcal), "protein_g": str(protein), "sodium_mg": None if sodium is None else str(sodium)}


def catalog(*foods):
    return {"schema_version": 1, "source": {"kind": "demo", "note": "合成测试数据"}, "foods": list(foods)}


class PlannerTests(unittest.TestCase):
    def targets(self, **changes):
        arguments = {"keep": ("保留品",), "min_kcal": Decimal("450"), "max_kcal": Decimal("650"), "min_protein": Decimal("30")}
        arguments.update(changes)
        return Constraints(**arguments)

    def test_retains_anchor_and_every_returned_plan_obeys_all_caps(self):
        data = catalog(food("保留品", 410, 25, 650), food("配菜", 90, 5, 120), food("高蛋白配菜", 130, 12, 240), food("高盐配菜", 110, 18, 2000))
        result = solve(data, self.targets(max_sodium=Decimal("1000"), top=10))
        expected_names = [{"保留品", "配菜"}, {"保留品", "高蛋白配菜"}]
        self.assertEqual([set(f["name"] for f in plan["foods"]) for plan in result["plans"]], list(reversed(expected_names)))
        for plan in result["plans"]:
            values = plan["totals"]
            self.assertGreaterEqual(Decimal(values["kcal"]), Decimal("450"))
            self.assertLessEqual(Decimal(values["kcal"]), Decimal("650"))
            self.assertGreaterEqual(Decimal(values["protein_g"]), Decimal("30"))
            self.assertLessEqual(Decimal(values["sodium_mg"]), Decimal("1000"))

    def test_inclusive_decimal_limits_without_rounding_leak(self):
        exact = solve(catalog(food("保留品", "650", "30")), self.targets())
        self.assertEqual(exact["status"], "ok")
        excess = solve(catalog(food("保留品", "650.0000001", "30")), self.targets())
        self.assertEqual(excess["status"], "infeasible")
        insufficient = solve(catalog(food("保留品", "500", "29.9999999")), self.targets())
        self.assertEqual(insufficient["status"], "infeasible")
        high_precision = solve(
            catalog(food("保留品", "500.00000000000000000000000000001", "30")),
            self.targets(max_kcal=Decimal("500")),
        )
        self.assertEqual(high_precision["status"], "infeasible")

    def test_missing_sodium_is_not_zero_and_cannot_satisfy_cap(self):
        data = catalog(food("保留品", 500, 35, None))
        unconstrained = solve(data, self.targets())
        self.assertIsNone(unconstrained["plans"][0]["totals"]["sodium_mg"])
        constrained = solve(data, self.targets(max_sodium=Decimal("1000")))
        self.assertEqual(constrained["status"], "infeasible")
        self.assertEqual(constrained["search"]["unknown_sodium_rejections"], 1)

    def test_infeasible_never_substitutes_favorite(self):
        data = catalog(food("保留品", 900, 40), food("很合适的另一个主食", 500, 35))
        result = solve(data, self.targets())
        self.assertEqual(result["status"], "infeasible")
        self.assertEqual(result["plans"], [])

    def test_exclude_matches_casefold_and_conflicting_keep_is_error(self):
        data = catalog(food("保留品", 410, 25), food("HOT配菜", 100, 10), food("普通配菜", 120, 12))
        result = solve(data, self.targets(exclude=("hot",)))
        self.assertEqual([f["name"] for f in result["plans"][0]["foods"]], ["保留品", "普通配菜"])
        with self.assertRaises(PlanningError):
            solve(data, self.targets(exclude=("保留",)))

    def test_multiple_favorites_retained_once_and_item_limit_respected(self):
        data = catalog(food("保留品", 300, 20), food("第二保留品", 200, 12), food("配菜", 50, 3))
        result = solve(data, self.targets(keep=("保留品", "第二保留品"), max_items=2))
        self.assertEqual([f["name"] for f in result["plans"][0]["foods"]], ["保留品", "第二保留品"])
        self.assertEqual(result["search"]["examined"], 1)
        with self.assertRaises(PlanningError):
            solve(data, self.targets(keep=("保留品", "保留品")))
        with self.assertRaises(PlanningError):
            solve(data, self.targets(keep=("不存在",)))

    def test_rank_prefers_fewer_changes_after_equal_calorie_distance(self):
        data = catalog(food("保留品", 400, 20), food("一个配菜", 150, 10), food("半份配菜A", 70, 10), food("半份配菜B", 80, 12))
        result = solve(data, self.targets())
        self.assertEqual([f["name"] for f in result["plans"][0]["foods"]], ["保留品", "一个配菜"])

    def test_invalid_targets_rejected(self):
        data = catalog(food("保留品", 500, 35))
        for changes in ({"min_kcal": Decimal("NaN")}, {"min_protein": Decimal("-1")}, {"min_kcal": Decimal("700")}, {"max_items": 4}, {"top": 0}):
            with self.subTest(changes=changes), self.assertRaises(PlanningError):
                solve(data, self.targets(**changes))

    def test_report_escapes_untrusted_food_and_provenance(self):
        name = '<img src=x onerror="alert(1)">'
        data = catalog(food(name, 500, 35))
        data["source"]["note"] = "</p><script>alert(2)</script>"
        result = solve(data, self.targets(keep=(name,)))
        html = render_html(result)
        self.assertNotIn("<img src=x", html)
        self.assertNotIn("<script>", html)
        self.assertIn("&lt;img", html)
        self.assertIn("&lt;script&gt;", html)


if __name__ == "__main__":
    unittest.main()
