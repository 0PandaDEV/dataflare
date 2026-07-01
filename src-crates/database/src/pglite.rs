use crate::utils::FirstCell;
use crate::{ChunkInsert, Database, PGliteConfig, Result, Value};
use pglite::Connection;
use query::Query;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PGliteConnection {
    conn: Arc<Connection>,
}

impl PGliteConnection {
    async fn conn(config: PGliteConfig) -> Result<Connection> {
        let conn = Connection::connect(&config.path).await?;
        if let Some(sql) = config.initial {
            conn.execute(&sql)?;
        }
        Ok(conn)
    }

    pub(crate) async fn test(config: PGliteConfig) -> Result<Option<String>> {
        Self::conn(config)
            .await?
            .query("SELECT concat('PGlite (', version(), ')');")?
            .rows
            .first_cell_string()
            .map(Some)
    }

    pub(crate) async fn connect(config: PGliteConfig) -> Result<Database> {
        Ok(Database::PGlite(Self {
            conn: Arc::new(Self::conn(config).await?),
        }))
    }

    pub(crate) async fn select(&self, sql: String) -> Result<Vec<Vec<Value>>> {
        Ok(self.conn.select(&sql)?)
    }

    pub(crate) async fn execute(&self, sql: String) -> Result<()> {
        Ok(self.conn.execute(&sql)?)
    }

    pub(crate) async fn transaction(&self, sqls: Vec<String>) -> Result<()> {
        Ok(self.conn.transaction(&sqls)?)
    }

    pub(crate) async fn query(&self, sql: String) -> Result<Query> {
        Ok(self.conn.query(&sql)?)
    }

    pub(crate) async fn batch_insert(&self, insert: ChunkInsert) -> Result<()> {
        for sql in insert {
            self.conn.execute(&sql)?;
        }
        Ok(())
    }
}
