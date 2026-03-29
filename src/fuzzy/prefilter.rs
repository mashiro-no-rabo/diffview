//! Prefilters running before the algo to optimize performance on unmatchable items

use super::Atom;
use super::constants::MAX_PAT_LEN;

pub(super) fn cheap_typo_prefilter<C: Atom>(
    pattern: &[C],
    choice: &[C],
    respect_case: bool,
) -> bool {
    let n = pattern.len();
    let m = choice.len();

    if n > m + 2 {
        return false;
    }

    let first = pattern[0];
    let Some(j_first) = first.find_first_in(choice, respect_case) else {
        return false;
    };

    if n == 1 {
        return true;
    }

    let min_tail = (n - 1) / 2;
    if min_tail == 0 {
        return true;
    }

    let window = &choice[j_first..];
    tail_freq_check(pattern, window, respect_case, min_tail)
}

#[inline(always)]
fn tail_freq_check<C: Atom>(
    pattern: &[C],
    window: &[C],
    respect_case: bool,
    min_tail: usize,
) -> bool {
    const MAX_TAIL: usize = MAX_PAT_LEN - 1;
    let tail = &pattern[1..];
    let tail_len = tail.len().min(MAX_TAIL);

    let placeholder = tail[0];
    let mut table: [(C, u8); MAX_TAIL] = [(placeholder, 0); MAX_TAIL];
    let mut table_len = 0usize;

    for &pi in &tail[..tail_len] {
        if !table[..table_len]
            .iter()
            .any(|&(c, _)| pi.eq(c, respect_case))
        {
            table[table_len] = (pi, 0);
            table_len += 1;
        }
    }

    for &c in window {
        if let Some(entry) = table[..table_len]
            .iter_mut()
            .find(|(tc, _)| Atom::eq(*tc, c, respect_case))
        {
            entry.1 = entry.1.saturating_add(1);
        }
    }

    let mut matched = 0usize;
    for &pi in &tail[..tail_len] {
        if let Some(entry) = table[..table_len]
            .iter_mut()
            .find(|(tc, _)| Atom::eq(pi, *tc, respect_case))
            && entry.1 > 0
        {
            entry.1 -= 1;
            matched += 1;
            if matched >= min_tail {
                return true;
            }
        }
    }

    false
}
