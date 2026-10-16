//! Compositional binding-incidence summaries. Small-to-large map merging follows
//! Maziarz et al. (PLDI 2021); declaration positions replace lambda removal so a
//! statement forest can close bindings introduced by any of its children.
//!
//! Names survive precisely while their declaration is outside the summary.
//! Position joins are tagged by a strictly increasing tree height. The shape
//! records which child supplied the larger map, so untouched positions remain
//! distinguishable from positions imported from the smaller map.
use std::rc::Rc;

pub(super) type Digest = [u8; 32];

pub(super) fn hash(domain: &[u8], parts: &[&[u8]]) -> Digest {
    let mut h = blake3::Hasher::new();
    h.update(b"entmoot.clone.summary.v1");
    h.update(&(domain.len() as u64).to_le_bytes());
    h.update(domain);
    for part in parts {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }
    *h.finalize().as_bytes()
}

fn xor(mut a: Digest, b: Digest) -> Digest {
    for (left, right) in a.iter_mut().zip(b) {
        *left ^= right;
    }
    a
}

#[derive(Clone, Copy)]
enum Entry {
    Binding {
        capture: Digest,
        declaration: Option<Digest>,
        uses: Option<Digest>,
    },
    Literal(Digest),
}

impl Entry {
    fn digest(self) -> Digest {
        match self {
            Self::Binding {
                declaration: Some(declaration),
                uses,
                ..
            } => hash(b"bound", &[&declaration, uses.as_ref().map_or(&[], |v| &v[..])]),
            Self::Binding { capture, uses, .. } => hash(b"capture", &[&capture, uses.as_ref().map_or(&[], |v| &v[..])]),
            Self::Literal(positions) => hash(b"literal", &[&positions]),
        }
    }

    fn import(self, previous: Option<Self>, tag: u32) -> Self {
        fn join(old: Option<Digest>, new: Option<Digest>, tag: u32) -> Option<Digest> {
            new.map(|new| {
                hash(
                    b"position",
                    &[&tag.to_le_bytes(), old.as_ref().map_or(&[], |v| &v[..]), &new],
                )
            })
            .or(old)
        }
        match (self, previous) {
            (
                Self::Binding {
                    capture,
                    declaration,
                    uses,
                },
                previous,
            ) => {
                let (old_declaration, old_uses) = match previous {
                    Some(Self::Binding { declaration, uses, .. }) => (declaration, uses),
                    _ => (None, None),
                };
                Self::Binding {
                    capture,
                    declaration: join(old_declaration, declaration, tag),
                    uses: join(old_uses, uses, tag),
                }
            }
            (Self::Literal(new), old) => Self::Literal(hash(
                b"position",
                &[
                    &tag.to_le_bytes(),
                    match &old {
                        Some(Self::Literal(old)) => &old[..],
                        _ => &[],
                    },
                    &new,
                ],
            )),
        }
    }
}

// Persistent AVL map: snapshots share untouched subtrees. Cloning a BTreeMap
// here would make saved overlapping statement summaries quadratic in size.
// Summaries are local to one callable fold; only owned digest strings escape.
#[derive(Clone, Default)]
struct Map(Option<Rc<Branch>>);
#[derive(Clone)]
struct Branch {
    key: usize,
    value: Entry,
    value_digest: Digest,
    left: Map,
    right: Map,
    height: u32,
    len: usize,
    digest: Digest,
}
impl Map {
    fn height(&self) -> u32 {
        self.0.as_ref().map_or(0, |n| n.height)
    }
    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.len)
    }
    fn digest(&self) -> Digest {
        self.0.as_ref().map_or([0; 32], |n| n.digest)
    }
    fn node(key: usize, value: Entry, value_digest: Digest, left: Self, right: Self) -> Self {
        Self(Some(Rc::new(Branch {
            key,
            value,
            value_digest,
            height: 1 + left.height().max(right.height()),
            len: 1 + left.len() + right.len(),
            digest: xor(value_digest, xor(left.digest(), right.digest())),
            left,
            right,
        })))
    }
    fn balanced(key: usize, value: Entry, value_digest: Digest, left: Self, right: Self) -> Self {
        if left.height() > right.height() + 1 {
            if let Some(l) = &left.0 {
                if l.right.height() > l.left.height() {
                    if let Some(mid) = &l.right.0 {
                        return Self::node(
                            mid.key,
                            mid.value,
                            mid.value_digest,
                            Self::node(l.key, l.value, l.value_digest, l.left.clone(), mid.left.clone()),
                            Self::node(key, value, value_digest, mid.right.clone(), right),
                        );
                    }
                }
                return Self::node(
                    l.key,
                    l.value,
                    l.value_digest,
                    l.left.clone(),
                    Self::node(key, value, value_digest, l.right.clone(), right),
                );
            }
        } else if right.height() > left.height() + 1 {
            if let Some(r) = &right.0 {
                if r.left.height() > r.right.height() {
                    if let Some(mid) = &r.left.0 {
                        return Self::node(
                            mid.key,
                            mid.value,
                            mid.value_digest,
                            Self::node(key, value, value_digest, left, mid.left.clone()),
                            Self::node(r.key, r.value, r.value_digest, mid.right.clone(), r.right.clone()),
                        );
                    }
                }
                return Self::node(
                    r.key,
                    r.value,
                    r.value_digest,
                    Self::node(key, value, value_digest, left, r.left.clone()),
                    r.right.clone(),
                );
            }
        }
        Self::node(key, value, value_digest, left, right)
    }
    fn insert(self, key: usize, value: Entry) -> Self {
        self.insert_hashed(key, value, value.digest())
    }
    fn insert_hashed(mut self, key: usize, value: Entry, value_digest: Digest) -> Self {
        let Some(root) = &mut self.0 else {
            return Self::node(key, value, value_digest, Self::default(), Self::default());
        };
        let n = Rc::make_mut(root);
        match key.cmp(&n.key) {
            std::cmp::Ordering::Less => {
                n.left = std::mem::take(&mut n.left).insert_hashed(key, value, value_digest);
            }
            std::cmp::Ordering::Greater => {
                n.right = std::mem::take(&mut n.right).insert_hashed(key, value, value_digest);
            }
            std::cmp::Ordering::Equal => {
                n.value = value;
                n.value_digest = value_digest;
            }
        }
        if n.left.height().abs_diff(n.right.height()) > 1 {
            return Self::balanced(n.key, n.value, n.value_digest, n.left.clone(), n.right.clone());
        }
        n.height = 1 + n.left.height().max(n.right.height());
        n.len = 1 + n.left.len() + n.right.len();
        n.digest = xor(n.value_digest, xor(n.left.digest(), n.right.digest()));
        self
    }
    fn get(&self, key: usize) -> Option<Entry> {
        let mut at = self;
        while let Some(n) = &at.0 {
            match key.cmp(&n.key) {
                std::cmp::Ordering::Equal => return Some(n.value),
                std::cmp::Ordering::Less => at = &n.left,
                std::cmp::Ordering::Greater => at = &n.right,
            }
        }
        None
    }
    fn visit(&self, f: &mut impl FnMut(usize, Entry)) {
        if let Some(n) = &self.0 {
            n.left.visit(f);
            f(n.key, n.value);
            n.right.visit(f);
        }
    }
}

#[derive(Clone)]
pub(super) struct Summary {
    shape: Digest,
    height: u32,
    entries: Map,
}
impl Summary {
    pub(super) fn node<'a>(kind: &str, children: impl Iterator<Item = &'a Self>) -> Self {
        let mut shape = blake3::Hasher::new();
        shape.update(b"entmoot.clone.node.v1");
        shape.update(&(kind.len() as u64).to_le_bytes());
        shape.update(kind.as_bytes());
        let mut entries = Map::default();
        let mut height = 0;
        for child in children {
            height = 1 + height.max(child.height);
            let left_big = entries.len() >= child.entries.len();
            shape.update(&[u8::from(left_big)]);
            shape.update(&child.shape);
            let small = if left_big {
                child.entries.clone()
            } else {
                std::mem::replace(&mut entries, child.entries.clone())
            };
            small.visit(&mut |key, value| {
                let updated = value.import(entries.get(key), height);
                entries = std::mem::take(&mut entries).insert(key, updated);
            });
        }
        Self {
            shape: *shape.finalize().as_bytes(),
            height,
            entries,
        }
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.shape == other.shape
            && match (&self.entries.0, &other.entries.0) {
                (None, None) => true,
                (Some(left), Some(right)) => Rc::ptr_eq(left, right),
                _ => false,
            }
    }
    pub(super) fn atom(kind: &str, text: &[u8]) -> Self {
        Self {
            shape: hash(b"atom", &[kind.as_bytes(), text]),
            height: 0,
            entries: Map::default(),
        }
    }
    pub(super) fn binding(kind: &str, key: usize, capture: Digest, declaration: bool) -> Self {
        let position = hash(b"here", &[]);
        Self {
            shape: hash(b"binding", &[kind.as_bytes()]),
            height: 0,
            entries: Map::default().insert(
                key,
                Entry::Binding {
                    capture,
                    declaration: declaration.then_some(position),
                    uses: (!declaration).then_some(position),
                },
            ),
        }
    }
    pub(super) fn literal(kind: &str, key: usize) -> Self {
        Self {
            shape: hash(b"literal", &[kind.as_bytes()]),
            height: 0,
            entries: Map::default().insert(key, Entry::Literal(hash(b"here", &[]))),
        }
    }
    pub(super) fn combine(&self, other: &Self) -> Self {
        let height = 1 + self.height.max(other.height);
        let left_big = self.entries.len() >= other.entries.len();
        let (big, small) = if left_big {
            (&self.entries, &other.entries)
        } else {
            (&other.entries, &self.entries)
        };
        let mut entries = big.clone();
        small.visit(&mut |key, value| {
            let updated = value.import(entries.get(key), height);
            entries = std::mem::take(&mut entries).insert(key, updated);
        });
        Self {
            shape: hash(b"join", &[&[u8::from(left_big)], &self.shape, &other.shape]),
            height,
            entries,
        }
    }
    pub(super) fn digest(&self) -> String {
        format!(
            "blake3:{}",
            blake3::Hash::from(hash(b"summary", &[&self.shape, &self.entries.digest()])).to_hex()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    fn audit(map: &Map) {
        if let Some(n) = &map.0 {
            assert!(n.left.height().abs_diff(n.right.height()) <= 1);
            assert_eq!(n.height, 1 + n.left.height().max(n.right.height()));
            assert_eq!(n.len, 1 + n.left.len() + n.right.len());
            assert_eq!(n.digest, xor(n.value.digest(), xor(n.left.digest(), n.right.digest())));
            audit(&n.left);
            audit(&n.right);
        }
    }

    #[test]
    fn persistent_maps_balance_all_small_insert_orders_and_preserve_old_values() {
        fn permutations(keys: &mut [usize], at: usize) {
            if at < keys.len() {
                for i in at..keys.len() {
                    keys.swap(at, i);
                    permutations(keys, at + 1);
                    keys.swap(at, i);
                }
                return;
            }
            let mut map = Map::default();
            let mut expected = BTreeMap::new();
            let mut snapshots = Vec::new();
            for key in keys {
                snapshots.push((map.clone(), expected.clone()));
                let entry = Entry::Literal(hash(b"value", &[&(*key as u64).to_le_bytes()]));
                expected.insert(*key, entry.digest());
                map = map.insert(*key, entry);
                audit(&map);
            }
            // Replace values too: no cached aggregate or old snapshot may change.
            for key in 0..6 {
                map = map.insert(key, Entry::Literal([99; 32]));
                audit(&map);
            }
            for (snapshot, expected) in snapshots {
                let mut actual = BTreeMap::new();
                snapshot.visit(&mut |key, value| {
                    actual.insert(key, value.digest());
                });
                assert_eq!(actual, expected);
                for key in 0..6 {
                    assert_eq!(snapshot.get(key).map(Entry::digest), expected.get(&key).copied());
                }
            }
        }
        permutations(&mut [0, 1, 2, 3, 4, 5], 0);
    }

    // Independent oracle: explicit leaf-position lists and sorted binding
    // groups. It uses neither tagged positions nor the production map/hash.
    fn oracle(keys: &[usize], declaration_mask: usize, literals: bool, balanced: bool) -> String {
        let mut groups = BTreeMap::<usize, Vec<usize>>::new();
        for (position, key) in keys.iter().copied().enumerate() {
            groups.entry(key).or_default().push(position);
        }
        let mut parts = Vec::new();
        for (key, positions) in groups {
            parts.push(if literals && key == 2 {
                format!("literal:{positions:?}")
            } else if declaration_mask & (1 << key) != 0 {
                format!("bound:{}:{:?}", positions[0], &positions[1..])
            } else {
                format!("free:{key}:{positions:?}")
            });
        }
        parts.sort();
        format!("{balanced}:{parts:?}")
    }

    #[test]
    fn summaries_match_an_exhaustive_binding_incidence_oracle() {
        fn tree(leaves: &[Summary], balanced: bool, compact: bool) -> Summary {
            if leaves.len() == 1 {
                return leaves[0].clone();
            }
            let split = if balanced { leaves.len() / 2 } else { leaves.len() - 1 };
            let left = tree(&leaves[..split], balanced, compact);
            let right = tree(&leaves[split..], balanced, compact);
            if compact {
                Summary::node("pair", [&left, &right].into_iter())
            } else {
                left.combine(&right)
            }
        }
        let mut by_hash = HashMap::new();
        let mut by_oracle = HashMap::new();
        for length in 1..=6 {
            for encoding in 0..3usize.pow(length) {
                let mut rest = encoding;
                let keys: Vec<_> = (0..length)
                    .map(|_| {
                        let key = rest % 3;
                        rest /= 3;
                        key
                    })
                    .collect();
                for mask in 0..8 {
                    for literals in [false, true] {
                        for balanced in [false, true] {
                            // Leaf count and binary tree shape are part of syntax.
                            let shape = if length <= 2 { false } else { balanced };
                            let expected = format!("{length}:{}", oracle(&keys, mask, literals, shape));
                            for renamed in [false, true] {
                                let mut seen = [false; 3];
                                let leaves: Vec<_> = keys
                                    .iter()
                                    .map(|key| {
                                        let declaration = !seen[*key] && mask & (1 << key) != 0;
                                        seen[*key] = true;
                                        let id = if renamed { [33, 7, 15][*key] } else { *key };
                                        let name = if renamed && mask & (1 << key) != 0 {
                                            key + 50
                                        } else {
                                            *key
                                        };
                                        if literals && *key == 2 {
                                            Summary::literal("number", id)
                                        } else {
                                            Summary::binding(
                                                "identifier",
                                                id,
                                                hash(b"name", &[&(name as u64).to_le_bytes()]),
                                                declaration,
                                            )
                                        }
                                    })
                                    .collect();
                                for compact in [false, true] {
                                    let expected = format!("{}:{expected}", compact && length > 1);
                                    let digest = tree(&leaves, balanced, compact).digest();
                                    if let Some(previous) = by_hash.insert(digest.clone(), expected.clone()) {
                                        assert_eq!(previous, expected, "hash collision");
                                    }
                                    if let Some(previous) = by_oracle.insert(expected.clone(), digest.clone()) {
                                        assert_eq!(previous, digest, "equivalent incidence must hash equally");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn tagged_positions_distinguish_unequal_depth_and_order() {
        let a = Summary::binding("identifier", 0, [1; 32], false);
        let b = Summary::binding("identifier", 1, [2; 32], false);
        let c = Summary::atom("+", b"+");
        assert_ne!(a.combine(&b).digest(), b.combine(&a).digest());
        assert_ne!(a.combine(&b).combine(&c).digest(), a.combine(&b.combine(&c)).digest());
        assert_ne!(a.combine(&a).digest(), a.combine(&b).digest());
    }

    #[test]
    fn large_skewed_summaries_keep_logarithmic_map_height() {
        let mut summary = Summary::atom("root", &[]);
        let mut saved = Vec::new();
        for key in 0..8192 {
            summary = summary.combine(&Summary::binding("identifier", key, [1; 32], true));
            if key % 128 == 0 {
                saved.push(summary.clone());
            }
        }
        audit(&summary.entries);
        assert_eq!(summary.entries.len(), 8192);
        assert!(summary.entries.height() <= 19);
        for (i, old) in saved.iter().enumerate() {
            assert_eq!(old.entries.len(), i * 128 + 1);
            audit(&old.entries);
        }
    }
}
