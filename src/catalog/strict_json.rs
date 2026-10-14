use crate::catalog::types::CatalogError;
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

pub struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a valid JSON value without duplicate keys")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if !v.is_finite() {
            return Err(serde::de::Error::custom("Nonfinite JSON constant"));
        }
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| serde::de::Error::custom("Nonfinite JSON constant"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element_seed(StrictValueSeed)? {
            vec.push(elem);
        }
        Ok(Value::Array(vec))
    }

    fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut map = serde_json::Map::new();
        let mut seen = HashSet::new();

        while let Some(key) = access.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(serde::de::Error::custom("Duplicate JSON property"));
            }
            let value = access.next_value_seed(StrictValueSeed)?;
            map.insert(key, value);
        }

        if map.len() == 1 {
            if let Some(Value::String(s)) = map.remove("$serde_json::private::Number") {
                let num: serde_json::Number = serde_json::from_str(&s)
                    .map_err(|e| serde::de::Error::custom(format!("Invalid raw number: {e}")))?;
                return Ok(Value::Number(num));
            }
            if let Some(Value::String(s)) = map.remove("$serde_json::private::RawValue") {
                let num: serde_json::Number = serde_json::from_str(&s)
                    .map_err(|e| serde::de::Error::custom(format!("Invalid raw number: {e}")))?;
                return Ok(Value::Number(num));
            }
        }

        Ok(Value::Object(map))
    }
}

pub struct StrictValueSeed;

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

pub fn parse_strict_json(text: &str) -> Result<Value, CatalogError> {
    let mut de = serde_json::Deserializer::from_str(text);
    let val = StrictValueSeed
        .deserialize(&mut de)
        .map_err(|e| CatalogError(format!("JSON parse error: {e}")))?;
    de.end()
        .map_err(|e| CatalogError(format!("JSON trailing data: {e}")))?;
    Ok(val)
}
