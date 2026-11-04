# 构建缓存治理（target/ 有界）

> 构建缓存不该无端占用用户存储。这份文档说明 `target/` 为什么会膨胀、仓库用什么
> 机制把它按住、以及怎么手动介入。

## 一句话

`target/` 是 Cargo 的构建产物目录。**它不进安装包、不进 git、不影响产品体积**，
但它会无声地长到几十 GB —— 本项目曾涨到 **36 GB**（2026-11-04 记录）。

## 为什么会长

Tauri 应用依赖树有 **659 个 crate**（`Cargo.lock` 实测），Rust 的每个 crate 都是
独立编译单元，产物带完整调试符号。四件事叠加：

| 来源 | 实测体量 | 成因 |
| --- | --- | --- |
| `.rcgu.o` 中间文件 | **26.5 GB**（122,805 个） | rustc 每个 codegen unit 吐一个，编译完留在 `deps/` 里**没人删**。当前工具链在 macOS 上仍在产生 |
| `incremental/` 旧代目录 | **12 GB**（438 个） | 本地 crate 每重编一次就多一个目录（`app_lib` 积了 82 个），cargo 不回收 |
| 旧代依赖产物 | **8.8 GB** | cargo 的 metadata hash 含**依赖图哈希链**：改一个传递依赖的 feature，整条下游链的哈希全变 → 新代写盘、**旧代原地留存**（`libtauri-` 有 11 代，rustc / features / lock 版本全都相同） |
| `release/` 的 LTO 中间物 | 5.3 GB | `[profile.release] lto = true, codegen-units = 1` ——「产物更小更快」的代价 |

另有一条**硬地板**：rlib 里最大的那块是 `.rmeta`（`libobjc2_app_kit` 单个 114 MB），
那是泛型 metadata，**任何 profile 设置都改不动**。能砍的只有对象文件里的
`__DWARF`（实测占对象文件 85%）。

## 三层机制

### ① 构建前自动兜底（默认生效，无需记得）

`app/package.json` 的 `pretauri` 钩子：任何 `npm run tauri dev|build` 之前先跑

```bash
python3 ../scripts/clean-target.py --auto 12
```

`target/` **超过 12 GB 才动手**，剪掉旧代产物但**保留活代**（当前编译单元的产物
不动）。没超阈值就安静退出，不刷屏、不误删。

剪代会让下一次构建重编被剪掉的那批 crate —— 但那本来就是这次构建要干的活，
净成本很小。

### ② 发布前硬门（`scripts/release-readiness.sh` 第 (i) 项）

`target/` **超过 20 GB 就 FAIL**，并给出修复命令。防止「放着不管几个月」
的膨胀一路带到发版那天。

### ③ 手动命令

```bash
npm run clean:target           # 干跑：只报计划，不动任何文件
npm run clean:target:apply     # 真删（默认模式：零重编代价）
```

`scripts/clean-target.py` 的直接用法（从仓库根）：

| 参数 | 作用 |
| --- | --- |
| （无） | 干跑，只报计划 |
| `--apply` | 真删 |
| `--auto <GB>` | 阈值模式：超过就自动剪代清理 |
| `--prune-generations` | 额外剪掉同一编译单元的旧代产物（多回收数 GB，代价是下次构建重编那批 crate） |
| `--keep-triples` | 保留交叉编译目录 |

## 清理判据（为什么按「编译单元」而不是包名）

同一个 crate 会同时有 **lib / build-script / 多个 test 单元**存活，各自带
**独立的哈希**。按包名分组会误删活单元 —— 实测这样会让 76 个 crate 被迫重编
（0.2s → 27s）。所以判据是 `(包名, 单元名, 编译模式)` 三元组，单元名从
`.fingerprint/<pkg>-<hash>/` 目录里的文件名读出（`lib-serde` /
`build-script-build-script-build` / `run-build-script-...`）。

同一单元**也可以同时有多代活着**（feature 解析不同 → 哈希不同），所以
「剪代」只保证每个单元保留**最新的一代**，其余视为残留。

排序时间戳用**产物的 mtime**（= 编译时刻），**不能**用 fingerprint 目录的
mtime —— cargo 每次调用都会刷新活单元的 `invoked.timestamp`，全用目录 mtime
会让所有活单元时间相同、排序失效。

## 正常稳态

| 场景 | target/ 体量 |
| --- | --- |
| 刚清完 | ~2 GB（release 缓存） |
| 一次 dev 构建后 | ~3~6 GB |
| 超过 12 GB | 自动兜底触发 |
| 超过 20 GB | 就绪门 FAIL |

## 已做的减量措施

- `[profile.dev.package."*"] debug = 0` —— 依赖不生成调试符号。
  cargo 语义：`*` 只作用于**非 workspace 成员**的依赖，本地四个 crate 仍带
  完整调试信息（调试自己的代码时变量/类型照样可见）。代价：依赖 crate 的
  panic 回溯没有行号。
- `.rcgu.o` 每次编译都回来（`debug = 0` 后会小很多）→ 由机制 ① 兜住。
