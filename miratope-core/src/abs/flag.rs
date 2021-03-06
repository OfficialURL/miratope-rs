//! Helpful methods and structs for operating on the
//! [`Flags`](Flag) of a polytope.
//!
//! Recall that a flag is a maximal set of pairwise incident elements in a
//! polytope. For convenience, we omit the minimal and maximal elements from our
//! flags, though we sometimes pretend like they're still there for convenience.

use std::{
    cmp::Ordering,
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    ops::{Index, IndexMut},
};

use super::{elements::ElementRef, rank::Rank, Abstract};
use crate::{Float, Polytope};

use vec_like::*;

/// Represents a [flag](https://polytope.miraheze.org/wiki/Flag) in a polytope.
/// Stores the indices of the elements of each rank, excluding the minimal and
/// maximal elements.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Flag(Vec<usize>);
impl_veclike!(Flag, Item = usize, Index = usize);

impl Flag {
    /// Gets the index of the element with a given rank, or returns `0` if it
    /// doesn't exist. This allows us to pretend that the flag stores a minimal
    /// and maximal element.
    pub fn get_or_zero(&self, rank: Rank) -> usize {
        match rank.try_usize() {
            Some(rank) => self.get(rank).cloned().unwrap_or(0),
            None => 0,
        }
    }

    /// Applies a specified flag change to the flag in place.
    ///
    /// # Panics
    /// This method should only panic if an invalid polytope is given as an
    /// argument.
    pub fn change_mut(&mut self, polytope: &Abstract, r: usize) {
        let rank = polytope.rank();
        debug_assert_ne!(
            rank,
            Rank::new(-1),
            "Can't iterate over flags of the nullitope."
        );

        // A flag change is a no-op in a point.
        if rank == Rank::new(0) {
            return;
        }

        let r_rank = Rank::from(r);
        let r_minus_one = r_rank.minus_one();
        let r_plus_one = r_rank.plus_one();

        // Determines the common elements between the subelements of the element
        // above and the superelements of the element below.
        let below_idx = self.get_or_zero(r_minus_one);
        let below = polytope
            .get_element(ElementRef::new(r_minus_one, below_idx))
            .unwrap();

        let above_idx = self.get_or_zero(r_plus_one);
        let above = polytope
            .get_element(ElementRef::new(r_plus_one, above_idx))
            .unwrap();

        let common = common(&below.sups.0, &above.subs.0);

        debug_assert_eq!(
            common.len(),
            2,
            "Diamond property fails between rank {}, index {}, and rank {}, index {}.",
            r_minus_one,
            self.get_or_zero(r_minus_one),
            r_plus_one,
            self.get_or_zero(r_plus_one),
        );

        // Changes the element at idx to the other element in the section
        // determined by the elements above and below.
        if self[r] == common[0] {
            self[r] = common[1];
        } else {
            self[r] = common[0];
        }
    }

    /// Applies a specified flag change to the flag.
    pub fn change(&self, polytope: &Abstract, idx: usize) -> Self {
        let mut clone = self.clone();
        clone.change_mut(polytope, idx);
        clone
    }
}

/// The parity of a flag, which flips on any flag change.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Orientation {
    /// A flag of even parity.
    Even,

    /// A flag of odd parity.
    Odd,
}

impl Orientation {
    /// Flips the parity of a flag.
    pub fn flip(&self) -> Self {
        match self {
            Orientation::Even => Orientation::Odd,
            Orientation::Odd => Orientation::Even,
        }
    }

    /// Returns the "sign" associated with a flag, which is either `1.0` or
    /// `-1.0`.
    pub fn sign(&self) -> Float {
        match self {
            Orientation::Even => 1.0,
            Orientation::Odd => -1.0,
        }
    }
}

/// An arbitrary orientation to serve as the default.
impl Default for Orientation {
    fn default() -> Self {
        Self::Even
    }
}

/// An iterator over all [`Flags`](Flag) of a polytope. This iterator works even
/// if the polytope is a compound polytope.
///
/// Each flag is associated with a sequence whose k-th entry stores the index of
/// the k-th element as a subelement of its superelement. We iterate over flags
/// in the lexicographic order given by these sequences.
///
/// You should use this iterator instead of an [`OrientedFlagIter`] when
/// * you don't care about the [`Orientation`] of the flags,
/// * you want to iterate over all flags.
pub struct FlagIter<'a> {
    /// The polytope whose flags we iterate over.
    polytope: &'a Abstract,

    /// The flag we just found, or `None` if we already went through the entire
    /// iterator.
    flag: Option<Flag>,

    /// The indices of each element of the flag, **as subelements of their
    /// superelements.** These indices **do not** coincide with the actual
    /// indices of the elements in their respective `ElementList`s.
    indices: Vec<usize>,
}

impl<'a> FlagIter<'a> {
    /// Initializes an iterator over all flags of a polytope.
    pub fn new(polytope: &'a Abstract) -> Self {
        assert!(
            polytope.sorted,
            "You must make sure that the polytope is sorted before iterating over its flags."
        );

        let r = polytope.rank().try_usize().unwrap_or(0);
        Self {
            polytope,
            flag: polytope.first_flag(),
            indices: vec![0; r],
        }
    }
}

impl<'a> Iterator for FlagIter<'a> {
    type Item = Flag;

    fn next(&mut self) -> Option<Self::Item> {
        let flag = self.flag.as_mut()?;
        let prev_flag = flag.clone();
        let rank = self.polytope.rank().into_usize();

        // The largest rank of the elements we'll update.
        let mut r = 0;
        loop {
            if r == rank {
                self.flag = None;
                return Some(prev_flag);
            }

            let r_plus_one = Rank::from(r + 1);
            let ranks = &self.polytope[r_plus_one];
            let idx = flag.get_or_zero(r_plus_one);

            if ranks[idx].subs.len() == self.indices[r] + 1 {
                self.indices[r] = 0;
                r += 1;
            } else {
                self.indices[r] += 1;
                break;
            }
        }

        // Updates all elements in the flag with ranks r down to 0.
        let r_plus_one = Rank::from(r + 1);
        let idx = flag.get(r + 1).copied().unwrap_or(0);
        let mut element = &self.polytope[r_plus_one][idx];
        loop {
            let idx = self.indices[r];
            flag[r] = element.subs[idx];

            if r == 0 {
                break;
            }

            element = &self.polytope[r.into()][flag[r]];
            r -= 1;
        }

        Some(prev_flag)
    }
}

#[derive(Clone, Default, Eq)]
/// A flag together with an orientation. Any flag change flips the orientation.
/// If the polytope associated to the flag is non-orientable, the orientation
/// will be garbage data.
pub struct OrientedFlag {
    /// The indices of the elements the flag contains, excluding the null and
    /// maximal elements.
    pub flag: Flag,

    /// The orientation of the flag. If the polytope is non-orientable, this
    /// will contain garbage.
    pub orientation: Orientation,
}

/// Makes an oriented flag from a normal flag.
impl From<Flag> for OrientedFlag {
    fn from(flag: Flag) -> Self {
        Self {
            flag,
            orientation: Default::default(),
        }
    }
}

impl From<Vec<usize>> for OrientedFlag {
    fn from(vec: Vec<usize>) -> Self {
        Flag::from(vec).into()
    }
}

impl AsRef<Vec<usize>> for OrientedFlag {
    fn as_ref(&self) -> &Vec<usize> {
        self.flag.as_ref()
    }
}

impl AsMut<Vec<usize>> for OrientedFlag {
    fn as_mut(&mut self) -> &mut Vec<usize> {
        self.flag.as_mut()
    }
}

/// Allows indexing an oriented flag by rank.
impl Index<usize> for OrientedFlag {
    type Output = usize;

    fn index(&self, index: usize) -> &Self::Output {
        &self.flag[index]
    }
}

/// Allows mutably indexing an oriented flag by rank.
impl IndexMut<usize> for OrientedFlag {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.flag[index]
    }
}

/// Iterates over the entries of an oriented flag.
impl IntoIterator for OrientedFlag {
    type Item = usize;

    type IntoIter = std::vec::IntoIter<usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.flag.into_iter()
    }
}

impl VecLike for OrientedFlag {
    type VecItem = usize;

    type VecIndex = usize;
}

impl Hash for OrientedFlag {
    /// Returns the hash of the flag. **Does not take orientation into
    /// account.**
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.flag.hash(state);
    }
}

impl PartialEq for OrientedFlag {
    /// Determines whether two flags are equal. **Does not take orientation into
    /// account.**
    fn eq(&self, other: &Self) -> bool {
        self.flag.eq(&other.flag)
    }
}

impl PartialOrd for OrientedFlag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.flag.partial_cmp(&other.flag)
    }
}

impl Ord for OrientedFlag {
    fn cmp(&self, other: &Self) -> Ordering {
        self.flag.cmp(&other.flag)
    }
}

/// Gets the common elements of two **sorted** lists.
fn common(vec0: &[usize], vec1: &[usize]) -> Vec<usize> {
    let mut common = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while let Some(&sub0) = vec0.get(i) {
        if let Some(sub1) = vec1.get(j) {
            match sub0.cmp(sub1) {
                Ordering::Equal => {
                    common.push(sub0);
                    i += 1;
                }
                Ordering::Greater => j += 1,
                Ordering::Less => i += 1,
            };
        } else {
            break;
        }
    }

    common
}

impl OrientedFlag {
    /// Applies a specified flag change to the flag.
    pub fn change(&self, polytope: &Abstract, idx: usize) -> Self {
        Self {
            flag: self.flag.change(polytope, idx),
            orientation: self.orientation.flip(),
        }
    }
}

/// Represents a set of flag changes.
#[derive(Clone)]
pub struct FlagChanges(Vec<usize>);
impl_veclike!(FlagChanges, Item = usize, Index = usize);

impl FlagChanges {
    /// Returns the set of all flag changes for a polytope of a given rank.
    pub fn all(rank: Rank) -> Self {
        Self((0..rank.into()).collect())
    }

    /// Returns an iterator over all subsets of flag changes created by taking
    /// out a single flag change.
    pub fn subsets(
        &self,
    ) -> std::iter::Map<std::ops::Range<usize>, impl FnMut(usize) -> Self + '_> {
        (0..self.len()).map(move |i| {
            let mut subset = self.clone();
            subset.remove(i);
            subset
        })
    }
}

/// An iterator over all of the [`FlagEvent`]s of a polytope. A [`FlagEvent`] is
/// either an [`OrientedFlag`], or an event that determines that a polytope is
/// non-orientable.
///
/// **All methods assume that the polytope has been [sorted](Abstract::sort)
/// beforehand.**
///
/// We store a queue of all [`Flags`](Flag) whose adjacencies need to be
/// searched, together with a `HashSet` which store all of the flags that have
/// been found so far. For each element in the queue, we apply all flag changes
/// in a given set to it. All new flags that we find are then returned and added
/// to the queue.
///
/// The reason we don't iterate over flags directly is that sometimes, we
/// realize that a polytope is non-orientable only after traversing every single
/// one of its flags. Hence, we can't bundle the information that the polytope
/// is non-orientable with the flags.
///
/// You should use this iterator instead of a [`FlagIter`] when
/// * you want to apply a specific set of flag changes,
/// * you care about the orientation of the flags.
pub struct OrientedFlagIter<'a> {
    /// The polytope whose flags we iterate over. For the algorithm that applies
    /// a flag change to work, **this polytope's subelement and superelement
    /// lists must be sorted.**
    ///
    /// Some associated methods will guarantee this condition by sorting the
    /// polytope, while others will assume it.
    polytope: &'a Abstract,

    /// The flags whose adjacencies are being searched.
    queue: VecDeque<OrientedFlag>,

    /// The flag changes we're applying.
    flag_changes: FlagChanges,

    /// The flag index we need to check next.
    flag_idx: usize,

    /// Have we already returned the first flag?
    first: bool,

    /// The flags that have already been found, but whose neighbors haven't all
    /// been found yet.
    found: HashMap<OrientedFlag, usize>,

    /// Whether all of the flags the iterator has checked so far have a parity.
    orientable: bool,
}

/// The result of trying to get the next flag.
pub enum FlagNext {
    /// We found a new flag event (either a flag or the non-orientable event).
    New(FlagEvent),

    /// We found a flag we had already found before.
    Repeat,

    /// There are no flags left to find.
    None,
}

impl<'a> OrientedFlagIter<'a> {
    /// Returns a dummy iterator that returns `None` every single time.
    pub fn empty(polytope: &'a Abstract) -> Self {
        Self {
            polytope,
            queue: VecDeque::new(), // This is the important bit.
            flag_changes: FlagChanges::new(),
            flag_idx: 0,
            first: true, // And also this.
            found: HashMap::new(),
            orientable: true,
        }
    }

    /// Initializes a new iterator over the flag events of a polytope, starting
    /// from an arbitrary flag and applying all flag changes.
    ///
    /// You must [sort](Abstract::sort) the polytope before calling this
    /// method.
    pub fn new(polytope: &'a Abstract) -> Self {
        // Initializes with any flag from the polytope and all flag changes.
        if let Some(first_flag) = polytope.first_oriented_flag() {
            let rank = polytope.rank();
            Self::with_flags(polytope, FlagChanges::all(rank), first_flag)
        }
        // A nullitope has no flags.
        else {
            Self::empty(polytope)
        }
    }

    /// Initializes a new iterator over the flag events of a polytope, starting
    /// from a specified flag and applying a given set of flag changes.
    ///
    /// You must [sort](Abstract::sort) the polytope before calling this
    /// method.
    pub fn with_flags(
        polytope: &'a Abstract,
        flag_changes: FlagChanges,
        first_flag: OrientedFlag,
    ) -> Self {
        let first = polytope.rank() == Rank::new(-1);

        // Initializes found flags.
        let mut found = HashMap::new();
        let mut queue = VecDeque::new();

        if !first {
            found.insert(first_flag.clone(), 0);

            // Initializes queue.
            queue.push_back(first_flag);
        }

        Self {
            polytope,
            queue,
            flag_changes,
            flag_idx: 0,
            first,
            found,
            orientable: true,
        }
    }

    /// Returns a new iterator over oriented flags, discarding the
    /// non-orientable event.
    pub fn filter_flags(
        self,
    ) -> std::iter::FilterMap<Self, impl FnMut(FlagEvent) -> Option<OrientedFlag>> {
        self.filter_map(FlagEvent::flag)
    }

    /// Attempts to get the next flag.
    pub fn try_next(&mut self) -> FlagNext {
        // We get the current flag from the queue.
        if let Some(current) = self.queue.front() {
            let rank = self.polytope.rank().into_usize();

            // Applies the current flag change to the current flag.
            let flag_change = self.flag_changes[self.flag_idx];
            let new_flag = current.change(&self.polytope, flag_change);

            // Increments the flag index.
            self.flag_idx = if self.flag_idx + 1 == self.flag_changes.len() {
                self.queue.pop_front();
                0
            } else {
                self.flag_idx + 1
            };

            let new_orientation = new_flag.orientation;
            match self.found.entry(new_flag) {
                // If the flag is already in the found dictionary:
                Entry::Occupied(mut occupied_entry) => {
                    *occupied_entry.get_mut() += 1;
                    let val = *occupied_entry.get();

                    // If there's a mismatch between the seen and the expected
                    // orientability, then we know the polytope isn't orientable.
                    if self.orientable && new_orientation != occupied_entry.key().orientation {
                        self.orientable = false;
                        return FlagNext::New(FlagEvent::NonOrientable);
                    }

                    // In any case, if we got here, we know this is a repeated
                    // flag.
                    //
                    // If we've found it all of the times we'll ever find it,
                    // there's no use in keeping it in the dictionary (profiling
                    // shows this is marginally faster than letting it be).
                    if val == rank {
                        occupied_entry.remove();
                    }

                    FlagNext::Repeat
                }

                // If this flag is new, we just add it and return it.
                Entry::Vacant(vacant_entry) => {
                    let new_flag = vacant_entry.key().clone();
                    self.queue.push_back(new_flag.clone());

                    // We've found the flag one (1) time.
                    vacant_entry.insert(1);

                    FlagNext::New(FlagEvent::Flag(new_flag))
                }
            }
        }
        // The queue is empty.
        else {
            FlagNext::None
        }
    }
}

/// Represents either a new found flag, or the event in which the iterator
/// realizes that the polytope is non-orientable.
pub enum FlagEvent {
    /// We found a new flag.
    Flag(OrientedFlag),

    /// We just realized the polytope is non-orientable.
    NonOrientable,
}

impl FlagEvent {
    /// Returns the flag contained in the event, if any.
    pub fn flag(self) -> Option<OrientedFlag> {
        match self {
            Self::Flag(oriented_flag) => Some(oriented_flag),
            Self::NonOrientable => None,
        }
    }

    /// Returns whether `self` matches `Self::NonOrientable`.
    pub fn non_orientable(&self) -> bool {
        matches!(self, Self::NonOrientable)
    }
}

impl<'a> Iterator for OrientedFlagIter<'a> {
    type Item = FlagEvent;

    /// Gets the next flag event.
    fn next(&mut self) -> Option<Self::Item> {
        let rank = self.polytope.rank();

        // The first flag is a special case.
        if !self.first {
            self.first = true;

            let flag = Some(FlagEvent::Flag(self.found.keys().next().cloned().unwrap()));

            // If we're dealing with a point, or if we're performing no flag
            // changes, this is the only flag.
            if rank == Rank::new(0) || self.flag_changes.is_empty() {
                self.queue = VecDeque::new();
            }

            return flag;
        }

        // Loops until we get a new flag event.
        loop {
            match self.try_next() {
                // We found a new flag event.
                FlagNext::New(flag_event) => {
                    return Some(flag_event);
                }

                // We already exhausted the flag supply.
                FlagNext::None => return None,

                // Repeat flag, try again.
                FlagNext::Repeat => {}
            }
        }
    }
}

/// Represents a set of flags, created by applying a specific set of flag
/// changes to a flag in a polytope.
pub struct FlagSet {
    /// The flags contained in the set.
    pub flags: HashSet<Flag>,

    /// The flag changes from which these flags were generated.
    pub flag_changes: FlagChanges,
}

// THIS IS ONLY MEANT FOR OMNITRUNCATES!!!
impl PartialEq for FlagSet {
    fn eq(&self, other: &Self) -> bool {
        if self.flag_changes.0 != other.flag_changes.0 {
            return false;
        }

        let flag = self.flags.iter().next().unwrap();
        other.flags.contains(&flag)
    }
}

impl Eq for FlagSet {}

impl FlagSet {
    /// Creates a new flag set from any flag of the polytope.
    pub fn new(polytope: &Abstract) -> Self {
        Self::with_flags(
            polytope,
            FlagChanges::all(polytope.rank()),
            polytope.first_flag().unwrap(),
        )
    }

    /// Creates a new flag set defined by all flags in a polytope that can be
    /// obtained by repeatedly applying any in a given set of flag changes to a
    /// specified flag.
    pub fn with_flags(polytope: &Abstract, flag_changes: FlagChanges, first_flag: Flag) -> Self {
        Self {
            flags: OrientedFlagIter::with_flags(polytope, flag_changes.clone(), first_flag.into())
                .filter_flags()
                .map(|oriented_flag| oriented_flag.flag)
                .collect(),
            flag_changes,
        }
    }

    /// Returns `true` if the flag set is empty.
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Returns the number of flags contained in the flag set.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// Returns the set of all flag sets obtained from this one after removing
    /// exactly one element.
    pub fn subsets(&self, polytope: &Abstract) -> Vec<Self> {
        let mut subsets = Vec::new();

        for flag_changes in self.flag_changes.subsets() {
            let mut flags = HashSet::new();

            for flag in &self.flags {
                if flags.insert(flag.clone()) {
                    let subset = Self::with_flags(&polytope, flag_changes.clone(), flag.clone());

                    for flag in &subset.flags {
                        flags.insert(flag.clone());
                    }

                    subsets.push(subset);
                }
            }
        }

        subsets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Polytope;

    /// Tests that a polytope has an expected number of flags, oriented or not.
    fn test(polytope: &mut Abstract, expected: usize) {
        polytope.abs_sort();

        let flag_count = polytope.flags().count();
        assert_eq!(
            expected, flag_count,
            "Expected {} flags, found {}.",
            expected, flag_count
        );

        let flag_count = polytope.flag_events().filter_flags().count();
        assert_eq!(
            expected, flag_count,
            "Expected {} oriented flags, found {}.",
            expected, flag_count
        );
    }

    #[test]
    fn nullitope() {
        test(&mut Abstract::nullitope(), 0)
    }

    #[test]
    fn point() {
        test(&mut Abstract::point(), 1)
    }

    #[test]
    fn dyad() {
        test(&mut Abstract::dyad(), 2)
    }

    #[test]
    fn polygon() {
        for n in 2..=10 {
            test(&mut Abstract::polygon(n), 2 * n);
        }
    }

    #[test]
    fn simplex() {
        for n in 0..=7 {
            test(
                &mut Abstract::simplex(Rank::from(n)),
                crate::factorial(n + 1) as usize,
            );
        }
    }

    #[test]
    fn hypercube() {
        for n in 0..=7 {
            test(
                &mut Abstract::hypercube(Rank::new(n as isize)),
                (1 << n) * crate::factorial(n) as usize,
            );
        }
    }

    #[test]
    fn orthoplex() {
        for n in 0..=7 {
            test(
                &mut Abstract::orthoplex(Rank::new(n as isize)),
                (1 << n) * crate::factorial(n) as usize,
            );
        }
    }
}
