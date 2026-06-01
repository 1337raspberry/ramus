use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Separator joining ancestor names into a path-based genre `id`.
/// The ASCII Unit Separator (U+001F) is used instead of a printable character
/// because genre names can themselves contain any printable character — a name
/// like "Reggae / Ska / Dancehall" would otherwise be mis-split into phantom
/// ancestors when the id is decomposed for breadcrumb rendering. The separator
/// must stay in sync with `GENRE_ID_SEP` on the frontend (`ui/src/lib/types.ts`).
pub const GENRE_ID_SEP: char = '\u{1f}';

/// A node in the genre hierarchy tree.
/// `children` is `None` for leaf nodes (required for UI tree rendering).
/// `id` is path-based (ancestors joined by `GENRE_ID_SEP`) to handle genres
/// appearing in multiple subtrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreNode {
    pub id: String,
    pub name: String,
    pub short_summary: Option<String>,
    pub children: Option<Vec<GenreNode>>,
    pub album_count: usize,
    pub deduplicated_total_count: usize,
}

impl GenreNode {
    /// Build a path-based id by appending `name` (lowercased) to `parent_path`,
    /// joined by `GENRE_ID_SEP`. Single source of truth for the id formula so
    /// every construction site stays consistent.
    pub fn build_id(parent_path: &str, name: &str) -> String {
        if parent_path.is_empty() {
            name.to_lowercase()
        } else {
            format!("{parent_path}{GENRE_ID_SEP}{}", name.to_lowercase())
        }
    }

    pub fn new(
        name: String,
        parent_path: &str,
        short_summary: Option<String>,
        children: Option<Vec<GenreNode>>,
        album_count: usize,
        deduplicated_total_count: usize,
    ) -> Self {
        let id = Self::build_id(parent_path, &name);
        Self {
            id,
            name,
            short_summary,
            children,
            album_count,
            deduplicated_total_count,
        }
    }

    /// All genre names in this subtree (self + all descendants), flattened.
    pub fn all_descendant_names(&self) -> Vec<String> {
        let mut result = vec![self.name.clone()];
        if let Some(ref children) = self.children {
            for child in children {
                result.extend(child.all_descendant_names());
            }
        }
        result
    }

    /// Collect all genre names in this subtree directly into a Set.
    pub fn collect_descendant_names(&self, set: &mut HashSet<String>) {
        set.insert(self.name.clone());
        if let Some(ref children) = self.children {
            for child in children {
                child.collect_descendant_names(set);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample_tree() -> GenreNode {
        GenreNode::new(
            "Metal".into(),
            "",
            None,
            Some(vec![
                GenreNode::new(
                    "Thrash Metal".into(),
                    "metal",
                    None,
                    Some(vec![GenreNode::new(
                        "Crossover Thrash".into(),
                        &format!("metal{GENRE_ID_SEP}thrash metal"),
                        None,
                        None,
                        0,
                        0,
                    )]),
                    0,
                    0,
                ),
                GenreNode::new("Death Metal".into(), "metal", None, None, 0, 0),
                GenreNode::new("Black Metal".into(), "metal", None, None, 0, 0),
            ]),
            0,
            0,
        )
    }

    #[test]
    fn test_path_based_ids() {
        let metal = make_sample_tree();
        assert_eq!(metal.id, "metal");
        let thrash = &metal.children.as_ref().unwrap()[0];
        assert_eq!(thrash.id, format!("metal{GENRE_ID_SEP}thrash metal"));
        let crossover = &thrash.children.as_ref().unwrap()[0];
        assert_eq!(
            crossover.id,
            format!("metal{GENRE_ID_SEP}thrash metal{GENRE_ID_SEP}crossover thrash")
        );
    }

    #[test]
    fn test_all_descendant_names() {
        let metal = make_sample_tree();
        let names = metal.all_descendant_names();
        assert!(names.contains(&"Metal".to_string()));
        assert!(names.contains(&"Thrash Metal".to_string()));
        assert!(names.contains(&"Crossover Thrash".to_string()));
        assert!(names.contains(&"Death Metal".to_string()));
        assert!(names.contains(&"Black Metal".to_string()));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn test_collect_descendant_names() {
        let metal = make_sample_tree();
        let mut set = HashSet::new();
        metal.collect_descendant_names(&mut set);
        assert_eq!(set.len(), 5);
        assert!(set.contains("Metal"));
        assert!(set.contains("Crossover Thrash"));
    }

    #[test]
    fn test_leaf_node_has_none_children() {
        let leaf = GenreNode::new("Jazz".into(), "", None, None, 0, 0);
        assert!(leaf.children.is_none());
    }
}
