use crate::catalog::types::casefold;
use crate::planner::PlanningResult;
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn numeric(value: Option<&str>) -> String {
    match value {
        None => "未知".to_string(),
        Some(s) => match Decimal::from_str(s) {
            Ok(d) => format!("{:.1}", d.round_dp(1)),
            Err(_) => escape_html(s),
        },
    }
}

pub fn render_html(result: &PlanningResult) -> String {
    let targets = &result.constraints;
    let keep_arr = targets["keep"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let kept: HashSet<String> = keep_arr.iter().map(|n| casefold(n.trim())).collect();
    let source = &result.source;

    let source_name = match source.kind {
        crate::catalog::types::SourceKind::Demo => "合成数据演示 · 不可据此点餐",
        crate::catalog::types::SourceKind::Mcp => "MCP 营养目录 · 来源由文件声明",
        crate::catalog::types::SourceKind::Import => "导入营养目录 · 来源由文件声明",
    };

    let min_kcal = targets["min_kcal"].as_str().unwrap_or("450");
    let max_kcal = targets["max_kcal"].as_str().unwrap_or("650");
    let min_protein = targets["min_protein_g"].as_str().unwrap_or("30");
    let max_items = targets["max_items"].as_u64().unwrap_or(3);

    let mut limits = vec![
        format!("{min_kcal}–{max_kcal} kcal"),
        format!("蛋白质 ≥ {min_protein} g"),
        format!("最多 {max_items} 种餐品"),
    ];

    if let Some(s) = targets["max_sodium_mg"].as_str() {
        limits.push(format!("钠 ≤ {s} mg"));
    }

    let badges: String = limits
        .iter()
        .map(|l| format!("<span class='limit'>{}</span>", escape_html(l)))
        .collect();

    let mut cards = Vec::new();
    for (index, plan) in result.plans.iter().enumerate() {
        let idx = index + 1;
        let mut rows = Vec::new();

        for food in &plan.foods {
            let label = if kept.contains(&casefold(food.name.trim())) {
                "<span class='kept'>保留</span>"
            } else {
                "<span class='added'>新增</span>"
            };
            rows.push(format!(
                "<tr><th scope='row'>{}{}</th><td>{}</td><td>{}</td><td>{}</td></tr>",
                label,
                escape_html(&food.name),
                numeric(Some(&food.kcal)),
                numeric(Some(&food.protein_g)),
                numeric(food.sodium_mg.as_deref())
            ));
        }

        let totals = &plan.totals;
        rows.push(format!(
            "<tr class='total'><th scope='row'>合计</th><td>{}</td><td>{}</td><td>{}</td></tr>",
            numeric(Some(&totals.kcal)),
            numeric(Some(&totals.protein_g)),
            numeric(totals.sodium_mg.as_deref())
        ));

        cards.push(format!(
            "<article class='card'>\n        <header class='card-head'><span class='index'>0{}</span><div><h2>方案 {}</h2><p>保留你的选择 · 新增 {} 种餐品</p></div><span class='pass'>满足约束</span></header>\n        <div class='table-wrap'><table><caption class='sr-only'>方案 {} 每份餐品营养及合计</caption><thead><tr><th scope='col'>每种一份</th><th scope='col'>热量<br><small>kcal</small></th><th scope='col'>蛋白质<br><small>g</small></th><th scope='col'>钠<br><small>mg</small></th></tr></thead><tbody>{}</tbody></table></div>\n        </article>",
            idx,
            idx,
            plan.added_items,
            idx,
            rows.join("")
        ));
    }

    if cards.is_empty() {
        let unknown = result.search.unknown_sodium_rejections;
        let missing = if unknown > 0 {
            format!("<p>{unknown} 个热量和蛋白质匹配的组合缺少钠信息，无法核验钠上限。</p>")
        } else {
            String::new()
        };
        cards.push(format!(
            "<article class='card empty'><h2>这次没有符合全部条件的组合</h2><p>没有放宽限制，也没有替换你要求保留的餐品。</p>{}<p>请明确调整热量区间、蛋白质目标、保留餐品或数量后重新计算。</p></article>",
            missing
        ));
    }

    let timestamp = match &source.fetched_at {
        Some(ts) => format!("<p>采集时间：{}</p>", escape_html(ts)),
        None => String::new(),
    };

    let exclude_arr = targets["exclude"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let excluded = if !exclude_arr.is_empty() {
        format!("<p>排除名称含：{}</p>", escape_html(&exclude_arr.join("、")))
    } else {
        String::new()
    };

    let note_str = source.note.as_deref().unwrap_or("");

    format!(
        r#"<!doctype html>
<html lang='zh-CN'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>
<meta name='color-scheme' content='dark light'><meta http-equiv='Content-Security-Policy' content="default-src 'none'; style-src 'unsafe-inline'; img-src 'none'; base-uri 'none'; form-action 'none'">
<title>麦麦刚刚好 · 配餐报告</title>
<style>
:root{{--bg:#111615;--card:#1a2220;--line:#34433d;--ink:#f1f5ed;--muted:#b1bdb5;--accent:#e7c968;--green:#90d0ac}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font-family:system-ui,-apple-system,'Noto Sans CJK SC',sans-serif;font-size:15px;line-height:1.65}}main{{max-width:1040px;margin:auto;padding:44px 30px 36px}}.eyebrow{{letter-spacing:.18em;color:var(--accent);font-size:12px;font-weight:700;text-transform:uppercase}}h1{{font-size:clamp(30px,5vw,48px);line-height:1.2;margin:16px 0 12px;letter-spacing:-.04em}}.intro{{font-size:17px;color:var(--muted);max-width:700px;margin:0 0 24px}}.limits{{display:flex;gap:8px;flex-wrap:wrap;margin:22px 0}}.limit{{border:1px solid var(--line);padding:7px 12px;border-radius:100px;font-variant-numeric:tabular-nums}}.preference{{border-left:3px solid var(--accent);padding:8px 16px;margin-bottom:30px;background:var(--card)}}.preference p{{margin:0}}.preference strong{{color:var(--accent)}}.preference>p+p{{margin-top:5px;font-size:13px;color:var(--muted)}}.cards{{display:grid;gap:18px}}.card{{background:var(--card);border:1px solid var(--line);border-radius:16px;overflow:hidden}}.card-head{{display:flex;gap:14px;align-items:center;padding:20px 24px;border-bottom:1px solid var(--line)}}.index{{color:var(--accent);font-size:30px;line-height:1;font-weight:700;letter-spacing:-.06em}}h2{{margin:0;font-size:19px}}.card-head p{{margin:1px 0 0;color:var(--muted);font-size:13px}}.pass{{margin-left:auto;color:var(--green);font-size:12px;white-space:nowrap}}.table-wrap{{overflow-x:auto}}table{{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}}th,td{{padding:12px 24px;text-align:right;border-bottom:1px solid var(--line);white-space:nowrap}}thead th{{font-size:12px;font-weight:400;color:var(--muted)}}thead small{{font-size:11px}}th:first-child{{text-align:left;white-space:normal;min-width:220px}}tbody th{{font-weight:400}}.kept,.added{{display:inline-block;font-size:10px;margin-right:10px;padding:1px 6px;border-radius:4px;border:1px solid var(--line);white-space:nowrap}}.kept{{color:var(--accent);border-color:#6b5d32}}.added{{color:var(--muted)}}.total th,.total td{{border-bottom:0;font-weight:700;color:var(--green)}}.empty{{padding:26px}}.empty p{{color:var(--muted)}}.method{{font-size:13px;color:var(--muted);margin:24px 0}}footer{{border-top:1px solid var(--line);padding-top:20px;margin-top:26px;font-size:12px;color:var(--muted)}}footer p{{margin:6px 0}}.source{{color:var(--accent);font-weight:600}}.sr-only{{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}}@media(max-width:600px){{main{{padding:24px 16px}}th,td{{padding:10px 12px}}.card-head{{padding:16px}}.pass{{display:none}}}}@media(prefers-color-scheme:light){{:root{{--bg:#f8f7f1;--card:#fff;--line:#d8ddd6;--ink:#20392d;--muted:#57675d;--accent:#796014;--green:#286540}}}}
</style></head><body><main>
<div class='eyebrow'>MAIMAI FIT / 喜好不用让步</div><h1>麦麦刚刚好</h1><p class='intro'>先保留你想吃的，再把剩下的搭配交给计算。每一份方案，都满足你明确设定的条件。</p>
<div class='limits'>{badges}</div><section class='preference' aria-label='你的偏好'><p><strong>一定保留</strong>　{kept_text}</p>{excluded}<p>默认数值仅为演示设置，不是推荐摄入标准；不保证主食、配菜、饮料的类别结构。</p></section>
<section class='cards' aria-label='配餐结果'>{cards_html}</section>
<p class='method'>排序方式：{ranking}。共检查 {examined} 个组合，{feasible} 个满足全部条件。</p>
<footer><p class='source'>{source_title}</p>{timestamp}<p>{note}</p><p>“未知”表示缺失数据，不是 0。显示数值四舍五入到一位小数；是否合格以未舍入数据计算。</p><p>{notice}</p><p>非麦当劳官方产品 · 此报告为静态本地文件，不含 Token，不会下单或兑换积分。</p></footer>
</main></body></html>"#,
        badges = badges,
        kept_text = escape_html(&keep_arr.join("、")),
        excluded = excluded,
        cards_html = cards.join(""),
        ranking = escape_html(&result.ranking),
        examined = result.search.examined,
        feasible = result.search.feasible,
        source_title = escape_html(source_name),
        timestamp = timestamp,
        note = escape_html(note_str),
        notice = escape_html(&result.notice),
    )
}
