"""CLI for real MCP capture, exact planning, demonstration and Skill install."""
import argparse
from datetime import datetime, timezone
from decimal import Decimal
import getpass
import json
import os
from pathlib import Path
import shutil
import sys
import warnings

from .catalog import CatalogError, load_catalog, normalize_tool_result, validate_catalog
from .mcp import McpClient, McpError
from .planner import Constraints, PlanningError, number, solve
from .report import render_html

ROOT = Path(__file__).resolve().parents[1]
DEMO_ANCHOR = "示例鸡肉主食（一份）"


def planning_arguments(parser):
    parser.add_argument("--keep", action="append", help="必须保留的精确餐品名称，可重复指定不同餐品")
    parser.add_argument("--exclude", action="append", default=[], help="排除名称包含此关键词的餐品")
    parser.add_argument("--min-kcal", default="450", help="热量下限 kcal（不是健康建议）")
    parser.add_argument("--max-kcal", default="650", help="热量上限 kcal")
    parser.add_argument("--min-protein", default="30", help="蛋白质下限 g")
    parser.add_argument("--max-sodium", help="可选钠上限 mg；未知钠的组合不合格")
    parser.add_argument("--max-items", type=int, default=3, help="每种一份，合计最多 1–3 种餐品")
    parser.add_argument("--top", type=int, default=3, help="输出 1–10 个方案")
    parser.add_argument("--json", action="store_true", help="输出结构化 JSON")
    parser.add_argument("--html", type=Path, help="另存静态 HTML 报告，不覆盖已有文件")


def parser():
    cli = argparse.ArgumentParser(prog="maimai", description="麦麦刚刚好：保留想吃的那一份，按你的营养约束配餐")
    commands = cli.add_subparsers(dest="command", required=True)
    demo = commands.add_parser("demo", help="合成数据演示，不请求网络")
    planning_arguments(demo)
    plan = commands.add_parser("plan", help="读取已有营养目录并计算")
    plan.add_argument("--catalog", required=True, type=Path)
    planning_arguments(plan)
    fetch = commands.add_parser("fetch", help="从官方 MCP 获取营养目录")
    fetch.add_argument("--out", required=True, type=Path)
    fetch.add_argument("--raw-out", type=Path, help="保存工具原始返回，用于未知格式的人工规范化")
    fetch.add_argument("--arguments", default="{}", help="按实时 inputSchema 提供的 JSON 对象；不是 Token")
    fetch.add_argument("--ask-token", action="store_true", help="在终端隐藏输入 Token")
    inspect = commands.add_parser("inspect", help="读取官方营养工具的实际参数定义")
    inspect.add_argument("--ask-token", action="store_true")
    install = commands.add_parser("install", help="安装自包含 Skill，不修改 MCP 配置")
    install.add_argument("--to", type=Path, default=Path.home() / ".agents" / "skills" / "maimai-fit")
    return cli


def check_new_file(path):
    if path.exists() or path.is_symlink():
        raise PlanningError(f"不覆盖已有路径：{path}")
    if not path.parent.is_dir():
        raise PlanningError(f"输出目录不存在：{path.parent}")


def save_text(path, text):
    # O_EXCL prevents accidentally replacing user data even after preflight.
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        stream.write(text)


def save_json(path, document):
    def decimal_text(value):
        if isinstance(value, Decimal):
            return str(value)
        raise TypeError("Unsupported JSON value")
    save_text(path, json.dumps(document, ensure_ascii=False, indent=2, allow_nan=False, default=decimal_text) + "\n")


def token_for(args):
    if args.ask_token and not sys.stdin.isatty():
        raise McpError("--ask-token 只能在本人交互终端使用；无法保证隐藏输入时拒绝读取凭证。")
    if args.ask_token:
        with warnings.catch_warnings():
            warnings.simplefilter("error", getpass.GetPassWarning)
            try:
                token = getpass.getpass("麦当劳 MCP Token（隐藏输入，不保存）: ")
            except (getpass.GetPassWarning, EOFError):
                raise McpError("隐藏输入不可用或已取消；未读取明文凭证。") from None
    else:
        token = os.environ.get("MCD_MCP_TOKEN", "")
    token = token.strip()
    if not token:
        raise McpError("未设置 MCD_MCP_TOKEN。请先在 https://open.mcd.cn/mcp 获取 Token，再用 --ask-token 在终端隐藏输入；不要发到聊天或写进仓库。")
    return token


def capture(args):
    if args.command == "fetch":
        check_new_file(args.out)
        if args.raw_out is not None:
            check_new_file(args.raw_out)
            if args.raw_out.resolve() == args.out.resolve():
                raise PlanningError("原始返回与规范目录不能使用同一输出路径")
        try:
            arguments = json.loads(args.arguments)
        except json.JSONDecodeError:
            raise PlanningError("--arguments 必须是有效 JSON 对象") from None
        if not isinstance(arguments, dict):
            raise PlanningError("--arguments 必须是 JSON 对象")
    client = McpClient(token_for(args))
    try:
        client.initialize()
        tool = client.nutrition_tool()
        if args.command == "inspect":
            print(json.dumps({"name": tool["name"], "inputSchema": tool.get("inputSchema", {}),
                              "description": tool.get("description", "")}, ensure_ascii=False, indent=2))
            return 0
        result = client.call_nutrition(arguments)
        if args.raw_out is not None:
            save_json(args.raw_out, result)
            print(f"营养工具原始返回已保存：{args.raw_out}", file=sys.stderr)
        diagnostics = {}
        foods = normalize_tool_result(result, diagnostics=diagnostics)
        note = "官方 MCP 工具返回的每份营养数据；并非门店实时菜单。"
        if diagnostics.get("format") == "mcd-toon":
            note += (f"原始 TOON {diagnostics['input_rows']} 行，"
                     f"合并 {diagnostics['duplicate_rows_removed']} 行完全相同条目，保留 {len(foods)} 项；"
                     "有冲突的同名数据仍拒绝。蛋白质 g、钠 mg 与单份口径依据官方营养计算器："
                     "https://www.mcdonalds.com.cn/nutrition_calculator")
        document = validate_catalog({
            "schema_version": 1,
            "source": {"kind": "mcp", "tool": "list-nutrition-foods",
                       "fetched_at": datetime.now(timezone.utc).isoformat(),
                       "note": note},
            "foods": foods,
        })
        save_json(args.out, document)
        print(f"已从官方 MCP 保存 {len(foods)} 项营养数据：{args.out}")
        return 0
    finally:
        client.close()


def pretty(value, unit=""):
    if value is None:
        return "未知"
    result = Decimal(str(value))
    # Presentation only; all inclusion decisions use unrounded Decimal totals.
    return f"{result:.1f}{unit}"


def show_result(result):
    source = result["source"]
    labels = {"demo": "合成演示数据，不能据此点餐", "mcp": "MCP 营养数据（目录文件声明）", "import": "导入数据（来源由文件提供）"}
    print("麦麦刚刚好 · 保留想吃的那一份")
    print(f"数据：{labels[source['kind']]}")
    if source.get("fetched_at"):
        print(f"采集时间：{source['fetched_at']}")
    if source.get("note"):
        print(f"说明：{source['note']}")
    targets = result["constraints"]
    print("保留：" + "、".join(targets["keep"]))
    print(f"条件：{targets['min_kcal']}–{targets['max_kcal']} kcal，蛋白质 ≥ {targets['min_protein_g']} g，最多 {targets['max_items']} 种")
    if targets["max_sodium_mg"] is not None:
        print(f"钠 ≤ {targets['max_sodium_mg']} mg；缺少钠数据的方案不采用")
    if result["status"] == "infeasible":
        print("\n没有符合全部条件的组合。未放宽任何限制、未替换你要求保留的餐品。")
        if result["search"]["unknown_sodium_rejections"]:
            print(f"其中 {result['search']['unknown_sodium_rejections']} 个营养条件匹配的组合因钠数据缺失无法核验。")
        print("你可以明确调整热量区间、蛋白质目标、保留餐品或餐品数量后再计算。")
    else:
        print("\n" + result["ranking"])
        for index, plan in enumerate(result["plans"], 1):
            totals = plan["totals"]
            print(f"\n方案 {index} · 新增 {plan['added_items']} 种")
            for food in plan["foods"]:
                print(f"  • {food['name']}：{pretty(food['kcal'], ' kcal')} / 蛋白质 {pretty(food['protein_g'], ' g')} / 钠 {pretty(food.get('sodium_mg'), ' mg')}")
            print(f"  合计：{pretty(totals['kcal'], ' kcal')} / 蛋白质 {pretty(totals['protein_g'], ' g')} / 钠 {pretty(totals['sodium_mg'], ' mg')}")
    print("\n" + result["notice"])


def install_skill(destination):
    source = ROOT / "skills" / "maimai-fit"
    if not (source / "SKILL.md").is_file():
        raise PlanningError("请从完整项目仓库运行 install，而不是从已安装 Skill 内再次安装")
    destination = destination.expanduser().absolute()
    if destination.exists() or destination.is_symlink():
        raise PlanningError(f"安装目标已存在，未覆盖：{destination}")
    if destination == source or source in destination.parents or destination in ROOT.parents:
        raise PlanningError("安装目标不能位于项目 Skill 内或成为项目的父目录")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir()
    try:
        shutil.copy2(source / "SKILL.md", destination / "SKILL.md")
        scripts = destination / "scripts"
        scripts.mkdir()
        shutil.copy2(source / "scripts" / "run.py", scripts / "run.py")
        runtime = scripts / "runtime"
        runtime.mkdir()
        shutil.copytree(ROOT / "maimai", runtime / "maimai", ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        shutil.copytree(ROOT / "examples", runtime / "examples")
    except OSError:
        shutil.rmtree(destination)
        raise
    print(f"已安装自包含 Skill：{destination}")
    print(f"可独立运行：python3 {destination / 'scripts' / 'run.py'} demo")
    print("客户端的 Skill 发现路径各有不同；如未自动识别，请在客户端导入这个目录。未修改 MCP 配置。")
    return 0


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command in ("fetch", "inspect"):
            return capture(args)
        if args.command == "install":
            return install_skill(args.to)
        if args.html is not None:
            check_new_file(args.html)
        path = ROOT / "examples" / "demo.json" if args.command == "demo" else args.catalog
        catalog = load_catalog(path)
        keep = args.keep or ([DEMO_ANCHOR] if args.command == "demo" else [])
        constraints = Constraints(
            keep=tuple(keep), exclude=tuple(args.exclude),
            min_kcal=number(args.min_kcal, "最低热量"), max_kcal=number(args.max_kcal, "最高热量"),
            min_protein=number(args.min_protein, "最低蛋白质"),
            max_sodium=None if args.max_sodium is None else number(args.max_sodium, "最高钠"),
            max_items=args.max_items, top=args.top,
        )
        result = solve(catalog, constraints)
        if args.html is not None:
            save_text(args.html, render_html(result))
        if args.json:
            print(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False))
        else:
            show_result(result)
        if args.html is not None:
            print(f"HTML 报告：{args.html.absolute()}", file=sys.stderr)
        return 0 if result["status"] == "ok" else 2
    except (CatalogError, PlanningError, McpError) as error:
        print(f"错误：{error}", file=sys.stderr)
        return 1
    except (OSError, UnicodeError) as error:
        print(f"文件或终端操作失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
