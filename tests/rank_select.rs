#[macro_use]
extern crate quickcheck;
use quickcheck::{quickcheck, TestResult};
use bv::BitVec;
use bio::data_structures::rank_select::RankSelect;

/// RankIndex stores all the ranks of things in a list.
///
/// To build an index of rankings of a thing in a list:
///
///  1. Construct the RankIndex: RankIndex::default()
///  2. Iterate over the list, for each item:
///     * If it is a thing call the bump() method
///     * If it is not a thing, call keep()
#[derive(Default)]
struct RankIndex {
    rank: Vec<u64>,
}

impl RankIndex {
    fn bump(&mut self) {
        let rank = self.rank.last().map_or(1, |x| x + 1) as u64;
        self.rank.push(rank);
    }

    fn keep(&mut self) {
        let rank = self.rank.last().map_or(0, |x| *x) as u64;
        self.rank.push(rank);
    }

    fn rank(&self, i: u64) -> Option<u64> {
        self.rank.get(i as usize).map(|x| *x)
    }
}

/// A test data structure for validating more sophisticated
/// methods of ranking bit vectors.
#[derive(Default)]
struct TestRank {
    rank_0: RankIndex,
    rank_1: RankIndex
}

impl TestRank {
    fn new(bv: &[bool]) -> Self {
        let mut tr = Self::default();
        for &bit in bv {
            if bit {
                tr.rank_1.bump();
                tr.rank_0.keep();
            } else {
                tr.rank_0.bump();
                tr.rank_1.keep();
            }
        }
        tr
    }

    fn rank_0(&self, i: u64) -> Option<u64> {
        self.rank_0.rank(i)
    }

    fn rank_1(&self, i: u64) -> Option<u64> {
        self.rank_1.rank(i)
    }
}

/// Given a randomly generated vector of booleans
/// Creates a `RankSelect` object and `TestRank` object
/// and compares rank_0 for all indexes.
fn prop_rank_0(s: Vec<bool>) -> TestResult {
    let tr = TestRank::new(&s);
    let mut bv = BitVec::new();
    for &bit in &s {
        bv.push(bit);
    }
    let rs = RankSelect::new(bv, 1);
    for i in 0..s.len() {
        let t = tr.rank_0(i as u64);
        let r = rs.rank_0(i as u64);
        if t != r {
            return TestResult::from_bool(false)
        }
    }
    TestResult::from_bool(true)
}

/// Given a randomly generated vector of booleans
/// Creates a `RankSelect` object and `TestRank` object
/// and compares rank_1 for all indexes.
fn prop_rank_1(s: Vec<bool>) -> TestResult {
    let tr = TestRank::new(&s);
    let mut bv = BitVec::new();
    for &bit in &s {
        bv.push(bit);
    }
    let rs = RankSelect::new(bv, 2);
    for i in 0..s.len() {
        let t = tr.rank_1(i as u64);
        let r = rs.rank_1(i as u64);
        if t != r {
            return TestResult::from_bool(false)
        }
    }
    TestResult::from_bool(true)
}


#[test]
fn test_rank() {
    quickcheck(prop_rank_0 as fn(Vec<bool>) -> TestResult);
    quickcheck(prop_rank_1 as fn(Vec<bool>) -> TestResult);
}
