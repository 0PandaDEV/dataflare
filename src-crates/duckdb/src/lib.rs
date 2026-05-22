mod ffi;

use dylib::Dylib;
use dylib::driver::{Error, Result};
use dylib::ffi::{ErrorMessage, StringRef};
use ffi::*;
use query::{Query, QueryColumn, Value};
use std::ffi::c_void;

// NOTE:
// Do not update manually
// Use `node ./src-dylib/driver-update.mjs` update the sha256 values.

const DUCKDB_DRIVER_VERSION: &str = "20260522";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const DUCKDB_SHA256: &str = "fd06a5f63a34360b7d60d3ef03912a0745cbfbb4c1ac00f67f578054baacc0b2";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const DUCKDB_SHA256: &str = "d005ce529265242bfb1eaa9803642ab9c2f133dc38476e5ccf9a45a17e16dfb9";
#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
const DUCKDB_SHA256: &str = "e9ab8b562564b0efa353a6677e74e7ea6ed56e027283c4b1d2f072ceff57dbf4";
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
const DUCKDB_SHA256: &str = "633450ae065c9b69895aed1cc22993c6d5302b275ea5e8723bb1e5cad91d53e2";
#[cfg(all(target_os = "windows", target_arch = "aarch64", target_env = "msvc"))]
const DUCKDB_SHA256: &str = "fa64f943cb25bbf6c6b356b5497b23cea73dac882a69a37d9a34f9242eea5d9c";
#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
const DUCKDB_SHA256: &str = "d1d239612a8c2250da8623ade8f71cc2a2db0c657c082dfb066c69533d18c419";

#[derive(Debug)]
pub struct Connection {
    conn: *mut c_void,
    dylib: Dylib,
}

unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn free_error(dylib: &Dylib, error: ErrorMessage) -> Result<Option<Error>, Error> {
    if !error.is_null() {
        let msg = error.as_str().to_string();
        dylib.symbol::<FreeErrorFn>(FREE_ERROR)?(error);
        return Ok(Some(Error::Message(msg)));
    }
    Ok(None)
}

impl Connection {
    pub async fn connect(path: &str, readonly: bool) -> Result<Self> {
        let dylib = Dylib::try_load("duckdb", DUCKDB_DRIVER_VERSION, DUCKDB_SHA256).await?;
        let mut error = ErrorMessage::null();
        let conn = dylib.symbol::<ConnectFn>(CONNECT)?(StringRef::new(path), readonly, &mut error);
        if let Some(err) = free_error(&dylib, error)? {
            return Err(err);
        }
        Ok(Self { conn, dylib })
    }

    fn close(&self) -> Result<(), Error> {
        self.dylib.symbol::<CloseFn>(CLOSE)?(self.conn);
        Ok(())
    }

    pub fn try_clone(&self) -> Result<Self> {
        let dylib = self.dylib.clone();
        let mut error = ErrorMessage::null();
        let conn = dylib.symbol::<CloneFn>(CLONE)?(self.conn, &mut error);
        if let Some(err) = free_error(&dylib, error)? {
            return Err(err);
        }
        Ok(Self { conn, dylib })
    }

    pub fn execute(&self, sql: &str) -> Result<(), Error> {
        let mut error = ErrorMessage::null();
        self.dylib.symbol::<ExecuteFn>(EXECUTE)?(self.conn, StringRef::new(sql), &mut error);
        if let Some(err) = free_error(&self.dylib, error)? {
            return Err(err);
        }
        Ok(())
    }

    pub fn query(&self, sql: &str) -> Result<Query, Error> {
        let mut error = ErrorMessage::null();
        let query =
            self.dylib.symbol::<QueryFn>(QUERY)?(self.conn, StringRef::new(sql), &mut error);
        if let Some(err) = free_error(&self.dylib, error)? {
            return Err(err);
        }
        let meta = self.dylib.symbol::<QueryMetaFn>(QUERY_META)?(query);
        let query_column = self.dylib.symbol::<QueryColumnFn>(QUERY_COLUMN)?;
        let query_value = self.dylib.symbol::<QueryValueFn>(QUERY_VALUE)?;

        let columns = (0..meta.column_count)
            .map(|i| {
                let col = query_column(query, i);
                QueryColumn {
                    name: col.name.as_str().to_string(),
                    datatype: col.datatype.as_str().to_string(),
                }
            })
            .collect::<Vec<_>>();

        let mut rows = Vec::with_capacity(meta.row_count);
        for y in 0..meta.row_count {
            let mut row = Vec::with_capacity(meta.column_count);
            for x in 0..meta.column_count {
                let data = query_value(query, y, x);
                row.push(unsafe {
                    match data.kind {
                        DataKind::Null => Value::Null,
                        DataKind::Bool => Value::Bool(data.value.bool),
                        DataKind::I8 => Value::I8(data.value.i8),
                        DataKind::I16 => Value::I16(data.value.i16),
                        DataKind::I32 => Value::I32(data.value.i32),
                        DataKind::I64 => Value::I64(data.value.i64),
                        DataKind::U8 => Value::U8(data.value.u8),
                        DataKind::U16 => Value::U16(data.value.u16),
                        DataKind::U32 => Value::U32(data.value.u32),
                        DataKind::U64 => Value::U64(data.value.u64),
                        DataKind::F32 => Value::F32(data.value.f32),
                        DataKind::F64 => Value::F64(data.value.f64),
                        DataKind::String => Value::String(data.value.string.as_str().to_string()),
                        DataKind::Bytes => Value::Bytes(data.value.bytes.as_bytes().into()),
                    }
                });
            }
            rows.push(row);
        }
        self.dylib.symbol::<FreeQueryFn>(FREE_QUERY)?(query);

        Ok(Query {
            columns,
            rows,
            rows_affected: Some(meta.rows_affected),
            duration: meta.duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use query::{QueryColumn, Value};

    #[tokio::test]
    async fn test_query() {
        let conn = Connection::connect("", false).await.unwrap();
        let mut query = conn.query("select 'hello' as hello").unwrap();
        assert_eq!(query.columns.len(), 1);
        assert_eq!(
            query.columns.remove(0),
            QueryColumn {
                name: "hello".into(),
                datatype: "varchar".into()
            }
        );
        assert_eq!(query.rows.len(), 1);
        assert_eq!(query.rows[0].len(), 1);
        assert_eq!(
            query.rows.remove(0).remove(0),
            Value::String("hello".into())
        );
        assert_eq!(query.rows_affected, Some(0));
    }

    #[tokio::test]
    async fn test_extension() {
        let conn = Connection::connect("", false).await.unwrap();
        let query = conn
            .query("SELECT * FROM 'https://duckdb.org/data/records.json'")
            .unwrap();
        assert_eq!(query.columns.len(), 2);
        assert_eq!(query.rows.len(), 3);
    }
}
