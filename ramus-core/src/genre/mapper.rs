use std::collections::{HashMap, HashSet};
use std::path::Path;

use parking_lot::Mutex;

use crate::genre::node::GenreNode;
use crate::search::engine::GenreExpander;

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum GenreMapperError {
    #[error("genre JSON file not found: {0}")]
    FileNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

// --- JSON types for deserialization ---

#[derive(serde::Deserialize)]
struct GenreFile {
    genres: Vec<GenreNodeRaw>,
}

#[derive(serde::Deserialize)]
struct GenreNodeRaw {
    name: String,
    short_summary: Option<String>,
    children: Option<Vec<GenreNodeRaw>>,
}

// --- GenreMapper ---

pub struct GenreMapper {
    /// The full hierarchy as loaded from JSON.
    pub root_nodes: Vec<GenreNode>,
    /// Case-insensitive lookup: lowercased genre name → list of nodes
    /// with that name. A list (not a single node) because the beets
    /// hierarchy — and user-imported custom trees even more so — can
    /// have the same display name under multiple parents (e.g. "Funk"
    /// under both R&B and Pop). Matching must expand all of them or
    /// `expand_genre` silently returns an incomplete subtree.
    exact_lookup: HashMap<String, Vec<GenreNode>>,
    /// All genre names for fuzzy search.
    all_names: Vec<String>,
    /// Cache for matchGenre results.
    cache: Mutex<MatchCache>,
}

struct MatchCache {
    matches: HashMap<String, Vec<GenreNode>>,
    misses: HashSet<String>,
}

impl GenreMapper {
    /// Load from a genre hierarchy JSON file.
    pub fn from_json_file(path: &Path) -> Result<Self, GenreMapperError> {
        let data = std::fs::read(path)?;
        Self::from_json_bytes(&data)
    }

    /// Load from raw JSON bytes.
    pub fn from_json_bytes(data: &[u8]) -> Result<Self, GenreMapperError> {
        let raw: GenreFile = serde_json::from_slice(data)?;
        let nodes: Vec<GenreNode> = raw
            .genres
            .iter()
            .map(|r| Self::convert_raw_node(r, ""))
            .collect();

        let mut lookup: HashMap<String, Vec<GenreNode>> = HashMap::new();
        Self::build_lookup(&nodes, &mut lookup);
        // Deduplicate display names so fuzzy search doesn't try the same
        // string multiple times when a name appears in several subtrees.
        let mut seen: HashSet<String> = HashSet::new();
        let all_names: Vec<String> = lookup
            .values()
            .flat_map(|v| v.iter())
            .filter_map(|n| {
                if seen.insert(n.name.to_lowercase()) {
                    Some(n.name.clone())
                } else {
                    None
                }
            })
            .collect();

        Ok(Self {
            root_nodes: nodes,
            exact_lookup: lookup,
            all_names,
            cache: Mutex::new(MatchCache {
                matches: HashMap::new(),
                misses: HashSet::new(),
            }),
        })
    }

    /// Match a Plex genre string to the genre hierarchy.
    /// Tries exact (case-insensitive) first, then fuzzy via strsim.
    ///
    /// Returns the first matching node when a name appears under
    /// multiple parents. Callers that need to collect descendants
    /// across all same-named subtrees should go via `expand_genre` on
    /// the `GenreExpander` trait, which unions all of them.
    pub fn match_genre(&self, plex_genre: &str) -> Option<GenreNode> {
        self.match_all(plex_genre).into_iter().next()
    }

    /// Match a Plex genre string to every node in the hierarchy that
    /// shares that display name. Returns an empty Vec on miss.
    fn match_all(&self, plex_genre: &str) -> Vec<GenreNode> {
        let key = plex_genre.to_lowercase();

        // Check caches first
        {
            let cache = self.cache.lock();
            if let Some(nodes) = cache.matches.get(&key) {
                return nodes.clone();
            }
            if cache.misses.contains(&key) {
                return Vec::new();
            }
        }

        // Exact match
        if let Some(nodes) = self.exact_lookup.get(&key) {
            let nodes = nodes.clone();
            self.cache.lock().matches.insert(key, nodes.clone());
            return nodes;
        }

        // Fuzzy fallback via strsim (expensive — runs outside lock)
        let mut best_score = 0.0_f64;
        let mut best_name: Option<&str> = None;

        for name in &self.all_names {
            let score = strsim::jaro_winkler(&name.to_lowercase(), &key);
            if score > best_score {
                best_score = score;
                best_name = Some(name);
            }
        }

        // Threshold ~0.8 for jaro_winkler (maps to ~0.4 Fuse threshold)
        if best_score > 0.8 {
            if let Some(name) = best_name {
                if let Some(nodes) = self.exact_lookup.get(&name.to_lowercase()) {
                    let nodes = nodes.clone();
                    self.cache.lock().matches.insert(key, nodes.clone());
                    return nodes;
                }
            }
        }

        self.cache.lock().misses.insert(key);
        Vec::new()
    }

    /// Build a display tree from album sets, pruning empty branches and computing
    /// deduplicated subtree counts via set unions.
    pub fn build_display_tree(
        &self,
        genre_album_sets: &HashMap<String, HashSet<i64>>,
    ) -> Vec<GenreNode> {
        let lowered: HashMap<String, &HashSet<i64>> = genre_album_sets
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();

        let mut matched_names = HashSet::new();
        let mut pruned: Vec<GenreNode> = self
            .root_nodes
            .iter()
            .filter_map(|n| Self::prune_node(n, &lowered, "", &mut matched_names))
            .collect();

        // Post-order traversal to compute deduplicated counts
        for node in &mut pruned {
            Self::compute_deduplicated_counts(node, &lowered);
        }

        // Collect unmatched genres into an "Other" node
        let unmatched: HashMap<&String, &HashSet<i64>> = genre_album_sets
            .iter()
            .filter(|(k, _)| !matched_names.contains(&k.to_lowercase()))
            .collect();

        if !unmatched.is_empty() {
            let mut other_children: Vec<GenreNode> = unmatched
                .iter()
                .map(|(name, albums)| {
                    GenreNode::new(
                        (*name).clone(),
                        "other",
                        None,
                        None,
                        albums.len(),
                        albums.len(),
                    )
                })
                .collect();
            other_children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            let other_union: HashSet<i64> = unmatched
                .values()
                .flat_map(|s| s.iter().copied())
                .collect();

            let other = GenreNode::new(
                "Other".into(),
                "",
                None,
                Some(other_children),
                0,
                other_union.len(),
            );
            pruned.push(other);
        }

        pruned
    }

    fn convert_raw_node(raw: &GenreNodeRaw, parent_path: &str) -> GenreNode {
        let display_name = title_case(&raw.name);
        let node_path = if parent_path.is_empty() {
            raw.name.to_lowercase()
        } else {
            format!("{}/{}", parent_path, raw.name.to_lowercase())
        };

        let children: Option<Vec<GenreNode>> = match &raw.children {
            Some(kids) if !kids.is_empty() => {
                Some(kids.iter().map(|k| Self::convert_raw_node(k, &node_path)).collect())
            }
            _ => None,
        };

        GenreNode::new(display_name, parent_path, raw.short_summary.clone(), children, 0, 0)
    }

    // --- Private: Lookup ---

    fn build_lookup(nodes: &[GenreNode], lookup: &mut HashMap<String, Vec<GenreNode>>) {
        for node in nodes {
            lookup
                .entry(node.name.to_lowercase())
                .or_default()
                .push(node.clone());
            if let Some(ref children) = node.children {
                Self::build_lookup(children, lookup);
            }
        }
    }

    // --- Private: Pruning ---

    fn prune_node(
        node: &GenreNode,
        album_sets: &HashMap<String, &HashSet<i64>>,
        parent_path: &str,
        matched_names: &mut HashSet<String>,
    ) -> Option<GenreNode> {
        let direct_count = album_sets
            .get(&node.name.to_lowercase())
            .map(|s| s.len())
            .unwrap_or(0);

        let pruned_children: Option<Vec<GenreNode>> = node.children.as_ref().and_then(|children| {
            let kept: Vec<GenreNode> = children
                .iter()
                .filter_map(|c| Self::prune_node(c, album_sets, &node.id, matched_names))
                .collect();
            if kept.is_empty() {
                None
            } else {
                Some(kept)
            }
        });

        if direct_count > 0 || pruned_children.is_some() {
            matched_names.insert(node.name.to_lowercase());
            Some(GenreNode::new(
                node.name.clone(),
                parent_path,
                node.short_summary.clone(),
                pruned_children,
                direct_count,
                0,
            ))
        } else {
            None
        }
    }

    fn compute_deduplicated_counts(
        node: &mut GenreNode,
        album_sets: &HashMap<String, &HashSet<i64>>,
    ) -> HashSet<i64> {
        let mut union_set: HashSet<i64> = album_sets
            .get(&node.name.to_lowercase())
            .map(|s| (*s).clone())
            .unwrap_or_default();

        if let Some(ref mut children) = node.children {
            for child in children.iter_mut() {
                let child_set = Self::compute_deduplicated_counts(child, album_sets);
                union_set.extend(child_set);
            }
        }

        node.deduplicated_total_count = union_set.len();
        union_set
    }
}

impl GenreExpander for GenreMapper {
    fn expand_genre(&self, name: &str) -> Option<HashSet<String>> {
        let nodes = self.match_all(name);
        if nodes.is_empty() {
            return None;
        }
        let mut set = HashSet::new();
        for node in &nodes {
            node.collect_descendant_names(&mut set);
        }
        Some(set)
    }
}

// --- Title-casing ---

/// Title-case a genre name for display.
/// All-lowercase words get first letter capitalised.
/// Words with any uppercase are left untouched (preserves "EBM", "R&B").
/// Handles hyphenated compounds ("lo-fi" → "Lo-Fi").
pub fn title_case(input: &str) -> String {
    input
        .split(' ')
        .map(|word| {
            word.split('-')
                .map(|segment| {
                    if segment.chars().all(|c| !c.is_uppercase()) {
                        // All lowercase → capitalize first letter
                        let mut chars = segment.chars();
                        match chars.next() {
                            Some(first) => {
                                let upper: String = first.to_uppercase().collect();
                                format!("{}{}", upper, chars.as_str())
                            }
                            None => String::new(),
                        }
                    } else {
                        // Has uppercase → leave it
                        segment.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    fn make_mapper(json: &str) -> GenreMapper {
        GenreMapper::from_json_bytes(json.as_bytes()).unwrap()
    }

    fn make_mapper_from_names(names: &[&str]) -> GenreMapper {
        let genres: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"name":"{}","children":[]}}"#, n))
            .collect();
        let json = format!(r#"{{"genres":[{}]}}"#, genres.join(","));
        make_mapper(&json)
    }

    const SAMPLE_JSON: &str = r#"{
      "genres": [
        {
          "name": "Metal",
          "children": [
            {
              "name": "Thrash Metal",
              "children": [
                { "name": "Crossover Thrash", "children": [] }
              ]
            },
            { "name": "Death Metal", "children": [] },
            { "name": "Black Metal", "children": [] }
          ]
        },
        {
          "name": "Rock",
          "children": [
            { "name": "Progressive Rock", "children": [] },
            { "name": "Alternative Rock", "children": [] }
          ]
        }
      ]
    }"#;

    // --- Tree Loading ---

    #[test]
    fn test_load_tree_from_json() {
        let mapper = make_mapper(SAMPLE_JSON);
        assert_eq!(mapper.root_nodes.len(), 2);
        assert_eq!(mapper.root_nodes[0].name, "Metal");
        assert_eq!(mapper.root_nodes[0].children.as_ref().unwrap().len(), 3);
        assert_eq!(mapper.root_nodes[1].name, "Rock");
    }

    #[test]
    fn test_leaf_nodes_have_none_children() {
        let mapper = make_mapper(SAMPLE_JSON);
        let death_metal = mapper.root_nodes[0]
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Death Metal")
            .unwrap();
        assert!(death_metal.children.is_none());
    }

    // --- Path-Based IDs ---

    #[test]
    fn test_path_based_ids() {
        let mapper = make_mapper(SAMPLE_JSON);
        let metal = &mapper.root_nodes[0];
        assert_eq!(metal.id, "metal");
        let thrash = metal
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Thrash Metal")
            .unwrap();
        assert_eq!(thrash.id, "metal/thrash metal");
        let crossover = thrash
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Crossover Thrash")
            .unwrap();
        assert_eq!(crossover.id, "metal/thrash metal/crossover thrash");
    }

    #[test]
    fn test_duplicate_genre_names_have_unique_ids() {
        let json = r#"{
          "genres": [
            { "name": "R&B", "children": [{ "name": "Funk", "children": [] }] },
            { "name": "Pop", "children": [{ "name": "Funk", "children": [] }] }
          ]
        }"#;
        let mapper = make_mapper(json);
        let rb_funk = &mapper.root_nodes[0].children.as_ref().unwrap()[0];
        let pop_funk = &mapper.root_nodes[1].children.as_ref().unwrap()[0];
        assert_eq!(rb_funk.name, "Funk");
        assert_eq!(pop_funk.name, "Funk");
        assert_ne!(rb_funk.id, pop_funk.id);
        assert_eq!(rb_funk.id, "r&b/funk");
        assert_eq!(pop_funk.id, "pop/funk");
    }

    #[test]
    fn test_expand_genre_unions_duplicate_name_subtrees() {
        // Two nodes named "Funk" under different parents, each with
        // a distinct descendant. `expand_genre("Funk")` must return
        // the union of both subtrees, not just one.
        let json = r#"{
          "genres": [
            {
              "name": "R&B",
              "children": [{
                "name": "Funk",
                "children": [{ "name": "P-Funk", "children": [] }]
              }]
            },
            {
              "name": "Pop",
              "children": [{
                "name": "Funk",
                "children": [{ "name": "Disco Funk", "children": [] }]
              }]
            }
          ]
        }"#;
        let mapper = make_mapper(json);
        let expanded = mapper.expand_genre("Funk").expect("Funk should match");
        assert!(expanded.contains("Funk"));
        assert!(expanded.contains("P-Funk"));
        assert!(expanded.contains("Disco Funk"));
        assert_eq!(expanded.len(), 3);
    }

    // --- Exact Matching ---

    #[test]
    fn test_exact_match_case_insensitive() {
        let mapper = make_mapper(SAMPLE_JSON);
        let node = mapper.match_genre("thrash metal");
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "Thrash Metal");
    }

    #[test]
    fn test_exact_match_mixed_case() {
        let mapper = make_mapper(SAMPLE_JSON);
        let node = mapper.match_genre("PROGRESSIVE ROCK");
        assert_eq!(node.unwrap().name, "Progressive Rock");
    }

    // --- Fuzzy Matching ---

    #[test]
    fn test_fuzzy_match_close_spelling() {
        let mapper = make_mapper(SAMPLE_JSON);
        let node = mapper.match_genre("Progressve Rock");
        assert!(node.is_some(), "Fuzzy match should find 'Progressive Rock' for close spelling");
        assert_eq!(node.unwrap().name, "Progressive Rock");
    }

    #[test]
    fn test_fuzzy_match_slight_typo() {
        let mapper = make_mapper(SAMPLE_JSON);
        let node = mapper.match_genre("Deth Metal");
        assert!(node.is_some());
        assert_eq!(node.unwrap().name, "Death Metal");
    }

    #[test]
    fn test_no_match_returns_none() {
        let mapper = make_mapper(SAMPLE_JSON);
        let node = mapper.match_genre("Baroque Chamber Opera");
        assert!(node.is_none(), "Completely unrelated genre should not match");
    }

    // --- Display Tree & Pruning ---

    #[test]
    fn test_build_display_tree_prunes_empty_branches() {
        let mapper = make_mapper(SAMPLE_JSON);
        let mut sets: HashMap<String, HashSet<i64>> = HashMap::new();
        sets.insert("Death Metal".into(), [1, 2, 3, 4, 5].into());
        sets.insert("Rock".into(), [10, 11].into());

        let tree = mapper.build_display_tree(&sets);

        let metal = tree.iter().find(|n| n.name == "Metal").unwrap();
        let thrash = metal
            .children
            .as_ref()
            .and_then(|c| c.iter().find(|n| n.name == "Thrash Metal"));
        assert!(thrash.is_none(), "Thrash Metal should be pruned — no albums");
        let death = metal
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Death Metal")
            .unwrap();
        assert_eq!(death.album_count, 5);

        let rock = tree.iter().find(|n| n.name == "Rock").unwrap();
        assert_eq!(rock.album_count, 2);
        assert!(
            rock.children.is_none(),
            "Rock has no surviving children so children should be nil"
        );
    }

    #[test]
    fn test_build_display_tree_empty_sets() {
        let mapper = make_mapper(SAMPLE_JSON);
        let tree = mapper.build_display_tree(&HashMap::new());
        assert!(tree.is_empty(), "Empty sets should produce empty tree");
    }

    // --- Deduplicated Album Counts ---

    #[test]
    fn test_deduplicated_count_with_shared_album() {
        let mapper = make_mapper(SAMPLE_JSON);
        let mut sets: HashMap<String, HashSet<i64>> = HashMap::new();
        sets.insert("Thrash Metal".into(), [1, 2].into());
        sets.insert("Death Metal".into(), [1, 3].into()); // album 1 shared

        let tree = mapper.build_display_tree(&sets);
        let metal = tree.iter().find(|n| n.name == "Metal").unwrap();

        assert_eq!(metal.deduplicated_total_count, 3); // albums 1, 2, 3
        let thrash = metal
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Thrash Metal")
            .unwrap();
        assert_eq!(thrash.album_count, 2);
        assert_eq!(thrash.deduplicated_total_count, 2);
        let death = metal
            .children
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.name == "Death Metal")
            .unwrap();
        assert_eq!(death.album_count, 2);
        assert_eq!(death.deduplicated_total_count, 2);
    }

    #[test]
    fn test_deduplicated_count_parent_and_child() {
        let mapper = make_mapper(SAMPLE_JSON);
        let mut sets: HashMap<String, HashSet<i64>> = HashMap::new();
        sets.insert("Metal".into(), [1].into());
        sets.insert("Death Metal".into(), [1, 2].into());

        let tree = mapper.build_display_tree(&sets);
        let metal = tree.iter().find(|n| n.name == "Metal").unwrap();

        assert_eq!(metal.deduplicated_total_count, 2); // only 2 unique albums (1, 2)
    }

    #[test]
    fn test_all_descendant_names() {
        let mapper = make_mapper(SAMPLE_JSON);
        let metal = mapper.root_nodes.iter().find(|n| n.name == "Metal").unwrap();
        let names = metal.all_descendant_names();
        assert!(names.contains(&"Metal".to_string()));
        assert!(names.contains(&"Thrash Metal".to_string()));
        assert!(names.contains(&"Crossover Thrash".to_string()));
        assert!(names.contains(&"Death Metal".to_string()));
        assert!(names.contains(&"Black Metal".to_string()));
        assert_eq!(names.len(), 5);
    }

    // --- Title Case Tests ---

    #[test]
    fn test_all_lowercase_gets_title_cased() {
        let mapper = make_mapper_from_names(&["ambient music"]);
        assert_eq!(mapper.root_nodes[0].name, "Ambient Music");
    }

    #[test]
    fn test_acronym_left_alone() {
        let mapper = make_mapper_from_names(&["EBM"]);
        assert_eq!(mapper.root_nodes[0].name, "EBM");
    }

    #[test]
    fn test_ampersand_acronym_left_alone() {
        let mapper = make_mapper_from_names(&["R&B"]);
        assert_eq!(mapper.root_nodes[0].name, "R&B");
    }

    #[test]
    fn test_hyphenated_compound() {
        let mapper = make_mapper_from_names(&["lo-fi"]);
        assert_eq!(mapper.root_nodes[0].name, "Lo-Fi");
    }

    #[test]
    fn test_mixed_case_per_word_preservation() {
        let mapper = make_mapper_from_names(&["death Metal"]);
        assert_eq!(mapper.root_nodes[0].name, "Death Metal");
    }

    #[test]
    fn test_word_with_existing_uppercase_left_alone() {
        let mapper = make_mapper_from_names(&["dEath metal"]);
        assert_eq!(mapper.root_nodes[0].name, "dEath Metal");
    }

    #[test]
    fn test_single_lowercase_word() {
        let mapper = make_mapper_from_names(&["jazz"]);
        assert_eq!(mapper.root_nodes[0].name, "Jazz");
    }

    #[test]
    fn test_empty_string_returns_empty() {
        let mapper = make_mapper_from_names(&[""]);
        assert_eq!(mapper.root_nodes[0].name, "");
    }

    #[test]
    fn test_multiple_hyphen_segments() {
        let mapper = make_mapper_from_names(&["drum-and-bass"]);
        assert_eq!(mapper.root_nodes[0].name, "Drum-And-Bass");
    }

    #[test]
    fn test_mixed_hyphen_segments() {
        let mapper = make_mapper_from_names(&["lo-FI"]);
        assert_eq!(mapper.root_nodes[0].name, "Lo-FI");
    }
}
