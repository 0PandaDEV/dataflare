#[path = "../../../src-crates/dylib/src/ffi.rs"]
mod ffi;

mod connection;
mod decode;

use connection::{Connection, Query, QueryValue};
use ffi::{ErrorMessage, StringRef, TypedValue};
use std::ptr::null_mut;

type Result<T, E = String> = std::result::Result<T, E>;

trait StringError<T> {
    fn string_err(self) -> Result<T>;
}

impl<T, E> StringError<T> for std::result::Result<T, E>
where
    E: ToString,
{
    fn string_err(self) -> Result<T> {
        self.map_err(|err| err.to_string())
    }
}

#[repr(C)]
pub struct Meta {
    pub column_count: usize,
    pub row_count: usize,
    pub duration: u32,
}

#[repr(C)]
pub struct Column {
    pub name: StringRef,
    pub datatype: StringRef,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub enum DataKind {
    Null,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    String,
}

#[repr(C)]
pub union Data {
    pub null: (),
    pub bool: bool,
    pub i8: i8,
    pub i16: i16,
    pub i32: i32,
    pub i64: i64,
    pub u8: u8,
    pub u16: u16,
    pub u32: u32,
    pub u64: u64,
    pub f32: f32,
    pub f64: f64,
    pub string: StringRef,
}

#[repr(C)]
struct ConnectOptions {
    pub path: StringRef,
    // TODO: readonly, database
}

impl ConnectOptions {
    fn path(&self) -> &str {
        self.path.as_str()
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_connect(options: ConnectOptions, error: *mut ErrorMessage) -> *mut Connection {
    let call = || {
        let conn = Connection::connect(options.path()).string_err()?;
        Ok(Box::into_raw(Box::new(conn)))
    };
    call()
        .map_err(|err| {
            unsafe { *error = ErrorMessage::new(err) };
        })
        .unwrap_or(null_mut())
}

#[unsafe(no_mangle)]
extern "C" fn df_close(handle: *mut Connection) {
    unsafe {
        let _ = Box::from_raw(handle);
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_execute(handle: *mut Connection, sql: StringRef, error: *mut ErrorMessage) {
    let call = || {
        let connection = unsafe { &*handle };
        let _ = connection.query(sql.as_str()).string_err()?;
        Ok(())
    };
    if let Err(err) = call() {
        unsafe { *error = ErrorMessage::new(err) }
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_query(
    handle: *mut Connection,
    sql: StringRef,
    error: *mut ErrorMessage,
) -> *mut Query {
    let call = || {
        let connection = unsafe { &*handle };
        let query = connection.query(sql.as_str()).string_err()?;
        Ok(Box::into_raw(Box::new(query)))
    };
    call()
        .map_err(|err| {
            unsafe { *error = ErrorMessage::new(err) };
        })
        .unwrap_or(null_mut())
}

#[unsafe(no_mangle)]
extern "C" fn df_query_meta(query: *mut Query) -> Meta {
    unsafe {
        let query = &*query;
        Meta {
            column_count: query.columns.len(),
            row_count: query.rows.len(),
            duration: query.duration,
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_query_column(query: *mut Query, index: usize) -> Column {
    unsafe {
        let column = &(&*query).columns[index];
        Column {
            name: StringRef::new(&column.name),
            datatype: StringRef::new(&column.datatype),
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_query_value(
    query: *mut Query,
    row: usize,
    col: usize,
) -> TypedValue<DataKind, Data> {
    unsafe {
        match &(&*query).rows[row][col] {
            QueryValue::Null => TypedValue::new(DataKind::Null, Data { null: () }),
            QueryValue::Bool(value) => TypedValue::new(DataKind::Bool, Data { bool: *value }),
            QueryValue::I8(value) => TypedValue::new(DataKind::I8, Data { i8: *value }),
            QueryValue::I16(value) => TypedValue::new(DataKind::I16, Data { i16: *value }),
            QueryValue::I32(value) => TypedValue::new(DataKind::I32, Data { i32: *value }),
            QueryValue::I64(value) => TypedValue::new(DataKind::I64, Data { i64: *value }),
            QueryValue::U8(value) => TypedValue::new(DataKind::U8, Data { u8: *value }),
            QueryValue::U16(value) => TypedValue::new(DataKind::U16, Data { u16: *value }),
            QueryValue::U32(value) => TypedValue::new(DataKind::U32, Data { u32: *value }),
            QueryValue::U64(value) => TypedValue::new(DataKind::U64, Data { u64: *value }),
            QueryValue::F32(value) => TypedValue::new(DataKind::F32, Data { f32: *value }),
            QueryValue::F64(value) => TypedValue::new(DataKind::F64, Data { f64: *value }),
            QueryValue::String(value) => TypedValue::new(
                DataKind::String,
                Data {
                    string: StringRef::new(value),
                },
            ),
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_free_query(query: *mut Query) {
    unsafe {
        let _ = Box::from_raw(query);
    }
}

#[unsafe(no_mangle)]
extern "C" fn df_free_error(error: ErrorMessage) {
    error.free();
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn options(path: &str) -> ConnectOptions {
        ConnectOptions {
            path: StringRef::new(path),
        }
    }

    fn conn() -> *mut Connection {
        let mut error = ErrorMessage::null();
        let conn = df_connect(options("  :memory:  "), &mut error);
        assert!(!conn.is_null());
        assert!(error.is_null());
        conn
    }

    fn query(sql: &str) -> (*mut Query, *mut Connection) {
        let conn = conn();
        let mut error = ErrorMessage::null();
        let query = df_query(conn, StringRef::new(sql), &mut error);
        assert!(!query.is_null());
        assert!(error.is_null());
        (query, conn)
    }

    #[test]
    fn test_close() {
        let conn = conn();
        df_close(conn);
    }

    #[test]
    fn test_query_meta() {
        let (query, conn) = query(
            "select version() as version, null as nothing, toUInt64(42) as count, 'hello' as greeting",
        );

        let meta = df_query_meta(query);
        assert_eq!(meta.column_count, 4);
        assert_eq!(meta.row_count, 1);

        let version_column = df_query_column(query, 0);
        assert_eq!(version_column.name.as_str(), "version");
        assert_eq!(version_column.datatype.as_str(), "String");

        let null_column = df_query_column(query, 1);
        assert_eq!(null_column.datatype.as_str(), "Nullable(Nothing)");

        unsafe {
            assert_eq!(df_query_value(query, 0, 0).kind, DataKind::String);
            assert!(!df_query_value(query, 0, 0).value.string.as_str().is_empty());

            assert_eq!(df_query_value(query, 0, 1).kind, DataKind::Null);
            assert_eq!(df_query_value(query, 0, 2).kind, DataKind::U64);
            assert_eq!(df_query_value(query, 0, 2).value.u64, 42);
            assert_eq!(df_query_value(query, 0, 3).kind, DataKind::String);
            assert_eq!(df_query_value(query, 0, 3).value.string.as_str(), "hello");
        }

        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_bool() {
        let (query, conn) = query("select true, false");
        unsafe {
            assert_eq!(df_query_value(query, 0, 0).kind, DataKind::Bool);
            assert!(df_query_value(query, 0, 0).value.bool);
            assert_eq!(df_query_value(query, 0, 1).kind, DataKind::Bool);
            assert!(!df_query_value(query, 0, 1).value.bool);
        }
        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_u64() {
        let (query, conn) = query("select toUInt64(123), toUInt64(456)");
        unsafe {
            assert_eq!(df_query_value(query, 0, 0).kind, DataKind::U64);
            assert_eq!(df_query_value(query, 0, 0).value.u64, 123);
            assert_eq!(df_query_value(query, 0, 1).kind, DataKind::U64);
            assert_eq!(df_query_value(query, 0, 1).value.u64, 456);
        }
        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_f64() {
        let (query, conn) = query("select toFloat64(123.456), toFloat64(789.012)");
        unsafe {
            assert_eq!(df_query_value(query, 0, 0).kind, DataKind::F64);
            assert_eq!(df_query_value(query, 0, 0).value.f64, 123.456);
            assert_eq!(df_query_value(query, 0, 1).kind, DataKind::F64);
            assert_eq!(df_query_value(query, 0, 1).value.f64, 789.012);
        }
        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_string() {
        let (query, conn) = query("select 'hello', 'world'");
        unsafe {
            assert_eq!(df_query_value(query, 0, 0).kind, DataKind::String);
            assert_eq!(df_query_value(query, 0, 0).value.string.as_str(), "hello");
            assert_eq!(df_query_value(query, 0, 1).kind, DataKind::String);
            assert_eq!(df_query_value(query, 0, 1).value.string.as_str(), "world");
        }
        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_query_empty_result_set() {
        let conn = conn();
        let mut error = ErrorMessage::null();
        let query = df_query(
            conn,
            StringRef::new("select toUInt64(number) as id from numbers(0)"),
            &mut error,
        );
        assert!(error.is_null());
        assert!(!query.is_null());

        let meta = df_query_meta(query);
        assert_eq!(meta.column_count, 1);
        assert_eq!(meta.row_count, 0);

        let column = df_query_column(query, 0);
        assert_eq!(column.name.as_str(), "id");
        assert_eq!(column.datatype.as_str(), "UInt64");

        df_free_query(query);
        df_close(conn);
    }

    #[test]
    fn test_query_error() {
        let conn = conn();
        let mut error = ErrorMessage::null();
        let query = df_query(
            conn,
            StringRef::new("select * from missing_table"),
            &mut error,
        );
        assert!(query.is_null());
        assert!(!error.is_null());

        let message = error.as_str().to_string();
        assert!(
            message.contains("missing_table") || message.contains("UNKNOWN_TABLE"),
            "{message}"
        );

        df_free_error(error);
        df_close(conn);
    }
}
