# 查询回复执行计划

## Ordered work

- [x] 父设计评审通过后，读取 backend index、terminal-model、terminal-colors、error-handling 和 quality-guidelines；激活本子任务。
- [x] 以旧缺陷补充精确回复测试：未知字段及顺序、已知但无值、默认/resize 后尺寸和超量参数。
- [x] 修改 colors/ingress 最小所有权位置，遵守既有 query budget。
- [x] 更新拥有这些行为的 specs；将实现与规范测试一起评审。

## Validation

`cargo test -p zterm-terminal --all-features`；`sh tests/source-policy.sh`。覆盖 synchronized-output 中查询及时回复及普通查询 corpus。无新的 UI 验证需求；完整宿主质量门禁由父任务最终执行。

无前置功能依赖。完成后进入基础控制/样式子任务。回退详见本任务 design；不得为回退丢弃其他工作区改动。

Implementation evidence (2026-09-24): terminal suite 57 tests passed; source checkout policy passed. New query_protocol covers unknown-field UTF-8 encoding/order/chunking, actual dimensions after resize/during hold, rejected extra parameters and reply overflow.

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
