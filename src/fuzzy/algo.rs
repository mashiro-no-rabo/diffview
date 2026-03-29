//! Arinae's algo itself

use std::cell::RefCell;

use thread_local::ThreadLocal;

use super::{IndexType, MatchIndices};

use super::banding::{compute_banding, typo_vband_row};
use super::constants::{
    CONSECUTIVE_BONUS, GAP_EXTEND, GAP_OPEN, MATCH_BONUS, MAX_PAT_LEN, MISMATCH_PENALTY,
    TYPO_PENALTY,
};
use super::atom::Atom;
use super::matrix::{CELL_ZERO, Cell, Dir, SWMatrix};
use super::Score;

/// Core cell scoring kernel shared by both score-only and full DP.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
fn compute_cell<const ALLOW_TYPOS: bool>(
    is_match: bool,
    is_first: bool,
    bonus_j: Score,
    diag_score: Score,
    diag_was_diag: bool,
    up_score: Score,
    left_score: Score,
    left_was_diag: bool,
) -> (Score, Dir) {
    let bonus =
        (bonus_j + CONSECUTIVE_BONUS * Score::from(diag_was_diag)) * (1 + Score::from(is_first));

    let match_val = (diag_score + MATCH_BONUS + bonus) * Score::from(is_match);
    let mismatch_val = if ALLOW_TYPOS {
        (diag_score - MISMATCH_PENALTY) * Score::from(!is_match)
    } else {
        0
    };
    let diag_val = match_val + mismatch_val;

    let up_val = if ALLOW_TYPOS {
        up_score - TYPO_PENALTY
    } else {
        0
    };

    let left_val = left_score - (GAP_EXTEND + (GAP_OPEN - GAP_EXTEND) * Score::from(left_was_diag));

    let best = diag_val.max(up_val).max(left_val);

    let diag_wins = if ALLOW_TYPOS {
        diag_val >= up_val && diag_val >= left_val
    } else {
        is_match && diag_val >= left_val
    };
    let up_wins = ALLOW_TYPOS && !diag_wins && up_val >= left_val;

    let dir_bits: u8 = Dir::Left as u8 - u8::from(up_wins) - u8::from(diag_wins) * 2;
    let positive = best > 0;
    let dir_val = dir_bits & u8::from(positive).wrapping_neg();

    // SAFETY: dir_val is in 0..=3 because of the construction above.
    let dir: Dir = unsafe { std::mem::transmute(dir_val) };

    (best, dir)
}

/// Full DP for byte slices using packed cells.
#[allow(clippy::too_many_lines)]
pub(super) fn full_dp<const ALLOW_TYPOS: bool, const COMPUTE_INDICES: bool, C: Atom>(
    cho: &[C],
    pat: &[C],
    bonuses: &[Score],
    respect_case: bool,
    full_buf: &ThreadLocal<RefCell<SWMatrix>>,
    indices_buf: &ThreadLocal<RefCell<MatchIndices>>,
    use_last_match: bool,
) -> Option<(Score, MatchIndices)> {
    let n = pat.len();
    let m = cho.len();

    let banding = compute_banding::<ALLOW_TYPOS, C>(pat, cho, respect_case)?;
    let j_start = banding.j_first;

    let col_off = j_start - 1;
    let mcols = m - col_off + 1;

    let mut buf = full_buf
        .get_or(|| RefCell::new(SWMatrix::zero(n + 1, mcols)))
        .borrow_mut();
    buf.resize(n + 1, mcols);

    let base_ptr = buf.data.as_mut_ptr();
    let cols = buf.cols;

    unsafe {
        std::ptr::write_bytes(base_ptr, 0, mcols);
        for i in 1..=n {
            *base_ptr.add(i * cols) = CELL_ZERO;
        }
    }

    let (row_lo_arr, row_hi_arr) = if ALLOW_TYPOS {
        ([0usize; MAX_PAT_LEN], [0usize; MAX_PAT_LEN])
    } else {
        let (lo, hi) = banding.row_bounds.as_ref().unwrap();
        (*lo, *hi)
    };

    let cho_ptr = cho.as_ptr();
    let bonuses_ptr = bonuses.as_ptr();

    for i in 1..=n {
        let pi = pat[i - 1];
        let is_first = i == 1;

        let (j_lo, j_hi) = typo_vband_row(i, m, banding.bandwidth, banding.j_first);

        if j_lo > j_hi || j_lo > m {
            if i < n {
                let (nj_lo, nj_hi) = if ALLOW_TYPOS {
                    typo_vband_row(i + 1, m, banding.bandwidth, banding.j_first)
                } else {
                    (row_lo_arr[i], row_hi_arr[i])
                };
                let nj_lo = nj_lo.max(j_start);
                if nj_lo <= nj_hi && nj_lo <= m {
                    let next_mat_lo = nj_lo - col_off;
                    let next_mat_hi = (nj_hi - col_off).min(mcols - 1);
                    let zero_lo = next_mat_lo.saturating_sub(1);
                    let zero_hi = next_mat_hi.min(mcols - 1);
                    unsafe {
                        let row_ptr = base_ptr.add(i * cols);
                        for k in zero_lo..=zero_hi {
                            *row_ptr.add(k) = CELL_ZERO;
                        }
                    }
                }
            }
            continue;
        }

        let mat_col_lo = j_lo - col_off;
        let mat_col_hi = j_hi - col_off;
        let jm_max = mcols - 1;

        unsafe {
            let row_ptr = base_ptr.add(i * cols);
            if mat_col_lo > 1 {
                *row_ptr.add(mat_col_lo - 1) = CELL_ZERO;
            }
            if mat_col_hi < jm_max {
                *row_ptr.add(mat_col_hi + 1) = CELL_ZERO;
            }
        }

        let (prev_row, cur_row) = unsafe {
            let pr = std::slice::from_raw_parts(base_ptr.add((i - 1) * cols), cols);
            let cr = std::slice::from_raw_parts_mut(base_ptr.add(i * cols), cols);
            (pr, cr)
        };

        let prev_ptr = prev_row.as_ptr();
        let cur_ptr = cur_row.as_mut_ptr();

        for j in j_lo..=j_hi {
            let jm = j - col_off;
            let cj = unsafe { *cho_ptr.add(j - 1) };
            let is_match = pi.eq(cj, respect_case);

            let diag_cell = unsafe { *prev_ptr.add(jm - 1) };
            let up_score = if ALLOW_TYPOS {
                let up_cell = unsafe { *prev_ptr.add(jm) };
                up_cell.score()
            } else {
                0
            };
            let left_cell = unsafe { *cur_ptr.add(jm - 1) };

            let (best, dir) = compute_cell::<ALLOW_TYPOS>(
                is_match,
                is_first,
                unsafe { *bonuses_ptr.add(j - 1) },
                diag_cell.score(),
                diag_cell.is_diag(),
                up_score,
                left_cell.score(),
                left_cell.is_diag(),
            );

            unsafe {
                *cur_ptr.add(jm) = Cell::new(best, dir);
            }
        }
    }

    let mut best_score: Score = 0;
    let mut best_j = 0usize;
    {
        let (last_j_lo_raw, last_j_hi) = if ALLOW_TYPOS {
            typo_vband_row(n, m, banding.bandwidth, banding.j_first)
        } else {
            (row_lo_arr[n - 1], row_hi_arr[n - 1])
        };
        let last_j_lo = last_j_lo_raw.max(j_start);
        let last_row_ptr = unsafe { base_ptr.add(n * cols) };
        if use_last_match {
            for j in last_j_lo..=last_j_hi {
                let jm = j - col_off;
                let s = unsafe { (*last_row_ptr.add(jm)).score() };
                let better = s >= best_score && s > 0;
                best_score = if better { s } else { best_score };
                best_j = if better { j } else { best_j };
            }
        } else {
            for j in last_j_lo..=last_j_hi {
                let jm = j - col_off;
                let s = unsafe { (*last_row_ptr.add(jm)).score() };
                let better = s > best_score;
                best_score = if better { s } else { best_score };
                best_j = if better { j } else { best_j };
            }
        }
    }

    if best_score <= 0 {
        return None;
    }

    if COMPUTE_INDICES {
        let indices_ref_cell = indices_buf.get_or(|| RefCell::new(Vec::new()));
        let mut indices_ref = indices_ref_cell.borrow_mut();
        indices_ref.clear();
        let mut i = n;
        let mut j = best_j;
        let mut true_matches = 0usize;

        while i > 0 && j >= j_start {
            let jm = j - col_off;
            let cell_val = unsafe { *base_ptr.add(i * cols).add(jm) };
            match cell_val.dir() {
                Dir::Diag => {
                    if pat[i - 1].eq(cho[j - 1], respect_case) {
                        indices_ref.push((j - 1) as IndexType);
                        true_matches += 1;
                    }
                    i -= 1;
                    j -= 1;
                }
                Dir::Up => {
                    i -= 1;
                }
                Dir::Left => {
                    j -= 1;
                }
                Dir::None => break,
            }
        }

        if true_matches < banding.min_true_matches {
            return None;
        }

        indices_ref.reverse();

        let out = indices_ref.to_vec();
        Some((best_score, out))
    } else {
        Some((best_score, Vec::default()))
    }
}
