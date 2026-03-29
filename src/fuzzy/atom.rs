//! Byte/Char helpers
use super::Score;
use super::constants::SEPARATOR_TABLE;
use memchr::memchr;

pub(super) trait Atom: PartialEq + Into<char> + Copy {
    #[inline(always)]
    fn eq(self, other: Self, respect_case: bool) -> bool
    where
        Self: PartialEq + Sized,
    {
        if respect_case {
            self == other
        } else {
            self.eq_ignore_case(other)
        }
    }
    fn eq_ignore_case(self, other: Self) -> bool;
    fn is_lowercase(self) -> bool;

    #[inline(always)]
    fn find_first_in(self, haystack: &[Self], respect_case: bool) -> Option<usize> {
        haystack.iter().position(|&c| self.eq(c, respect_case))
    }

    #[inline(always)]
    fn separator_bonus(self) -> Score {
        let ch = self.into() as usize;
        SEPARATOR_TABLE.get(ch).copied().unwrap_or(0)
    }
}

impl Atom for u8 {
    #[inline(always)]
    fn eq_ignore_case(self, b: Self) -> bool {
        self.eq_ignore_ascii_case(&b)
    }
    #[inline(always)]
    fn is_lowercase(self) -> bool {
        self.is_ascii_lowercase()
    }

    #[inline(always)]
    fn find_first_in(self, haystack: &[Self], respect_case: bool) -> Option<usize> {
        if respect_case {
            memchr(self, haystack)
        } else {
            let lo = self.to_ascii_lowercase();
            let hi = self.to_ascii_uppercase();
            if lo == hi {
                memchr(lo, haystack)
            } else {
                let p_lo = memchr(lo, haystack);
                let p_hi = memchr(hi, haystack);
                match (p_lo, p_hi) {
                    (None, x) | (x, None) => x,
                    (Some(a), Some(b)) => Some(a.min(b)),
                }
            }
        }
    }
}

impl Atom for char {
    #[inline(always)]
    fn eq_ignore_case(self, b: Self) -> bool {
        self.to_lowercase().eq(b.to_lowercase())
    }
    #[inline(always)]
    fn is_lowercase(self) -> bool {
        self.is_lowercase()
    }
}
