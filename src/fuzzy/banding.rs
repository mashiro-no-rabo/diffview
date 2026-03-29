//! Banding utils

use super::atom::Atom;
use super::constants::{MAX_PAT_LEN, TYPO_BAND_SLACK};
use super::helpers::{compute_last_match_cols, compute_row_col_bounds, find_first_char};

/// Precomputed banding information shared by both score-only and full DP.
pub(super) struct BandingInfo {
    pub(super) row_bounds: Option<([usize; MAX_PAT_LEN], [usize; MAX_PAT_LEN])>,
    pub(super) j_first: usize,
    pub(super) bandwidth: usize,
    pub(super) min_true_matches: usize,
}

pub(super) fn compute_banding<const ALLOW_TYPOS: bool, C: Atom>(
    pat: &[C],
    cho: &[C],
    respect_case: bool,
) -> Option<BandingInfo> {
    let n = pat.len();
    let m = cho.len();
    let row_bounds;
    let j_first;

    if ALLOW_TYPOS {
        j_first = find_first_char(pat, cho, respect_case)?;
        row_bounds = None;
    } else {
        let fm = compute_first_match_cols(pat, cho, respect_case)?;
        let lm = compute_last_match_cols(pat, cho, respect_case)?;
        j_first = fm[0];
        row_bounds = Some(compute_row_col_bounds(n, m, &fm, &lm));
    }

    let bandwidth = if ALLOW_TYPOS { n + TYPO_BAND_SLACK } else { 0 };
    let min_true_matches = if ALLOW_TYPOS { n.div_ceil(2) } else { 0 };

    Some(BandingInfo {
        row_bounds,
        j_first,
        bandwidth,
        min_true_matches,
    })
}

#[inline(always)]
pub(super) fn typo_vband_row(
    i: usize,
    m: usize,
    bandwidth: usize,
    j_first: usize,
) -> (usize, usize) {
    let j = i + j_first - 1;
    let lo = j.saturating_sub(bandwidth).max(j_first);
    (lo, m)
}

fn compute_first_match_cols<C: Atom>(
    pat: &[C],
    cho: &[C],
    respect_case: bool,
) -> Option<[usize; MAX_PAT_LEN]> {
    let n = pat.len();
    if n > MAX_PAT_LEN {
        return None;
    }
    let mut first = [0usize; MAX_PAT_LEN];
    let mut start = 0usize;
    for i in 0..n {
        let found = cho[start..].iter().position(|&c| pat[i].eq(c, respect_case));
        match found {
            Some(pos) => {
                first[i] = start + pos + 1;
                start = start + pos + 1;
            }
            None => return None,
        }
    }
    Some(first)
}
