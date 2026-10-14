# 麦麦刚刚好 · maimai-fit

**先保留你想吃的那一份，再寻找符合你自己设定条件的搭配。**

这是一个原创的麦当劳 MCP Agent Skill 与零依赖 Python 命令行工具。它不把“想吃的餐品”替换成算法更喜欢的选择，而是锁定指定名称、规格的一份餐品，在营养目录中枚举其他不同餐品，输出可行组合及逐项数字。适合已经有个人餐次目标、希望核对组合而不是获取医疗建议的用户。

每个组合总共包含 **1–3 个不同餐品，每个恰好一份**，包括保留项；不是“再额外添加三份”。可同时保留多个不同餐品。不判断主食/小食类别，不支持同一餐品多份、半份或自行替换规格。只满足用户明确给出的热量区间、蛋白质下限，以及可选的钠上限。

> 这是营养数字的决策辅助，不是医疗、减重或过敏安全建议。默认 `450–650 kcal`、蛋白质 `≥30 g` 仅为演示参数，不是个人推荐摄入量。营养目录不代表门店实时在售、价格、库存或优惠；程序不下单、不领券、不消耗积分。

## 当前状态与真实性

- 提供标准库 MCP 客户端、营养目录规范化、固定保留项的组合枚举、静态 HTML 报告，以及可安装的 Skill。
- 实际集成目标只有官方 `https://mcp.mcd.cn` 的 **`list-nutrition-foods`**；先发现真实工具 schema，再调用，不硬编码未知参数。
- `demo` 使用完全虚构的餐品名称和合成营养数据，离线运行；**不得当成麦当劳真实营养信息**。
- 已完成官方服务的已认证端到端验证：发现真实工具 schema、调用 `list-nutrition-foods`、解析 TOON 返回、运行 CLI 配餐并生成浏览器报告。原始 160 行中有 2 行完全相同条目，按公开规则合并为 158 项；详见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。
- 44 个行为测试通过；实际 CLI 验证了真实及合成规划、无解退出码、保留/排除冲突、缺少凭证和拒绝覆盖。更新后的自包含 Skill 在仓库外运行真实目录成功，报告已在浏览器检查。凭证未写入项目；原始返回、目录和报告只保存在被忽略的 `private-data/`。

## 1. 离线体验：无需 Token、无需 pip

要求 Python **3.10 或以上**。取得项目后，在项目根目录运行：

```sh
python3 -m maimai demo
python3 -m maimai demo --keep '示例鸡肉主食（一份）' --min-kcal 450 --max-kcal 650 --min-protein 30 --max-sodium 1400 --top 3
python3 -m maimai demo --html report.html
```

`--html` 生成可直接在浏览器打开的自包含静态报告，包含来源、条件、逐项数字、总计和限制说明。无需 Web 服务或前端依赖，不包含 Token。不要重复使用已有输出路径；保留或自行移走旧文件后再生成。

## 2. 安装 Agent Skill

```sh
python3 -m maimai install
# 或者选择你的客户端要求的 Skill 目录（必须是尚不存在的目标目录）
python3 -m maimai install --to /your/skills/maimai-fit
```

默认目标是 `~/.agents/skills/maimai-fit`。安装器复制 `SKILL.md`、入口脚本与独立运行所需的 `maimai` 模块和演示数据，不依赖原仓库位置；目标已经存在时拒绝覆盖。安装后可直接运行：

```sh
python3 "$HOME/.agents/skills/maimai-fit/scripts/run.py" demo
```

安装目录不等于客户端已加载 Skill。请按客户端的 Skill 导入/启用方式选择该目录，并确认它可读到 `SKILL.md`。**没有验证 Antigravity 或其他客户端会自动发现这个路径**，不作自动发现承诺。Skill 入口见 [skills/maimai-fit/SKILL.md](skills/maimai-fit/SKILL.md)。

可以对已启用 Skill 的 Agent 说：

> 用麦麦刚刚好保留我选择的餐品。先从官方 MCP 查营养目录并让我确认具体规格，再按我提供的热量区间和蛋白质下限找组合。不要下单；没有可行组合就说明原因。

## 3. 获取 Token 与真实营养目录

### 申请与保管

1. 本人访问官方 [MCP 开放平台](https://open.mcd.cn/mcp)，手机号验证登录。
2. 打开“控制台”，点击“激活”；**本人阅读并决定是否同意**服务协议。
3. 申请成功后复制 Token。不要发给聊天 Agent，不要写进命令参数、公开配置、仓库、截图或日志。

官方步骤和协议说明见 [MCP 使用指南](https://github.com/M-China/mcd-mcp-server)。本项目不会替用户登录、接受协议或申请凭证。

### 最简单的安全方式：在本人终端隐藏输入

```sh
python3 -m maimai inspect --ask-token
python3 -m maimai fetch --ask-token --out catalog.json --raw-out nutrition.raw.json
```

`inspect` 输出真实 `list-nutrition-foods` 的 `inputSchema`，不输出 Token。`fetch` 只向官方 endpoint 发送凭证。若工具有必填参数，请按照 **本次 inspect 的 schema** 准备 JSON 对象，通过 `--arguments '{...}'` 提供；不要照抄虚构的工具参数。参数值不应包含 Token。没有必填参数时无需 `--arguments`，使用空对象。

也可以由本人在 Bash 终端为当前会话设置环境变量（隐藏输入，不把 Token 写入命令历史）：

```sh
read -r -s -p 'MCP Token: ' MCD_MCP_TOKEN; printf '\n'
export MCD_MCP_TOKEN
python3 -m maimai inspect
python3 -m maimai fetch --out catalog.json --raw-out nutrition.raw.json
unset MCD_MCP_TOKEN
```

不要在共享终端、开启命令追踪的 shell 或共享运行环境中操作。未使用 `--ask-token` 时，CLI 读取 `MCD_MCP_TOKEN`；显式 `--ask-token` 改为本人交互终端隐藏输入。非交互环境或无法控制回显时拒绝读取，不降级为明文输入。Agent 不应模拟终端输入 Token。

### 让 Agent 直接调用已配置的 MCP

[mcp-config.example.json](mcp-config.example.json) 仅含 `${MCD_MCP_TOKEN}` 占位符，供支持 Streamable HTTP 的客户端参考。**环境变量插值不是所有客户端都支持的通用 JSON 标准**：先核实当前客户端的语法和继承环境方式；不支持时在客户端本地私密凭证设置中配置，不提交含真实 Token 的文件。详见集成文档。

成功 `fetch` 会保存标准目录。`--out`、`--raw-out` 均拒绝覆盖已有路径；失败时不生成伪目录。若工具调用成功而自动规范化不认识返回格式，`--raw-out` 可保留原始营养返回供 Agent 审阅转换，`--out` 不会写入不合格数据。未经本人检查与脱敏，原始返回不要上传或发布。

## 4. 保留真实餐品，再规划

先阅读成功获得的 `catalog.json` 中 `foods[].name`，选择**精确包含规格的名称**。示例命令中名字仅是提示，必须换成当前真实目录中的名称：

```sh
python3 -m maimai plan --catalog catalog.json \
  --keep '从目录复制的精确餐品名称和规格' \
  --min-kcal 450 --max-kcal 650 --min-protein 30 \
  --max-items 3 --top 3 --html report.html
```

可重复 `--keep` 保留多个不同项，可重复 `--exclude '关键词'` 排除名称包含该子串的餐品。CLI 没有过敏原证据，名称排除不能保证过敏安全。可选 `--max-sodium 1400`（单位 mg）；不设置就不约束钠，未知值仍显示未知。`--json` 返回机器可读结果。所有目标数值应由用户确定；请勿自动沿用演示值为个人设置。

### 完整 CLI 契约

| 子命令 | 参数 / 行为 |
|---|---|
| `demo` | 使用合成目录；未给 `--keep` 时保留 `示例鸡肉主食（一份）`；可使用所有规划参数 |
| `plan` | 必须 `--catalog PATH`、至少一个 `--keep EXACT_NAME`；不联网 |
| `inspect` | `[--ask-token]`；发现真实营养工具并显示输入 schema，不抓取目录 |
| `fetch` | 必须 `--out PATH`；可选 `--ask-token`、`--arguments JSON`、`--raw-out PATH`；仅调用营养工具 |
| `install` | `[--to DIR]`；复制自包含 Skill，拒绝覆盖已有目标 |

规划参数：`--keep`、`--exclude` 可重复；`--min-kcal` 默认 450，`--max-kcal` 默认 650，`--min-protein` 默认 30，`--max-sodium` 默认不限制，`--max-items` 默认 3 且范围 1–3，`--top` 默认 3 且范围 1–10，另有 `--json`、`--html PATH`。数值必须有限且非负；热量下限不能超过上限。区间端点包含在内。保留项不能同时被排除，也不能用未知名称。

每项恰好一份，枚举所有允许组合；合格结果按“离热量区间中点最近 → 添加项更少 → 蛋白质更高 → 规范化名称”排序，最多返回 `--top` 条。结果少于三条时不凑数。不放松保留项或任何硬约束。

退出码：`0` 成功，`1` 输入/数据/传输错误，`2` 无可行组合。无解会说明当前条件下没有组合，不给出违反条件的“最佳推荐”；用户可以明确决定是否修改目标或保留项。

开发者可运行行为测试：

```sh
python3 -m unittest discover -s tests -v
```


## 5. 数据契约与未知值

标准 JSON 如下；这是**格式示例而非真实官方营养数据**：

```json
{
  "schema_version": 1,
  "source": {
    "kind": "import",
    "note": "格式示例；合成数值，不用于真实推荐"
  },
  "foods": [
    {"name": "虚构餐品（一份）", "kcal": "300", "protein_g": "20", "sodium_mg": null}
  ]
}
```

- `kcal` 为每份千卡，`protein_g` 为每份克，`sodium_mg` 为每份毫克；数字或十进制字符串均可，内部用 `Decimal` 计算。
- `foods` 非空，名称必须唯一（去首尾空格并忽略大小写后比较），保留份量/规格区别。拒绝负数、布尔值、NaN/Infinity，拒绝缺失能量/蛋白质；不默默跳过错误行。
- 官方 TOON 适配仅识别已核验的九列 schema；所有字段完全相同的重复条目可以合并，并在来源备注记录原始行数和合并数量。同名但任一字段冲突仍整体报错；普通导入目录仍拒绝重复名称。
- 钠未知用 `null`，绝不写成 0。设置钠上限后，任何包含未知钠的组合均不能证明合格，不进入结果。
- 不猜无标签 `energy/calories` 的单位。明确 kJ 可除以 `4.184` 换算 kcal。每 100 g 的数值不能直接充当每份；只有已提供、可信的每份重量/换算依据才能由 Agent 明确转换并记录，CLI 不自动猜份量。
- 实际 MCP 格式未知时，按 Skill 审阅原始数据并生成标准目录；缺少单位/份量证据就停止，而不是用演示数据补齐。`source.kind=mcp` 应包含 `tool=list-nutrition-foods` 和实际 `fetched_at`；手工导入用 `import` 并说明来源。导入文件自报的来源不等于程序验证其官方真实性。

## 6. 参赛准备清单

以官方完整 [activityGuidelines.md](https://github.com/M-China/mcd-developer-innovation-challenge/blob/main/activityGuidelines.md) 为准，同时参考官方 [README](https://github.com/M-China/mcd-developer-innovation-challenge)。官方 README 的文件表未列配置示例，**完整规则明确要求 `mcp-config.example.json`，本项目将它纳入必备项**。

- [x] `README.md`：介绍、安装、示例、目标用户。
- [x] `CONTEST_DECLARATION.md`：已与官方原文逐字节核对，文件名及内容未改；参赛者本人仍需审阅并承担声明责任。
- [x] `MCP_INTEGRATION.md`：实际 Server、Tool、调用流程、业务价值及真实验证记录。
- [x] `mcp-config.example.json`：凭证只使用环境变量占位符；发布前仍应检查版本历史、截图与报告没有敏感信息。
- [x] 可运行源码与 Skill；有效 Token 的官方营养调用、格式适配和真实目录配餐已执行，脱敏记录见集成文档。这是技术调用证据，不代表赛事资格审核通过。
- [ ] 个人公开 GitHub 仓库、原创或合法授权、内容符合完整规则；项目创建时间在北京时间 2025-12-25 00:00 至 2026-10-25 23:59。
- [ ] 本人确认参赛资格、阅读活动/MCP 规则；在北京时间 2026-10-25 23:59 前按官方指定 Issue 模板报名。最终 Star 数据截止北京时间 2026-10-26 00:00；不刷榜、不承诺奖项。

本项目未使用 WorkBuddy、不参加其专项核验，不创建 `workbuddy.md`。专项奖励需要**真实使用 WorkBuddy 且提交真实上下文**，不能只生成文件。本地开发和安装不等于已发布或已报名；上传、接受条款和提交报名均由用户本人决定。

## 文件导航

- `skills/maimai-fit/SKILL.md`：完整 Agent 工作流与安全边界。
- `maimai/`：标准库 CLI、MCP 传输、目录验证、组合枚举与报告。
- `examples/demo.json`：明确标注的合成数据。
- `tests/`：本地自动化用例，不代表官方接口认证验证。
- `MCP_INTEGRATION.md`、`mcp-config.example.json`：真实接入步骤。
- `CONTEST_DECLARATION.md`、`LICENSE`、`CHANGELOG.md`：声明、许可、变更记录。
