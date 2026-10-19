//! Copy-on-write storage for immutable syntax and its disposable indexes.
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// An inventory buffer shared by candidate views until either view changes it.
/// Serialization is exactly the wrapped value; sharing never changes identities.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SharedInventory<T>(Arc<T>);

impl<T> SharedInventory<T> {
    pub fn iter<'a>(&'a self) -> <&'a T as IntoIterator>::IntoIter
    where
        &'a T: IntoIterator,
    {
        self.0.as_ref().into_iter()
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl<T: Clone> SharedInventory<T> {
    pub fn iter_mut<'a>(&'a mut self) -> <&'a mut T as IntoIterator>::IntoIter
    where
        &'a mut T: IntoIterator,
    {
        Arc::make_mut(&mut self.0).into_iter()
    }
    pub fn into_inner(self) -> T {
        Arc::unwrap_or_clone(self.0)
    }
}
impl<T: Default> Default for SharedInventory<T> {
    fn default() -> Self {
        Self(Arc::new(T::default()))
    }
}
impl<T> From<T> for SharedInventory<T> {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}
impl<T> Deref for SharedInventory<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: Clone> DerefMut for SharedInventory<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}
impl<'a, T> IntoIterator for &'a SharedInventory<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.as_ref().into_iter()
    }
}
impl<'a, T: Clone> IntoIterator for &'a mut SharedInventory<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0).into_iter()
    }
}
impl<T: Clone + IntoIterator> IntoIterator for SharedInventory<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.into_inner().into_iter()
    }
}
impl<A, T: FromIterator<A>> FromIterator<A> for SharedInventory<T> {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        T::from_iter(iter).into()
    }
}

#[cfg(test)]
mod tests {
    use crate::{NodeKind, TopologyGraph, TopologyNode};

    #[test]
    fn candidates_share_syntax_but_mutations_preserve_the_original() {
        let mut original = TopologyGraph::new("/fixture");
        original.upsert_node(TopologyNode::shell("file-a", NodeKind::File, "a.py", None));
        let bytes = serde_json::to_vec(&original).unwrap();
        let mut candidate = original.clone();
        assert!(original.nodes.shares_storage(&candidate.nodes));
        assert!(original.node_by_id.shares_storage(&candidate.node_by_id));
        assert_eq!(serde_json::to_vec(&candidate).unwrap(), bytes);
        candidate.remove_artifacts(&[]);
        assert!(original.nodes.shares_storage(&candidate.nodes));
        assert!(original.node_by_id.shares_storage(&candidate.node_by_id));
        candidate.nodes[0].label = "changed.py".into();
        assert!(!original.nodes.shares_storage(&candidate.nodes));
        assert_eq!(original.nodes[0].label, "a.py");
        assert_eq!(serde_json::to_vec(&original).unwrap(), bytes);
        assert_eq!(
            serde_json::to_value(&original.nodes).unwrap(),
            serde_json::to_value(original.nodes.as_slice()).unwrap()
        );
    }
}
