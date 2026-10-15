mod web;

use clap::{Args, Parser, Subcommand};
use maimai::catalog::{
    catalog_from_tool_result, load_catalog, parse_strict_json, validate_catalog, CatalogDocument,
    SourceKind,
};
use maimai::mcp::{McpClient, McpError};
use maimai::planner::{parse_number, solve, Constraints, PlanningError, PlanningResult};
use maimai::report::render_html;
use rust_decimal::Decimal;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Parser, Debug)]
#[command(
    name = "maimai",
    about = "麦麦刚刚好：保留想吃的那一份，按你的营养约束配餐"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Debug)]
struct PlanningArgs {
    #[arg(long, help = "必须保留的精确餐品名称，可重复指定不同餐品")]
    keep: Vec<String>,

    #[arg(long, help = "排除名称包含此关键词的餐品，可重复指定")]
    exclude: Vec<String>,

    #[arg(long, default_value = "450", help = "热量下限 kcal（不是健康建议）")]
    min_kcal: String,

    #[arg(long, default_value = "650", help = "热量上限 kcal")]
    max_kcal: String,

    #[arg(long, default_value = "30", help = "蛋白质下限 g")]
    min_protein: String,

    #[arg(long, help = "可选钠上限 mg；未知钠的组合不合格")]
    max_sodium: Option<String>,

    #[arg(long, default_value_t = 3, help = "每种一份，合计最多 1–3 种餐品")]
    max_items: usize,

    #[arg(long, default_value_t = 3, help = "输出 1–10 个方案")]
    top: usize,

    #[arg(long, help = "输出结构化 JSON")]
    json: bool,

    #[arg(long, help = "另存静态 HTML 报告，不覆盖已有文件")]
    html: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "合成数据演示，不请求网络")]
    Demo(PlanningArgs),

    #[command(about = "读取已有营养目录并计算")]
    Plan {
        #[arg(long, help = "营养目录路径")]
        catalog: PathBuf,
        #[command(flatten)]
        args: PlanningArgs,
    },

    #[command(about = "启动本机图形配餐界面，实时获取官方营养数据或导入目录")]
    Serve {
        #[arg(long, help = "可选离线营养目录路径；省略时在页面获取或导入")]
        catalog: Option<PathBuf>,
        #[arg(long, default_value_t = 8765, help = "本机端口；0 为自动分配")]
        port: u16,
    },

    #[command(about = "从官方 MCP 获取营养目录")]
    Fetch {
        #[arg(long, help = "输出目录路径")]
        out: PathBuf,
        #[arg(long, help = "保存工具原始返回，用于未知格式的人工规范化")]
        raw_out: Option<PathBuf>,
        #[arg(long, default_value = "{}", help = "按实时 inputSchema 提供的 JSON 对象；不是 Token")]
        arguments: String,
        #[arg(long, help = "在终端隐藏输入 Token")]
        ask_token: bool,
    },

    #[command(about = "读取官方营养工具的实际参数定义")]
    Inspect {
        #[arg(long, help = "在终端隐藏输入 Token")]
        ask_token: bool,
    },

    #[command(about = "安装自包含 Skill，不修改 MCP 配置")]
    Install {
        #[arg(long, help = "目标安装目录")]
        to: Option<PathBuf>,
    },
}

const DEMO_ANCHOR: &str = "示例鸡肉主食（一份）";
const DEMO_CATALOG: &str = include_str!("../examples/demo.json");
const SKILL: &str = include_str!("../skills/maimai-fit/SKILL.md");

fn output_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn check_new_file(path: &Path) -> Result<(), PlanningError> {
    if path.exists() || path.is_symlink() {
        return Err(PlanningError(format!("不覆盖已有路径：{}", path.display())));
    }
    let parent = output_parent(path);
    if !parent.is_dir() {
        return Err(PlanningError(format!(
            "输出目录不存在：{}",
            parent.display()
        )));
    }
    Ok(())
}

fn output_identity(path: &Path) -> Result<PathBuf, PlanningError> {
    let parent = output_parent(path);
    let parent = parent.canonicalize().map_err(|_| {
        PlanningError(format!("无法解析输出目录：{}", parent.display()))
    })?;
    let name = path
        .file_name()
        .ok_or_else(|| PlanningError(format!("输出路径缺少文件名：{}", path.display())))?;
    Ok(parent.join(name))
}

fn save_text(path: &Path, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

fn save_json(path: &Path, val: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let text = serde_json::to_string_pretty(val)? + "\n";
    save_text(path, &text)
}

fn token_for(ask_token: bool) -> Result<String, McpError> {
    if ask_token && !io::stdin().is_terminal() {
        return Err(McpError(
            "--ask-token 只能在本人交互终端使用；无法保证隐藏输入时拒绝读取凭证。".into(),
        ));
    }

    let token = if ask_token {
        rpassword::prompt_password("麦当劳 MCP Token（隐藏输入，不保存）: ").map_err(|_| {
            McpError("隐藏输入不可用或已取消；未读取明文凭证。".into())
        })?
    } else {
        std::env::var("MCD_MCP_TOKEN").unwrap_or_default()
    };

    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(McpError(
            "未设置 MCD_MCP_TOKEN。请先在 https://open.mcd.cn/mcp 获取 Token，再用 --ask-token 在终端隐藏输入；不要发到聊天或写进仓库。".into(),
        ));
    }

    Ok(trimmed.to_string())
}

fn pretty(val: Option<&str>, unit: &str) -> String {
    match val {
        None => "未知".to_string(),
        Some(s) => match Decimal::from_str(s) {
            Ok(d) => format!("{:.1}{unit}", d.round_dp(1)),
            Err(_) => format!("{s}{unit}"),
        },
    }
}

fn show_result(result: &PlanningResult) {
    let source = &result.source;
    let label = match source.kind {
        SourceKind::Demo => "合成演示数据，不能据此点餐",
        SourceKind::Mcp => "MCP 营养数据（目录文件声明）",
        SourceKind::Import => "导入数据（来源由文件提供）",
    };

    println!("麦麦刚刚好 · 保留想吃的那一份");
    println!("数据：{label}");
    if let Some(ref ts) = source.fetched_at {
        println!("采集时间：{ts}");
    }
    if let Some(ref note) = source.note {
        println!("说明：{note}");
    }

    let targets = &result.constraints;
    let keep_arr = targets["keep"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("、")
        })
        .unwrap_or_default();

    println!("保留：{keep_arr}");

    let min_k = targets["min_kcal"].as_str().unwrap_or("450");
    let max_k = targets["max_kcal"].as_str().unwrap_or("650");
    let min_p = targets["min_protein_g"].as_str().unwrap_or("30");
    let max_i = targets["max_items"].as_u64().unwrap_or(3);
    println!("条件：{min_k}–{max_k} kcal，蛋白质 ≥ {min_p} g，最多 {max_i} 种");

    if let Some(max_s) = targets["max_sodium_mg"].as_str() {
        println!("钠 ≤ {max_s} mg；缺少钠数据的方案不采用");
    }

    if result.status == "infeasible" {
        println!("\n没有符合全部条件的组合。未放宽任何限制、未替换你要求保留的餐品。");
        if result.search.unknown_sodium_rejections > 0 {
            println!(
                "其中 {} 个营养条件匹配的组合因钠数据缺失无法核验。",
                result.search.unknown_sodium_rejections
            );
        }
        println!("你可以明确调整热量区间、蛋白质目标、保留餐品或餐品数量后再计算。");
    } else {
        println!("\n{}", result.ranking);
        for (index, plan) in result.plans.iter().enumerate() {
            let idx = index + 1;
            let totals = &plan.totals;
            println!("\n方案 {idx} · 新增 {} 种", plan.added_items);
            for food in &plan.foods {
                println!(
                    "  • {}：{} / 蛋白质 {} / 钠 {}",
                    food.name,
                    pretty(Some(&food.kcal), " kcal"),
                    pretty(Some(&food.protein_g), " g"),
                    pretty(food.sodium_mg.as_deref(), " mg")
                );
            }
            println!(
                "  合计：{} / 蛋白质 {} / 钠 {}",
                pretty(Some(&totals.kcal), " kcal"),
                pretty(Some(&totals.protein_g), " g"),
                pretty(totals.sodium_mg.as_deref(), " mg")
            );
        }
    }
    println!("\n{}", result.notice);
}

fn execute_plan(
    catalog: &CatalogDocument,
    plan_args: PlanningArgs,
) -> Result<i32, Box<dyn std::error::Error>> {
    if let Some(ref html_path) = plan_args.html {
        check_new_file(html_path)?;
    }

    let constraints = Constraints {
        keep: plan_args.keep,
        exclude: plan_args.exclude,
        min_kcal: parse_number(&plan_args.min_kcal, "最低热量")?,
        max_kcal: parse_number(&plan_args.max_kcal, "最高热量")?,
        min_protein: parse_number(&plan_args.min_protein, "最低蛋白质")?,
        max_sodium: plan_args.max_sodium.as_deref()
            .map(|s| parse_number(s, "最高钠")).transpose()?,
        max_items: plan_args.max_items,
        top: plan_args.top,
    };

    let result = solve(catalog, &constraints)?;

    if let Some(ref html_path) = plan_args.html {
        save_text(html_path, &render_html(&result))?;
    }

    if plan_args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        show_result(&result);
    }

    if let Some(ref html_path) = plan_args.html {
        eprintln!("HTML 报告：{}", html_path.canonicalize().unwrap_or_else(|_| html_path.clone()).display());
    }

    Ok(if result.status == "ok" { 0 } else { 2 })
}

fn execute_fetch(
    out_path: &Path,
    raw_out: Option<&Path>,
    args_json: &str,
    ask_token: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    check_new_file(out_path)?;

    if let Some(raw_path) = raw_out {
        check_new_file(raw_path)?;
        let out_identity = output_identity(out_path)?;
        let raw_identity = output_identity(raw_path)?;
        if raw_identity == out_identity {
            return Err(PlanningError("原始返回与规范目录不能使用同一输出路径".into()).into());
        }
    }

    let args_val = parse_strict_json(args_json).map_err(|_| {
        PlanningError("--arguments 必须是有效 JSON 对象".into())
    })?;

    if !args_val.is_object() {
        return Err(PlanningError("--arguments 必须是 JSON 对象".into()).into());
    }

    let token = token_for(ask_token)?;
    let mut client = McpClient::new(&token, "https://mcp.mcd.cn", 30.0)?;

    let result = client.call_nutrition(&args_val)?;

    save_fetched_catalog(out_path, raw_out, &result)
}

fn save_fetched_catalog(
    out_path: &Path,
    raw_out: Option<&Path>,
    result: &Value,
) -> Result<i32, Box<dyn std::error::Error>> {
    if let Some(raw_path) = raw_out {
        save_json(raw_path, result)?;
        eprintln!("营养工具原始返回已保存：{}", raw_path.display());
    }

    let validated = catalog_from_tool_result(result)?;
    let doc_json = serde_json::to_value(&validated)?;
    save_json(out_path, &doc_json)?;

    println!("已从官方 MCP 保存 {} 项营养数据：{}", validated.foods.len(), out_path.display());
    Ok(0)
}

fn create_install_parents(path: &Path, created: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    create_install_parents(output_parent(path), created)?;
    match fs::create_dir(path) {
        Ok(()) => created.push(path.to_owned()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn execute_install(to_path: Option<&Path>) -> Result<i32, Box<dyn std::error::Error>> {
    let default_dest = dirs_home().join(".agents").join("skills").join("maimai-fit");
    let destination = to_path.unwrap_or(&default_dest);
    if destination.exists() || destination.is_symlink() {
        return Err(PlanningError(format!("安装目标已存在，未覆盖：{}", destination.display())).into());
    }

    let mut executable = fs::File::open(std::env::current_exe()?)?;
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let scripts = destination.join("scripts");
    let binary = scripts.join(format!("maimai{}", std::env::consts::EXE_SUFFIX));
    let install = (|| -> io::Result<()> {
        create_install_parents(output_parent(destination), &mut directories)?;
        // Unlike create_dir_all, this refuses a destination created concurrently.
        fs::create_dir(destination)?;
        directories.push(destination.to_owned());
        fs::create_dir(&scripts)?;
        directories.push(scripts.clone());

        let skill_path = destination.join("SKILL.md");
        let mut skill = OpenOptions::new().write(true).create_new(true).open(&skill_path)?;
        files.push(skill_path);
        skill.write_all(SKILL.as_bytes())?;

        let mut installed = OpenOptions::new().write(true).create_new(true).open(&binary)?;
        files.push(binary.clone());
        io::copy(&mut executable, &mut installed)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            installed.set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    })();
    if let Err(error) = install {
        // Never recursively remove a directory: it may contain someone else's files.
        for path in files.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for path in directories.iter().rev() {
            let _ = fs::remove_dir(path);
        }
        return Err(error.into());
    }

    println!("已安装自包含 Skill：{}", destination.display());
    println!("可独立运行：{} demo", binary.display());
    println!("客户端的 Skill 发现路径各有不同；如未自动识别，请在客户端导入这个目录。未修改 MCP 配置。");
    Ok(0)
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn run(command: Commands) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        Commands::Demo(mut args) => {
            if args.keep.is_empty() {
                args.keep.push(DEMO_ANCHOR.into());
            }
            let catalog = validate_catalog(&parse_strict_json(DEMO_CATALOG)?)?;
            execute_plan(&catalog, args)
        }
        Commands::Plan { catalog, args } => execute_plan(&load_catalog(&catalog)?, args),
        Commands::Serve { catalog, port } => {
            let catalog = catalog.as_deref().map(load_catalog).transpose()?;
            web::serve(catalog, port)?;
            Ok(0)
        }
        Commands::Fetch { out, raw_out, arguments, ask_token } =>
            execute_fetch(&out, raw_out.as_deref(), &arguments, ask_token),
        Commands::Inspect { ask_token } => {
            let token = token_for(ask_token)?;
            let mut client = McpClient::new(&token, "https://mcp.mcd.cn", 30.0)?;
            let tool = client.nutrition_tool()?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                "name": tool.get("name"),
                "inputSchema": tool.get("inputSchema").unwrap_or(&Value::Null),
                "description": tool.get("description").unwrap_or(&Value::Null),
            }))?);
            Ok(0)
        }
        Commands::Install { to } => execute_install(to.as_deref()),
    }
}

fn main() {
    let code = run(Cli::parse().command).unwrap_or_else(|e| {
        eprintln!("错误：{e}");
        1
    });
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fetch_output_keeps_raw_evidence_when_normalization_fails() {
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("catalog.json");
        let raw = directory.path().join("raw.json");
        let payload = parse_strict_json(
            r#"{"content":[{"type":"text","text":"unknown nutrition format"}],"diagnostic":0.12345678901234567890123456789}"#,
        ).unwrap();
        assert!(save_fetched_catalog(&out, Some(&raw), &payload).is_err());
        assert!(!out.exists());
        let saved = parse_strict_json(&fs::read_to_string(&raw).unwrap()).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(saved["diagnostic"].to_string(), "0.12345678901234567890123456789");
    }

    #[test]
    fn fetch_output_normalizes_exact_numbers_and_never_overwrites_raw() {
        let directory = tempfile::tempdir().unwrap();
        let out = directory.path().join("catalog.json");
        let raw = directory.path().join("raw.json");
        let payload = parse_strict_json(
            r#"{"structuredContent":{"foods":[{"name":"Official A","kcal":500.00000000000000000000000000001,"protein_g":30}]}}"#,
        ).unwrap();
        let before = chrono::Utc::now();
        assert_eq!(save_fetched_catalog(&out, Some(&raw), &payload).unwrap(), 0);
        let catalog = load_catalog(&out).unwrap();
        assert_eq!(catalog.foods[0].kcal, "500.00000000000000000000000000001");
        assert_eq!(catalog.foods[0].sodium_mg, None);
        assert_eq!(catalog.source.kind, SourceKind::Mcp);
        let fetched = chrono::DateTime::parse_from_rfc3339(catalog.source.fetched_at.as_ref().unwrap()).unwrap();
        assert!(fetched >= before && fetched <= chrono::Utc::now());
        assert_eq!(parse_strict_json(&fs::read_to_string(&raw).unwrap()).unwrap(), payload);

        let another = directory.path().join("another.json");
        assert!(save_fetched_catalog(&another, Some(&raw), &json!({"content": []})).is_err());
        assert!(!another.exists());
        assert_eq!(parse_strict_json(&fs::read_to_string(&raw).unwrap()).unwrap(), payload);
    }
}
