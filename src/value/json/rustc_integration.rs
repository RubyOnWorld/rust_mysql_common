use rustc_serialize::{Decodable, Encodable};
use rustc_serialize::json::{self, Json};
use super::{Serialized, Deserialized, DeserializedIr};
use value::Value;
use value::convert::{ConvIr, FromValue, FromValueError};
use std::str::{from_utf8, from_utf8_unchecked};

impl From<Json> for Value {
    fn from(x: Json) -> Value {
        Value::Bytes(json::encode(&x).unwrap().into())
    }
}

impl<T: Encodable> From<Serialized<T>> for Value {
    fn from(x: Serialized<T>) -> Value {
        Value::Bytes(json::encode(&x.0).unwrap().into())
    }
}


impl<T: Decodable> ConvIr<Deserialized<T>> for DeserializedIr<T> {
    fn new(v: Value) -> MyResult<DeserializedIr<T>> {
        let (output, bytes) = {
            let bytes = match v {
                Value::Bytes(bytes) => {
                    match from_utf8(&*bytes) {
                        Ok(_) => bytes,
                        Err(_) => return Err(Error::FromValueError(Value::Bytes(bytes))),
                    }
                }
                v => return Err(Error::FromValueError(v)),
            };
            let output = {
                match json::decode(unsafe { from_utf8_unchecked(&*bytes) }) {
                    Ok(output) => output,
                    Err(_) => return Err(Error::FromValueError(Value::Bytes(bytes))),
                }
            };
            (output, bytes)
        };
        Ok(DeserializedIr {
            bytes: bytes,
            output: Deserialized(output),
        })
    }

    fn commit(self) -> Deserialized<T> {
        self.output
    }

    fn rollback(self) -> Value {
        Value::Bytes(self.bytes)
    }
}

impl<T: Decodable> FromValue for Deserialized<T> {
    type Intermediate = DeserializedIr<T>;
}

/// Intermediate result of a Value-to-Json conversion.
#[derive(Debug)]
pub struct JsonIr {
    bytes: Vec<u8>,
    output: Json,
}

impl ConvIr<Json> for JsonIr {
    fn new(v: Value) -> MyResult<JsonIr> {
        let (output, bytes) = {
            let bytes = match v {
                Value::Bytes(bytes) => {
                    match from_utf8(&*bytes) {
                        Ok(_) => bytes,
                        Err(_) => return Err(Error::FromValueError(Value::Bytes(bytes))),
                    }
                }
                v => return Err(Error::FromValueError(v)),
            };
            let output = {
                match Json::from_str(unsafe { from_utf8_unchecked(&*bytes) }) {
                    Ok(output) => output,
                    Err(_) => return Err(Error::FromValueError(Value::Bytes(bytes))),
                }
            };
            (output, bytes)
        };
        Ok(JsonIr {
            bytes: bytes,
            output: output,
        })
    }

    fn commit(self) -> Json {
        self.output
    }

    fn rollback(self) -> Value {
        Value::Bytes(self.bytes)
    }
}

impl FromValue for Json {
    type Intermediate = JsonIr;
}
