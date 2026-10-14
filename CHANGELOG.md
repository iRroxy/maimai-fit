# 变更记录

## 未发布

- 精简 README、MCP 与自包含 Skill 说明；合并重复信息，保留安装、示例、数据契约和安全边界。
- 简化 Rust 命令分派与统一错误出口；规划参数直接转移所有权，删除重复分支和不必要复制，不改变命令行为。
- Rust 搜索改为流式枚举、缓存营养，仅构建前 `top` 个结果；保留排序和完整统计。
- 修复相对 HTML 路径、仓库外 Rust demo/Skill、字面子串 `--exclude`；fetch 在凭证前预检路径及参数，拒绝输出别名冲突。
- 本次 Rust 54 项、Python 44 项测试通过；11 个 CLI 差分场景、仓库外 Skill、HTML 和拒绝覆盖已实际验证。未重新调用官方服务。

## 1.0.0

- 提供 Python 标准库与 Rust CLI：只读 MCP 营养接入、严格目录/TOON/Markdown 解析、精确单份组合规划、自包含 Skill 与静态 HTML。
- 支持保留项、排除词、热量区间、蛋白质下限及可选钠上限；无解不放宽条件，凭证不跟随重定向，输出拒绝覆盖。
- 历史官方联调将 160 行完整重复记录合并为 158 项，完成真实配餐与报告；证据边界见 [MCP_INTEGRATION.md](MCP_INTEGRATION.md)。
- 参赛记录：[官方 Issue #707](https://github.com/M-China/mcd-developer-innovation-challenge/issues/707)。
