#!/usr/bin/env python3
"""scripts/clean-target.py — 清掉 Cargo `target/` 里的构建残留（旧代产物）。

## 为什么需要它

Cargo **从不回收** `target/`。三处残留叠加，把工作区撑到几十 GB：

1. **`.rcgu.o` 中间文件**——rustc 每个 codegen unit 吐一个，编译完就留在
   `deps/` 里没人删。Tauri 依赖树几百个 crate × 每个 crate 多个 codegen unit
   → 单次全量编译就是十万级文件。**这是最大的一块**。
2. **旧代依赖产物**——依赖树一变（改 `Cargo.lock`、改 feature、换 rustc），
   cargo 算出新哈希写新文件，**旧的那代原地不动**。实测 `libtauri-` 有 11 个
   不同哈希版本，`libserde-` 同样 11 个；`libobjc2_app_kit` 一个 crate 就躺着
   4 份 180MB 的 rlib。
3. **旧代 `incremental/` 目录**——本地 crate 每重编一次就多一个目录
   （`app_lib` 积了 82 个），cargo 同样不删。

以上都**不是**"不同版本的产品"——是同一份源码在不同时间编译出的中间物。

## 判定"陈旧"的依据

**`Cargo.lock` 是唯一权威**：锁文件里没有的 `name-version` 组合，其产物必然
来自某次过去的依赖状态 → 整代删除。版本的来源是 `deps/*.d`（dep-info 文件）
里的 registry 源码路径：`~/.cargo/registry/src/<hash>/<name>-<version>/`。

**读不出版本的（本地 path 依赖、git 依赖）一律保守保留**——宁可留垃圾，
不可误删当前构建要用的东西。

## 四类清理

| 类别 | 内容 | 安全性 |
|---|---|---|
| `intermediates` | 所有 `deps/**/*.rcgu.o` | 安全：rustc 的增量复用走 `incremental/`，不依赖这些文件；下次编译重新生成 |
| `stale-deps` | `Cargo.lock` 里没有的 `name-version` 对应的整代产物（rlib/rmeta/dylib/d）+ 其 `.fingerprint/` + `build/` 输出 | 安全：锁文件决定当前该用什么版本 |
| `stale-incremental` | 同一 crate 名下**非最新**的 `incremental/<name>-<hash>/` | 安全：增量缓存只影响速度，删了最多重编一次 |
| `stale-bundle` | `bundle/` 里版本号 ≠ 当前版本号的产物（`HawkSnap_0.2.0_*.dmg`） | 安全：bundle 可重新生成 |

另外可选清掉**非本机三元组**的构建目录（`target/<triple>/`，交叉编译残留）。

## 「剪代」为什么是 opt-in

同一编译单元**可以同时有多代活着**：feature 解析不同 → 单元哈希不同，cargo
全部保留、用到哪代算哪代。实测同一个 `serde` 的 lib 单元能有两代都有效
（`cargo check --lib` 与 `--all-targets` 解析出不同的上游 feature 集）。

因此「每个单元只留最新一代」必然牺牲其中一些 → 下次构建要重编那批 crate
（农场实测 0.2s → 27s；**构建本身仍然成功**）。默认**不剪代**，零代价零重编；
`--prune-generations` 才剪。

## 用法

```bash
scripts/clean-target.py            # 干跑：只报计划，不动任何文件（默认）
scripts/clean-target.py --apply    # 真删
scripts/clean-target.py --apply --prune-generations   # 额外剪掉旧代产物
scripts/clean-target.py --apply --keep-triples   # 保留交叉编译目录
```

干跑输出每个类别的文件数与体积；`--apply` 打印实际回收量。

**不碰**：`target/CACHEDIR.TAG`、`target/<profile>/` 顶层的最终二进制、
`Cargo.lock` 之外的任何源文件。脚本**只删 `target/` 里的东西**。
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

# registry 源码路径里的 <name>-<version>：版本号从数字起，名字可含连字符。
# 非贪婪 + 回溯：objc2-app-kit-0.3.0 → name=objc2-app-kit, version=0.3.0
REGISTRY_RE = re.compile(r"registry/src/[^/\s]+/(.+?)-(\d+\.\d+\.\d+[^\s/]*)/")
# 产物文件名：<name>-<hash>（hash 是 16 位十六进制）
ARTIFACT_RE = re.compile(r"^(?P<name>.+?)-(?P<hash>[0-9a-f]{16})$")
# rcgu.o：<name>-<hash>.<name>.<hash2>-cgu.<n>.rcgu.o
RCGU_RE = re.compile(r"^(?P<name>.+?)-(?P<hash>[0-9a-f]{16})\..*\.rcgu\.o$")
# 产物文件名里带版本号：HawkSnap_0.2.0_aarch64.dmg
VERSION_IN_NAME_RE = re.compile(r"(\d+\.\d+\.\d+)")

# 仓库根目录（main 里按参数覆写）
repo_root: Path = Path(__file__).resolve().parent.parent


# ---------------------------------------------------------------------------
# 基础工具
# ---------------------------------------------------------------------------


def human(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(n) < 1024 or unit == "TB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024.0
    return f"{n:.1f} TB"


def dir_size(path: Path) -> int:
    """目录总字节（重复硬链接只算一次，与 du 一致）。"""
    total = 0
    seen: set[tuple[int, int]] = set()
    for root, _dirs, files in os.walk(path, onerror=lambda e: None):
        for name in files:
            try:
                st = os.lstat(os.path.join(root, name))
            except OSError:
                continue
            if st.st_nlink > 1:
                key = (st.st_dev, st.st_ino)
                if key in seen:
                    continue
                seen.add(key)
            total += st.st_size
    return total


def parse_lock(path: Path) -> set[tuple[str, str]]:
    """从 Cargo.lock 取出全部 (name, version) 对。

    只解析 `[[package]]` 段下的 name/version —— Cargo.lock 的格式稳定，
    不值得为此引入 tomllib 依赖（老 python 没有）。
    """
    pairs: set[tuple[str, str]] = set()
    name = version = None
    in_package = False
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if line == "[[package]]":
            if name and version:
                pairs.add((name, version))
            name = version = None
            in_package = True
            continue
        if line.startswith("["):  # 其它段（[metadata] 等）
            if name and version:
                pairs.add((name, version))
            name = version = None
            in_package = False
            continue
        if not in_package:
            continue
        m = re.match(r'^name\s*=\s*"(.+)"$', line)
        if m:
            name = m.group(1)
            continue
        m = re.match(r'^version\s*=\s*"(.+)"$', line)
        if m:
            version = m.group(1)
    if name and version:
        pairs.add((name, version))
    return pairs


def resolve_target_dir(repo: Path) -> Path:
    """用 cargo metadata 权威解析 target 目录（与 check-size.sh 同款口径）。"""
    try:
        out = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=repo,
            capture_output=True,
            text=True,
            timeout=120,
            check=True,
        ).stdout
        import json

        return Path(json.loads(out)["target_directory"])
    except Exception:
        return repo / "target"


def host_triple() -> str:
    try:
        out = subprocess.run(
            ["rustc", "-vV"], capture_output=True, text=True, timeout=30, check=True
        ).stdout
        m = re.search(r"^host:\s*(\S+)$", out, re.M)
        if m:
            return m.group(1)
    except Exception:
        pass
    return ""


def workspace_version(repo: Path) -> str:
    """当前版本号：app/src-tauri/Cargo.toml 的 version。"""
    manifest = repo / "app" / "src-tauri" / "Cargo.toml"
    try:
        for line in manifest.read_text(encoding="utf-8").splitlines():
            m = re.match(r'^version\s*=\s*"(.+)"$', line.strip())
            if m:
                return m.group(1)
    except OSError:
        pass
    return ""


# ---------------------------------------------------------------------------
# 清理计划
# ---------------------------------------------------------------------------


class Plan:
    """收集待删条目，按类别归组，并统计体积。"""

    def __init__(self) -> None:
        self.files: dict[str, list[Path]] = defaultdict(list)
        self.dirs: dict[str, list[Path]] = defaultdict(list)
        self.bytes: dict[str, int] = defaultdict(int)

    def add_file(self, category: str, path: Path, size: int) -> None:
        self.files[category].append(path)
        self.bytes[category] += size

    def add_dir(self, category: str, path: Path) -> None:
        self.dirs[category].append(path)
        self.bytes[category] += dir_size(path)

    def total_files(self) -> int:
        return sum(len(v) for v in self.files.values())

    def total_dirs(self) -> int:
        return sum(len(v) for v in self.dirs.values())


def dep_version_from_dfile(path: Path) -> tuple[str, str] | None:
    """从 dep-info 文件里取出 (name, version)。

    文件内容形如：
        /path/.cargo/registry/src/<hash>/serde-1.0.219/src/lib.rs /path/...
    只读前几 KB——路径都在开头。
    """
    try:
        with path.open("r", encoding="utf-8", errors="replace") as fh:
            head = fh.read(8192)
    except OSError:
        return None
    m = REGISTRY_RE.search(head)
    if not m:
        return None
    return m.group(1), m.group(2)


def read_unit_name(fp_dir: Path) -> str:
    """从 fingerprint 目录里读出**编译单元**的名字。

    cargo 的单元身份写在这个目录里的文件名上：
        lib-serde                        → serde 的 lib 单元
        build-script-build-script-build  → serde 的构建脚本单元
        run-build-script-build-script-build → 构建脚本的「运行」单元
        test-integration-test-foo        → 集成测试单元

    **单元才是清理的正确原子**——同一个包可以同时有 lib / build-script / 多个
    test 单元，各自的哈希都是活的。早期版本按「包名」分组，把这些单元当成
    「同一单元的不同代」，误删了活的 lib 单元 → cargo 被迫重编 222 个 crate。
    """
    try:
        names = os.listdir(fp_dir)
    except OSError:
        return ""
    cands = [
        n
        for n in names
        if n != "invoked.timestamp"
        and not n.startswith("dep-")
        and not n.startswith("output-")
        and not n.endswith(".json")
    ]
    if not cands:
        return ""
    for c in sorted(cands):
        if c.startswith("lib-"):
            return c
    return sorted(cands)[0]


def scan_profile(
    profile_dir: Path,
    lock: set[tuple[str, str]],
    plan: Plan,
    prune_generations: bool = False,
) -> None:
    """扫描一个 profile 目录（target/debug 或 target/release）。"""
    deps = profile_dir / "deps"
    fingerprints = profile_dir / ".fingerprint"
    build_root = profile_dir / "build"

    # —— 类别 1 + 2：.rcgu.o 中间文件 与 陈旧单元产物 ——
    #
    # 关联键是**哈希**：deps 里的 `.d` 叫 `adler2-<hash>.d`，对应的 rlib 叫
    # `libadler2-<hash>.rlib`（差一个 lib 前缀），rcgu.o 更是
    # `adler2-<hash>.adler2.<hash2>-cgu.N.rcgu.o`。三者只有 `<hash>` 共通，
    # 且哈希全局唯一（一个编译单元一个）。
    #
    # **「陈旧」的判据不是版本号**：实测同一 crate 的 11 代产物 rustc 版本相同、
    # features 相同、Cargo.lock 版本也相同——差异来自**依赖图的哈希链**：
    # 改一个传递依赖的版本或 feature，整条下游链的 metadata hash 全变，新代
    # 写盘、旧代原地留存。cargo 从不回收旧代。
    #
    # 因此代际存活性按「新旧」判定：**同一个编译单元**（包名 + 单元名 + 编译
    # 模式）的多代里，只保留 mtime 最新的那一代，其余皆为残留。
    #
    # 时间戳取**产物的 mtime**（= 编译时刻），**不能**取 fingerprint 目录的
    # mtime：cargo 每次调用都会刷新活单元的 `invoked.timestamp`，全用目录
    # mtime 会让所有活单元时间相同，排序失去意义。
    if deps.is_dir():
        gen_files: dict[str, list[Path]] = defaultdict(list)
        gen_mtime: dict[str, float] = {}

        for entry in deps.iterdir():
            if not entry.is_file():
                continue
            name = entry.name
            # `.rcgu.o` 是 codegen 中间物，**全部**要清——不需要知道它属于哪一代，
            # 所以在解析哈希之前就按后缀拦下来。
            #
            # 它的命名有两种形态，别指望一个正则吃全：
            #   依赖 crate：<crate>-<16hex>.<crate>.<16hex>-cgu.<N>.rcgu.o
            #   本地 crate：<crate>.<base36>.<base36>.rcgu.o     ← 没有 -<16hex> 段
            # 早先只按 16 位十六进制匹配，漏掉了 42,753 个（本地 crate 的那批）。
            if name.endswith(".rcgu.o"):
                try:
                    plan.add_file("intermediates", entry, entry.stat().st_size)
                except OSError:
                    pass
                continue
            stem = name.rsplit(".", 1)[0] if "." in name else name
            m = ARTIFACT_RE.match(stem)
            if not m:
                continue
            try:
                mt = entry.stat().st_mtime
            except OSError:
                continue
            gen_files[m.group("hash")].append(entry)
            gen_mtime[m.group("hash")] = max(gen_mtime.get(m.group("hash"), 0.0), mt)

        # 每代的编译模式：有 rlib/dylib/so = build，只有 rmeta = check
        mode_of: dict[str, str] = {
            h: (
                "build"
                if any(f.name.endswith((".rlib", ".dylib", ".so")) for f in files)
                else "check"
            )
            for h, files in gen_files.items()
        }

        # 附加信号：不在 Cargo.lock 里的版本 → 铁定陈旧
        version_stale: set[str] = set()
        for h, files in gen_files.items():
            for f in files:
                if f.name.endswith(".d"):
                    got = dep_version_from_dfile(f)
                    if got is not None and got not in lock:
                        version_stale.add(h)
                    break

        # 按**编译单元**分组：(包名, 单元名, 编译模式)
        #
        # 【为什么剪代是 opt-in】同一编译单元**可以同时有多代活着**：feature 解析
        # 不同 → 单元哈希不同，cargo 全部保留、用到哪代算哪代。实测同一个 serde
        # 的 lib 单元能有两代都有效（`cargo check --lib` 与 `--all-targets` 解析出
        # 不同的上游 feature 集）。因此「只留最新一代」必然牺牲其中一些 → 下次构建
        # 要重编那批 crate（农场实测 0.2s → 27s，构建本身仍然成功）。
        #
        # 默认**不剪代**（零代价、零重编）；`--prune-generations` 才剪。
        groups: dict[tuple[str, str, str], list[tuple[float, str, Path]]] = defaultdict(list)
        if fingerprints.is_dir():
            for fp in fingerprints.iterdir():
                if not fp.is_dir():
                    continue
                m = ARTIFACT_RE.match(fp.name)
                if not m:
                    continue
                pkg, h = m.group("name"), m.group("hash")
                unit = read_unit_name(fp)
                mt = gen_mtime.get(h)
                if mt is None:
                    # 该单元没在 deps/ 留产物（如 run-build-script）→ 用目录里
                    # 除 invoked.timestamp 外的最新 mtime
                    mt = 0.0
                    try:
                        for child in fp.iterdir():
                            if child.name == "invoked.timestamp":
                                continue
                            mt = max(mt, child.stat().st_mtime)
                    except OSError:
                        continue
                groups[(pkg, unit, mode_of.get(h, "check"))].append((mt, h, fp))

        stale_hashes: set[str] = set(version_stale)
        if prune_generations:
            for _key, items in groups.items():
                if len(items) < 2:
                    continue
                items.sort(key=lambda it: it[0], reverse=True)
                for _mt, h, fp in items[1:]:
                    stale_hashes.add(h)
                    plan.add_dir("stale-deps", fp)
        elif fingerprints.is_dir():
            # 不剪代，但版本已不在 Cargo.lock 的那几代照删（它们的 fingerprint
            # 目录留着只会被 cargo 当垃圾扫）
            for fp in fingerprints.iterdir():
                if not fp.is_dir():
                    continue
                m = ARTIFACT_RE.match(fp.name)
                if m and m.group("hash") in version_stale:
                    plan.add_dir("stale-deps", fp)

        # 落盘分类：deps/ 里同哈希的产物（.rcgu.o 已在上面按后缀单独归类）
        for h, files in gen_files.items():
            for f in files:
                if h not in stale_hashes:
                    continue
                try:
                    size = f.stat().st_size
                except OSError:
                    continue
                plan.add_file("stale-deps", f, size)

        # build/ 里构建脚本的输出目录，同哈希的也是这一代的
        if build_root.is_dir() and stale_hashes:
            for entry in build_root.iterdir():
                if not entry.is_dir():
                    continue
                m = ARTIFACT_RE.match(entry.name)
                if m and m.group("hash") in stale_hashes:
                    plan.add_dir("stale-deps", entry)

    # —— 类别 3：旧代 incremental 目录 ——
    # 同一 crate 名下只保留**最新**的那个（mtime 最新）；其余删掉。
    # 增量缓存只影响编译速度，删掉最多让某个 crate 重编一次。
    incremental = profile_dir / "incremental"
    if incremental.is_dir():
        by_name: dict[str, list[Path]] = defaultdict(list)
        for entry in incremental.iterdir():
            if not entry.is_dir():
                continue
            stem = entry.name
            cut = stem.rfind("-")
            if cut <= 0:
                continue
            by_name[stem[:cut]].append(entry)
        for _name, dirs in by_name.items():
            if len(dirs) < 2:
                continue
            try:
                dirs.sort(key=lambda p: p.stat().st_mtime, reverse=True)
            except OSError:
                continue
            for stale in dirs[1:]:
                plan.add_dir("stale-incremental", stale)

    # —— 类别 4：bundle 里版本号过时的产物 ——
    bundle = profile_dir / "bundle"
    current = workspace_version(repo_root)
    if bundle.is_dir() and current:
        for root, _dirs, files in os.walk(bundle):
            for name in files:
                m = VERSION_IN_NAME_RE.search(name)
                if not m:
                    continue  # 文件名里没有版本号（icon.icns / bundle_dmg.sh）→ 保留
                if m.group(1) == current:
                    continue
                p = Path(root) / name
                try:
                    plan.add_file("stale-bundle", p, p.stat().st_size)
                except OSError:
                    pass


def scan_triples(target_dir: Path, host: str, plan: Plan) -> None:
    """清掉非本机三元组的构建目录（交叉编译残留）。"""
    if not host or not target_dir.is_dir():
        return
    for entry in target_dir.iterdir():
        if not entry.is_dir():
            continue
        if entry.name in ("debug", "release", "tmp"):
            continue
        # 三元组目录形如 <triple>/{debug,release}
        if not any((entry / p).is_dir() for p in ("debug", "release")):
            continue
        if entry.name == host:
            continue
        plan.add_dir("stale-triples", entry)


# ---------------------------------------------------------------------------
# 执行
# ---------------------------------------------------------------------------


def apply_plan(plan: Plan) -> None:
    for category, files in plan.files.items():
        for path in files:
            try:
                path.unlink()
            except OSError:
                pass
        print(f"  ✓ {category}: 删除 {len(files)} 个文件")
    for category, dirs in plan.dirs.items():
        for path in dirs:
            shutil.rmtree(path, ignore_errors=True)
        print(f"  ✓ {category}: 删除 {len(dirs)} 个目录")


def build_in_progress(repo: Path, target_dir: Path) -> list[str]:
    """返回**正在本仓库范围内**运行的 cargo/rustc 命令行。

    判据是路径而非进程名：别的目录的构建（比如另一个工作副本）不该拦住这里
    的清理——曾经被一个跑了七天的、别的会话遗留的 `cargo test` 误伤过。
    """
    try:
        out = subprocess.run(
            ["ps", "-axo", "pid=,args="], capture_output=True, text=True, timeout=30
        ).stdout
    except Exception:
        return []
    needles = (str(repo), str(target_dir))
    hits: list[str] = []
    for line in out.splitlines():
        parts = line.strip().split(None, 1)
        if len(parts) < 2:
            continue
        pid, cmdline = parts[0], parts[1]
        exe = os.path.basename(cmdline.split()[0]) if cmdline.split() else ""
        if exe not in ("cargo", "rustc", "clippy-driver"):
            continue
        if any(n in cmdline for n in needles):
            hits.append(f"pid {pid}: {cmdline[:120]}")
    return hits


def target_size_gb(target_dir: Path) -> float:
    """target 目录磁盘占用（GB）。用 du -sk——尊重硬链接计数，与 du -sh 一致。"""
    try:
        out = subprocess.run(
            ["du", "-sk", str(target_dir)],
            capture_output=True,
            text=True,
            timeout=600,
            check=True,
        ).stdout
        return int(out.split()[0]) / (1024 * 1024)
    except Exception:
        return 0.0


def main() -> int:
    global repo_root

    ap = argparse.ArgumentParser(
        description="清掉 Cargo target/ 里的构建残留（旧代产物）",
    )
    ap.add_argument("--apply", action="store_true", help="真删（默认干跑，只报计划）")
    ap.add_argument("--keep-triples", action="store_true", help="保留交叉编译目录")
    ap.add_argument(
        "--prune-generations",
        action="store_true",
        help="额外剪掉同一编译单元的旧代产物（多回收数 GB，但下次构建会重编那批 crate）",
    )
    ap.add_argument(
        "--auto",
        type=float,
        metavar="GB",
        help=(
            "阈值模式：target/ 超过该 GB 数就自动清理（剪代，保留活代），"
            "否则什么都不做。供构建前钩子调用——构建缓存不得无端占用存储。"
        ),
    )
    ap.add_argument("--repo", default=None, help="仓库根目录（默认取脚本所在目录的上一级）")
    args = ap.parse_args()

    repo_root = Path(args.repo).resolve() if args.repo else Path(__file__).resolve().parent.parent
    prune_generations = args.prune_generations

    target_dir = resolve_target_dir(repo_root)

    # 阈值模式：超了才动，且用「剪代」模式（那才是大头）；没超就安静退出——
    # 这是给构建前钩子用的，不能每次构建都刷屏或误删。
    if args.auto is not None:
        gb = target_size_gb(target_dir)
        if gb <= args.auto:
            print(f"[cache-guard] target/ {gb:.1f}GB ≤ 阈值 {args.auto:g}GB，跳过清理")
            return 0
        print(
            f"[cache-guard] target/ {gb:.1f}GB > 阈值 {args.auto:g}GB —— 自动清理"
            "（剪掉旧代产物，保留活代；本次构建会重编被剪掉的那批 crate）"
        )
        args.apply = True
        prune_generations = True

    # 本仓库正在构建就别动——删掉正在用的产物会让这次构建失败
    busy = build_in_progress(repo_root, target_dir)
    if busy:
        print("✗ 本仓库正在构建，不能清理：", file=sys.stderr)
        for b in busy:
            print(f"    {b}", file=sys.stderr)
        print("  等构建结束再试。", file=sys.stderr)
        return 2

    lock = parse_lock(repo_root / "Cargo.lock")
    host = host_triple()

    plan = Plan()
    profiles: list[Path] = []
    for name in ("debug", "release"):
        p = target_dir / name
        if p.is_dir():
            profiles.append(p)
    # 交叉编译目录下的 profile 也扫
    for entry in sorted(target_dir.iterdir()) if target_dir.is_dir() else []:
        if entry.is_dir() and entry.name not in ("debug", "release", "tmp"):
            for name in ("debug", "release"):
                p = entry / name
                if p.is_dir():
                    profiles.append(p)

    for profile_dir in profiles:
        scan_profile(profile_dir, lock, plan, prune_generations=prune_generations)

    if not args.keep_triples:
        scan_triples(target_dir, host, plan)

    print(f"仓库      : {repo_root}")
    print(f"target 目录: {target_dir}")
    print(f"host 三元组: {host or '未知'}")
    print(f"Cargo.lock : {len(lock)} 个 name-version")
    print()
    print("清理计划：")
    if not plan.files and not plan.dirs:
        print("  （没有残留可清）")
    for category in ("intermediates", "stale-deps", "stale-incremental", "stale-bundle", "stale-triples"):
        files = plan.files.get(category, [])
        dirs = plan.dirs.get(category, [])
        if not files and not dirs:
            continue
        size = human(plan.bytes.get(category, 0))
        print(f"  {category:18} {len(files):>7} 文件  {len(dirs):>4} 目录   {size:>10}")
    total = human(sum(plan.bytes.values()))
    print(f"  {'合计':18} {plan.total_files():>7} 文件  {plan.total_dirs():>4} 目录   {total:>10}")
    print("  （以上为逻辑体积；实际磁盘释放量因硬链接共享会小于此数）")

    if not args.apply:
        print()
        print("干跑模式，未删除任何东西。加 --apply 执行。")
        return 0

    print()
    print("执行清理：")
    before = dir_size(target_dir)
    apply_plan(plan)
    after = dir_size(target_dir)
    print()
    print(f"target 目录：{human(before)} → {human(after)}（实际释放 {human(max(0, before - after))}）")
    print("提示：上面各类别的体积是**逻辑体积**——`.rcgu.o` 之间存在硬链接共享，")
    print("      所以逻辑体积会大于磁盘实际占用；实际释放量以上一行为准。")
    print("      下次 cargo 构建会重新生成中间产物（当前代的 rlib 若仍在则不受影响）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
