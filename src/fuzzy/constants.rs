use super::Score;

pub(super) const MATCH_BONUS: Score = 18;
pub(super) const START_OF_STRING_BONUS: Score = 16;
pub(super) const CAMEL_CASE_BONUS: Score = 6;
pub(super) const CONSECUTIVE_BONUS: Score = 11;
pub(super) const GAP_OPEN: Score = 6;
pub(super) const GAP_EXTEND: Score = 4;
pub(super) const TYPO_PENALTY: Score = 10;
pub(super) const MISMATCH_PENALTY: Score = 16;
pub(super) const MAX_PAT_LEN: usize = 32;
pub(super) const TYPO_BAND_SLACK: usize = 4;

pub(super) const SEPARATOR_TABLE: [Score; 128] = {
    let mut t = [0 as Score; 128];
    t[b' ' as usize] = 16;
    t[b'-' as usize] = 10;
    t[b'.' as usize] = 12;
    t[b'/' as usize] = 16;
    t[b'\\' as usize] = 16;
    t[b'_' as usize] = 12;
    t
};
