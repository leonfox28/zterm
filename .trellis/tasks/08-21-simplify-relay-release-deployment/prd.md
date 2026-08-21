# Simplify Relay release and deployment

## Goal

让 zterm Relay 的发布、部署和验证只保留当前真实需求所要求的机制：版本名称在人和工具之间保持一致，默认服务器通过一个最小 Compose 手动更新无状态 Relay，并在唯一信任边界做一次必要校验。删除为假设性故障添加的重复 digest 校验、回滚演练、未使用 metrics 和多层兜底。

## Background

- Relay 只转发端到端加密流量，不保存 zterm 业务数据、终端内容或会话状态；当前默认部署没有业务数据库或数据卷。
- 用户明确要求所有更新由人手动触发。Compose 不得通过 `pull_policy: always` 或其他机制在普通重启时自动获取新版本。
- 默认服务器已有 OpenResty/Cloudflare TLS，固定反代宿主回环 `127.0.0.1:38451`；没有 Prometheus 或其他组件消费当前 `9090` metrics。
- 现有镜像已经是 scratch runtime、无 shell、以 UID/GID 65532 运行；Compose 又叠加了 command、environment、configs、healthcheck、自研 health probe、只读/tmpfs/capability hardening、stop grace 和显式 json-file 轮转。
- 服务器 Docker 没有全局 `log-driver`/`log-opts`；直接删除当前 Compose 日志限制会回到无界 `json-file`。Docker 官方建议普通场景使用默认自动轮转的 `local` driver，因此最小 Compose 仍需保留 `logging: { driver: local }`。
- 当前发布脚本把 Release `v0.1.0` 转换为镜像 `0.1.0`，并通过自定义 SemVer/parser/output matrix 与部署 digest validator 反复证明同一版本关系。

## Requirements

### R1. One public version spelling

- 根 `Cargo.toml` 继续是产品版本唯一来源，其中版本为 Cargo SemVer（例如 `0.1.1`）。
- GitHub Release tag 和版本化 GHCR tag 使用完全相同的公开拼写（例如 `v0.1.1`）；稳定 release 同时维护 `latest`。
- Release 只需检查 tag 是否精确等于 `v${workspace_version}`。Cargo 负责解析 workspace SemVer；发布逻辑不得再自行实现一套完整 SemVer parser 或去掉 `v`。
- 稳定 release 仍只写 `ghcr.io/leonfox28/zterm-relay`，prerelease/manual 仍只写 `ghcr.io/leonfox28/zterm-relay-dev`；保留必要的 package 隔离和最小 OCI tag 合法性检查，不保留与真实信任边界无关的组合爆炸测试。
- 当前没有消费者验证 workflow provenance/SBOM attestation，因此新构建不再生成或测试这些额外 manifest；保留实际使用的上游 checksum、full-SHA Action pin 和最小 GHCR 权限。
- 现有 `v0.1.0` Release、`:0.1.0` image 和 digest 不修改、不删除；简化后的首次稳定发布使用新版本 `v0.1.1`。

### R2. Minimal default-server deployment

- 默认服务器 Compose 直接声明 `ghcr.io/leonfox28/zterm-relay:latest`，不再使用 `RELAY_IMAGE`、`.env` image indirection、digest validator、`--no-build` 或 digest-only gate。
- 手动更新路径只有 `docker compose pull` 和 `docker compose up -d`；普通 Docker/主机重启继续使用本地已拉取镜像。
- Compose project `name` 固定为 `zterm-relay`，单实例容器 `container_name` 同样固定为 `zterm-relay`。此外仅保留 image、只读 relay config bind mount、宿主回环 `38451:38451`、`restart: unless-stopped`，以及因服务器没有全局轮转而需要的 `logging: driver: local`。
- 镜像提供默认 `CMD` 指向 `/etc/iroh-relay/relay.toml`，从 Compose 删除重复 command。
- 删除未使用的 `RUST_LOG` environment、Compose `configs` 抽象、container healthcheck、自研 healthcheck 构建阶段/二进制、`read_only`、`tmpfs`、`security_opt`、`cap_drop` 和 `stop_grace_period`。scratch + non-root 镜像边界保留。
- 默认服务器没有 metrics consumer，因此配置中关闭 metrics，并删除宿主 `9090` 发布、相关环境变量、测试和验收要求。以后真实引入监控时作为独立需求重新增加。
- 删除从未部署的 direct TLS/ACME/QAD `compose.production.yaml`、专属 env/config smoke 与文档入口。当前自建者也使用反代模式；只有 Phase 1 真实 QAD 数据或实际无反代自建需求出现后，才单独设计 direct 模式。
- GitHub 镜像构建仍在下载官方 Iroh artifact 的信任边界校验一次上游 checksum；Docker/GHCR 自身负责已发布镜像内容寻址，不在部署层重复实现哈希校验。

### R3. Proportionate validation and recovery

- 每次默认服务器发布只执行一次启动后验收：宿主 `/healthz`、公开 HTTP 路径和真实 authenticated Iroh Relay handshake。通过即结束，不再主动切换旧镜像或重复证明同一事实。
- 容器运行态异常时直接 recreate。只有确认新镜像本身存在缺陷时，运维人员才手动选择上一版本 tag；该逃生路径只保留简短文档，不提供自动回滚脚本、不做常规演练、不作为发布 Gate。
- 删除 Phase Zero 和父任务中把固定 digest、metrics、回滚演练、重复 restart/reconnect 当作完成条件的遗留描述，同时保留已经发生过的历史验证记录，不把历史改写成未发生。

### R4. Project-wide simplicity guideline

- 在项目开发规范中新增可执行原则：每项校验、兜底和恢复机制必须对应明确且现实的故障模式；先选择满足契约的最短路径，再按证据增加复杂度。
- 同一不变量默认只在所属信任边界验证一次；跨边界确有不同风险时才允许重复验证，并必须说明新增验证捕获的不同故障。
- 无状态组件默认 replace/recreate；没有持久化数据、不可逆 migration、显著安全风险或真实故障证据时，不增加自动回滚、回滚演练或多层 fallback。
- 测试同时以覆盖关键契约和维护成本为约束，禁止为了“更全面”枚举不会改变行为的组合或在多个层级重复证明同一结果。

### R5. Publication and selected-server migration

- 通过正常 stable Release `v0.1.1` 构建并发布 multi-platform `zterm-relay:v0.1.1` 与 `:latest`，不移动或改写 `v0.1.0`。
- 将 1Panel Compose 根目录中的默认 Relay 切换为仓库中审核后的最小 Compose，移除不再需要的 `.env`/validator/metrics 暴露，并手动 pull/recreate。
- 首次迁移先用旧 Compose 停止并删除无状态的 `zterm-relay-reverse-proxy-relay-1` 及旧 project network，再以新 project/container 名 `zterm-relay` 启动；这是一次命名迁移，不是回滚或重复验收。后续更新不再执行 `down`。
- 迁移只影响 zterm Relay Compose，不修改 OpenResty、Cloudflare、防火墙或其他 1Panel 项目。
- 迁移后只做一次 health + authenticated handshake 验收，并确认仅监听宿主回环 38451、容器使用 `latest`、日志 driver 为 `local`。

## Acceptance Criteria

- [ ] workspace `0.1.1`、GitHub Release `v0.1.1` 与 GHCR `:v0.1.1` 形成直接映射；`:v0.1.1` 和 `:latest` 指向同一 multi-platform image。
- [ ] 发布路径没有去 `v` 转换、自研完整 SemVer parser、digest reference validator 或无人消费的 provenance/SBOM manifest；只保留 workspace/tag 精确相等、stable/dev package 隔离、full-SHA Action pin、上游 checksum 和必要输入合法性检查。
- [ ] 默认反代 Compose 不再包含 image env/digest validator、metrics/9090、command/environment/configs/healthcheck/stop grace/runtime hardening 或自研 health probe，只保留 R2 明确列出的字段；Compose project 与单实例容器均命名为 `zterm-relay`；未使用的 direct TLS/ACME/QAD 模板已删除。
- [ ] 默认服务器完成一次旧 project 到 `zterm-relay` 的无状态命名迁移；此后通过显式 `pull` + `up -d` 更新，普通重启不会自动拉取。运行时只有名为 `zterm-relay` 的 Relay 容器、`127.0.0.1:38451` listener，日志使用自动轮转的 Docker `local` driver。
- [ ] 发布后一次性 `/healthz`、公开 HTTP 和 authenticated Iroh handshake 全部通过；没有执行回滚演练或重复 restart/reconnect 测试。
- [ ] 父任务、Relay 文档、测试与 `.trellis/spec/` 不再把 digest-only、9090 metrics 或回滚演练作为当前默认部署契约，同时准确保留 `v0.1.0` 历史证据。
- [ ] 项目规范包含 R4 的证据驱动简洁性原则，后续 implement/check agent 可以据此拒绝无现实故障模型的复杂度。
- [ ] 本地质量门和 GitHub CI 全绿，仓库与服务器未写入凭据，其他 1Panel 服务未改变。

## Out of Scope

- zterm 终端、daemon、配对、session 或 NAT 直连实现。
- 启用 QAD/UDP、修改防火墙、OpenResty 或 Cloudflare。
- 部署 Prometheus 或其他监控系统。
- 自动更新、自动回滚、迁移持久化数据或修改既有 Release/tag/package 历史。
- 删除服务器上的历史备份、旧镜像或 tar；清理属于单独的破坏性操作。
