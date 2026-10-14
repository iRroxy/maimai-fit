# 麦麦刚刚好 · maimai-fit

**先保留你想吃的那一份，再寻找符合你自己设定条件的搭配。**

麦当劳 MCP Agent Skill 与本地命令行工具。先锁定指定名称、规格的一份餐品，再按你给出的热量区间、蛋白质下限及可选钠上限枚举组合，输出逐项营养、总计和静态 HTML 报告。提供 Rust 与 Python 两种实现。

- 每个组合总共 **1–3 个不同餐品，每个恰好一份**，包括保留项；支持保留多个不同餐品。
- 不替换保留项，不自动放宽条件，不支持同一餐品多份、半份或自行换规格。
- 只读取官方 MCP 的 `list-nutrition-foods`；**不下单、不领券、不消耗积分**。
- 离线 `demo` 的名称和数值全部为合成数据，**不能作为真实点餐依据**。

> 本工具是营养数字的决策辅助，不是医疗、减重或过敏安全建议。默认 `450–650 kcal`、蛋白质 `≥30 g` 仅为演示参数，不是个人推荐摄入量。营养目录不代表门店实时在售、价格、库存或优惠。

## 目标用户

- 已有个人餐次热量预算、蛋白质目标或钠上限，希望核对麦当劳组合的用户。
- 希望保留心仪餐品、拒绝算法擅自替换规格或放宽条件的食客。
- 需要可追溯数据、精确十进制计算和机器可读结果的开发者与 Agent 用户。
- 非医疗、非疾病干预场景；本工具不提供过敏保障。

## 快速开始

### 1. 获取源码

```sh
git clone https://github.com/iRroxy/maimai-fit.git
cd maimai-fit
```

### 2. 选择运行方式

| 实现 | 前置条件 | 运行入口 | 适用场景 |
|---|---|---|---|
| Python | Python 3.10 或更新版本 | `python3 -m maimai` | 无需编译、无需 pip；从仓库根目录运行 |
| Rust | Rust stable 工具链与 Cargo；首次构建需下载依赖 | `./target/release/maimai` | 编译后使用单个二进制；无需 Python，安装 Skill 除外 |

**无需 Token 的离线体验：**

```sh
python3 -m maimai demo
python3 -m maimai demo --json
python3 -m maimai demo --html report.html
```

使用 Rust 时先在仓库根目录构建，再将后文命令的 `python3 -m maimai` 替换为 `./target/release/maimai`：

```sh
cargo build --release --locked
./target/release/maimai demo
./target/release/maimai demo --html rust-report.html
```

Rust 二进制内置合成演示目录，复制到其他目录后也可运行 `demo`。`plan`、`--html` 等用户提供的相对路径始终相对于**当前工作目录**。`install` 需要完整仓库中的 Skill 与 Python 文件，不能只带二进制安装。

`--html` 生成自包含静态报告，无需 Web 服务或前端依赖，不包含 Token。输出文件已存在时拒绝覆盖；请使用新的路径。浏览器直接打开生成的 HTML 即可。

## 获取真实营养目录

### 申请与保管 Token

1. 本人访问官方 [MCP 开放平台](https://open.mcd.cn/mcp)，手机号验证登录。
2. 打开“控制台”并点击“激活”；**本人阅读并决定是否同意**服务协议。
3. 申请成功后复制 Token。不要发给聊天 Agent，不要写进命令参数、公开配置、仓库、截图或日志。

官方流程见 [MCP 使用指南](https://github.com/M-China/mcd-mcp-server)。本项目不会替用户登录、接受协议或申请凭证。

### 在本人终端隐藏输入

建议将真实数据保存在被忽略的私有目录：

```sh
mkdir -p private-data
python3 -m maimai inspect --ask-token
python3 -m maimai fetch --ask-token \
  --out private-data/catalog.json \
  --raw-out private-data/nutrition.raw.json
```

两次命令各自提示输入 Token，不保存凭证。`inspect` 只显示真实工具的 `inputSchema`；`fetch` 只向官方 `https://mcp.mcd.cn` 发送凭证。若 schema 有必填参数，按**本次返回的定义**准备 JSON 对象，用 `--arguments '{...}'` 提供；不要猜参数，不要在参数中放 Token。没有必填参数时使用默认空对象。

也可以由本人在 **Bash** 终端为当前会话设置环境变量，隐藏输入且不把 Token 写入命令历史：

```sh
read -r -s -p 'MCP Token: ' MCD_MCP_TOKEN; printf '\n'
export MCD_MCP_TOKEN
python3 -m maimai inspect
python3 -m maimai fetch --out private-data/catalog.json
unset MCD_MCP_TOKEN
```

不要在共享终端、开启命令追踪的 shell 或共享运行环境中操作。未使用 `--ask-token` 时读取 `MCD_MCP_TOKEN`；显式指定后改为交互终端隐藏输入。非交互环境或无法控制回显时拒绝读取，不降级为明文输入。Agent 不应模拟输入 Token。

成功 `fetch` 才保存合格目录。`--out`、`--raw-out` 都拒绝覆盖，父目录必须已存在。若调用成功但格式无法规范化，已保存的 `--raw-out` 可供本地审阅转换，`--out` 不写入伪目录。未经本人检查与脱敏，不要上传原始返回、目录或报告；`.gitignore` 不能代替文件权限控制。

## 保留真实餐品并规划

先打开成功获取的目录，在 `foods[].name` 中复制**包含规格的精确名称**。下面的名称是提示，必须替换：

```sh
python3 -m maimai plan --catalog private-data/catalog.json \
  --keep '从目录复制的精确餐品名称和规格' \
  --min-kcal 450 --max-kcal 650 --min-protein 30 \
  --max-sodium 1400 --max-items 3 --top 3 \
  --html private-data/report.html
```

所有目标数值应由用户确定，请勿将示例值自动用作个人设置。`plan` 完全离线，显示的是目录来源，不是实时查询。

### CLI 契约

| 子命令 | 必需项与行为 |
|---|---|
| `demo` | 合成目录；未给 `--keep` 时保留 `示例鸡肉主食（一份）`；支持全部规划参数 |
| `plan` | `--catalog PATH` 和至少一个 `--keep EXACT_NAME`；不联网 |
| `inspect` | 可选 `--ask-token`；发现营养工具并显示输入 schema，不抓取目录 |
| `fetch` | `--out PATH`；可选 `--ask-token`、`--arguments JSON`、`--raw-out PATH` |
| `install` | 可选 `--to DIR`；从仓库复制自包含 Skill，拒绝覆盖已有目标 |

| 规划参数 | 默认值 / 含义 |
|---|---|
| `--keep EXACT_NAME` | 可重复，保留多个不同餐品；`plan` 必需 |
| `--exclude TEXT` | 可重复，按名称子串匹配；逗号是普通字符，不是分隔符 |
| `--min-kcal` / `--max-kcal` | `450` / `650`，单位 kcal，端点包含在内 |
| `--min-protein` | `30`，单位 g |
| `--max-sodium` | 默认不限制，单位 mg；设置后未知钠的组合不合格 |
| `--max-items` | `3`，范围 1–3，包含保留项 |
| `--top` | `3`，范围 1–10；结果不足时不凑数 |
| `--json` | 机器可读 JSON；数值以十进制字符串输出 |
| `--html PATH` | 另存 HTML；不覆盖已有路径 |

数值必须有限且非负，热量下限不能超过上限。保留项不能重复、使用未知名称或同时被排除。排除名称不是过敏原过滤，不能保证过敏安全。

枚举全部允许组合，按“离热量区间中点最近 → 添加项更少 → 蛋白质更高 → 餐品名确定性排序”选取前 `top` 条。不是健康评分，不放宽硬约束。

**退出码：** `0` 成功；`1` 输入、数据或传输错误；`2` 无可行组合。命令语法错误也可能由参数解析器返回 `2`，需结合 stderr 判断。无解不是程序故障；用户可以明确决定是否调整条件。

## 安装 Agent Skill

在仓库根目录运行，要求 Python 3.10 或更新版本：

```sh
python3 -m maimai install
# 或选择客户端要求的、尚不存在的目标目录
python3 -m maimai install --to /your/skills/maimai-fit
```

默认目标为 `~/.agents/skills/maimai-fit`。Python 安装器复制 `SKILL.md`、入口脚本、Python 运行模块与演示数据，不依赖原仓库位置。已构建 Rust 时，也可用 `./target/release/maimai install`；该安装器会携带仓库中的 `target/release/maimai`。入口优先使用可执行的 Rust 二进制，否则运行 Python。

安装后，在仓库外也能执行：

```sh
python3 "$HOME/.agents/skills/maimai-fit/scripts/run.py" demo
```

安装目录不等于客户端已加载 Skill。按客户端的导入/启用方式选择该目录，并确认它能读取 `SKILL.md`；不承诺 Antigravity 或其他客户端自动发现路径。完整入口见 [SKILL.md](skills/maimai-fit/SKILL.md)。

对已启用 Skill 的 Agent 可以说：

> 用麦麦刚刚好保留我选择的餐品。先从官方 MCP 查营养目录并让我确认具体规格，再按我提供的热量区间和蛋白质下限找组合。不要下单；没有可行组合就说明原因。

[mcp-config.example.json](mcp-config.example.json) 只含 `${MCD_MCP_TOKEN}` 占位符，供支持 Streamable HTTP 的客户端参考。环境变量插值不是通用 JSON 标准：先核实客户端语法和环境继承方式；不支持时在本地私密凭证设置中配置。CLI 不读取该配置文件。详见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。

## 数据契约与未知值

下例是**格式示例，不是真实官方营养数据**：

```json
{
  "schema_version": 1,
  "source": {"kind": "import", "note": "格式示例；合成数值，不用于真实推荐"},
  "foods": [
    {"name": "虚构餐品（一份）", "kcal": "300", "protein_g": "20", "sodium_mg": null}
  ]
}
```

- `kcal` 为每份千卡，`protein_g` 为每份克，`sodium_mg` 为每份毫克；输入接受数字或十进制字符串，使用十进制运算而非二进制浮点判定约束。
- `foods` 非空，名称去首尾空格并忽略大小写后必须唯一，保留份量/规格区别。负数、布尔值、NaN/Infinity、缺失能量或蛋白质均拒绝，不默默跳过错误行。
- 未知钠为 `null`，绝不当作 0；有钠上限时，任何含未知钠的组合都不能证明合格。
- 官方 TOON 仅适配已核验的九列 schema。所有字段完全相同的重复记录可合并，来源备注记录数量；同名但字段冲突仍整体失败，普通导入目录仍拒绝重复名称。
- 不猜无标签 `energy/calories` 的单位；明确 kJ 可除以 `4.184` 换算 kcal。每 100 g 数据不能直接当作每份，CLI 不猜份重；Agent 转换必须有可信换算依据并记录来源。
- 未知 MCP 格式按 Skill 审阅原始数据，缺少单位或份量证据就停止，不用 demo 补值。`source.kind=mcp` 应包含 `tool=list-nutrition-foods` 和实际 `fetched_at`；手工导入使用 `import` 并说明来源。文件自报来源不等于官方真实性认证。

## 开发与故障排查

```sh
# 从仓库根目录运行；不需要真实 Token
python3 -m unittest discover -s tests -v
cargo test --locked

# 查看子命令帮助
python3 -m maimai plan --help
./target/release/maimai plan --help
```

Rust 规划器逐个枚举候选，缓存每项十进制营养，只保留前 `top` 个轻量方案，最后构建输出。仍然遍历所有允许组合以保持统计完整；至少保留一项、总项数最多三项时，附加组合数量至多为二次量级。不要把单机测试耗时当作跨环境性能保证。

| 问题 | 处理 |
|---|---|
| `No module named maimai` | Python 模块入口应从仓库根目录运行；安装后使用 `scripts/run.py` |
| `cargo: command not found` | 安装 Rust 工具链，或直接使用无需编译的 Python 入口 |
| 缺少 Token、401/403 | 本人核实凭证与权限；不要将 Token 发给 Agent |
| 已有输出 / 输出目录不存在 | 使用新文件名；先创建父目录，不自动覆盖 |
| 同名餐品、单位不明或返回格式不支持 | 审阅本地数据与原始返回；不删除错误行来绕过校验 |
| 没有合格组合 | 无解退出码为 `2`；由用户明确选择是否调整条件 |
| Skill 未被发现 | 按客户端要求导入安装目录，而不是仅执行安装命令 |

反馈问题时附上运行入口、版本、脱敏命令、退出码与错误文本；不要上传 Token、认证头、未经检查的真实目录或报告。

## 验证边界与参赛信息

仓库保留了此前官方已认证营养联调的脱敏记录：发现工具 schema、获取 TOON、合并完全重复记录、规划并生成报告。原始 160 行合并为 158 项，详见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。这不是每次运行都已重新验证服务可用，也不证明其他业务工具有效。离线测试不代表官方认证验证。

参赛规则以官方完整 [activityGuidelines.md](https://github.com/M-China/mcd-developer-innovation-challenge/blob/main/activityGuidelines.md) 为准。仓库包含介绍、安装、示例、目标用户，以及 [CONTEST_DECLARATION.md](CONTEST_DECLARATION.md)、集成说明和 MCP 配置示例。参赛者本人仍需审阅声明并承担责任。

公开仓库：[iRroxy/maimai-fit](https://github.com/iRroxy/maimai-fit)。报名记录：[官方 Issue #707](https://github.com/M-China/mcd-developer-innovation-challenge/issues/707)；此前官方系统回复“您的作品已成功参赛”，最终资格和奖项由组织方决定。项目未使用 WorkBuddy，不参加其专项核验，不创建虚构上下文。上传、接受条款和报名均由用户本人决定。

## 文件导航

| 路径 | 内容 |
|---|---|
| `src/`、`Cargo.toml`、`Cargo.lock` | Rust CLI、目录规范化、MCP、组合规划与报告 |
| `maimai/` | Python 标准库实现 |
| `skills/maimai-fit/` | Agent 工作流与独立入口脚本 |
| `examples/demo.json` | 明确标注的合成演示数据 |
| `tests/` | Rust 与 Python 本地行为测试 |
| `MCP_INTEGRATION.md`、`mcp-config.example.json` | 官方接入步骤与凭证占位配置 |
| `CHANGELOG.md`、`LICENSE` | 变更记录与 MIT 许可证 |
