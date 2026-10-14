"""Self-contained, escaped HTML report. No network requests or scripts."""
from decimal import Decimal
from html import escape


def text(value):
    return escape(str(value), quote=True)


def numeric(value):
    return "未知" if value is None else f"{Decimal(str(value)):.1f}"


def render_html(result):
    targets = result["constraints"]
    kept = {name.strip().casefold() for name in targets["keep"]}
    source = result["source"]
    source_names = {"demo": "合成数据演示 · 不可据此点餐", "mcp": "MCP 营养目录 · 来源由文件声明", "import": "导入营养目录 · 来源由文件声明"}
    limits = [f"{targets['min_kcal']}–{targets['max_kcal']} kcal", f"蛋白质 ≥ {targets['min_protein_g']} g", f"最多 {targets['max_items']} 种餐品"]
    if targets["max_sodium_mg"] is not None:
        limits.append(f"钠 ≤ {targets['max_sodium_mg']} mg")
    badges = "".join(f"<span class='limit'>{text(limit)}</span>" for limit in limits)
    cards = []
    for index, plan in enumerate(result["plans"], 1):
        rows = []
        for food in plan["foods"]:
            label = "<span class='kept'>保留</span>" if food["name"].casefold() in kept else "<span class='added'>新增</span>"
            rows.append(f"<tr><th scope='row'>{label}{text(food['name'])}</th><td>{numeric(food['kcal'])}</td><td>{numeric(food['protein_g'])}</td><td>{numeric(food.get('sodium_mg'))}</td></tr>")
        totals = plan["totals"]
        rows.append(f"<tr class='total'><th scope='row'>合计</th><td>{numeric(totals['kcal'])}</td><td>{numeric(totals['protein_g'])}</td><td>{numeric(totals['sodium_mg'])}</td></tr>")
        cards.append(f"""<article class='card'>
        <header class='card-head'><span class='index'>0{index}</span><div><h2>方案 {index}</h2><p>保留你的选择 · 新增 {plan['added_items']} 种餐品</p></div><span class='pass'>满足约束</span></header>
        <div class='table-wrap'><table><caption class='sr-only'>方案 {index} 每份餐品营养及合计</caption><thead><tr><th scope='col'>每种一份</th><th scope='col'>热量<br><small>kcal</small></th><th scope='col'>蛋白质<br><small>g</small></th><th scope='col'>钠<br><small>mg</small></th></tr></thead><tbody>{''.join(rows)}</tbody></table></div>
        </article>""")
    if not cards:
        unknown = result["search"]["unknown_sodium_rejections"]
        missing = f"<p>{unknown} 个热量和蛋白质匹配的组合缺少钠信息，无法核验钠上限。</p>" if unknown else ""
        cards.append(f"<article class='card empty'><h2>这次没有符合全部条件的组合</h2><p>没有放宽限制，也没有替换你要求保留的餐品。</p>{missing}<p>请明确调整热量区间、蛋白质目标、保留餐品或数量后重新计算。</p></article>")
    timestamp = f"<p>采集时间：{text(source['fetched_at'])}</p>" if source.get("fetched_at") else ""
    excluded = f"<p>排除名称含：{text('、'.join(targets['exclude']))}</p>" if targets["exclude"] else ""
    return f"""<!doctype html>
<html lang='zh-CN'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>
<meta name='color-scheme' content='dark light'><meta http-equiv='Content-Security-Policy' content="default-src 'none'; style-src 'unsafe-inline'; img-src 'none'; base-uri 'none'; form-action 'none'">
<title>麦麦刚刚好 · 配餐报告</title>
<style>
:root{{--bg:#111615;--card:#1a2220;--line:#34433d;--ink:#f1f5ed;--muted:#b1bdb5;--accent:#e7c968;--green:#90d0ac}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font-family:system-ui,-apple-system,'Noto Sans CJK SC',sans-serif;font-size:15px;line-height:1.65}}main{{max-width:1040px;margin:auto;padding:44px 30px 36px}}.eyebrow{{letter-spacing:.18em;color:var(--accent);font-size:12px;font-weight:700;text-transform:uppercase}}h1{{font-size:clamp(30px,5vw,48px);line-height:1.2;margin:16px 0 12px;letter-spacing:-.04em}}.intro{{font-size:17px;color:var(--muted);max-width:700px;margin:0 0 24px}}.limits{{display:flex;gap:8px;flex-wrap:wrap;margin:22px 0}}.limit{{border:1px solid var(--line);padding:7px 12px;border-radius:100px;font-variant-numeric:tabular-nums}}.preference{{border-left:3px solid var(--accent);padding:8px 16px;margin-bottom:30px;background:var(--card)}}.preference p{{margin:0}}.preference strong{{color:var(--accent)}}.preference>p+p{{margin-top:5px;font-size:13px;color:var(--muted)}}.cards{{display:grid;gap:18px}}.card{{background:var(--card);border:1px solid var(--line);border-radius:16px;overflow:hidden}}.card-head{{display:flex;gap:14px;align-items:center;padding:20px 24px;border-bottom:1px solid var(--line)}}.index{{color:var(--accent);font-size:30px;line-height:1;font-weight:700;letter-spacing:-.06em}}h2{{margin:0;font-size:19px}}.card-head p{{margin:1px 0 0;color:var(--muted);font-size:13px}}.pass{{margin-left:auto;color:var(--green);font-size:12px;white-space:nowrap}}.table-wrap{{overflow-x:auto}}table{{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}}th,td{{padding:12px 24px;text-align:right;border-bottom:1px solid var(--line);white-space:nowrap}}thead th{{font-size:12px;font-weight:400;color:var(--muted)}}thead small{{font-size:11px}}th:first-child{{text-align:left;white-space:normal;min-width:220px}}tbody th{{font-weight:400}}.kept,.added{{display:inline-block;font-size:10px;margin-right:10px;padding:1px 6px;border-radius:4px;border:1px solid var(--line);white-space:nowrap}}.kept{{color:var(--accent);border-color:#6b5d32}}.added{{color:var(--muted)}}.total th,.total td{{border-bottom:0;font-weight:700;color:var(--green)}}.empty{{padding:26px}}.empty p{{color:var(--muted)}}.method{{font-size:13px;color:var(--muted);margin:24px 0}}footer{{border-top:1px solid var(--line);padding-top:20px;margin-top:26px;font-size:12px;color:var(--muted)}}footer p{{margin:6px 0}}.source{{color:var(--accent);font-weight:600}}.sr-only{{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}}@media(max-width:600px){{main{{padding:24px 16px}}th,td{{padding:10px 12px}}.card-head{{padding:16px}}.pass{{display:none}}}}@media(prefers-color-scheme:light){{:root{{--bg:#f8f7f1;--card:#fff;--line:#d8ddd6;--ink:#20392d;--muted:#57675d;--accent:#796014;--green:#286540}}}}
</style></head><body><main>
<div class='eyebrow'>MAIMAI FIT / 喜好不用让步</div><h1>麦麦刚刚好</h1><p class='intro'>先保留你想吃的，再把剩下的搭配交给计算。每一份方案，都满足你明确设定的条件。</p>
<div class='limits'>{badges}</div><section class='preference' aria-label='你的偏好'><p><strong>一定保留</strong>　{text('、'.join(targets['keep']))}</p>{excluded}<p>默认数值仅为演示设置，不是推荐摄入标准；不保证主食、配菜、饮料的类别结构。</p></section>
<section class='cards' aria-label='配餐结果'>{''.join(cards)}</section>
<p class='method'>排序方式：{text(result['ranking'])}。共检查 {result['search']['examined']} 个组合，{result['search']['feasible']} 个满足全部条件。</p>
<footer><p class='source'>{text(source_names[source['kind']])}</p>{timestamp}<p>{text(source.get('note', ''))}</p><p>“未知”表示缺失数据，不是 0。显示数值四舍五入到一位小数；是否合格以未舍入数据计算。</p><p>{text(result['notice'])}</p><p>非麦当劳官方产品 · 此报告为静态本地文件，不含 Token，不会下单或兑换积分。</p></footer>
</main></body></html>"""
