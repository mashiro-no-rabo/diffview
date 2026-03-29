//! &[dyn Atom] manipulation helpers

use super::Atom;
use super::constants::MAX_PAT_LEN;

#[inline(always)]
pub(super) fn find_first_char<C: Atom>(pat: &[C], cho: &[C], respect_case: bool) -> Option<usize> {
    pat[0].find_first_in(cho, respect_case).map(|idx| idx + 1)
}

pub(super) fn compute_last_match_cols<C: Atom>(
    pat: &[C],
    cho: &[C],
    respect_case: bool,
) -> Option<[usize; MAX_PAT_LEN]> {
    let n = pat.len();
    if n > MAX_PAT_LEN {
        return None;
    }
    let m = cho.len();
    let mut last = [0usize; MAX_PAT_LEN];
    let mut end = m;
    for i in (0..n).rev() {
        let found = cho[..end].iter().rposition(|&c| pat[i].eq(c, respect_case));
        match found {
            Some(pos) => {
                last[i] = pos + 1;
                end = pos;
            }
            None => return None,
        }
    }
    Some(last)
}

pub(super) fn compute_row_col_bounds(
    n: usize,
    m: usize,
    first_match: &[usize; MAX_PAT_LEN],
    last_match: &[usize; MAX_PAT_LEN],
) -> ([usize; MAX_PAT_LEN], [usize; MAX_PAT_LEN]) {
    let mut lo = [0usize; MAX_PAT_LEN];
    let mut hi = [0usize; MAX_PAT_LEN];

    lo[..n].copy_from_slice(&first_match[..n]);
    hi[..n].copy_from_slice(&last_match[..n]);

    for i in 0..n.saturating_sub(1) {
        let next_lo = lo[i + 1];
        if next_lo > 1 {
            hi[i] = hi[i].max(next_lo - 1);
        }
    }

    for i in 1..n {
        lo[i] = lo[i].min(hi[i - 1] + 1);
    }

    for i in 0..n {
        lo[i] = lo[i].max(1).min(m);
        hi[i] = hi[i].max(lo[i]).min(m);
    }

    (lo, hi)
}
