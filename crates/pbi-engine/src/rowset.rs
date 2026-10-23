//! [`RowSet`]: a bitset over the rows of one table.

use std::fmt;
use std::ops::{BitAnd, BitOr, Not, Sub};

use arrow::array::{Array, BooleanArray};
use arrow::buffer::{BooleanBuffer, Buffer};

const BITS: usize = 64;

/// A set of row indexes `0..universe` of one table, stored as a bitset.
///
/// Binary operations are defined for sets over different universes (the
/// result keeps the left operand's universe; missing bits count as unset), so
/// they never panic.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct RowSet {
    universe: usize,
    words: Vec<u64>,
}

impl RowSet {
    /// No rows of a table with `universe` rows.
    pub fn empty(universe: usize) -> Self {
        Self {
            universe,
            words: vec![0; universe.div_ceil(BITS)],
        }
    }

    /// All rows of a table with `universe` rows.
    pub fn all(universe: usize) -> Self {
        let mut set = Self {
            universe,
            words: vec![u64::MAX; universe.div_ceil(BITS)],
        };
        set.clear_tail();
        set
    }

    /// The given rows; indexes `>= universe` are ignored.
    pub fn from_rows(universe: usize, rows: impl IntoIterator<Item = usize>) -> Self {
        let mut set = Self::empty(universe);
        for row in rows {
            set.insert(row);
        }
        set
    }

    /// Rows whose flag is `true`.
    pub fn from_bools(flags: &[bool]) -> Self {
        Self::from_rows(
            flags.len(),
            flags
                .iter()
                .enumerate()
                .filter(|(_, f)| **f)
                .map(|(i, _)| i),
        )
    }

    /// Rows where `mask` is true (nulls count as false).
    pub fn from_boolean_array(mask: &BooleanArray) -> Self {
        Self::from_rows(
            mask.len(),
            (0..mask.len()).filter(|&i| mask.is_valid(i) && mask.value(i)),
        )
    }

    /// Number of rows of the underlying table (the set's universe).
    pub fn universe(&self) -> usize {
        self.universe
    }

    /// Number of rows in the set.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// `true` if no row is in the set.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// `true` if every row of the universe is in the set.
    pub fn is_all(&self) -> bool {
        self.count() == self.universe
    }

    pub fn contains(&self, row: usize) -> bool {
        row < self.universe
            && self
                .words
                .get(row / BITS)
                .is_some_and(|w| w & (1u64 << (row % BITS)) != 0)
    }

    /// Adds `row`; returns `false` if it is out of range.
    pub fn insert(&mut self, row: usize) -> bool {
        if row >= self.universe {
            return false;
        }
        if let Some(w) = self.words.get_mut(row / BITS) {
            *w |= 1u64 << (row % BITS);
        }
        true
    }

    pub fn remove(&mut self, row: usize) {
        if let Some(w) = self.words.get_mut(row / BITS) {
            *w &= !(1u64 << (row % BITS));
        }
    }

    /// Keeps only rows also in `other`.
    pub fn intersect_with(&mut self, other: &RowSet) {
        for (i, w) in self.words.iter_mut().enumerate() {
            *w &= other.words.get(i).copied().unwrap_or(0);
        }
    }

    /// Adds the rows of `other` (rows outside this universe are ignored).
    pub fn union_with(&mut self, other: &RowSet) {
        for (w, o) in self.words.iter_mut().zip(&other.words) {
            *w |= o;
        }
        self.clear_tail();
    }

    /// Removes the rows of `other`.
    pub fn difference_with(&mut self, other: &RowSet) {
        for (w, o) in self.words.iter_mut().zip(&other.words) {
            *w &= !o;
        }
    }

    /// Rows in both sets.
    pub fn and(&self, other: &RowSet) -> RowSet {
        let mut out = self.clone();
        out.intersect_with(other);
        out
    }

    /// Rows in either set.
    pub fn or(&self, other: &RowSet) -> RowSet {
        let mut out = self.clone();
        out.union_with(other);
        out
    }

    /// Rows in `self` but not in `other`.
    pub fn and_not(&self, other: &RowSet) -> RowSet {
        let mut out = self.clone();
        out.difference_with(other);
        out
    }

    /// Complement within the universe.
    pub fn complement(&self) -> RowSet {
        let mut out = Self {
            universe: self.universe,
            words: self.words.iter().map(|w| !w).collect(),
        };
        out.clear_tail();
        out
    }

    /// Row indexes in ascending order.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            words: &self.words,
            index: 0,
            current: self.words.first().copied().unwrap_or(0),
        }
    }

    /// Smallest row in the set.
    pub fn first(&self) -> Option<usize> {
        self.iter().next()
    }

    /// The set over a different universe (truncated, or extended with unset
    /// rows).
    pub fn resized(&self, universe: usize) -> RowSet {
        let mut words = self.words.clone();
        words.resize(universe.div_ceil(BITS), 0);
        let mut out = Self { universe, words };
        out.clear_tail();
        out
    }

    /// Arrow boolean mask (for `arrow::compute::filter`).
    pub fn to_boolean_array(&self) -> BooleanArray {
        let bytes: Vec<u8> = self.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        BooleanArray::new(
            BooleanBuffer::new(Buffer::from_vec(bytes), 0, self.universe),
            None,
        )
    }

    fn clear_tail(&mut self) {
        let used = self.universe.div_ceil(BITS);
        self.words.truncate(used);
        let rem = self.universe % BITS;
        if rem != 0
            && let Some(last) = self.words.last_mut()
        {
            *last &= (1u64 << rem) - 1;
        }
    }
}

impl fmt::Debug for RowSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const SHOWN: usize = 32;
        let rows: Vec<usize> = self.iter().take(SHOWN + 1).collect();
        let mut s = f.debug_struct("RowSet");
        s.field("universe", &self.universe)
            .field("count", &self.count());
        if rows.len() > SHOWN {
            s.field("rows", &format_args!("{:?}…", &rows[..SHOWN]));
        } else {
            s.field("rows", &rows);
        }
        s.finish()
    }
}

/// Iterator over the rows of a [`RowSet`].
#[derive(Debug, Clone)]
pub struct Iter<'a> {
    words: &'a [u64],
    index: usize,
    current: u64,
}

impl Iterator for Iter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.current != 0 {
                let bit = self.current.trailing_zeros() as usize;
                self.current &= self.current - 1;
                return Some(self.index * BITS + bit);
            }
            self.index += 1;
            self.current = *self.words.get(self.index)?;
        }
    }
}

impl<'a> IntoIterator for &'a RowSet {
    type Item = usize;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl BitAnd for &RowSet {
    type Output = RowSet;
    fn bitand(self, rhs: &RowSet) -> RowSet {
        self.and(rhs)
    }
}

impl BitOr for &RowSet {
    type Output = RowSet;
    fn bitor(self, rhs: &RowSet) -> RowSet {
        self.or(rhs)
    }
}

impl Sub for &RowSet {
    type Output = RowSet;
    fn sub(self, rhs: &RowSet) -> RowSet {
        self.and_not(rhs)
    }
}

impl Not for &RowSet {
    type Output = RowSet;
    fn not(self) -> RowSet {
        self.complement()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_operations() {
        let a = RowSet::from_rows(130, [0, 5, 64, 129, 500]);
        let b = RowSet::from_rows(130, [5, 64, 100]);
        assert_eq!(a.count(), 4);
        assert!(a.contains(129) && !a.contains(500) && !a.contains(1));
        assert_eq!((&a & &b).iter().collect::<Vec<_>>(), [5, 64]);
        assert_eq!((&a | &b).count(), 5);
        assert_eq!((&a - &b).iter().collect::<Vec<_>>(), [0, 129]);
        let not_a = !&a;
        assert_eq!(not_a.count(), 126);
        assert!(!not_a.contains(129));
        assert!(RowSet::all(130).is_all());
        assert!(RowSet::empty(130).is_empty());
        assert_eq!(RowSet::all(0).count(), 0);
        assert_eq!(a.first(), Some(0));
    }

    #[test]
    fn mismatched_universes_do_not_panic() {
        let a = RowSet::all(10);
        let b = RowSet::all(200);
        assert_eq!(a.and(&b).count(), 10);
        assert_eq!(b.and(&a).count(), 10);
        assert_eq!(a.or(&b).count(), 10);
        assert_eq!(b.and_not(&a).count(), 190);
        assert_eq!(a.resized(70).count(), 10);
        assert_eq!(b.resized(65).count(), 65);
    }

    #[test]
    fn converts_to_arrow_mask() {
        let set = RowSet::from_rows(70, [1, 69]);
        let mask = set.to_boolean_array();
        assert_eq!(mask.len(), 70);
        assert_eq!(mask.true_count(), 2);
        assert_eq!(RowSet::from_boolean_array(&mask), set);
        assert_eq!(RowSet::from_bools(&[true, false, true]).count(), 2);
    }
}
