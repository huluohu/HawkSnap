# M2 Task M4 Report: OCR（RapidOCR + 模型下载器）

## 交付

- `crates/hawksnap-ocr/`（新 crate）：models.rs（下载器/SHA256 门/ORT dylib 保障）、
  preprocess.rs（PP-OCR 预处理）、det.rs（DBNet 后处理）、rec.rs（CTC 解码/标签表）、
  pipeline.rs（ORT 三会话串联：det → cls → rec）。
- app：`app/src-tauri/src/ocr.rs`（`ocr_recognize` / `ocr_export_txt` 命令 + 模型进度
  事件 `hawk://ocr-model-progress` + 引擎常驻缓存）；lib.rs 注册。
- history：`shots_fts` 扩 **ocr_text** 列（原 M0 只索引 path+app），触发器三向同步；
  `PRAGMA user_version` 迁移闸门 1→2（v0/v1 统一：DROP 旧 FTS 对象 → 重建 →
  rebuild 回填 → 置 2）；`set_ocr_text(path, text)` 回填 + UPDATE 触发器即时可检索。
- 编辑器前端：工具栏 OCR 按钮 → 首次运行下载进度横幅（下载 %/校验中文案）→ 右侧
  结果浮层（逐行[分数+文本]点击复制 / 全部复制 / 导出 txt / 关闭）；13 个新 i18n
  key 双语在盘（check:i18n 双向覆盖通过）。

## 模型 URL + 实测 SHA256（裁定 2：实现者实测）

官方源 = RapidAI/RapidOCR **v3.10.0 官方发布通道**（modelscope，rapidocr 自身
`default_models.yaml` 逐字引用的 URL；YAML 内置 SHA256 与实测一致，实现者另行独立
下载复测）。实测于 2026-10-23（本机直连下载 + shasum -a 256）：

| 文件 | URL | 实测 SHA256 |
|---|---|---|
| det | `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv4/det/ch_PP-OCRv4_det_mobile.onnx` | `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9` |
| cls | `…/onnx/PP-OCRv4/cls/ch_ppocr_mobile_v2.0_cls_mobile.onnx` | `e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c` |
| rec | `…/onnx/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile.onnx` | `48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b` |
| 字典 | `…/paddle/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile/ppocr_keys_v1.txt`（6623 行） | `28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7` |

合计 ~16.2MB（计划估 ~12MB；rec 大头 10.9MB）。

ORT dylib（microsoft/onnxruntime **v1.31.0** 官方 GitHub release，与 ort 2.0.0-rc.15
绑定目标对齐）。归档 SHA256 + 解包后库文件 SHA256 全部实测：

| 平台 | 归档 | 归档 SHA256 | 内件 SHA256 |
|---|---|---|---|
| macOS arm64 | `onnxruntime-osx-arm64-1.31.0.tgz`（36.5MB） | `1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc` | `libonnxruntime.1.31.0.dylib` `2a50a63e7f41f8d4d551d3ee47248df3864f6540109fce8ad520ca3b51af4b70` |
| Linux x64 | `onnxruntime-linux-x64-1.31.0.tgz` | `cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57` | `libonnxruntime.so.1.31.0` `469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e` |
| Linux aarch64 | `onnxruntime-linux-aarch64-1.31.0.tgz` | `c5b8b3cca31f3d643a3b313b8f42e2d0f16a4fcefd02cda343d97bd5306afaab` | `libonnxruntime.so.1.31.0` `595c7f7a6775f5c7b3d567c3ed94c04f722092575abcc092560f5208a54daf99` |
| Windows x64 | `onnxruntime-win-x64-1.31.0.zip` | `3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195` | `onnxruntime.dll` `0c42dc4e278f0968fdf9ebf1b1e54a74c657a9942e70803e453c91993451cf4c` |

macOS x64：上游 v1.31.0 **未发布** osx-x64 包 → `ort_spec()` 明确 Err（不做无校验
降级）。下载器：`.tmp` + `Range` 断点续传（206 续写 / 200 重来 / 416 清空重来），
SHA256 不匹配 → 删半成品重下一次 → 仍坏 Err（坏文件绝不落盘）；ORT 归档解包用系统
tar（macOS/Linux `-xzf`、Windows bsdtar `-xf`），解包后对**内件**二次 SHA256 校验，
通过后清理归档。

## ort 策略裁定：load-dynamic

`ort = "=2.0.0-rc.15", default-features = false, features = ["load-dynamic","ndarray"]`
（2.x 系列全部是 rc，`"2"` 不匹配预发布版，必须精确钉 rc.15）。

- **选 load-dynamic**：`download-binaries` 默认链在 build.rs 构建期从 pyke CDN 拉
  ORT 二进制——离线/vendored 构建破裂；load-dynamic 把唯一外联收束到与模型同一
  「官方 release + SHA256」下载通道（运行期，首次 OCR 触发），`ort::init_from(dylib)`
  装载。代价：dylib 归档 36.5MB（任务提示估 ~5MB，实测为准）落盘 models 目录。
- **CI 含义**：构建期零外联（无 pyke 依赖）；集成真模型测试 `#[ignore]`（需网络），
  CI 常规跑不下载。
- ndarray 直接依赖需对齐 ort 内部版本 **0.17**（0.16 会因类型不统一编译失败）。

## 推理流水线与 M2 简化裁定（诚实记录）

1. **det 后处理 = 轴对齐框**（非 DBNet 多边形/min-area-rect）：probmap 阈值 0.3 →
   4 邻接 BFS 连通域 → 外接框 + unclip 外扩（`d = bbox面积 × 1.5 / 连通域边界周长`，
   paddle 轮廓多边形面积的轴对齐近似）→ 行序排序。**局限**：倾斜/环形文本会得到
   偏大框；完整多边形留后续。框分 = 连通域概率均值（非 bbox 均值——稀疏文本不被
   空白稀释），box_thresh 0.5（paddle 默认 0.6 按其多边形口径，此处实测校准）。
2. **det 限长 = rapidocr 实际默认 limit_type=min/736**（短边不足放大，否则原样）。
   任务提示的「resize to 960」长边压缩口径实测不可用：1920×1080 密排文本半分辨率
   后 24 行只检出 11 行；min/736 下原生分辨率 24/24。
3. **rec 宽度动态**：`imgW = round(48 × 宽高比)`（clamp [16,2048]、8 倍数）。
   paddle 的 320 只是批处理初始默认；钉死 320 会把 1000px 长行压成乱码。
4. **cls 门控（aspect < 3 才跑 0/180 校验）**：极宽横条被压到 48×192 后分类器失真，
   实测 24 行中 4 行被误翻转 → 空解码；截图场景不存在整幅倒置文本，纵向/近方形
   裁剪保留校验。
5. 标签表：blank + 字典 6623 + 末位空格 = 6625 类（与 rec 输出实测吻合）。

## FTS ocr_text 索引（任务前置问题的答案）

M0 的 `shots_fts` **未**索引 ocr_text（只有 path+app）。本任务扩为三列 + 触发器 +
迁移闸门 user_version 1→2（v1 旧虚表缺列必须 DROP 重建，CREATE IF NOT EXISTS 不够）。
测试实证：M0 形态旧库升级后 ocr 文本即时可检索；覆盖更新（UPDATE 复合触发器）旧词
失效新词生效；删除级联。

## 计时证据（F6：1080p ≤1.5s；release 构建，真模型）

- `bench_1080p.png`（1920×1080，24 行中英文文本，Helvetica 36px）：
  **1.21s / 1.24s（两次运行），24/24 行，score 0.96-1.00** ✅
- `fixture.png`（900×300，中文+英文两行）：**0.24-0.48s**，3 行
  （"HawkSnap" 1.00 / "OCR 2026" 0.98 / "中文识别测试" 1.00）。
- 模型首装（本次网络实测）：ORT 归档 + 三模型 + 字典全链 ensure_models 正常，
  进度事件流式发出；二次调用零请求短路（测试断言）。

## TDD 与测试矩阵

- 下载器（tiny_http 本地源，11 用例）：新鲜下载/幂等短路（零请求）/SHA 不匹配重试
  一次后 Err 且坏文件不落盘/断点续传（预置半截 .tmp + 206 拼装）/服务端不支持 Range
  降级全量/坏目标重下修复/进度事件流（downloaded 单调 + Verify 事件）/tar 解包链路
  /ort_spec 平台全值性/常量哨兵（URL+SHA 字面值防回退）。
- 预处理（6）：NCHW 平面序/Det ImageNet 统计/limit_type=min 尺寸+补齐/rec 宽度桶/
  RGBA 长度校验。
- det 后处理（6）：双热区双框+unclip/弱响应过滤/单像素噪点/scale 回原图/行序排序/
  畸形输入防御。
- rec（6）：CTC 折叠重复跳 blank/blank 分隔重复保留/连续折叠/标签表空格位/全 blank
  空文本/畸形防御。
- app ocr（3）：export_txt 路径校验+落点/进度载荷 serde/OcrLine `box` 字段形状。
- history（+3）：M0 形态升级 1→2 重索引/set_ocr_text 全生命周期/升级短路。
- 真模型集成 `#[ignore]`（tests/real_models.rs，本地手跑）：全链下载→装载→双语
  fixture 识别断言→1080p 计时守护（断言 24/24 + <1.5s + 首尾行文本正确）。

## 全绿证据

- `cargo test --workspace`：**135 passed / 0 failed / 5 ignored**（ignored = 真模型
  集成 + platform-mac 既有 4 个）。
- `npm test`：110 passed（12 文件）。
- `npm run build`（tsc + vite）：✓；`check:i18n`：OK（A⊆Z、A⊆E 且 Z==E，100/100）。
- `cargo build -p hawksnap`（debug）：✓。

## 备注 / 遗留

- 包体：ORT dylib（44.8MB）+ 模型（16.2MB）均按需下载到 app_data_dir/models，不进
  安装包（≤15MB 约束不含 OCR 模型，spec 明示）；OR 进程内存为 CPU 推理（无 GPU EP）。
- crate 布局按计划保留 `crates/hawksnap-ocr`（app 唯一消费者，但独立 crate 让流水线
  测试不背 tauri 链接；app 测试经工作区依赖正常引用）。
- FTS 迁移对 v1 库每次首开做一次 DROP+rebuild（幂等闸门后短路）；并发首开竞态下
  重复 rebuild 无害（rebuild 以 content 表为权威）。
- fixture（PIL/Helvetica+Hiragino 生成）提交于 `tests/data/`，真模型测试可复现。
