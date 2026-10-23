use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// 历史记录行（C3）：一次保存 = 一行 + 截图文件 + 缩略图文件。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotRow {
    pub id: i64,
    pub created_at: String,
    pub path: String,
    pub thumb_path: String,
    pub w: i64,
    pub h: i64,
    pub app: String,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS shots(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'region',
    path TEXT NOT NULL,
    thumb_path TEXT NOT NULL,
    w INTEGER NOT NULL,
    h INTEGER NOT NULL,
    app TEXT NOT NULL);";

/// FTS5（M0，Task M0 计划接口）：external-content 虚表（content='shots'，rowid 对齐
/// shots.id），索引 path + app 两列——unicode61 分词会把绝对路径按非字母数字切开，
/// 文件名段与 app 名自然成为可检索单元。ocr_text 列本任务先加列不索引（M2-OCR 任务
/// 填充并扩索引）。触发器是 external-content 的标准同步模式（INSERT/UPDATE/DELETE
/// 三向，UPDATE 走 delete+insert 复合）；shots 现无 UPDATE 路径，留作标准完整性。
const FTS_SCHEMA: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS shots_fts USING fts5(
    path, app, content='shots', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS shots_fts_ai AFTER INSERT ON shots BEGIN
    INSERT INTO shots_fts(rowid, path, app) VALUES (new.id, new.path, new.app);
END;
CREATE TRIGGER IF NOT EXISTS shots_fts_ad AFTER DELETE ON shots BEGIN
    INSERT INTO shots_fts(shots_fts, rowid, path, app)
    VALUES ('delete', old.id, old.path, old.app);
END;
CREATE TRIGGER IF NOT EXISTS shots_fts_au AFTER UPDATE ON shots BEGIN
    INSERT INTO shots_fts(shots_fts, rowid, path, app)
    VALUES ('delete', old.id, old.path, old.app);
    INSERT INTO shots_fts(rowid, path, app) VALUES (new.id, new.path, new.app);
END;";

/// history.db 落点（app_data_dir()/history.db）。
fn db_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("history.db")
}

/// 打开连接并确保 schema（幂等）。连接策略：每次操作 `Connection::open` 新建——
/// SQLite 文件型打开是 µs 级，换来无跨线程 Send/生命周期负担（Tauri 命令线程
/// 不定），不持长连接、不引连接池。
fn open_at(db: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open(db).map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    ensure_ocr_text_column(&conn)?;
    conn.execute_batch(FTS_SCHEMA).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// ocr_text 列迁移（M0 预埋，M2-OCR 填充）：PRAGMA table_info 探测 → 缺则
/// ALTER TABLE ADD COLUMN（NULL 直到 OCR 任务回填）。幂等：已存在则跳过——
/// 老 history.db（C3 建）升级零拷贝。
fn ensure_ocr_text_column(conn: &Connection) -> Result<(), String> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('shots') WHERE name = 'ocr_text'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if has == 0 {
        conn.execute_batch("ALTER TABLE shots ADD COLUMN ocr_text TEXT;")
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 建目录 + 建表。幂等（CREATE TABLE IF NOT EXISTS），启动时调用；失败仅记录，
/// 不阻塞启动（历史是旁路功能，命令侧再失败会向用户返回错误）。
pub fn init(app: &AppHandle) -> Result<(), String> {
    let path = db_path(app);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    open_at(&path).map(|_| ())
}

pub fn record(
    app: &AppHandle,
    path: &str,
    thumb_path: &str,
    w: i64,
    h: i64,
    app_name: &str,
) -> Result<i64, String> {
    record_at(&db_path(app), path, thumb_path, w, h, app_name)
}

pub(crate) fn record_at(
    db: &std::path::Path,
    path: &str,
    thumb_path: &str,
    w: i64,
    h: i64,
    app_name: &str,
) -> Result<i64, String> {
    let conn = open_at(db)?;
    let created_at = chrono::Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO shots(created_at, path, thumb_path, w, h, app) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![created_at, path, thumb_path, w, h, app_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn list(app: &AppHandle, limit: i64, offset: i64) -> Result<Vec<ShotRow>, String> {
    list_at(&db_path(app), limit, offset)
}

/// M0：offset 分页（网格分页留用，默认 0）。新→旧不变。
pub(crate) fn list_at(db: &std::path::Path, limit: i64, offset: i64) -> Result<Vec<ShotRow>, String> {
    let conn = open_at(db)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, path, thumb_path, w, h, app FROM shots
             ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit, offset], |r| {
            Ok(ShotRow {
                id: r.get(0)?,
                created_at: r.get(1)?,
                path: r.get(2)?,
                thumb_path: r.get(3)?,
                w: r.get(4)?,
                h: r.get(5)?,
                app: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

// —— FTS5 搜索（M0） ——

/// MATCH 查询转义（防 FTS5 语法注入）：整串包双引号做短语查询，内部双引号逐字翻倍
/// ——这是 FTS5 字符串字面量的唯一转义规则，`*`/`(`/`NEAR` 等语法字符在短语内全部
/// 退化为普通文本（分词器切掉非字母数字段），构造不出语法树。
fn escape_match(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

/// FTS5 搜索：命中按 bm25 rank（越小越相关）升序，同分按 id 新→旧。
/// 空/全空白查询返回空集（前端空输入走 list，此为防御）。
pub fn search(app: &AppHandle, query: &str, limit: i64) -> Result<Vec<ShotRow>, String> {
    search_at(&db_path(app), query, limit)
}

pub(crate) fn search_at(db: &std::path::Path, query: &str, limit: i64) -> Result<Vec<ShotRow>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let conn = open_at(db)?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.created_at, s.path, s.thumb_path, s.w, s.h, s.app
             FROM shots_fts
             JOIN shots s ON s.id = shots_fts.rowid
             WHERE shots_fts MATCH ?1
             ORDER BY shots_fts.rank, s.id DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let escaped = escape_match(query);
    let rows = stmt
        .query_map(rusqlite::params![escaped, limit], |r| {
            Ok(ShotRow {
                id: r.get(0)?,
                created_at: r.get(1)?,
                path: r.get(2)?,
                thumb_path: r.get(3)?,
                w: r.get(4)?,
                h: r.get(5)?,
                app: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn delete(app: &AppHandle, id: i64) -> Result<(), String> {
    delete_at(&db_path(app), id)
}

/// 删行 + 删两个文件（文件已不在（NotFound）忽略——行存在而文件丢失是合法状态：
/// 用户可手动清理截图目录，历史条目仍可删）。
pub(crate) fn delete_at(db: &std::path::Path, id: i64) -> Result<(), String> {
    let conn = open_at(db)?;
    let (path, thumb): (String, String) = conn
        .query_row("SELECT path, thumb_path FROM shots WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM shots WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    for p in [path, thumb] {
        if p.is_empty() {
            continue;
        }
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // 文件已不在，忽略
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub fn prune(app: &AppHandle, max: i64) -> Result<(), String> {
    prune_at(&db_path(app), max)
}

/// 保留最新 max 条：id 大于第 max 新的行的条目逐条 delete（连带删文件）。
pub(crate) fn prune_at(db: &std::path::Path, max: i64) -> Result<(), String> {
    let conn = open_at(db)?;
    let mut stmt = conn
        .prepare("SELECT id FROM shots ORDER BY id DESC")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(conn);
    // 保留最新 max 条（id 最大的 max 个），其余逐条 delete（连带删文件）。
    for id in ids.into_iter().skip(max.max(0) as usize) {
        delete_at(db, id)?;
    }
    Ok(())
}

/// M0：limit/offset 可选（前端省略时 200/0——面板首屏口径不变）。
#[tauri::command]
pub fn history_list(
    app: AppHandle,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<ShotRow>, String> {
    list(&app, limit.unwrap_or(200).clamp(1, 500), offset.unwrap_or(0).max(0))
}

/// M0 计划接口：history_search(query) -> Vec<ShotRow>。F12 目标 ≤200ms/500 条
/// （fs_500_rows_search_timing 单测计时守护）。查询转义在 search_at 内做（防注入）。
#[tauri::command]
pub fn history_search(
    app: AppHandle,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<ShotRow>, String> {
    search(&app, &query, limit.unwrap_or(50).clamp(1, 200))
}

#[tauri::command(async)]
pub fn history_delete(app: AppHandle, id: i64) -> Result<(), String> {
    delete(&app, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hs-history-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 1×1 真 PNG 文件（core rgba_to_png 生成），供 delete 断言文件被删。
    fn real_png(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let png = hawksnap_core::crop::rgba_to_png(&[1, 2, 3, 255], 1, 1).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, &png).unwrap();
        path
    }

    fn db_file(tag: &str) -> std::path::PathBuf {
        let dir = temp_dir(tag);
        let db = dir.join("history.db");
        open_at(&db).unwrap();
        db
    }

    #[test]
    fn init_is_idempotent() {
        let dir = temp_dir("idem");
        let db = dir.join("history.db");
        open_at(&db).unwrap();
        open_at(&db).unwrap(); // 二次 init 不炸（CREATE TABLE IF NOT EXISTS）
        assert!(db.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn record_list_roundtrip() {
        let dir = temp_dir("roundtrip");
        let db = dir.join("history.db");
        let png1 = real_png(&dir, "a.png");
        let png2 = real_png(&dir, "b.png");
        let id1 = record_at(&db, png1.to_str().unwrap(), "", 100, 50, "Safari").unwrap();
        let id2 = record_at(&db, png2.to_str().unwrap(), "", 200, 80, "Finder").unwrap();
        assert!(id1 < id2, "自增 id 单调");
        let rows = list_at(&db, 10, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, id2, "list 按新→旧");
        assert_eq!(rows[0].path, png2.to_str().unwrap());
        assert_eq!(rows[0].app, "Finder");
        assert_eq!((rows[0].w, rows[0].h), (200, 80));
        assert_eq!(rows[1].id, id1);
        assert_eq!(rows[1].app, "Safari");
        assert!(!rows[0].created_at.is_empty());
        assert!(rows[0].thumb_path.is_empty() || rows[0].thumb_path == "");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_removes_row_and_files() {
        let dir = temp_dir("delete");
        let db = dir.join("history.db");
        let png = real_png(&dir, "c.png");
        let thumb = real_png(&dir, "c.thumb.png");
        let id = record_at(
            &db,
            png.to_str().unwrap(),
            thumb.to_str().unwrap(),
            10, 10, "App",
        )
        .unwrap();
        assert!(png.exists() && thumb.exists());
        delete_at(&db, id).unwrap();
        assert!(list_at(&db, 10, 0).unwrap().is_empty(), "行被删");
        assert!(!png.exists(), "截图文件被删");
        assert!(!thumb.exists(), "缩略图文件被删");
        // NotFound 忽略：对已删除文件路径的条目再删不炸
        let id2 = record_at(&db, dir.join("gone.png").to_str().unwrap(), "", 1, 1, "App").unwrap();
        delete_at(&db, id2).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prune_keeps_newest_max() {
        let dir = temp_dir("prune");
        let db = dir.join("history.db");
        let mut ids = Vec::new();
        for i in 0..6 {
            let png = real_png(&dir, &format!("p{i}.png"));
            ids.push(record_at(&db, png.to_str().unwrap(), "", i, i, "App").unwrap());
        }
        prune_at(&db, 5).unwrap();
        let rows = list_at(&db, 100, 0).unwrap();
        assert_eq!(rows.len(), 5, "只保留最新 5 条");
        let kept: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert!(!kept.contains(&ids[0]), "最旧的被清");
        assert!(kept.contains(&ids[5]), "最新的保留");
        assert!(!dir.join("p0.png").exists(), "最旧的文件连带删除");
        assert!(dir.join("p5.png").exists(), "保留条目的文件不动");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_offset_paginates() {
        let dir = temp_dir("offset");
        let db = dir.join("history.db");
        let mut ids = Vec::new();
        for i in 0..3 {
            ids.push(record_at(&db, dir.join(format!("o{i}.png")).to_str().unwrap(), "", 1, 1, "App").unwrap());
        }
        let page1 = list_at(&db, 2, 0).unwrap();
        let page2 = list_at(&db, 2, 2).unwrap();
        assert_eq!(page1.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[2], ids[1]], "第一页新→旧");
        assert_eq!(page2.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[0]], "第二页只剩最旧一条");
        assert!(list_at(&db, 2, 9).unwrap().is_empty(), "越界 offset 空集");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ocr_text_column_migrates_and_is_idempotent() {
        let dir = temp_dir("ocr-col");
        let db = dir.join("history.db");
        // 造一个 C3 老库（无 ocr_text 列、无 FTS），再经 open_at 升级
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO shots(created_at, path, thumb_path, w, h, app) VALUES ('t', '/x/a.png', '', 1, 1, 'App')",
                [],
            )
            .unwrap();
        }
        open_at(&db).unwrap();
        open_at(&db).unwrap(); // 二次迁移幂等（ALTER 不重复执行）
        let conn = Connection::open(&db).unwrap();
        let has: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('shots') WHERE name = 'ocr_text'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(has, 1, "ocr_text 列已加");
        // 老数据不受影响
        let (n, ocr): (i64, Option<String>) = conn
            .query_row("SELECT COUNT(*), MAX(ocr_text) FROM shots", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((n, ocr), (1, None), "老行保留且 ocr_text 为 NULL");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// FTS5 可用性实证（Task M0 验证项）：bundled libsqlite3-sys 0.38.2 的 build.rs
    /// 带 `-DSQLITE_ENABLE_FTS5`（registry src 159 行），此处用真实
    /// `CREATE VIRTUAL TABLE ... USING fts5` 探测坐实——编译不过/运行不支持都红。
    #[test]
    fn fts5_available_and_open_idempotent() {
        let db = db_file("fts5-avail");
        open_at(&db).unwrap(); // 含 FTS_SCHEMA 二次执行（CREATE VIRTUAL TABLE IF NOT EXISTS）
        let conn = Connection::open(&db).unwrap();
        conn.execute("CREATE VIRTUAL TABLE IF NOT EXISTS fts5_probe USING fts5(x)", [])
            .expect("bundled SQLite 必须启用 FTS5");
        std::fs::remove_dir_all(db.parent().unwrap()).unwrap();
    }

    #[test]
    fn search_hits_by_filename_and_app() {
        let dir = temp_dir("fts-hit");
        let db = dir.join("history.db");
        record_at(&db, "/pics/HawkSnap/Safari 2026-10-23 101010.png", "", 10, 10, "Safari").unwrap();
        record_at(&db, "/pics/HawkSnap/Finder 2026-10-23 101111.png", "", 10, 10, "Finder").unwrap();
        record_at(&db, "/pics/HawkSnap/safari-web 2026-10-23 101212.png", "", 10, 10, "Xcode").unwrap();
        // 文件名段命中
        let by_name = search_at(&db, "safari", 50).unwrap();
        assert_eq!(by_name.len(), 2, "大小写不敏感，命中两个 safari 文件名");
        // app 列命中
        let by_app = search_at(&db, "finder", 50).unwrap();
        assert_eq!(by_app.len(), 1);
        assert_eq!(by_app[0].app, "Finder");
        // 未命中
        assert!(search_at(&db, "nomatch", 50).unwrap().is_empty());
        // bm25 rank 排序：更长的同名串相关度低，safari-web 在后（rank 升序 + id 降序 tiebreak）
        assert_eq!(by_name[0].app, "Safari");
        // 空查询防御：空集不 Err
        assert!(search_at(&db, "   ", 50).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn search_syntax_characters_do_not_crash() {
        // 防注入（Task M0 验收）：`"` 与 `*` 等语法字符经整串引号包裹 + 内部引号翻倍后
        // 不构成 FTS 语法——任何输入都不炸、只可能空/命中。
        let dir = temp_dir("fts-inject");
        let db = dir.join("history.db");
        record_at(&db, "/pics/HawkSnap/a.png", "", 10, 10, "App").unwrap();
        for q in ["\"", "*", "a*", "\"*", "NEAR(", ")*(", "\"\"", "a OR b", "\\'", ""] {
            let r = search_at(&db, q, 50);
            assert!(r.is_ok(), "查询 {q:?} 不得报错: {:?}", r.err());
        }
        // 引号查询仍能命中普通词（转义不破坏匹配语义）
        assert_eq!(search_at(&db, "a.pn\"g", 50).unwrap().len(), 0);
        assert_eq!(search_at(&db, "a.p\"ng", 50).unwrap().len(), 0, "含引号的短语按分词匹配");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_cascades_fts_index() {
        let dir = temp_dir("fts-delete");
        let db = dir.join("history.db");
        let id = record_at(&db, "/pics/HawkSnap/cascade.png", "", 10, 10, "Zed").unwrap();
        assert_eq!(search_at(&db, "cascade", 50).unwrap().len(), 1);
        delete_at(&db, id).unwrap();
        assert!(search_at(&db, "cascade", 50).unwrap().is_empty(), "删除后 FTS 索引同步失效");
        // 触发器 delete 路径无残留：FTS 行数与 shots 行数一致
        let conn = Connection::open(&db).unwrap();
        let fts_n: i64 = conn.query_row("SELECT COUNT(*) FROM shots_fts", [], |r| r.get(0)).unwrap();
        assert_eq!(fts_n, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// F12 计时守护（≤200ms/500 条）：合成 500 行后整链计时（含每次操作重开连接——
    /// 生产口径）。bm25 检索本身 µs 级，200ms 预算极宽裕；实测值打印留档（报告引用）。
    #[test]
    fn fs_500_rows_search_timing() {
        let dir = temp_dir("fts-perf");
        let db = dir.join("history.db");
        for i in 0..500 {
            let app = if i % 2 == 0 { "Safari" } else { "Finder" };
            record_at(&db, &format!("/pics/HawkSnap/{app} 2026-10-23 {i:06}.png"), "", 10, 10, app).unwrap();
        }
        let t0 = std::time::Instant::now();
        let hits = search_at(&db, "safari", 50).unwrap();
        let elapsed = t0.elapsed();
        assert_eq!(hits.len(), 50, "limit=50 截断");
        assert_eq!(hits[0].app, "Safari");
        println!("fts search over 500 rows: {elapsed:?} (F12 预算 200ms)");
        assert!(
            elapsed.as_millis() < 200,
            "F12：500 行搜索必须 <200ms，实测 {elapsed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
