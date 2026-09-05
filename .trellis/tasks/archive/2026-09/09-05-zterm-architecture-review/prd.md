# 重新审视 zterm 架构与实现简洁性

## Goal

重新检视 zterm 的架构和实际实现，以明确的状态所有权、更少的重复机制和有证据的热路径优化，降低维护成本并保持终端交互正确性。

交付架构与代码审查、故障证据及已验证的实现。2026-09-05 用户确认实施三项架构调整：独立 SessionClient、收拢 UI 状态转移、模型一次捕获 update/checkpoint。客户端 deadline 与内容所有权简化作为这三个边界内已识别的问题一并处理。

## Background

- 审查基线：`fde63f6`，分支 `fix-zterm-herdr-not-synchronized-attachment-snapshot`；开始时工作区干净。
- 产品由 core、proto、platform、terminal、daemon、cli 六个 Rust crate 构成，另有内部发布工具。
- 最近一次变更统一了本地/远端附件路径；终端引擎迁移、选择复制等相关任务已有契约，本任务保持其当前产品行为。
- 审查发现 1 项已复现的响应性缺陷和 5 项有源码依据的简化机会。编号、严重程度、触发条件和全部代码锚点统一保存在 [审查记录](research/architecture-audit.md) 的 F1–F6，不把静态重复工作写成已测得的性能瓶颈。

## Requirements

- R1：从用户输入到 PTY、模型更新、附件传输、前端呈现的完整链路审查职责边界和状态所有权，并覆盖会话生命周期、连接管理、操作重放及资源限制。
- R2：所有缺陷和简化建议提供代码位置、触发条件、影响和验证方式，明确区分已证实问题、静态复杂度结论与待测性能假设。
- R3（F2–F4）：消除同一附件更新的重复完整投影、嵌套候选复制、消费式 protobuf 转换前的深复制，以及运行态无消费者的初始屏幕保留；不以文件数量或行数作为架构质量依据。
- R4：方案保留当前公开命令、wire-v2、host-authoritative terminal、daemon-lifetime Session 和本地/远端一致性契约，除非审查证明某项契约必须调整并单独提出。
- R5：给出按风险和收益排序的最小改造路径、必要测试和回退边界。
- R6：全程由主代理直接执行，不使用子代理，包括 Trellis 定义的子代理。
- R7（F1，P1）：已提交的客户端控制操作必须有有限等待和有界暂存；停止响应的对端不能长期占据命令所有者。部分写失败后不能复用损坏流，且保留已有输入非重放与 mutation ambiguity 语义。
- R8（F5–F6）：源码明确表达 server、同一前端 client 和 UI 状态所有者；重组不增加第二个 Session interpreter，不改变事件入口 ACK、输入 fence 或成功呈现后提交的顺序。

## Acceptance Criteria

- [x] AC1（R1）：有基于当前代码的模块图、关键数据流及单一状态所有者清单。
- [x] AC2（R2）：审查记录逐项包含证据、影响、建议和验证方法，区分实测故障与静态成本。
- [x] AC3（R4）：设计说明保留边界、拟简化机制及兼容性影响。
- [x] AC4（R5）：实施计划具备顺序、验证命令和完成标准，且与问题对应。
- [x] AC5（R2、R6）：现有工作区测试和 fast gate 通过，隔离故障观察器验证 F1；证据限制见 [verification.md](verification.md)。全程未使用子代理。
- [x] AC6（R7）：静默租约、无关帧积累和阻塞写在控制预算内得到明确结果，暂存有数量/字节界限，owner 可释放；普通空闲附件不被误判为超时，部分写后的流不再复用。
- [x] AC7（R3）：附件同步一次完整投影同时返回 update/checkpoint；前端 delta 仅构造一份完整 semantic 候选；三个消费式 protobuf 转换不先 clone；运行态不保留无用途的初始完整屏幕。
- [x] AC8（R8、R4）：client/server/UI owner 及其字段职责清楚，现有 local/remote trace、snapshot ACK、history/copy、失败不提交和 lifecycle 行为不变；最终适用质量门通过。

## Out of Scope

- 新功能、替换终端引擎或网络栈、新增运行平台、发布和部署。
- 缺少失败模型的抽象层、兼容层、监控系统或大规模拆文件。
- Session actor 整体异步化、精确 history eviction、共享 dirty-row 缓存和性能 benchmark；本任务不承诺未经测量的加速比例。

## Implementation Status

2026-09-05：F1–F6 已实施，主代理验收通过。`just check` 通过，47 个标准 harness 汇总为 507 passed / 0 failed / 6 ignored；Herdr 黑盒另行通过。实现与规范已完成；用户回复“确认 走发布流程吧”，工作代码与 specs 已提交为 `0c21738`。归档和 journal 完成后继续正常 PR、CI 与发布流程。
