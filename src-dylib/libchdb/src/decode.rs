// From: /src-crates/clickhouse/src/decode.rs

use crate::connection::QueryValue as Value;
use std::str::FromStr;

pub(super) fn decode_value(val: String, datatype: &str) -> Value {
    if val == "ᴺᵁᴸᴸ" && datatype.starts_with("Nullable(") {
        return Value::Null;
    }
    match datatype {
        "Bool" | "Nullable(Bool)" => decode(val, Value::Bool),
        "Int8" | "Nullable(Int8)" => decode(val, Value::I8),
        "UInt8" | "Nullable(UInt8)" => decode(val, Value::U8),
        "Int16" | "Nullable(Int16)" => decode(val, Value::I16),
        "UInt16" | "Nullable(UInt16)" => decode(val, Value::U16),
        "Int32" | "Nullable(Int32)" => decode(val, Value::I32),
        "UInt32" | "Nullable(UInt32)" => decode(val, Value::U32),
        "Int64" | "Nullable(Int64)" => decode(val, Value::I64),
        "UInt64" | "Nullable(UInt64)" => decode(val, Value::U64),
        "Float32" | "Nullable(Float32)" => decode(val, Value::F32),
        "Float64" | "Nullable(Float64)" => decode(val, Value::F64),
        _ => Value::String(val),
    }
}

#[inline]
fn decode<T: FromStr>(val: String, f: fn(T) -> Value) -> Value {
    match val.parse::<T>() {
        Ok(val) => f(val),
        Err(_) => Value::String(val),
    }
}
