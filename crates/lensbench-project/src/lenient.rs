//! 系ファイルの JSON の値を型に読む（IOO-32）。
//!
//! JSON は inf・NaN を書けないので、系ファイルは `{"$f": "inf"}` で書き、読むときは CBOR の値を経由する
//! （[`crate::sysfile::from_json`]）。ciborium の値の読み手は整数を実数の欄に入れないので、手や Python が `"t": 3` と
//! 書いた系ファイルが読めなかった。ここでは値の形（整数・実数・文字列・配列・表）を見て読む側に渡すので、実数の欄に
//! 整数の書き方でも読める（serde の実数は整数を受ける）。誤りには欄の道（`surfaces[1].t`）を付ける。
use ciborium::value::Value as C;
use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, Visitor};
use std::fmt;

/// 欄の道つきの誤り
#[derive(Debug)]
pub struct Error {
    path: Vec<String>,
    msg: String,
}

impl Error {
    /// 外側の欄の名前（表のキー・配列の番号）を前に足す
    fn within(mut self, seg: String) -> Error {
        self.path.push(seg);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            return f.write_str(&self.msg);
        }
        let mut p = String::new();
        for s in self.path.iter().rev() {
            if s.starts_with('[') || p.is_empty() {
                p.push_str(s);
            } else {
                p.push('.');
                p.push_str(s);
            }
        }
        write!(f, "{p}: {}", self.msg)
    }
}

impl std::error::Error for Error {}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error {
            path: Vec::new(),
            msg: msg.to_string(),
        }
    }
}

/// CBOR の値を型に読む（実数の欄は整数でもよい）。誤りの文は「root.欄の道: 理由」（道が無ければ「root: 理由」）
pub fn decode<T: DeserializeOwned>(v: C, root: &str) -> Result<T, String> {
    T::deserialize(Lenient(v)).map_err(|e| {
        let sep = match e.path.last() {
            None => ": ",
            Some(s) if s.starts_with('[') => "",
            Some(_) => ".",
        };
        format!("{root}{sep}{e}")
    })
}

struct Lenient(C);

impl<'de> de::Deserializer<'de> for Lenient {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, Error> {
        match self.0 {
            C::Null => v.visit_unit(),
            C::Bool(b) => v.visit_bool(b),
            C::Integer(i) => {
                let i = i128::from(i);
                if let Ok(u) = u64::try_from(i) {
                    v.visit_u64(u)
                } else if let Ok(s) = i64::try_from(i) {
                    v.visit_i64(s)
                } else {
                    v.visit_i128(i)
                }
            }
            C::Float(x) => v.visit_f64(x),
            C::Text(s) => v.visit_string(s),
            C::Bytes(b) => v.visit_byte_buf(b),
            C::Array(a) => v.visit_seq(Seq(a.into_iter().enumerate())),
            C::Map(m) => v.visit_map(MapA {
                it: m.into_iter(),
                next: None,
            }),
            C::Tag(_, x) => Lenient(*x).deserialize_any(v),
            _ => Err(de::Error::custom("読めない値")),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value, Error> {
        match self.0 {
            C::Null => v.visit_none(),
            x => v.visit_some(Lenient(x)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value, Error> {
        v.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            C::Text(s) => v.visit_enum(s.into_deserializer()),
            C::Map(m) if m.len() == 1 => {
                let (k, x) = m.into_iter().next().expect("1 つ");
                let C::Text(k) = k else {
                    return Err(de::Error::custom("列挙の名前が文字列でない"));
                };
                v.visit_enum(Variant(k, x))
            }
            _ => Err(de::Error::custom("列挙は文字列か、1 つのキーの表")),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

struct Seq(std::iter::Enumerate<std::vec::IntoIter<C>>);

impl<'de> de::SeqAccess<'de> for Seq {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.0.next() {
            None => Ok(None),
            Some((i, x)) => seed
                .deserialize(Lenient(x))
                .map(Some)
                .map_err(|e| e.within(format!("[{i}]"))),
        }
    }
}

struct MapA {
    it: std::vec::IntoIter<(C, C)>,
    next: Option<(String, C)>,
}

impl<'de> de::MapAccess<'de> for MapA {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        let Some((k, v)) = self.it.next() else {
            return Ok(None);
        };
        let name = match &k {
            C::Text(s) => s.clone(),
            other => format!("{other:?}"),
        };
        self.next = Some((name, v));
        seed.deserialize(Lenient(k)).map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let (name, v) = self
            .next
            .take()
            .ok_or_else(|| de::Error::custom("値の前にキーがない"))?;
        seed.deserialize(Lenient(v)).map_err(|e| e.within(name))
    }
}

struct Variant(String, C);

impl<'de> de::EnumAccess<'de> for Variant {
    type Error = Error;
    type Variant = Lenient;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Lenient), Error> {
        let name = self.0.clone();
        let k = seed.deserialize(name.into_deserializer())?;
        Ok((k, Lenient(self.1)))
    }
}

impl<'de> de::VariantAccess<'de> for Lenient {
    type Error = Error;
    fn unit_variant(self) -> Result<(), Error> {
        match self.0 {
            C::Null => Ok(()),
            _ => Err(de::Error::custom("値のない列挙に値がある")),
        }
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(self)
    }
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, v: V) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_any(self, v)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_any(self, v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct S {
        t: f64,
        n: usize,
        v: Vec<f64>,
        o: Option<f64>,
        e: E,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    enum E {
        A,
        B { x: f64 },
    }

    fn c(v: serde_json::Value) -> C {
        crate::sysfile::from_json(v).unwrap()
    }

    #[test]
    fn integers_in_float_fields_and_paths() {
        let s: S = decode(
            c(serde_json::json!({
                "t": 3, "n": 2, "v": [0, 2e-5, {"$f": "inf"}], "o": null, "e": {"B": {"x": 1}}
            })),
            "s",
        )
        .unwrap();
        assert_eq!((s.t, s.n, s.o), (3.0, 2, None));
        assert_eq!(s.v[..2], [0.0, 2e-5]);
        assert!(s.v[2].is_infinite());
        assert_eq!(s.e, E::B { x: 1.0 });
        let a: E = decode(c(serde_json::json!("A")), "e").unwrap();
        assert_eq!(a, E::A);
        let e = decode::<Vec<S>>(
            c(serde_json::json!([
                { "t": 1, "n": 1, "v": [], "o": 1, "e": "A" },
                { "t": "x", "n": 1, "v": [], "o": 1, "e": "A" },
            ])),
            "v",
        )
        .unwrap_err();
        assert!(e.starts_with("v[1].t: "), "{e}");
        // 一番外で足りない欄は道なし
        let e = decode::<S>(c(serde_json::json!({})), "s").unwrap_err();
        assert!(e.starts_with("s: missing field"), "{e}");
    }
}
