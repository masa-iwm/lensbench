//! The Japanese translation table of the messages of the computing crates (`lang/ja.tsv`, the format of
//! [`lensbench_core::i18n`]).
//!
//! The table lives outside `lensbench-core` so that adding a line rebuilds only this crate and the entry
//! points, not every crate that depends on the core. The entry points (CLI, GUI and its Web version,
//! Python) call [`register`] at start-up. A program that uses the computing crates directly calls it too,
//! otherwise the messages stay in English.

use std::sync::Once;

/// The table: `English template<TAB>Japanese template` per line
pub const JA: &str = include_str!("../lang/ja.tsv");

/// Add [`JA`] to the translation tables ([`lensbench_core::i18n::add_table`]). Calling it again does nothing
pub fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| lensbench_core::i18n::add_table(JA));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line has the same `{…}` in English and Japanese (the rules of [`lensbench_core::i18n::check_table`])
    #[test]
    fn every_line_has_the_same_slots() {
        lensbench_core::i18n::check_table(JA).unwrap_or_else(|e| panic!("lang/ja.tsv {e}"));
    }

    #[test]
    fn registered_messages_are_translated() {
        register();
        register();
        assert_eq!(lensbench_core::i18n::translate("cancelled"), "中断しました");
    }
}
