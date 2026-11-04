#!/usr/bin/env bash
# scripts/release-readiness.sh —— 1.0 分发就绪门（M4 Task P4）。
#
# 纯静态检查：只读文件 + 跑纯 node（i18n 门禁，无依赖）+ 纯 python/ruby（YAML 语法）。
# **不构建、不联网、不跑 cargo/npm install**——重量级验证（cargo test / clippy /
# npm test+build / tauri build + check-size / bench-* / soak）是 macOS-only 的人工清单，
# 见 docs/superpowers/bench/2026-10-14-m4-release-gate.md。
#
# 用法：bash scripts/release-readiness.sh   （任意 cwd；自动 cd 到仓库根）
# 退出码：0 = 全绿；1 = 有失败项（逐项打印 FAIL 原因）。
set -u
cd "$(dirname "$0")/.." || exit 1

PASS=0
FAIL=0

ok()   { PASS=$((PASS + 1)); printf '  PASS  %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }
skip() { printf '  SKIP  %s\n' "$1"; }

echo "== HawkSnap 1.0 分发就绪门（release readiness gate）=="
echo "repo: $(pwd)"

# ---------- (a) LICENSE 存在 ----------
echo
echo "(a) LICENSE"
if [ -f LICENSE ]; then
  ok "LICENSE 存在（$(wc -c < LICENSE | tr -d ' ') bytes）"
else
  bad "LICENSE 缺失——MIT 分发必须随仓库附带完整许可文本"
fi

# ---------- (b) 版本一致性 ----------
# 期望版本**不再硬编码**：单一权威取 app/src-tauri/tauri.conf.json 的 version
# （scripts/make-latest.mjs 组装 updater feed 时会拿它与 tag 交叉校验，不一致即
# CI 红）。硬编码 1.0.0 让本门禁在 v1.1.x 之后全部失效（2026-11-04 修复）。
#
# 产品版本（app crate + tauri.conf + package.json + package-lock + Cargo.lock 的
# hawksnap）必须 == 期望版本；三个库 crate（hawksnap-core / hawksnap-ocr /
# hawksnap-platform-mac）走**自己的**版本线，对齐 Cargo.toml [workspace.package]。
echo
EXPECT_VERSION=$(sed -nE 's/^[[:space:]]*"version":[[:space:]]*"([^"]+)".*/\1/p' app/src-tauri/tauri.conf.json | head -1)
WORKSPACE_VERSION=$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' Cargo.toml | head -1)
echo "(b) 版本一致性（期望产品版本 ${EXPECT_VERSION}，库 crate 期望 ${WORKSPACE_VERSION}）"
if [ -z "$EXPECT_VERSION" ]; then
  bad "读不到 app/src-tauri/tauri.conf.json 的 version"
  EXPECT_VERSION="__UNKNOWN__"
fi

check_ver() { # file pattern label expected
  if grep -qE "$2" "$1" 2>/dev/null; then
    ok "$3 = $4"
  else
    bad "$3 不是 $4（$1 未匹配 /$2/）"
  fi
}
check_ver app/src-tauri/Cargo.toml      "^version = \"$EXPECT_VERSION\""  "app/src-tauri/Cargo.toml [package]" "$EXPECT_VERSION"
check_ver app/src-tauri/tauri.conf.json "\"version\": \"$EXPECT_VERSION\"" "app/src-tauri/tauri.conf.json" "$EXPECT_VERSION"
check_ver app/package.json              "\"version\": \"$EXPECT_VERSION\"" "app/package.json" "$EXPECT_VERSION"

# (c) app/package-lock.json 根版本（顶层 "version" 与 packages[""].version 两处都必须是期望版本）
if grep -qE "^  \"version\": \"$EXPECT_VERSION\"," app/package-lock.json \
  && grep -qE "^      \"version\": \"$EXPECT_VERSION\"," app/package-lock.json; then
  ok "app/package-lock.json 根版本（顶层 + packages[\"\"]）= $EXPECT_VERSION"
else
  bad "app/package-lock.json 根版本不是 ${EXPECT_VERSION}（需同时改顶层与 packages[\"\"] 两处）"
fi

# Cargo.lock：hawksnap 跟产品版本；三个库 crate 跟 workspace 版本
lock_version() { # crate -> version 或空
  awk -v crate="$1" '
      $1 == "name" && $3 == "\""crate"\"" { seen=1; next }
      seen && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
      seen && $1 == "name" { exit }' Cargo.lock
}
if [ "$(lock_version hawksnap)" = "$EXPECT_VERSION" ]; then
  ok "Cargo.lock: hawksnap = $EXPECT_VERSION"
else
  bad "Cargo.lock: hawksnap 不是 ${EXPECT_VERSION}（手改 name/version 相邻两行）"
fi
for crate in hawksnap-core hawksnap-ocr hawksnap-platform-mac; do
  if [ "$(lock_version "$crate")" = "$WORKSPACE_VERSION" ]; then
    ok "Cargo.lock: $crate = $WORKSPACE_VERSION"
  else
    bad "Cargo.lock: $crate 不是 ${WORKSPACE_VERSION}（手改 name/version 相邻两行）"
  fi
done

# ---------- (d) i18n 门禁（纯 node，只读文件） ----------
echo
echo "(d) i18n key 覆盖率（scripts/check-i18n.mjs，纯 node 无依赖）"
if command -v node >/dev/null 2>&1; then
  if node scripts/check-i18n.mjs; then
    ok "check-i18n 通过（A⊆Z 且 A⊆E 且 Z==E）"
  else
    bad "check-i18n 失败（见上方缺失/孤儿 key 清单）"
  fi
else
  bad "node 不可用——无法跑 i18n 门禁（PENDING：在有 node 的机器执行 scripts/check-i18n.mjs）"
fi

# ---------- (e) workflow YAML 可解析 ----------
echo
echo "(e) workflow YAML 语法"
yaml_files=$(find .github/workflows -maxdepth 1 -name '*.yml' 2>/dev/null)
if [ -z "$yaml_files" ]; then
  bad "找不到 .github/workflows/*.yml"
else
  if python3 -c 'import yaml' >/dev/null 2>&1; then
    if python3 -c 'import yaml,sys; [yaml.safe_load(open(f, encoding="utf-8")) for f in sys.argv[1:]]' $yaml_files; then
      ok "python3+PyYAML 解析通过：$yaml_files"
    else
      bad "workflow YAML 解析失败（见上方报错）"
    fi
  elif command -v ruby >/dev/null 2>&1; then
    if ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' $yaml_files >/dev/null; then
      ok "ruby+psych 解析通过：${yaml_files}（本机无 PyYAML，走 ruby 后备）"
    else
      bad "workflow YAML 解析失败（ruby psych 报错）"
    fi
  else
    bad "本机既无 python3+PyYAML 也无 ruby，无法验证 YAML（安装其一后重跑）"
  fi
fi

# ---------- (f) release job 里有源码归档命令（随 Release 附对应源码） ----------
# workflow 文件名：主工作流是 pipeline.yml（ci.yml 是旧名，保留兼容）——
# 2026-11-04 修复：门禁只查 ci.yml → 文件改名后这项恒红、形同虚设。
echo
echo "(f) 源码包（corresponding source）"
FOUND_ARCHIVE=0
for wf in .github/workflows/pipeline.yml .github/workflows/ci.yml; do
  [ -f "$wf" ] || continue
  if awk '/^  release:/{inrel=1; next} /^  [a-zA-Z-]+:/{inrel=0} inrel' "$wf" \
      | grep -q 'git archive'; then
    FOUND_ARCHIVE=1
    ok "release job 内含 git archive 源码归档步骤（${wf}）"
    break
  fi
done
if [ "$FOUND_ARCHIVE" -eq 0 ]; then
  bad "release job 内未找到 git archive——二进制发布必须附对应源码"
fi

# ---------- (g) plugin/dist 已 gitignore ----------
echo
echo "(g) 插件构建产物不入库"
if git check-ignore -q -- plugin/dist/ 2>/dev/null || grep -qE '^plugin/dist/?$' .gitignore 2>/dev/null; then
  ok "plugin/dist/ 已 gitignore"
else
  bad "plugin/dist/ 未被 gitignore（构建产物不得入库）"
fi

# ---------- (h) 许可证一致性（MIT：依赖树不得引入 copyleft） ----------
# 2026-11-04 修复：原实现是「LICENSE-THIRD-PARTY.md 里不能出现 AGPL 字样」——
# 但那份文件**必须**记录「原 gifski 是 AGPL-3.0、已替换为 gif」，于是恒红、形同
# 虚设（且真有人重新引入 copyleft 依赖时它也不报警）。
#
# 改成真正有意义的把守：
#   1. LICENSE = MIT；
#   2. 第三方声明文件在，且写明了「无 copyleft 组件」的口径；
#   3. **依赖树里真的没有 copyleft 触发源**——Cargo.lock 中不含 gifski，
#      且替代品 gif 在册。谁把 gifski 加回来，这项就红。
echo
echo "(h) 许可证一致性"
H_OK=1
if [ ! -f LICENSE ] || ! grep -q "MIT License" LICENSE; then
  bad "LICENSE 缺失或不是 MIT"
  H_OK=0
fi
if [ ! -f LICENSE-THIRD-PARTY.md ]; then
  bad "LICENSE-THIRD-PARTY.md 缺失"
  H_OK=0
elif ! grep -q "无 copyleft 组件" LICENSE-THIRD-PARTY.md; then
  bad "LICENSE-THIRD-PARTY.md 未声明「无 copyleft 组件」口径"
  H_OK=0
fi
if grep -q 'name = "gifski"' Cargo.lock 2>/dev/null; then
  bad "Cargo.lock 里仍有 copyleft 触发源 gifski（AGPL-3.0）——MIT 分发口径被破坏"
  H_OK=0
fi
if ! grep -q 'name = "gif"' Cargo.lock 2>/dev/null; then
  bad "Cargo.lock 里缺少替代品 gif（MIT/Apache-2.0）"
  H_OK=0
fi
if [ "$H_OK" -eq 1 ]; then
  ok "LICENSE = MIT，第三方声明口径在位，依赖树无 copyleft（gifski → gif）"
fi

# ---------- 汇总 ----------
echo
echo "== 结果：$PASS 项通过，$FAIL 项失败 =="
if [ "$FAIL" -gt 0 ]; then
  echo
  echo "就绪门未通过：修复上方 FAIL 项后重跑。"
  echo "注意：本脚本只覆盖静态项；发布前还须完成 macOS-only 人工清单"
  echo "（cargo test/clippy、npm test/build、tauri build + check-size、bench-*、soak），"
  echo "以及 GitHub 发布前置（repo public、签名 secrets、push tag v1.0.0）——"
  echo "见 docs/superpowers/bench/2026-10-14-m4-release-gate.md。"
  exit 1
fi
echo "静态就绪项全绿。发布前仍须完成 macOS 人工清单与 GitHub 发布前置（见上方路径）。"
exit 0
