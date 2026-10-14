import json
import unittest

from maimai.catalog import CatalogError, normalize_tool_result


FIELDS = "productName,nutritionDescription,energyKj,energyKcal,protein,fat,carbohydrate,sodium,calcium"


def payload(rows, declared=None, **changes):
    document = {"success": True, "code": 200, "data":
                f"[{len(rows) if declared is None else declared}]{{{FIELDS}}}:\n" +
                "\n".join("  " + row for row in rows)}
    document.update(changes)
    return {"structuredContent": document}


class OfficialToonTests(unittest.TestCase):
    def test_explicit_official_schema_and_quoted_serving_name(self):
        name = '示例,含"引号"与\\符号（一份）'
        row = json.dumps(name, ensure_ascii=False) + ",null,2092,500,30,10,40,null,20"
        rows = normalize_tool_result(payload([row]))
        self.assertEqual(rows[0], {"name": name, "kcal": "500", "protein_g": "30", "sodium_mg": None})

    def test_identical_source_entries_merge_with_quality_diagnostics(self):
        row = "示例（一份）,null,2092,500,30,10,40,100,20"
        diagnostics = {}
        rows = normalize_tool_result(payload([row, row]), diagnostics=diagnostics)
        self.assertEqual(rows, [{"name": "示例（一份）", "kcal": "500", "protein_g": "30", "sodium_mg": "100"}])
        self.assertEqual(diagnostics["input_rows"], 2)
        self.assertEqual(diagnostics["duplicate_rows_removed"], 1)

    def test_conflicting_same_serving_is_not_silently_selected(self):
        row = "示例（一份）,null,2092,500,30,10,40,100,20"
        for other in (row.replace(",500,", ",600,"), row.replace(",10,40,", ",11,40,")):
            with self.subTest(other=other), self.assertRaises(CatalogError):
                normalize_tool_result(payload([row, other]))

    def test_truncated_or_extra_rows_fail_declared_count(self):
        row = "示例（一份）,null,2092,500,30,10,40,100,20"
        for count in (0, 2):
            with self.subTest(count=count), self.assertRaises(CatalogError):
                normalize_tool_result(payload([row], declared=count))

    def test_wrong_fields_width_and_numeric_values_fail_closed(self):
        valid = "示例（一份）,null,2092,500,30,10,40,100,20"
        for row in (valid.rsplit(",", 1)[0], valid.replace(",30,", ",true,"), valid.replace(",500,", ",null,")):
            with self.subTest(row=row), self.assertRaises(CatalogError):
                normalize_tool_result(payload([row]))
        changed = payload([valid])
        changed["structuredContent"]["data"] = changed["structuredContent"]["data"].replace("energyKcal", "energy")
        with self.assertRaises(CatalogError):
            normalize_tool_result(changed)

    def test_business_error_is_not_treated_as_nutrition_success(self):
        row = "示例（一份）,null,2092,500,30,10,40,100,20"
        for values in ({"success": False}, {"code": 500}):
            with self.subTest(values=values), self.assertRaises(CatalogError):
                normalize_tool_result(payload([row], **values))

    def test_bad_quotes_and_row_indentation_are_rejected(self):
        for row in ('"示例"extra,null,2092,500,30,10,40,100,20', '"unterminated,null,2092,500,30,10,40,100,20'):
            with self.subTest(row=row), self.assertRaises(CatalogError):
                normalize_tool_result(payload([row]))
        changed = payload(["示例（一份）,null,2092,500,30,10,40,100,20"])
        changed["structuredContent"]["data"] = changed["structuredContent"]["data"].replace("\n  ", "\n    ")
        with self.assertRaises(CatalogError):
            normalize_tool_result(changed)


if __name__ == "__main__":
    unittest.main()
