# 麦麦刚刚好 · maimai-fit

**先保留想吃的那一份，再按你设定的条件找搭配。**

Rust CLI 与本机网页界面：保留指定餐品，按热量、蛋白质和可选钠上限找搭配，输出逐项营养、总计、JSON 或 HTML。页面交互使用 Rust/WebAssembly。

## 快速开始

需要 Rust stable/Cargo，浏览器须支持 WebAssembly；无需 Python、Node 或前端构建工具。

```sh
git clone https://github.com/iRroxy/maimai-fit.git
cd maimai-fit
cargo build --release --locked
./target/release/maimai serve
```

打开终端打印的地址，默认 `http://127.0.0.1:8765/`；按 `Ctrl+C` 停止。二进制内置界面、演示数据和 Skill，可复制到仓库外使用。

## 在线配餐

1. 本人按[官方指南](https://github.com/M-China/mcd-mcp-server)申请 Token，不在聊天中提供凭证。
2. 页面输入 Token，点击“获取官方营养”。如需必填参数，先“读取参数定义”，按 schema 填写 JSON，再重新输入 Token 获取。
3. 选择精确餐品与规格，设置营养条件，生成搭配或下载 JSON。

页面默认不加载 demo，也可导入目录 JSON，或用 `serve --catalog private-data/catalog.json` 启动。在线失败明确报错，不用旧数据冒充成功。

Token 仅用于本机服务请求官方 MCP，每次请求后清空输入，不写入文件、URL、浏览器存储或日志。**仅在本人设备使用，不要将服务端口暴露到公网。**

## CLI

```sh
# 合成数据演示，不需要 Token
./target/release/maimai demo --json --html report.html

# 本人终端隐藏输入 Token；如有必填参数，按 inspect 结果追加 --arguments JSON
mkdir -p private-data
./target/release/maimai inspect --ask-token
./target/release/maimai fetch --ask-token --out private-data/catalog.json

# 从目录复制包含规格的精确名称；数值仅为演示，请替换为自己的目标
./target/release/maimai plan --catalog private-data/catalog.json \
  --keep '从目录复制的精确餐品名称和规格' \
  --min-kcal 450 --max-kcal 650 --min-protein 30 \
  --max-sodium 1400 --html private-data/report.html
```

`plan` 完全离线；多个保留项重复 `--keep`，无需钠限制时省略 `--max-sodium`。输出文件不覆盖，父目录须已存在；相对路径以当前目录为准。完整参数见 `maimai <子命令> --help`，数据与协议见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。

## 安装 Agent Skill

```sh
./target/release/maimai install
# 或安装到尚不存在的自定义目录
./target/release/maimai install --to /your/skills/maimai-fit
```

默认安装至 `~/.agents/skills/maimai-fit`，包含说明和独立原生程序。按客户端方式启用该目录；约束见 [SKILL.md](skills/maimai-fit/SKILL.md)，MCP 配置参考 [示例](mcp-config.example.json)。

## 使用边界

- 每组 **1–3 个不同餐品，每项一份**，包括全部保留项；不换规格、不放宽条件，无解就说明原因。
- 只读取官方 `list-nutrition-foods`；**不下单、不领券、不消耗积分**。目录不代表实时在售、价格或库存。
- demo 全为合成数据；演示营养目标不是个人推荐，不提供医疗或过敏安全建议。
- 凭证、原始数据、目录和报告按私有文件保管，公开前本人审阅脱敏。

## 开发

```sh
cargo test --workspace --locked
```

修改 `ui/src/` 后重建界面；普通使用无需此步骤：

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cargo run --locked -p xtask -- web
cargo build --release --locked
```

界面 Rust 源码、生成的 `web/pkg/` 与 `web/loader.js` 一起提交；HTML/CSS 位于 `web/`。

参赛：[官方 Issue #707](https://github.com/M-China/mcd-developer-innovation-challenge/issues/707) · [参赛声明](CONTEST_DECLARATION.md) · [联调与规则](MCP_INTEGRATION.md#历史联调与官方依据)。资格和奖项由组织方决定。
