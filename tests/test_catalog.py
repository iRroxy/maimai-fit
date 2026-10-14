import json
from decimal import Decimal
from pathlib import Path
import tempfile
import unittest

from maimai.catalog import CatalogError, load_catalog, normalize_tool_result, validate_catalog


def catalog(foods=None):
    return {"schema_version": 1, "source": {"kind": "import"},
            "foods": foods if foods is not None else [{"name": " Food (one serving) ", "kcal": "12.50", "protein_g": "1.25"}]}


def tool(*texts):
    return {"content": [{"type": "text", "text": text} for text in texts]}


class CatalogTests(unittest.TestCase):
    def test_decimal_preservation_and_unknown_sodium(self):
        value = "123456789012345678901234567890.123456789"
        result = validate_catalog(catalog([{"name": " A ", "kcal": value, "protein_g": 0}]))
        self.assertEqual(result["foods"], [{"name": "A", "kcal": value, "protein_g": "0", "sodium_mg": None}])

    def test_missing_is_not_zero_and_invalid_numbers_rejected(self):
        for value in (None, True, False, "", "NaN", "Infinity", float("inf"), -1, "2 mg", "1,000"):
            with self.subTest(value=value), self.assertRaises(CatalogError):
                validate_catalog(catalog([{"name": "A", "kcal": value, "protein_g": 1}]))
        with self.assertRaises(CatalogError):
            validate_catalog(catalog([{"name": "A", "kcal": 1}]))

    def test_duplicate_serving_names_are_not_merged(self):
        for names in ((" A ", "a"), ("Straße", "STRASSE")):
            with self.subTest(names=names), self.assertRaises(CatalogError):
                validate_catalog(catalog([{"name": name, "kcal": 1, "protein_g": 0} for name in names]))
        result = validate_catalog(catalog([{"name": name, "kcal": 1, "protein_g": 0}
                                           for name in ("A (small)", "A (large)")]))
        self.assertEqual(len(result["foods"]), 2)

    def test_source_and_version_are_required(self):
        for version in (True, 1.0, "1", 2, None):
            data = catalog()
            data["schema_version"] = version
            with self.subTest(version=version), self.assertRaises(CatalogError):
                validate_catalog(data)
        for source in ({}, {"kind": "real"}, {"kind": "mcp"},
                       {"kind": "mcp", "tool": "list-nutrition-foods", "fetched_at": "2026-10-14T10:00:00"}):
            data = catalog()
            data["source"] = source
            with self.subTest(source=source), self.assertRaises(CatalogError):
                validate_catalog(data)

    def test_load_preserves_json_decimal_and_rejects_duplicate_properties(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "catalog.json"
            path.write_text('{"schema_version":1,"source":{"kind":"import"},"foods":'
                            '[{"name":"A","kcal":1.12345678901234567890123456789,"protein_g":0}]}', encoding="utf-8")
            self.assertEqual(load_catalog(path)["foods"][0]["kcal"], "1.12345678901234567890123456789")
            path.write_text('{"schema_version":1,"schema_version":1}', encoding="utf-8")
            with self.assertRaises(CatalogError):
                load_catalog(path)

    def test_structured_aliases_and_authoritative_structured_content(self):
        result = {"structuredContent": {"foods": [{"foodName": "A", "energy_kcal": "100.5", "protein_g": "2", "sodium_mg": None}]},
                  "content": [{"type": "text", "text": "unrelated presentation"}]}
        self.assertEqual(normalize_tool_result(result)[0],
                         {"name": "A", "kcal": "100.5", "protein_g": "2", "sodium_mg": None})

    def test_multiple_json_blocks_are_all_validated(self):
        rows = normalize_tool_result(tool(json.dumps({"foods": [{"name": "A", "kcal": 2, "protein_g": 1}]}),
                                          json.dumps({"foods": [{"name": "B", "kcal": 3, "protein_g": 2}]})))
        self.assertEqual([row["name"] for row in rows], ["A", "B"])
        with self.assertRaises(CatalogError):
            normalize_tool_result(tool(json.dumps({"foods": [{"name": "A", "kcal": 2, "protein_g": 1}]}),
                                       json.dumps({"foods": [{"name": "B", "kcal": 3}]})))

    def test_explicit_markdown_units_and_kj_conversion(self):
        rows = normalize_tool_result(tool("| 餐品名称 | 能量（kJ） | 蛋白质(g) | 钠(mg) |\n"
                                          "| --- | ---: | --- | --- |\n"
                                          "| A (一份) | 418.4 kJ | 2.50 g | — |\n"
                                          "| B (一份) | 836.8 | 4 | 10.25 mg |"))
        self.assertEqual(Decimal(rows[0]["kcal"]), Decimal("100"))
        self.assertIsNone(rows[0]["sodium_mg"])
        self.assertEqual(rows[1]["sodium_mg"], "10.25")

    def test_markdown_missing_sodium_is_unknown_not_zero(self):
        rows = normalize_tool_result(tool("| 名称 | 热量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 0 |"))
        self.assertIsNone(rows[0]["sodium_mg"])

    def test_unknown_and_incomplete_units_are_rejected(self):
        for header, cell in (("能量", "100"), ("能量(kcal)", "100 kJ"), ("能量(kcal)", "")):
            text = "| 名称 | %s | 蛋白质(g) |\n| --- | --- | --- |\n| A | %s | 1 |" % (header, cell)
            with self.subTest(header=header, cell=cell), self.assertRaises(CatalogError):
                normalize_tool_result(tool(text))
        for extra in ({"sodium": 100}, {"钠(g)": 1}, {"protein": 3}, {"calories": 5}):
            row = {"name": "A", "kcal": 1, "protein_g": 0, **extra}
            with self.subTest(extra=extra), self.assertRaises(CatalogError):
                normalize_tool_result({"structuredContent": {"foods": [row]}})

    def test_no_partial_import_for_malformed_table_rows(self):
        for last in ("| B | 2 |", "| B | 2 | unknown |", "| a | 2 | 1 |"):
            with self.subTest(last=last), self.assertRaises(CatalogError):
                normalize_tool_result(tool("| 名称 | 能量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 1 |\n" + last))

    def test_per_100g_metadata_is_rejected(self):
        for basis in ("per 100g", "每100克", "每 100 ml", "/100g"):
            with self.subTest(basis=basis), self.assertRaises(CatalogError):
                normalize_tool_result(tool(basis + "\n| 名称 | 能量(kcal) | 蛋白质(g) |\n| --- | --- | --- |\n| A | 1 | 1 |"))
        data = catalog()
        data["basis"] = "per 100g"
        with self.assertRaises(CatalogError):
            validate_catalog(data)

    def test_converted_per_serving_values_can_record_original_basis(self):
        data = catalog([{"name": "一份", "kcal": "500", "protein_g": "30"}])
        data["source"]["note"] = "源数据每100g，按明确份重200g转换为每份"
        self.assertEqual(validate_catalog(data)["foods"][0]["kcal"], "500")

    def test_errors_and_unknown_formats_do_not_fabricate_foods(self):
        for result in ({"isError": True, "content": []}, {"content": []}, tool("No supported foods"),
                       {"structuredContent": {"unexpected": []}}, tool('{"foods": []}'), tool('{broken}')):
            with self.subTest(result=result), self.assertRaises(CatalogError):
                normalize_tool_result(result)


if __name__ == "__main__":
    unittest.main()
