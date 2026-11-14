//! serde の既定値（項目を省いたときの値）

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn is_yes(b: &bool) -> bool {
    *b
}

pub(crate) fn one() -> usize {
    1
}

pub(crate) fn default_catalogs() -> Vec<String> {
    vec!["SCHOTT".into()]
}

pub(crate) fn wave_weight() -> f64 {
    1.0
}

pub(crate) fn one_f64() -> f64 {
    1.0
}
