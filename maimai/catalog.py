"""Strict per-serving nutrition imports; unknown formats never become demo data."""

from datetime import datetime
from decimal import Decimal, InvalidOperation, localcontext
import json
from pathlib import Path
import re


class CatalogError(ValueError):
    pass


_NUMBER = re.compile(r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?\Z", re.ASCII)
_PER_100 = re.compile(r"(?:per\s*100\s*(?:grams?|g|millilit(?:er|re)s?|ml)\b|每\s*(?:100|一百)\s*(?:公克|克|毫升|g|ml)|/\s*100\s*(?:grams?|g|ml)\b)", re.I)
_UNKNOWN = {"", "-", "—", "未知", "unknown", "n/a", "na", "null"}
_NAMES = {"name", "foodname", "名称", "食品名称", "餐品名称"}
_ENERGY = {"kcal": "kcal", "energy_kcal": "kcal", "energy_kj": "kJ",
           "能量(kcal)": "kcal", "热量(kcal)": "kcal", "能量(kj)": "kJ",
           "热量(kj)": "kJ", "energy(kcal)": "kcal", "energy(kj)": "kJ"}
_PROTEIN = {"protein_g", "蛋白质(g)", "protein(g)"}
_SODIUM = {"sodium_mg", "钠(mg)", "sodium(mg)"}


def _key(value):
    return re.sub(r"\s+", "", value.replace("（", "(").replace("）", ")")).casefold()


def _decimal(value, field):
    if isinstance(value, bool) or not isinstance(value, (str, int, float, Decimal)):
        raise CatalogError(field + " must be an explicit finite nonnegative number.")
    text = str(value).strip()
    if len(text) > 128 or not _NUMBER.fullmatch(text):
        raise CatalogError(field + " must be numeric without unknown or inferred units.")
    try:
        number = Decimal(text)
    except InvalidOperation:
        raise CatalogError(field + " is not a decimal number.") from None
    if not number.is_finite() or number < 0 or abs(number.as_tuple().exponent) > 1000:
        raise CatalogError(field + " must be finite, nonnegative and within supported decimal precision.")
    return number


def _text(number):
    # Do not use normalize(): it rounds large supplied decimals to the active context.
    result = format(number, "f")
    return "0" if number == 0 else result


def _rows(rows):
    if not isinstance(rows, list) or not rows:
        raise CatalogError("Catalog foods must be a nonempty array; no rows were imported.")
    result = []
    seen = set()
    for index, row in enumerate(rows, 1):
        if not isinstance(row, dict):
            raise CatalogError("Food row %d must be an object." % index)
        name = row.get("name")
        if not isinstance(name, str) or not name.strip():
            raise CatalogError("Food row %d requires a serving-specific name." % index)
        name = name.strip()
        identity = name.casefold()
        if identity in seen:
            raise CatalogError("Duplicate serving name at food row %d; use distinct explicit serving names." % index)
        seen.add(identity)
        if "kcal" not in row or "protein_g" not in row:
            raise CatalogError("Food row %d requires kcal and protein_g; missing values are not zero." % index)
        sodium = row.get("sodium_mg")
        result.append({"name": name, "kcal": _text(_decimal(row["kcal"], "kcal")),
                       "protein_g": _text(_decimal(row["protein_g"], "protein_g")),
                       "sodium_mg": None if sodium is None else _text(_decimal(sodium, "sodium_mg"))})
    return result


def _check_basis(value):
    if _PER_100.search(json.dumps(value, ensure_ascii=False, default=str)):
        raise CatalogError("Per-100g/per-100ml data cannot be used as per-serving nutrition; provide explicit per-serving data.")


def validate_catalog(document: object) -> dict:
    if not isinstance(document, dict):
        raise CatalogError("Catalog must be a JSON object.")
    if type(document.get("schema_version")) is not int or document["schema_version"] != 1:
        raise CatalogError("Catalog schema_version must be integer 1.")
    source = document.get("source")
    if not isinstance(source, dict) or source.get("kind") not in ("mcp", "demo", "import"):
        raise CatalogError("Catalog source.kind must be mcp, demo or import.")
    canonical_source = {"kind": source["kind"]}
    if source["kind"] == "mcp":
        if source.get("tool") != "list-nutrition-foods":
            raise CatalogError("MCP catalog source.tool must be list-nutrition-foods.")
        timestamp = source.get("fetched_at")
        if not isinstance(timestamp, str) or "T" not in timestamp:
            raise CatalogError("MCP catalog requires an ISO8601 fetched_at timestamp.")
        try:
            parsed = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
        except ValueError:
            raise CatalogError("MCP catalog requires an ISO8601 fetched_at timestamp.") from None
        if parsed.tzinfo is None:
            raise CatalogError("MCP fetched_at must include a timezone.")
        canonical_source.update(tool="list-nutrition-foods", fetched_at=timestamp)
    if "note" in source:
        if not isinstance(source["note"], str):
            raise CatalogError("Catalog source.note must be text.")
        canonical_source["note"] = source["note"]
    # Canonical values are per serving; provenance may describe an already
    # completed, evidence-backed conversion from a per-100g source.
    _check_basis({key: value for key, value in document.items() if key != "source"})
    return {"schema_version": 1, "source": canonical_source, "foods": _rows(document.get("foods"))}


def _json_loads(text):
    def invalid_constant(value):
        raise ValueError("Nonfinite JSON constant")
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Duplicate JSON property")
            result[key] = value
        return result
    return json.loads(text, parse_float=Decimal, parse_constant=invalid_constant, object_pairs_hook=unique_object)


def load_catalog(path: str | Path) -> dict:
    try:
        document = _json_loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValueError):
        raise CatalogError("Cannot load catalog: provide a readable UTF-8 JSON file without duplicate properties or nonfinite numbers.") from None
    return validate_catalog(document)


def _measure(value, unit, field):
    if isinstance(value, str):
        text = value.strip()
        if field == "sodium_mg" and text.casefold() in _UNKNOWN:
            return None
        # A suffix is optional because the column/key already provides its unit.
        match = re.fullmatch(r"(.+?)\s*(kcal|kj|mg|g)", text, re.I)
        if match:
            if match[2].casefold() != unit.casefold():
                raise CatalogError(field + " cell unit conflicts with its explicit header/key unit.")
            value = match[1].strip()
    if value is None and field == "sodium_mg":
        return None
    number = _decimal(value, field)
    if unit == "kJ":
        with localcontext() as context:
            context.prec = max(40, len(number.as_tuple().digits) + 16)
            number = number / Decimal("4.184")
    return _text(number)


def _alias_row(row):
    if not isinstance(row, dict) or any(not isinstance(key, str) for key in row):
        raise CatalogError("Each nutrition food must be an object with named fields.")
    mapped = {}
    for original, value in row.items():
        key = _key(original)
        if key in _NAMES:
            field, unit = "name", None
        elif key in _ENERGY:
            field, unit = "kcal", _ENERGY[key]
        elif key in _PROTEIN:
            field, unit = "protein_g", "g"
        elif key in _SODIUM:
            field, unit = "sodium_mg", "mg"
        elif any(word in key for word in ("sodium", "钠", "protein", "蛋白", "energy", "calori", "能量", "热量")):
            raise CatalogError("Unsupported nutrient field or unit; use explicit kcal (or kJ), protein_g and sodium_mg keys.")
        else:
            continue
        if field in mapped:
            raise CatalogError("Nutrition row contains ambiguous duplicate nutrient/name aliases.")
        mapped[field] = value if field == "name" else _measure(value, unit, field)
    return mapped


def _toon_cells(line):
    """Decode only scalar comma-separated rows, not a general TOON document."""
    cells = []
    position = 0
    decoder = json.JSONDecoder()
    while position < len(line):
        while position < len(line) and line[position] in " \t":
            position += 1
        if position < len(line) and line[position] == '"':
            try:
                value, end = decoder.raw_decode(line, position)
            except ValueError:
                raise CatalogError("Malformed quoted TOON scalar.") from None
            if any(0xD800 <= ord(char) <= 0xDFFF for char in value):
                raise CatalogError("TOON strings cannot contain surrogate codepoints.")
            if any(match[1] not in '"\\nrtu' for match in re.finditer(r"\\(.)", line[position:end])):
                raise CatalogError("Unsupported TOON string escape.")
            position = end
            while position < len(line) and line[position] in " \t":
                position += 1
            if position < len(line) and line[position] != ",":
                raise CatalogError("Unexpected content after a quoted TOON scalar.")
        else:
            end = line.find(",", position)
            if end < 0:
                end = len(line)
            token = line[position:end].strip(" \t")
            value = {"null": None, "true": True, "false": False}.get(token, token)
            position = end
        cells.append(value)
        if position < len(line):
            position += 1
            if position == len(line):
                cells.append("")
    return cells


def _mcd_toon_rows(document, diagnostics):
    if document.get("success") is not True or type(document.get("code")) is not int or document["code"] != 200:
        raise CatalogError("Nutrition API payload was not successful; no rows imported.")
    data = document.get("data")
    if not isinstance(data, str):
        raise CatalogError("Nutrition API data must be an observed TOON table string.")
    lines = data.strip().splitlines()
    header = re.fullmatch(r"\[(0|[1-9][0-9]{0,8})\]\{([^{}]+)\}:", lines[0]) if lines else None
    if header is None:
        raise CatalogError("Unsupported nutrition TOON header.")
    fields = header[2].split(",")
    known = {"productName", "nutritionDescription", "energyKj", "energyKcal",
             "protein", "fat", "carbohydrate", "sodium", "calcium"}
    if len(fields) != len(known) or set(fields) != known:
        raise CatalogError("Nutrition TOON fields differ from the verified official schema.")
    count = int(header[1])
    if len(lines) - 1 != count:
        raise CatalogError("Nutrition TOON declared row count does not match its data.")
    rows = []
    identities = {}
    for line in lines[1:]:
        if not line.startswith("  ") or line.startswith("   "):
            raise CatalogError("Nutrition TOON row must have the observed two-space indentation.")
        values = _toon_cells(line[2:])
        if len(values) != len(fields):
            raise CatalogError("Nutrition TOON row width does not match its header.")
        row = dict(zip(fields, values))
        name = row["productName"]
        if not isinstance(name, str) or not name.strip():
            raise CatalogError("Nutrition TOON row has no serving name.")
        identity = name.strip().casefold()
        # Identical upstream catalog entries are not additional physical servings.
        if identity in identities:
            if row != identities[identity]:
                raise CatalogError("Conflicting nutrition TOON rows share a serving name.")
            continue
        identities[identity] = row
        # Unit/basis evidence: official nutrition_calculator states single servings,
        # protein in g and sodium in mg; milk/McChicken values were cross-checked.
        rows.append({"name": name, "kcal": row["energyKcal"],
                     "protein_g": row["protein"], "sodium_mg": row["sodium"]})
    diagnostics.update(format="mcd-toon", input_rows=count,
                       duplicate_rows_removed=count - len(rows))
    return rows


def _json_rows(document, diagnostics=None):
    _check_basis(document)
    if isinstance(document, dict) and "data" in document:
        return _mcd_toon_rows(document, {} if diagnostics is None else diagnostics)
    if isinstance(document, list):
        rows = document
    elif isinstance(document, dict) and "foods" in document:
        rows = document["foods"]
    else:
        raise CatalogError("Unsupported nutrition JSON: expected a foods array with explicit unit-bearing keys; save --raw-out and adapt the actual schema.")
    if not isinstance(rows, list) or not rows:
        raise CatalogError("Nutrition foods must be a nonempty array.")
    return [_alias_row(row) for row in rows]


def _cells(line):
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def _markdown_rows(text):
    _check_basis(text)
    lines = text.splitlines()
    rows = []
    found = False
    index = 0
    while index + 1 < len(lines):
        if "|" not in lines[index] or "|" not in lines[index + 1]:
            index += 1
            continue
        headers = _cells(lines[index])
        divider = _cells(lines[index + 1])
        if not divider or not all(re.fullmatch(r":?-{3,}:?", cell) for cell in divider):
            index += 1
            continue
        found = True
        keys = [_key(header) for header in headers]
        if len(divider) != len(headers) or len(set(keys)) != len(keys):
            raise CatalogError("Malformed or duplicate Markdown nutrition table headers.")
        if not any(key in _NAMES for key in keys) or not any(key in _ENERGY for key in keys) or not any(key in _PROTEIN for key in keys):
            raise CatalogError("Nutrition table needs a name, explicit energy(kcal/kJ) and protein(g) headers; unlabeled units cannot be guessed.")
        index += 2
        count = 0
        while index < len(lines) and "|" in lines[index] and lines[index].strip():
            cells = _cells(lines[index])
            if len(cells) != len(headers):
                raise CatalogError("Nutrition table row has a different number of cells than its header.")
            rows.append(_alias_row(dict(zip(headers, cells))))
            count += 1
            index += 1
        if count == 0:
            raise CatalogError("Nutrition Markdown table contains no food rows.")
    return rows if found else None


def normalize_tool_result(result: dict, *, diagnostics: dict | None = None) -> list[dict]:
    if diagnostics is None:
        diagnostics = {}
    else:
        diagnostics.clear()
    if not isinstance(result, dict):
        raise CatalogError("Nutrition tool result must be an object.")
    if result.get("isError"):
        raise CatalogError("Nutrition tool returned an error; no food data was imported.")
    if "structuredContent" in result:
        return _rows(_json_rows(result["structuredContent"], diagnostics))
    blocks = result.get("content")
    if not isinstance(blocks, list):
        raise CatalogError("Nutrition tool has no supported content; save --raw-out and inspect the actual schema.")
    rows = []
    for block in blocks:
        if not isinstance(block, dict) or block.get("type") != "text":
            continue
        text = block.get("text")
        if not isinstance(text, str):
            raise CatalogError("Nutrition text block does not contain text.")
        text = text.strip()
        if text.startswith("```"):
            lines = text.splitlines()
            if len(lines) >= 3 and lines[-1].strip() == "```":
                text = "\n".join(lines[1:-1]).strip()
        if text.startswith(("{", "[")):
            try:
                document = _json_loads(text)
            except (ValueError, TypeError):
                raise CatalogError("Nutrition tool returned malformed JSON; save --raw-out and inspect the actual schema.") from None
            rows.extend(_json_rows(document, diagnostics))
        else:
            table = _markdown_rows(text)
            if table is not None:
                rows.extend(table)
    if not rows:
        raise CatalogError("Unsupported nutrition format: need a foods JSON array or explicit-unit Markdown table; save --raw-out and adapt the actual schema.")
    return _rows(rows)
