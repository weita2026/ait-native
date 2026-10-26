//! Lossless JSON inputs. napi's default serde conversion stringifies overflowing
//! BigInts and turns larger ordinary integers into floats; neither is suitable
//! for typed domain requests.
use ait_core::json_support::{JsonMap, JsonValue};
use napi::bindgen_prelude::{
    BigInt, FromNapiValue, Object, ToNapiValue, TypeName, ValidateNapiValue,
};
use napi::{check_status, sys, Error, Result, Status, ValueType};
use std::cell::Cell;

thread_local! { static JSON_DEPTH: Cell<usize> = const { Cell::new(0) }; }

struct JsonDepthGuard;

impl JsonDepthGuard {
    fn enter() -> Result<Self> {
        JSON_DEPTH.with(|depth| {
            if depth.get() >= 128 {
                return Err(Error::new(
                    Status::InvalidArg,
                    "JSON input is cyclic or exceeds 128 nested values.",
                ));
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}

impl Drop for JsonDepthGuard {
    fn drop(&mut self) {
        JSON_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

#[derive(Debug)]
pub struct JsonInput(pub JsonValue);

impl std::ops::Deref for JsonInput {
    type Target = JsonValue;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<JsonValue> for JsonInput {
    fn from(value: JsonValue) -> Self {
        Self(value)
    }
}

impl ToNapiValue for JsonInput {
    unsafe fn to_napi_value(env: sys::napi_env, value: Self) -> Result<sys::napi_value> {
        unsafe { JsonValue::to_napi_value(env, value.0) }
    }
}

impl TypeName for JsonInput {
    fn type_name() -> &'static str {
        "JsonValue"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl ValidateNapiValue for JsonInput {
    // The converter validates each JSON variant, including nested values.
    unsafe fn validate(_: sys::napi_env, _: sys::napi_value) -> Result<sys::napi_value> {
        Ok(std::ptr::null_mut())
    }
}

impl FromNapiValue for JsonInput {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> Result<Self> {
        let _depth = JsonDepthGuard::enter()?;
        let mut kind = 0;
        // SAFETY: napi supplies a live environment and value for this callback.
        check_status!(unsafe { sys::napi_typeof(env, value, &mut kind) })?;
        let payload = match ValueType::from(kind) {
            ValueType::BigInt => {
                let integer = unsafe { BigInt::from_napi_value(env, value) }?;
                if integer.sign_bit {
                    let (number, lossless) = integer.get_i64();
                    if !lossless {
                        return Err(integer_range_error());
                    }
                    JsonValue::from(number)
                } else {
                    let (_, number, lossless) = integer.get_u64();
                    if !lossless {
                        return Err(integer_range_error());
                    }
                    JsonValue::from(number)
                }
            }
            ValueType::Number => {
                let number = unsafe { f64::from_napi_value(env, value) }?;
                if number.is_finite() && number.fract() == 0.0 {
                    if number.abs() > 9_007_199_254_740_991.0 {
                        return Err(Error::new(
                            Status::InvalidArg,
                            "Unsafe JavaScript integer; pass a 64-bit BigInt instead.",
                        ));
                    }
                    if number >= 0.0 {
                        JsonValue::from(number as u64)
                    } else {
                        JsonValue::from(number as i64)
                    }
                } else {
                    unsafe { JsonValue::from_napi_value(env, value) }?
                }
            }
            ValueType::Object => {
                let mut array = false;
                check_status!(unsafe { sys::napi_is_array(env, value, &mut array) })?;
                if array {
                    JsonValue::Array(
                        unsafe { Vec::<JsonInput>::from_napi_value(env, value) }?
                            .into_iter()
                            .map(|item| item.0)
                            .collect(),
                    )
                } else {
                    let object = unsafe { Object::from_napi_value(env, value) }?;
                    let mut fields = JsonMap::new();
                    for key in Object::keys(&object)? {
                        if let Some(item) = object.get::<JsonInput>(&key)? {
                            fields.insert(key, item.0);
                        }
                    }
                    JsonValue::Object(fields)
                }
            }
            _ => unsafe { JsonValue::from_napi_value(env, value) }?,
        };
        Ok(Self(payload))
    }
}

fn integer_range_error() -> Error {
    Error::new(
        Status::InvalidArg,
        "BigInt is outside the native JSON i64/u64 range.",
    )
}
