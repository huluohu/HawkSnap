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
    Ok(conn)
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

pub fn list(app: &AppHandle, limit: i64) -> Result<Vec<ShotRow>, String> {
    list_at(&db_path(app), limit)
}

pub(crate) fn list_at(db: &std::path::Path, limit: i64) -> Result<Vec<ShotRow>, String> {
    let conn = open_at(db)?;
    let mut stmt = conn
        .prepare("SELECT id, created_at, path, thumb_path, w, h, app FROM shots ORDER BY id DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |r| {
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

#[tauri::command]
pub fn history_list(app: AppHandle) -> Result<Vec<ShotRow>, String> {
    list(&app, 200)
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
        let rows = list_at(&db, 10).unwrap();
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
        assert!(list_at(&db, 10).unwrap().is_empty(), "行被删");
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
        let rows = list_at(&db, 100).unwrap();
        assert_eq!(rows.len(), 5, "只保留最新 5 条");
        let kept: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert!(!kept.contains(&ids[0]), "最旧的被清");
        assert!(kept.contains(&ids[5]), "最新的保留");
        assert!(!dir.join("p0.png").exists(), "最旧的文件连带删除");
        assert!(dir.join("p5.png").exists(), "保留条目的文件不动");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
