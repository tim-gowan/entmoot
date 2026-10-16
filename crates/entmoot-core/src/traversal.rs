//! Language-neutral, iterative syntax traversal. No dialect or semantic policy.
use tree_sitter::Node;

pub(crate) fn preorder(root: Node<'_>) -> Vec<Node<'_>> {
    iter(root).collect()
}

/// Lazy preorder, allowing callers to stop before unrelated subtrees.
pub(crate) fn iter(root: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    let mut cursor = root.walk();
    let mut first = true;
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        if !first && !cursor.goto_first_child() {
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    done = true;
                    return None;
                }
            }
        }
        first = false;
        Some(cursor.node())
    })
}
