use crate::decode;
use chdb_rust::arg::Arg;
use chdb_rust::connection::Connection as ChdbConnection;
use chdb_rust::format::OutputFormat;
use chdb_rust::query_result::QueryResult;
use chdb_rust::session::{Session, SessionBuilder};
use serde::Deserialize;
use std::sync::Mutex;

/// Global slot that enforces chdb's coexistence constraint:
/// a persistent connection cannot share the process with any other connection,
/// but multiple in-memory connections may coexist freely.
struct ConnectionSlot {
    in_memory_count: u32,
    has_persistent: bool,
}

static CONNECTION_SLOT: Mutex<ConnectionSlot> = Mutex::new(ConnectionSlot {
    in_memory_count: 0,
    has_persistent: false,
});

#[derive(Debug)]
pub(crate) struct Connection {
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    InMemory(ChdbConnection),
    Persistent(Session),
}

#[derive(Debug)]
pub(crate) struct Query {
    pub(crate) columns: Vec<QueryColumn>,
    pub(crate) rows: Vec<Vec<QueryValue>>,
    pub(crate) duration: u32,
}

#[derive(Debug)]
pub(crate) struct QueryColumn {
    pub(crate) name: String,
    pub(crate) datatype: String,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub(crate) enum QueryValue {
    Null,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    String(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Chdb(#[from] chdb_rust::error::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("connection slot mutex was poisoned")]
    MutexPoisoned,
    #[error(
        "a persistent connection is already active; close it before opening another connection"
    )]
    PersistentConnectionActive,
    #[error("cannot open a persistent connection while other connections are active")]
    OtherConnectionsActive,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    meta: Vec<ResponseColumn>,
    data: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ResponseColumn {
    name: String,
    #[serde(rename = "type")]
    datatype: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ConnectPath<'a> {
    InMemory,
    Persistent(&'a str),
}

impl<'a> ConnectPath<'a> {
    fn from_path(path: &'a str) -> Self {
        let path = path.trim();
        if path.is_empty() || path == ":memory:" {
            ConnectPath::InMemory
        } else {
            ConnectPath::Persistent(path)
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut slot = CONNECTION_SLOT.lock().unwrap();
        match &self.inner {
            Inner::InMemory(_) => slot.in_memory_count -= 1,
            Inner::Persistent(_) => slot.has_persistent = false,
        }
    }
}

impl Connection {
    pub fn connect(path: &str) -> std::result::Result<Self, Error> {
        let mut slot = CONNECTION_SLOT.lock().map_err(|_| Error::MutexPoisoned)?;
        match ConnectPath::from_path(path) {
            ConnectPath::InMemory => {
                if slot.has_persistent {
                    return Err(Error::PersistentConnectionActive);
                }
                match ChdbConnection::open(&["-n"]) {
                    Ok(conn) => {
                        slot.in_memory_count += 1;
                        Ok(Self {
                            inner: Inner::InMemory(conn),
                        })
                    }
                    Err(e) => Err(e.into()),
                }
            }
            ConnectPath::Persistent(path) => {
                if slot.has_persistent || slot.in_memory_count > 0 {
                    return Err(Error::OtherConnectionsActive);
                }
                match SessionBuilder::new()
                    .with_data_path(path)
                    .with_arg(Arg::MultiQuery)
                    .build()
                {
                    Ok(session) => {
                        slot.has_persistent = true;
                        Ok(Self {
                            inner: Inner::Persistent(session),
                        })
                    }
                    Err(e) => Err(e.into()),
                }
            }
        }
    }

    fn raw_query(
        &self,
        sql: &str,
        format: OutputFormat,
    ) -> std::result::Result<QueryResult, Error> {
        match &self.inner {
            Inner::InMemory(conn) => Ok(conn.query(sql, format)?),
            Inner::Persistent(session) => {
                let args = [Arg::OutputFormat(format)];
                Ok(session.execute(sql, Some(&args))?)
            }
        }
    }

    pub fn execute(&self, sql: &str) -> std::result::Result<(), Error> {
        let _ = self.raw_query(sql, OutputFormat::Null)?;
        Ok(())
    }

    pub fn query(&self, sql: &str) -> std::result::Result<Query, Error> {
        let result = self.raw_query(sql, OutputFormat::JSONCompactStrings)?;

        let query = serde_json::from_slice::<QueryResponse>(result.data_ref())?;

        let columns = query
            .meta
            .into_iter()
            .map(|column| QueryColumn {
                name: column.name,
                datatype: column.datatype,
            })
            .collect::<Vec<_>>();

        let rows = query
            .data
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .enumerate()
                    .map(|(index, value)| decode::decode_value(value, &columns[index].datatype))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let duration = result.elapsed().as_millis() as u32;

        Ok(Query {
            columns,
            rows,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectPath, Connection, Error, QueryValue};

    #[test]
    fn connect_path_resolution() {
        assert_eq!(ConnectPath::from_path(""), ConnectPath::InMemory);
        assert_eq!(ConnectPath::from_path("   "), ConnectPath::InMemory);
        assert_eq!(ConnectPath::from_path(":memory:"), ConnectPath::InMemory);
        assert_eq!(
            ConnectPath::from_path("  :memory:  "),
            ConnectPath::InMemory
        );
        assert!(matches!(
            ConnectPath::from_path("/some/path"),
            ConnectPath::Persistent("/some/path")
        ));
    }

    #[test]
    fn open_in_memory_connection() {
        // multiple in-memory connections may coexist
        let c1 = Connection::connect(":memory:").unwrap();
        let c2 = Connection::connect("").unwrap();
        let c3 = Connection::connect("   ").unwrap();
        drop(c1);
        drop(c2);
        drop(c3);
    }

    #[test]
    fn persistent_blocks_new_connections() {
        let id = std::process::id();
        let path = std::env::temp_dir().join(format!("libchdb_test_persistent_blocks_{}", id));
        let conn = Connection::connect(path.to_str().unwrap()).unwrap();
        // any new connection must be rejected while persistent is alive
        assert!(matches!(
            Connection::connect(":memory:"),
            Err(Error::PersistentConnectionActive)
        ));
        assert!(matches!(
            Connection::connect(path.to_str().unwrap()),
            Err(Error::OtherConnectionsActive)
        ));
        drop(conn);
        std::fs::remove_dir_all(&path).unwrap();
        // after drop, in-memory is allowed again
        assert!(Connection::connect(":memory:").is_ok());
    }

    #[test]
    fn in_memory_blocks_persistent() {
        let conn = Connection::connect(":memory:").unwrap();
        let id = std::process::id();
        let path = std::env::temp_dir().join(format!("libchdb_test_imblocks_{}", id));
        assert!(matches!(
            Connection::connect(path.to_str().unwrap()),
            Err(Error::OtherConnectionsActive)
        ));
        drop(conn);
    }

    #[test]
    fn open_persistent_connection() {
        let id = std::process::id();
        let path = std::env::temp_dir().join(format!("libchdb_test_open_persistent_{}", id));
        Connection::connect(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn query_success() {
        let conn = Connection::connect(":memory:").unwrap();
        conn.execute("create table t (id UInt64, name String) engine = Memory")
            .unwrap();
        conn.execute("insert into t values (1, 'hello'), (2, 'world')")
            .unwrap();

        let result = conn.query("select id, name from t order by id").unwrap();

        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[1].name, "name");
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], QueryValue::U64(1));
        assert_eq!(result.rows[0][1], QueryValue::String("hello".into()));
        assert_eq!(result.rows[1][0], QueryValue::U64(2));
        assert_eq!(result.rows[1][1], QueryValue::String("world".into()));

        // empty result set
        let empty = conn.query("select id from t where id = 999").unwrap();
        assert_eq!(empty.columns.len(), 1);
        assert_eq!(empty.rows.len(), 0);
    }

    #[test]
    fn query_failure() {
        let conn = Connection::connect(":memory:").unwrap();
        // DDL via query(), invalid SQL, missing table
        assert!(
            conn.query("create table ddl_test (id UInt64) engine = Memory")
                .is_err()
        );
        assert!(conn.query("this is not valid sql").is_err());
        assert!(conn.query("select * from nonexistent_table_xyz").is_err());
        assert!(conn.execute("not valid sql at all").is_err());
    }

    #[test]
    fn empty_sql_statement_is_ok() {
        let conn = Connection::connect(":memory:").unwrap();
        assert!(conn.execute("").is_ok());
        assert!(conn.execute("   ").is_ok());
    }

    #[test]
    fn query_success_empty() {
        let conn = Connection::connect(":memory:").unwrap();
        conn.execute("create table e (id UInt64, name String) engine = Memory")
            .unwrap();
        let empty = conn.query("select id from e where id = 999").unwrap();
        assert_eq!(empty.columns.len(), 1);
        assert_eq!(empty.rows.len(), 0);
    }

    #[test]
    fn execute_batch_success() {
        let conn = Connection::connect(":memory:").unwrap();
        // multiple statements and semicolons inside string literals
        conn.execute(
            "create table bt (id UInt64, note String) engine = Memory; \
             insert into bt values (1, 'hello;world'), (2, 'plain')",
        )
        .unwrap();

        let result = conn.query("select id, note from bt order by id").unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], QueryValue::U64(1));
        assert_eq!(result.rows[0][1], QueryValue::String("hello;world".into()));
        assert_eq!(result.rows[1][0], QueryValue::U64(2));
        assert_eq!(result.rows[1][1], QueryValue::String("plain".into()));
    }

    #[test]
    fn execute_batch_failure() {
        let conn = Connection::connect(":memory:").unwrap();
        assert!(
            conn.execute("this is not valid sql; also not valid;")
                .is_err()
        );
    }

    #[test]
    fn database_lifecycle() {
        let conn = Connection::connect(":memory:").unwrap();

        conn.execute("CREATE DATABASE IF NOT EXISTS test_db")
            .unwrap();
        conn.execute("USE test_db").unwrap();
        conn.execute(
            "CREATE TABLE users (id UInt32, name String) ENGINE = MergeTree() ORDER BY id",
        )
        .unwrap();
        conn.execute("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob')")
            .unwrap();

        let result = conn
            .query("SELECT id, name FROM users ORDER BY id")
            .unwrap();
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], QueryValue::U32(1));
        assert_eq!(result.rows[0][1], QueryValue::String("Alice".into()));
        assert_eq!(result.rows[1][0], QueryValue::U32(2));
        assert_eq!(result.rows[1][1], QueryValue::String("Bob".into()));

        conn.execute("DROP DATABASE IF EXISTS test_db").unwrap();
    }
}
