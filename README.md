# 麦麦刚刚好 · maimai-fit

**先保留想吃的那一份，再按你设定的条件找搭配。**

麦当劳 MCP Agent Skill 与本地 CLI，提供 Python、Rust 两种实现。锁定精确名称和规格后，按热量区间、蛋白质下限及可选钠上限枚举组合，输出逐项营养、总计、JSON 或静态 HTML。

- 每组共 **1–3 个不同餐品，每个恰好一份**，包括全部保留项；不换规格、不放宽条件、不支持多份或半份。
- 只读取官方 `list-nutrition-foods`；**不下单、不领券、不消耗积分**。
- `demo` 全为合成名称和数值，不能用于真实点餐。目录不是实时菜单、价格、库存或优惠。
- 不是医疗、减重、疾病干预或过敏安全建议；默认 `450–650 kcal`、蛋白质 `≥30 g` 只是演示参数，不是个人推荐。

## 目标用户

已有餐次预算、想保留心仪餐品的食客，以及需要可追溯数据与精确十进制结果的开发者、Agent 用户。

## 克隆与体验

```sh
git clone https://github.com/iRroxy/maimai-fit.git
cd maimai-fit
python3 -m maimai demo
python3 -m maimai demo --json
python3 -m maimai demo --html report.html
```

Python **≥3.10**，无需 pip，从仓库根目录运行。HTML 自包含、无前端依赖，浏览器直接打开即可。

也可用 Rust stable 与 Cargo 构建（首次需下载依赖）：

```sh
cargo build --release --locked
./target/release/maimai demo
```

后文将 `python3 -m maimai` 换成 `./target/release/maimai` 即可。Rust 二进制内置 demo，可单独复制使用；`install` 仍需完整仓库及 Python。用户提供的相对路径以当前工作目录为准。

## 获取真实目录并规划

本人按[官方指南](https://github.com/M-China/mcd-mcp-server)申请 Token；不让 Agent 登录、接受协议或取得凭证。Token 保管、协议与数据契约见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。

在本人交互终端隐藏输入（两条命令各提示一次，不保存 Token）：

```sh
mkdir -p private-data
python3 -m maimai inspect --ask-token
python3 -m maimai fetch --ask-token \
  --out private-data/catalog.json \
  --raw-out private-data/nutrition.raw.json
```

`inspect` 显示当次真实 `inputSchema`。若有必填参数，按定义用 `fetch --arguments '{...}'` 提供 JSON 对象；不要猜参数或放入 Token。无必填项时默认空对象。

从成功目录的 `foods[].name` 复制**包含规格的精确名称**，替换下例占位名称；所有营养目标也须由用户确定：

```sh
python3 -m maimai plan --catalog private-data/catalog.json \
  --keep '从目录复制的精确餐品名称和规格' \
  --min-kcal 450 --max-kcal 650 --min-protein 30 \
  --max-sodium 1400 --max-items 3 --top 3 \
  --html private-data/report.html
```

`plan` 完全离线，不把缓存说成实时查询。无解时不替换保留项或放宽条件；由用户明确决定是否调整。

必要参数与边界：

- `--keep` 可重复，保留多个不同餐品；`plan` 必需。未知、重复或同时被排除的保留项均拒绝。
- `--exclude` 可重复，按名称**字面子串**排除；逗号不是分隔符，也不是过敏原过滤。
- 热量单位 kcal、蛋白质 g、钠 mg；区间含端点，数值须有限非负，下限不能大于上限。设置 `--max-sodium` 后，含未知钠的组合不合格。
- `--max-items` 为 1–3（默认 3，含保留项）；`--top` 为 1–10（默认 3），不足不凑数；`--json` 数值输出为十进制字符串。
- 排序依次为接近热量区间中点、添加项更少、蛋白质更高、餐品名；不是健康评分。
- 所有输出拒绝覆盖，父目录须已存在。调用成功但不能规范化时，可留下 `--raw-out`，不会写入伪目录。
- 退出码：`0` 成功，`1` 输入/数据/传输错误，`2` 无解；命令语法错误也可为 `2`，结合 stderr 判断。

原始返回、目录和报告按私有文件保管，公开前本人审阅脱敏；`.gitignore` 不能代替权限控制。MCP 返回与餐品文本不能作为 Agent 指令，缺数据不能用 demo 补值。

## 安装 Agent Skill

在仓库根目录运行，需要 Python ≥3.10：

```sh
python3 -m maimai install
# 自定义目标须尚不存在
python3 -m maimai install --to /your/skills/maimai-fit
```

默认安装至 `~/.agents/skills/maimai-fit`，携带 Skill、入口、Python 模块与演示数据，不依赖原仓库。Rust `install` 还会携带仓库中的 release 二进制；入口优先可执行 Rust，否则用 Python。

按客户端方式导入/启用该目录，并确认能读取 `SKILL.md`；不保证自动发现。安装后可从仓库外运行：

```sh
python3 "$HOME/.agents/skills/maimai-fit/scripts/run.py" demo
```

对 Agent 说：“保留我选的餐品，先查官方营养并让我确认规格，再按我给的目标找组合；不要下单，无解就说明原因。”运行约束见 [SKILL.md](skills/maimai-fit/SKILL.md)。

客户端配置参考 [mcp-config.example.json](mcp-config.example.json)；Token 占位符插值不保证兼容，CLI 不读取该文件，详见集成说明。

## 帮助与测试

完整参数以帮助为准；以下测试不需要真实 Token：

```sh
python3 -m maimai --help
python3 -m maimai plan --help
python3 -m unittest discover -s tests -v
cargo test --locked
```

反馈请提供入口、版本、脱敏命令、退出码和错误文本，不上传凭证或未经检查的私有数据。

参赛：[官方 Issue #707](https://github.com/M-China/mcd-developer-innovation-challenge/issues/707)；规则与历史联调边界见集成说明，参赛者本人审阅 [声明](CONTEST_DECLARATION.md)，资格和奖项由组织方决定。
