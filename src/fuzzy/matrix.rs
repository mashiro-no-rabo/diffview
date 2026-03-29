//! Base structs for the matching algorithm: Cell & SWMatrix

use super::Score;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub(super) enum Dir {
    None = 0,
    Diag = 1,
    Up = 2,
    Left = 3,
}

#[derive(Copy, Clone)]
pub(super) struct Cell(u32);

pub(super) const CELL_ZERO: Cell = Cell::new(0, Dir::None);

impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cell")
            .field("score", &self.score())
            .field("dir", &self.dir())
            .finish()
    }
}

impl Cell {
    #[inline(always)]
    pub(super) const fn new(score: Score, dir: Dir) -> Cell {
        Cell((score.cast_unsigned() as u32) | ((dir as u32) << 16))
    }
    #[inline(always)]
    pub(super) fn score(self) -> Score {
        #[allow(clippy::cast_possible_truncation)]
        let low16 = self.0 as u16;
        low16.cast_signed()
    }
    #[inline(always)]
    pub(super) fn dir(self) -> Dir {
        #[allow(clippy::cast_possible_truncation)]
        let tag = (self.0 >> 16) as u8 & 0x3;
        unsafe { std::mem::transmute(tag) }
    }
    #[inline(always)]
    pub(super) fn is_diag(self) -> bool {
        (self.0 >> 16) & 0x3 == 1
    }
}

#[derive(Default, Debug)]
pub(super) struct SWMatrix {
    pub(super) data: Vec<Cell>,
    pub(super) cols: usize,
    pub(super) rows: usize,
}

impl SWMatrix {
    pub fn zero(rows: usize, cols: usize) -> Self {
        let mut res = SWMatrix::default();
        res.resize(rows, cols);
        res
    }
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let needed = rows * cols;
        if needed > self.data.len() {
            self.data.resize(needed, CELL_ZERO);
        }
        self.rows = rows;
        self.cols = cols;
    }
}
