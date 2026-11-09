//! Web Worker との受け渡しの形。画面の側と計算の側（別の wasm のインスタンス）で、
//! 要求（呼び出しの並び）・結果・ファイルの変更（[`lensbench_core::vfs`]）をバイト列にする。
//!
//! 形はリトルエンディアンの長さ付き（u32 の長さ＋中身）。JSON は文字列のまま、配列は f64 の並び。

use serde_json::Value;

use crate::worker::Arrays;

/// ファイルの変更（パス、中身。None は消した）
pub type Files = Vec<(String, Option<Vec<u8>>)>;

/// 計算の側へ渡す要求
#[derive(Debug, PartialEq)]
pub struct Request {
    pub threads: Option<usize>,
    pub calls: Vec<(String, Value)>,
    pub files: Files,
}

/// 計算の側から戻る結果（呼び出しごと。エラーで止めたらそこまで）
#[derive(Debug, PartialEq)]
pub struct Response {
    pub results: Vec<Result<(Value, Arrays), String>>,
    pub ms: f64,
    pub files: Files,
}

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: usize) {
        self.0.extend_from_slice(&(v as u32).to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len());
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn files(&mut self, f: &Files) {
        self.u32(f.len());
        for (p, b) in f {
            self.str(p);
            match b {
                Some(b) => {
                    self.0.push(1);
                    self.bytes(b);
                }
                None => self.0.push(0),
            }
        }
    }
}

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("受け渡しのデータが途中で切れている".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<usize, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()?;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?).map_err(|e| e.to_string())
    }
    fn f64(&mut self) -> Result<f64, String> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_le_bytes(a))
    }
    fn json(&mut self) -> Result<Value, String> {
        serde_json::from_slice(&self.bytes()?).map_err(|e| e.to_string())
    }
    fn files(&mut self) -> Result<Files, String> {
        let n = self.u32()?;
        let mut out = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            let p = self.str()?;
            let b = match self.u8()? {
                0 => None,
                _ => Some(self.bytes()?),
            };
            out.push((p, b));
        }
        Ok(out)
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.u32(self.threads.map_or(0, |t| t + 1));
        w.u32(self.calls.len());
        for (m, p) in &self.calls {
            w.str(m);
            w.str(&p.to_string());
        }
        w.files(&self.files);
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Request, String> {
        let mut r = R(b);
        let threads = r.u32()?.checked_sub(1);
        let n = r.u32()?;
        let mut calls = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            let m = r.str()?;
            calls.push((m, r.json()?));
        }
        let files = r.files()?;
        Ok(Request {
            threads,
            calls,
            files,
        })
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.f64(self.ms);
        w.u32(self.results.len());
        for x in &self.results {
            match x {
                Ok((v, arrays)) => {
                    w.0.push(1);
                    w.str(&v.to_string());
                    w.u32(arrays.len());
                    for (name, a) in arrays {
                        w.str(name);
                        w.u32(a.len());
                        for &f in a {
                            w.f64(f);
                        }
                    }
                }
                Err(e) => {
                    w.0.push(0);
                    w.str(e);
                }
            }
        }
        w.files(&self.files);
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Response, String> {
        let mut r = R(b);
        let ms = r.f64()?;
        let n = r.u32()?;
        let mut results = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            if r.u8()? == 0 {
                results.push(Err(r.str()?));
                continue;
            }
            let v = r.json()?;
            let na = r.u32()?;
            let mut arrays = Vec::with_capacity(na.min(1 << 16));
            for _ in 0..na {
                let name = r.str()?;
                let len = r.u32()?;
                let mut a = Vec::with_capacity(len.min(r.0.len() / 8));
                for _ in 0..len {
                    a.push(r.f64()?);
                }
                arrays.push((name, a));
            }
            results.push(Ok((v, arrays)));
        }
        let files = r.files()?;
        Ok(Response { results, ms, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip() {
        let q = Request {
            threads: Some(0),
            calls: vec![
                (
                    "load".into(),
                    json!({"file": "/uploads/1/a.zmx", "x": [1.5, null]}),
                ),
                ("info".into(), Value::Null),
            ],
            files: vec![
                ("/uploads/1/a.zmx".into(), Some(b"VERS".to_vec())),
                ("/tmp/x".into(), None),
            ],
        };
        assert_eq!(Request::decode(&q.encode()).unwrap(), q);
        let q2 = Request {
            threads: None,
            calls: vec![],
            files: vec![],
        };
        assert_eq!(Request::decode(&q2.encode()).unwrap(), q2);

        let a = Response {
            results: vec![
                Ok((
                    json!({"ok": true}),
                    vec![("x".into(), vec![1.0, -2.5, f64::MAX])],
                )),
                Err("だめ".into()),
            ],
            ms: 12.5,
            files: vec![("/downloads/a.zmx".into(), Some(vec![0, 255]))],
        };
        assert_eq!(Response::decode(&a.encode()).unwrap(), a);
        // 切れたデータはエラー（panic しない）
        let b = a.encode();
        for n in 0..b.len() {
            assert!(Response::decode(&b[..n]).is_err());
        }
    }
}
