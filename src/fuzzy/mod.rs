//! Arinae fuzzy matching algorithm.
//!
//! Adapted from the skim fuzzy finder's arinae module.
//! Uses a Smith-Waterman local alignment approach with affine gap penalties
//! and context-sensitive bonuses.

#![allow(clippy::inline_always)]
#![allow(dead_code)]

mod algo;
mod atom;
mod banding;
mod constants;
mod helpers;
mod matrix;
mod prefilter;

use std::cell::RefCell;

use thread_local::ThreadLocal;

use self::algo::full_dp;
use self::atom::Atom;
use self::constants::{CAMEL_CASE_BONUS, START_OF_STRING_BONUS};
use self::matrix::SWMatrix;

pub(crate) type IndexType = usize;
pub(crate) type ScoreType = i64;
pub(crate) type MatchIndices = Vec<IndexType>;

type Score = i16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CaseMatching {
    Respect,
    Ignore,
    #[default]
    Smart,
}

fn precompute_bonuses<C: Atom>(cho: &[C], buf: &mut Vec<Score>) {
    buf.clear();
    let bonus_iter = std::iter::once(START_OF_STRING_BONUS).chain(cho.windows(2).map(|w| {
        let prev = w[0];
        let cur = w[1];
        prev.separator_bonus()
            + CAMEL_CASE_BONUS * Score::from(prev.is_lowercase() && !cur.is_lowercase())
    }));
    buf.extend(bonus_iter);
}

/// Arinae fuzzy matcher: Smith-Waterman local alignment with affine gap
/// penalties and context-sensitive bonuses.
#[derive(Debug, Default)]
pub(crate) struct ArinaeMatcher {
    case: CaseMatching,

    full_buf: ThreadLocal<RefCell<SWMatrix>>,
    indices_buf: ThreadLocal<RefCell<MatchIndices>>,
    #[allow(clippy::type_complexity)]
    char_buf: ThreadLocal<RefCell<(Vec<char>, Vec<char>)>>,
    bonus_buf: ThreadLocal<RefCell<Vec<Score>>>,
}

impl ArinaeMatcher {
    pub fn new(case: CaseMatching) -> Self {
        Self {
            case,
            ..Default::default()
        }
    }

    #[inline(always)]
    fn respect_case<C: Atom>(&self, pattern: &[C]) -> bool {
        self.case == CaseMatching::Respect
            || (self.case == CaseMatching::Smart && !pattern.iter().all(|b| b.is_lowercase()))
    }

    fn dispatch_dp<C: Atom>(
        &self,
        cho: &[C],
        pat: &[C],
        bonuses: &[Score],
        respect_case: bool,
        compute_indices: bool,
    ) -> Option<(ScoreType, MatchIndices)> {
        let res = match compute_indices {
            true => full_dp::<false, true, _>(
                cho,
                pat,
                bonuses,
                respect_case,
                &self.full_buf,
                &self.indices_buf,
                false,
            ),
            false => full_dp::<false, false, _>(
                cho,
                pat,
                bonuses,
                respect_case,
                &self.full_buf,
                &self.indices_buf,
                false,
            ),
        };
        res.map(|(s, idx)| (ScoreType::from(s), idx))
    }

    fn match_slices<C: Atom>(
        &self,
        cho: &[C],
        pat: &[C],
        compute_indices: bool,
    ) -> Option<(ScoreType, MatchIndices)> {
        if pat.is_empty() {
            return Some((0, MatchIndices::new()));
        }
        if cho.is_empty() {
            return None;
        }

        let respect_case = self.respect_case(pat);

        let mut bonus_buf = self
            .bonus_buf
            .get_or(|| RefCell::new(Vec::new()))
            .borrow_mut();
        precompute_bonuses(cho, &mut bonus_buf);

        self.dispatch_dp(cho, pat, &bonus_buf, respect_case, compute_indices)
    }

    fn run(
        &self,
        choice: &str,
        pattern: &str,
        compute_indices: bool,
    ) -> Option<(ScoreType, MatchIndices)> {
        if pattern.is_empty() {
            return Some((0, MatchIndices::new()));
        }
        if choice.is_empty() {
            return None;
        }

        if choice.is_ascii() && pattern.is_ascii() {
            let cho = choice.as_bytes();
            let pat = pattern.as_bytes();
            return self.match_slices(cho, pat, compute_indices);
        }

        let mut bufs = self
            .char_buf
            .get_or(|| RefCell::new((Vec::new(), Vec::new())))
            .borrow_mut();
        let (ref mut pat_buf, ref mut cho_buf) = *bufs;
        pat_buf.clear();
        pat_buf.extend(pattern.chars());
        cho_buf.clear();
        cho_buf.extend(choice.chars());

        let respect_case = self.respect_case(pat_buf);

        let mut bonus_buf = self
            .bonus_buf
            .get_or(|| RefCell::new(Vec::new()))
            .borrow_mut();
        precompute_bonuses(cho_buf, &mut bonus_buf);

        self.dispatch_dp(cho_buf, pat_buf, &bonus_buf, respect_case, compute_indices)
    }

    /// Fuzzy match and return score only.
    pub fn fuzzy_match(&self, choice: &str, pattern: &str) -> Option<ScoreType> {
        self.run(choice, pattern, false).map(|x| x.0)
    }

    /// Fuzzy match and return score + matched character indices.
    pub fn fuzzy_indices(&self, choice: &str, pattern: &str) -> Option<(ScoreType, MatchIndices)> {
        self.run(choice, pattern, true)
    }
}
