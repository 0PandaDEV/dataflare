use crate::decode;
use chdb_rust::arg::Arg;
use chdb_rust::connection::Connection as ChdbConnection;
use chdb_rust::format::OutputFormat;
use chdb_rust::query_result::QueryResult;
use chdb_rust::session::{Session, SessionBuilder};
use serde::Deserialize;

#[derive(Debug)]
pub(crate) enum Connection {
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

impl Connection {
    pub fn connect(path: &str) -> std::result::Result<Self, Error> {
        match ConnectPath::from_path(path) {
            ConnectPath::InMemory => match ChdbConnection::open(&["-n"]) {
                Ok(conn) => Ok(Self::InMemory(conn)),
                Err(e) => Err(e.into()),
            },
            ConnectPath::Persistent(path) => {
                match SessionBuilder::new()
                    .with_data_path(path)
                    .with_arg(Arg::MultiQuery)
                    .build()
                {
                    Ok(session) => Ok(Self::Persistent(session)),
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
        match &self {
            Connection::InMemory(conn) => Ok(conn.query(sql, format)?),
            Connection::Persistent(session) => {
                let args = [Arg::OutputFormat(format)];
                Ok(session.execute(sql, Some(&args))?)
            }
        }
    }

    pub fn query(&self, sql: &str) -> std::result::Result<Query, Error> {
        let result = self.raw_query(sql, OutputFormat::JSONCompactStrings)?;
        let duration = result.elapsed().as_millis() as u32;

        if result.data_ref().is_empty() {
            return Ok(Query {
                columns: vec![],
                rows: vec![],
                duration,
            });
        }

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

        Ok(Query {
            columns,
            rows,
            duration,
        })
    }
}

#[cfg(test)]
// NOTE: run tests with `cargo test -- --test-threads 1`
mod tests {
    use super::{ConnectPath, Connection, QueryValue};

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
        let c1 = Connection::connect(":memory:").unwrap();
        let c2 = Connection::connect("").unwrap();
        let c3 = Connection::connect("   ").unwrap();
        drop(c1);
        drop(c2);
        drop(c3);
    }

    #[test]
    fn open_persistent_connection() {
        let id = std::process::id();
        let path = std::env::temp_dir()
            .join(format!("libchdb_test_open_persistent_{}", id))
            .display()
            .to_string();

        {
            Connection::connect(&path).unwrap();
        }
        {
            let conn = Connection::connect(&path).unwrap();
            conn.query(
                "CREATE TABLE persist_t (id UInt64, note String) ENGINE = MergeTree() ORDER BY id",
            )
            .unwrap();
            conn.query("INSERT INTO persist_t VALUES (1, 'hello'), (2, 'world')")
                .unwrap();
        }
        {
            let conn = Connection::connect(&path).unwrap();
            let result = conn
                .query("SELECT id, note FROM persist_t ORDER BY id")
                .unwrap();
            assert_eq!(result.rows.len(), 2);
            assert_eq!(result.rows[0][0], QueryValue::U64(1));
            assert_eq!(result.rows[0][1], QueryValue::String("hello".into()));
            assert_eq!(result.rows[1][0], QueryValue::U64(2));
            assert_eq!(result.rows[1][1], QueryValue::String("world".into()));
        }

        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn test_query() {
        let conn = Connection::connect(":memory:").unwrap();

        let result = conn
            .query(
                "SELECT \
                    toInt8(-1) AS i8, \
                    toUInt8(1) AS u8, \
                    toInt16(-2) AS i16, \
                    toUInt16(2) AS u16, \
                    toInt32(-3) AS i32, \
                    toUInt32(3) AS u32, \
                    toInt64(-4) AS i64, \
                    toUInt64(4) AS u64, \
                    toFloat32(1.5) AS f32, \
                    toFloat64(2.5) AS f64, \
                    true AS b, \
                    'hello' AS s, \
                    NULL AS n",
            )
            .unwrap();

        assert_eq!(result.columns.len(), 13);
        assert_eq!(result.columns[0].name, "i8");
        assert_eq!(result.columns[0].datatype, "Int8");
        assert_eq!(result.columns[1].name, "u8");
        assert_eq!(result.columns[1].datatype, "UInt8");
        assert_eq!(result.columns[2].name, "i16");
        assert_eq!(result.columns[2].datatype, "Int16");
        assert_eq!(result.columns[3].name, "u16");
        assert_eq!(result.columns[3].datatype, "UInt16");
        assert_eq!(result.columns[4].name, "i32");
        assert_eq!(result.columns[4].datatype, "Int32");
        assert_eq!(result.columns[5].name, "u32");
        assert_eq!(result.columns[5].datatype, "UInt32");
        assert_eq!(result.columns[6].name, "i64");
        assert_eq!(result.columns[6].datatype, "Int64");
        assert_eq!(result.columns[7].name, "u64");
        assert_eq!(result.columns[7].datatype, "UInt64");
        assert_eq!(result.columns[8].name, "f32");
        assert_eq!(result.columns[8].datatype, "Float32");
        assert_eq!(result.columns[9].name, "f64");
        assert_eq!(result.columns[9].datatype, "Float64");
        assert_eq!(result.columns[10].name, "b");
        assert_eq!(result.columns[10].datatype, "Bool");
        assert_eq!(result.columns[11].name, "s");
        assert_eq!(result.columns[11].datatype, "String");
        assert_eq!(result.columns[12].name, "n");
        assert_eq!(result.columns[12].datatype, "Nullable(Nothing)");

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], QueryValue::I8(-1));
        assert_eq!(result.rows[0][1], QueryValue::U8(1));
        assert_eq!(result.rows[0][2], QueryValue::I16(-2));
        assert_eq!(result.rows[0][3], QueryValue::U16(2));
        assert_eq!(result.rows[0][4], QueryValue::I32(-3));
        assert_eq!(result.rows[0][5], QueryValue::U32(3));
        assert_eq!(result.rows[0][6], QueryValue::I64(-4));
        assert_eq!(result.rows[0][7], QueryValue::U64(4));
        assert_eq!(result.rows[0][8], QueryValue::F32(1.5));
        assert_eq!(result.rows[0][9], QueryValue::F64(2.5));
        assert_eq!(result.rows[0][10], QueryValue::Bool(true));
        assert_eq!(result.rows[0][11], QueryValue::String("hello".into()));
        assert_eq!(result.rows[0][12], QueryValue::Null);
    }

    #[test]
    fn query_empty_result() {
        let conn = Connection::connect(":memory:").unwrap();
        conn.query("CREATE TABLE e (id UInt64, name String) ENGINE = Memory")
            .unwrap();
        let empty = conn.query("SELECT id FROM e WHERE id = 999").unwrap();
        assert_eq!(empty.columns.len(), 1);
        assert_eq!(empty.rows.len(), 0);
    }

    #[test]
    fn query_failure() {
        let conn = Connection::connect(":memory:").unwrap();
        assert!(conn.query("this is not valid sql").is_err());
        assert!(conn.query("SELECT * FROM nonexistent_table_xyz").is_err());
    }

    #[test]
    fn query_as_execute() {
        let conn = Connection::connect(":memory:").unwrap();

        let r1 = conn
            .query("CREATE TABLE exec_t (id UInt64, name String) ENGINE = Memory")
            .unwrap();
        assert_eq!(r1.columns.len(), 0);
        assert_eq!(r1.rows.len(), 0);

        let r2 = conn
            .query("INSERT INTO exec_t VALUES (1, 'Alice'), (2, 'Bob')")
            .unwrap();
        assert_eq!(r2.columns.len(), 0);
        assert_eq!(r2.rows.len(), 0);

        let result = conn
            .query("SELECT id, name FROM exec_t ORDER BY id")
            .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], QueryValue::U64(1));
        assert_eq!(result.rows[0][1], QueryValue::String("Alice".into()));
        assert_eq!(result.rows[1][0], QueryValue::U64(2));
        assert_eq!(result.rows[1][1], QueryValue::String("Bob".into()));
    }

    #[test]
    fn query_multiple_statements() {
        let conn = Connection::connect(":memory:").unwrap();
        conn.query(
            "CREATE TABLE bt (id UInt64, note String) ENGINE = Memory; \
             INSERT INTO bt VALUES (1, 'hello;world'), (2, 'plain')",
        )
        .unwrap();

        let result = conn.query("SELECT id, note FROM bt ORDER BY id").unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], QueryValue::U64(1));
        assert_eq!(result.rows[0][1], QueryValue::String("hello;world".into()));
        assert_eq!(result.rows[1][0], QueryValue::U64(2));
        assert_eq!(result.rows[1][1], QueryValue::String("plain".into()));
    }

    #[test]
    fn query_multiple_statements_failure() {
        let conn = Connection::connect(":memory:").unwrap();
        assert!(conn.query("SELECT 1; this is not valid sql").is_err());
    }

    #[test]
    fn database_lifecycle() {
        let conn = Connection::connect(":memory:").unwrap();

        conn.query("CREATE DATABASE IF NOT EXISTS test_db").unwrap();
        conn.query("USE test_db").unwrap();
        conn.query("CREATE TABLE users (id UInt32, name String) ENGINE = MergeTree() ORDER BY id")
            .unwrap();
        conn.query("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob')")
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

        conn.query("DROP DATABASE IF EXISTS test_db").unwrap();
    }

    #[test]
    fn query_nullable_types() {
        let conn = Connection::connect(":memory:").unwrap();
        let result = conn
            .query(
                "SELECT \
                    toNullable(toInt8(-1)) AS ni8, \
                    toNullable(toUInt8(1)) AS nu8, \
                    toNullable(toInt16(-2)) AS ni16, \
                    toNullable(toUInt16(2)) AS nu16, \
                    toNullable(toInt32(-3)) AS ni32, \
                    toNullable(toUInt32(3)) AS nu32, \
                    toNullable(toInt64(-4)) AS ni64, \
                    toNullable(toUInt64(4)) AS nu64, \
                    toNullable(toFloat32(1.5)) AS nf32, \
                    toNullable(toFloat64(2.5)) AS nf64, \
                    toNullable(true) AS nb, \
                    toNullable('hello') AS ns, \
                    NULL AS nn",
            )
            .unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], QueryValue::I8(-1));
        assert_eq!(result.rows[0][1], QueryValue::U8(1));
        assert_eq!(result.rows[0][2], QueryValue::I16(-2));
        assert_eq!(result.rows[0][3], QueryValue::U16(2));
        assert_eq!(result.rows[0][4], QueryValue::I32(-3));
        assert_eq!(result.rows[0][5], QueryValue::U32(3));
        assert_eq!(result.rows[0][6], QueryValue::I64(-4));
        assert_eq!(result.rows[0][7], QueryValue::U64(4));
        assert_eq!(result.rows[0][8], QueryValue::F32(1.5));
        assert_eq!(result.rows[0][9], QueryValue::F64(2.5));
        assert_eq!(result.rows[0][10], QueryValue::Bool(true));
        assert_eq!(result.rows[0][11], QueryValue::String("hello".into()));
        assert_eq!(result.rows[0][12], QueryValue::Null);
    }
}
