//! 自有 SQLite 存储。约定（docs/ARCHITECTURE.md 第 5 节、DATA_CONTRACTS 第 9 节）：
//! - WAL、外键开启、有界 busy_timeout、单写入队列由调用方保证。
//! - 普通表禁止出现 credential_plaintext / prompt_body / response_body 字段。
//! - Token 列为可空 INTEGER（范围校验 ≤ i64::MAX）；聚合在 Rust 侧用检查算术。

use rusqlite::Connection;
use std::path::Path;

pub mod migrations;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration failed at version {version}: {source}")]
    Migration {
        version: u32,
        #[source]
        source: rusqlite::Error,
    },
    #[error("token value does not fit SQLite INTEGER: {0}")]
    TokenOverflow(u64),
}

pub type Result<T> = std::result::Result<T, StorageError>;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// 打开（或创建）应用数据库并执行迁移。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// 内存库，供测试使用。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 2000)?;
        let mut db = Self { conn };
        migrations::migrate(&mut db.conn)?;
        Ok(db)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 可写事务入口。事务失败整体回滚（Drop 语义），调用方不得部分提交。
    pub fn transaction(&mut self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.conn.transaction()?)
    }

    pub fn schema_version(&self) -> Result<u32> {
        migrations::current_version(&self.conn)
    }
}

/// u64 → SQLite INTEGER，越界显式报错，不静默截断。
pub fn token_to_sql(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StorageError::TokenOverflow(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_is_at_latest_version() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), migrations::LATEST);
    }

    #[test]
    fn migration_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        // 二次执行不报错、版本不变
        let mut db = db;
        migrations::migrate(&mut db.conn).unwrap();
        assert_eq!(db.schema_version().unwrap(), migrations::LATEST);
    }

    #[test]
    fn token_integer_range_checked() {
        assert_eq!(token_to_sql(i64::MAX as u64).unwrap(), i64::MAX);
        assert!(matches!(
            token_to_sql(i64::MAX as u64 + 1),
            Err(StorageError::TokenOverflow(_))
        ));
    }
}
