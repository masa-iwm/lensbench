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

    /// A lens without a name gets its own line, so no English is left in the Japanese one
    #[test]
    fn unnamed_lens_edge_note_is_all_japanese() {
        register();
        let tail = "between faces 1 and 2 isn't made (not coaxial or not a round aperture)";
        let ja = lensbench_core::i18n::translate(&format!("scene: the edge of a lens {tail}"));
        assert!(ja.starts_with("場面: レンズの面 1 と 2"), "{ja}");
        let ja = lensbench_core::i18n::translate(&format!("scene: the edge of L1 {tail}"));
        assert!(ja.starts_with("場面: L1 の面 1 と 2"), "{ja}");
    }
}
