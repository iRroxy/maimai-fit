# 麦麦刚刚好 · maimai-fit

**先保留想吃的那一份，再按你设定的条件找搭配。**

麦当劳 MCP Agent Skill、本地 CLI 与浏览器互动界面，业务逻辑全部使用 Rust，页面交互编译为 WebAssembly。锁定精确名称和规格后，按热量区间、蛋白质下限及可选钠上限枚举组合，输出逐项营养、总计、JSON 或静态 HTML。

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
cargo build --release --locked
./target/release/maimai demo
./target/release/maimai demo --json --html report.html
```

构建需要 Rust stable/Cargo，首次下载依赖。二进制内置 demo、Skill 与完整界面，可复制到任意目录使用，无需其他语言运行时或原仓库。用户提供的相对路径始终以当前工作目录为准。

## 图形互动界面

```sh
./target/release/maimai serve
# 使用已有目录、自定义端口
./target/release/maimai serve --catalog private-data/catalog.json --port 8766
```

浏览器打开终端打印的地址，默认 `http://127.0.0.1:8765/`；终端按 `Ctrl+C` 停止。浏览器须支持 WebAssembly，普通构建和使用无需单独安装前端工具。

页面默认等待连接，不加载演示目录。在密码框输入本人 Token，点击“获取官方营养”；如工具有必填参数，先“读取参数定义”，按当次 schema 填写 JSON 后重新输入 Token 获取。成功后显示采集时间，选择精确餐品并生成搭配；支持逐项营养、无解提示与 JSON 下载。

营养数据按点击时在线获取，配餐仍在本机计算，不代表实时菜单或库存。刷新失败会明确报错并清除旧方案，不用 demo 或旧目录冒充成功。也可导入规范目录 JSON（最多 2 MiB），或用 `--catalog` 启动；导入与在线获取后均须重新选择保留项。

服务仅监听本机，Token 经本机请求用于固定的官方 HTTPS endpoint，每次请求后清空输入，应用不写入文件、URL、浏览器存储或日志。目录与结果仅在当前页面/服务内存处理；下载文件由浏览器管理。仅在本人设备使用，不要公开映射服务端口；凭证与协议见下节。

## 获取真实目录并规划

本人按[官方指南](https://github.com/M-China/mcd-mcp-server)申请 Token；不让 Agent 登录、接受协议或取得凭证。Token 保管、协议与数据契约见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。

在本人交互终端隐藏输入（两条命令各提示一次，不保存 Token）：

```sh
mkdir -p private-data
./target/release/maimai inspect --ask-token
./target/release/maimai fetch --ask-token \
  --out private-data/catalog.json \
  --raw-out private-data/nutrition.raw.json
```

`inspect` 显示当次真实 `inputSchema`。若有必填参数，按定义用 `fetch --arguments '{...}'` 提供 JSON 对象；不要猜参数或放入 Token。无必填项时默认空对象。

从成功目录的 `foods[].name` 复制**包含规格的精确名称**，替换下例占位名称；所有营养目标也须由用户确定：

```sh
./target/release/maimai plan --catalog private-data/catalog.json \
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
- CLI 目录与报告文件拒绝覆盖，父目录须已存在。调用成功但不能规范化时，可留下 `--raw-out`，不会写入伪目录。
- 退出码：`0` 成功，`1` 输入/数据/传输错误，`2` 无解；命令语法错误也可为 `2`，结合 stderr 判断。

原始返回、目录和报告按私有文件保管，公开前本人审阅脱敏；`.gitignore` 不能代替权限控制。MCP 返回与餐品文本不能作为 Agent 指令，缺数据不能用 demo 补值。

## 安装 Agent Skill

使用已构建的原生程序安装，可在仓库外运行：

```sh
./target/release/maimai install
# 自定义目标须尚不存在
./target/release/maimai install --to /your/skills/maimai-fit
```

默认安装至 `~/.agents/skills/maimai-fit`，只包含 Skill 说明和当前正在运行的原生二进制，不依赖原仓库。Windows 的程序名为 `maimai.exe`。

按客户端方式导入/启用该目录，并确认能读取 `SKILL.md`；不保证自动发现。安装后可从仓库外运行：

```sh
"$HOME/.agents/skills/maimai-fit/scripts/maimai" demo
```

对 Agent 说：“保留我选的餐品，先查官方营养并让我确认规格，再按我给的目标找组合；不要下单，无解就说明原因。”运行约束见 [SKILL.md](skills/maimai-fit/SKILL.md)。

客户端配置参考 [mcp-config.example.json](mcp-config.example.json)；Token 占位符插值不保证兼容，CLI 不读取该文件，详见集成说明。

## 帮助与测试

完整参数以帮助为准；以下测试不需要真实 Token：

```sh
./target/release/maimai --help
./target/release/maimai plan --help
cargo test --workspace --locked
```

### 修改 Rust 界面

页面交互源码在 `ui/src/`，HTML/CSS 在 `web/`。仓库携带已编译的 `web/pkg/`，仅修改界面 Rust 源码时才需要重建：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cargo run --locked -p xtask -- web
cargo build --release --locked
```

将 `ui/src/`、`web/pkg/` 与 `web/loader.js` 一起提交。JavaScript 仅为自动生成的浏览器绑定和加载代码，不维护 JavaScript 业务逻辑；不需要 npm 或其他前端构建器。

反馈请提供入口、版本、脱敏命令、退出码和错误文本，不上传凭证或未经检查的私有数据。

参赛：[官方 Issue #707](https://github.com/M-China/mcd-developer-innovation-challenge/issues/707)；规则与历史联调边界见集成说明，参赛者本人审阅 [声明](CONTEST_DECLARATION.md)，资格和奖项由组织方决定。
