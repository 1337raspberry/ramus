use serde::Serialize;

// --- Errors ---

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CustomGenreParseError {
    #[error("the file is empty")]
    EmptyFile,
    #[error("this looks like JSON or another structured format — use a plain text file with indented genre names instead")]
    NotPlainText,
    #[error("file is too large ({0} bytes). Maximum is 1 MB")]
    FileTooLarge(usize),
    #[error("file has {0} lines. Maximum is 50,000")]
    TooManyLines(usize),
    #[error("line {0}: genre name is too long ({1}…). Maximum is 200 characters")]
    NameTooLong(usize, String),
    #[error("line {0}: opening '[' without a closing ']'")]
    UnmatchedBracket(usize),
    #[error("line {0}: indentation jumps from level {1} to {2}")]
    IndentationJump(usize, usize, usize),
    #[error("line {0}: nesting too deep (depth {1}). Maximum is {2}")]
    DepthTooDeep(usize, usize, usize),
    #[error("no root-level genres found")]
    NoRootGenresFound,
    #[error("JSON serialization error: {0}")]
    SerializeError(String),
}

// --- Constants ---

pub const MAX_FILE_SIZE: usize = 1_048_576;
pub const MAX_LINE_COUNT: usize = 50_000;
pub const MAX_NAME_LENGTH: usize = 200;
/// Maximum nesting depth for the genre tree. Bounds stack usage on the
/// downstream recursive walks (`convert_raw_node`, `build_lookup`,
/// `collect_akas`, `prune_node`, `compute_deduplicated_counts`,
/// `collect_descendant_names`, `node_to_json`). The bundled `open.json`
/// peaks at depth 6; 32 leaves generous headroom for hand-curated
/// custom trees while keeping every recursive walk well inside an
/// 8 MB thread stack.
pub const MAX_DEPTH: usize = 32;

// --- Parser ---

pub struct CustomGenreParser;

impl CustomGenreParser {
    /// Parse indented text and return JSON bytes in GenreMapper format.
    /// Returns the JSON data and any non-fatal warnings.
    pub fn parse(text: &str) -> Result<(Vec<u8>, Vec<String>), CustomGenreParseError> {
        let byte_count = text.len();
        if byte_count == 0 {
            return Err(CustomGenreParseError::EmptyFile);
        }
        if byte_count > MAX_FILE_SIZE {
            return Err(CustomGenreParseError::FileTooLarge(byte_count));
        }

        let raw_lines: Vec<&str> = text.lines().collect();
        let lines: Vec<String> = raw_lines.iter().map(|l| strip_control_characters(l)).collect();
        if lines.len() > MAX_LINE_COUNT {
            return Err(CustomGenreParseError::TooManyLines(lines.len()));
        }

        // 1-based line numbers for error reporting.
        let indexed_lines: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((i + 1, line.as_str()))
                }
            })
            .collect();

        if indexed_lines.is_empty() {
            return Err(CustomGenreParseError::EmptyFile);
        }

        if let Some(first_char) = indexed_lines[0].1.trim().chars().next() {
            if first_char == '{' || first_char == '[' {
                return Err(CustomGenreParseError::NotPlainText);
            }
        }

        let indent_unit = detect_indent_unit(&indexed_lines);

        // (depth, name, desc, line_number)
        let mut entries: Vec<(usize, String, Option<String>, usize)> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut previous_depth: usize = 0;

        for &(line_number, line_text) in &indexed_lines {
            let (depth, content) = measure_indent(line_text, &indent_unit);

            let (name, desc) = parse_line(content, line_number)?;

            if name.is_empty() {
                warnings.push(format!(
                    "Line {}: skipped — no genre name found.",
                    line_number
                ));
                continue;
            }

            if depth > previous_depth + 1 {
                return Err(CustomGenreParseError::IndentationJump(
                    line_number,
                    previous_depth,
                    depth,
                ));
            }

            if depth > MAX_DEPTH {
                return Err(CustomGenreParseError::DepthTooDeep(
                    line_number,
                    depth,
                    MAX_DEPTH,
                ));
            }

            if name.chars().count() > MAX_NAME_LENGTH {
                return Err(CustomGenreParseError::NameTooLong(
                    line_number,
                    name.chars().take(40).collect(),
                ));
            }

            previous_depth = depth;
            entries.push((depth, name, desc, line_number));
        }

        if !entries.iter().any(|(depth, _, _, _)| *depth == 0) {
            return Err(CustomGenreParseError::NoRootGenresFound);
        }

        let (roots, dupe_warnings) = build_tree(&entries);
        warnings.extend(dupe_warnings);

        let file = GenreFileJson {
            genres: roots.iter().map(node_to_json).collect(),
        };
        let data = serde_json::to_vec_pretty(&file)
            .map_err(|e| CustomGenreParseError::SerializeError(e.to_string()))?;

        Ok((data, warnings))
    }
}

// --- Indent Detection ---

#[derive(Debug)]
enum IndentUnit {
    Tab,
    Spaces(usize),
}

fn detect_indent_unit(lines: &[(usize, &str)]) -> IndentUnit {
    for &(_, text) in lines {
        if let Some(first) = text.chars().next() {
            if first == '\t' {
                return IndentUnit::Tab;
            }
            if first == ' ' {
                let space_count = text.chars().take_while(|&c| c == ' ').count();
                if space_count >= 4 {
                    return IndentUnit::Spaces(4);
                } else if space_count >= 2 {
                    return IndentUnit::Spaces(2);
                } else {
                    return IndentUnit::Spaces(1);
                }
            }
        }
    }
    IndentUnit::Spaces(2)
}

fn measure_indent<'a>(line: &'a str, unit: &IndentUnit) -> (usize, &'a str) {
    match unit {
        IndentUnit::Tab => {
            let tab_count = line.chars().take_while(|&c| c == '\t').count();
            let content = &line[tab_count..];
            (tab_count, content.trim())
        }
        IndentUnit::Spaces(size) => {
            let space_count = line.chars().take_while(|&c| c == ' ').count();
            let depth = space_count / size;
            let skip = depth * size;
            let content = &line[skip..];
            (depth, content.trim())
        }
    }
}

// --- Line Parsing ---

fn parse_line(content: &str, line_number: usize) -> Result<(String, Option<String>), CustomGenreParseError> {
    let bracket_start = match content.find('[') {
        Some(pos) => pos,
        None => {
            return Ok((content.trim().to_string(), None));
        }
    };

    let name = content[..bracket_start].trim().to_string();
    let after_bracket = &content[bracket_start + 1..];

    let bracket_end = match after_bracket.rfind(']') {
        Some(pos) => pos,
        None => {
            return Err(CustomGenreParseError::UnmatchedBracket(line_number));
        }
    };

    let raw_description = after_bracket[..bracket_end].trim();
    let description = if raw_description.is_empty() {
        None
    } else {
        Some(raw_description.chars().take(500).collect::<String>())
    };

    Ok((name, description))
}

// --- Tree Building ---

#[derive(Debug)]
struct ParseNode {
    name: String,
    description: Option<String>,
    children: Vec<ParseNode>,
}

fn build_tree(
    entries: &[(usize, String, Option<String>, usize)],
) -> (Vec<ParseNode>, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let mut dupe_sets: Vec<HashSet<String>> = vec![HashSet::new()];

    let mut stack: Vec<(usize, ParseNode)> = Vec::new();
    let mut roots: Vec<ParseNode> = Vec::new();

    for (depth, name, desc, line_number) in entries {
        let new_node = ParseNode {
            name: name.clone(),
            description: desc.clone(),
            children: Vec::new(),
        };

        let mut did_pop = false;
        while let Some(last) = stack.last() {
            if last.0 >= *depth {
                let popped = stack.pop().unwrap();
                if stack.is_empty() {
                    roots.push(popped.1);
                } else {
                    let parent = stack.last_mut().unwrap();
                    parent.1.children.push(popped.1);
                }
                did_pop = true;
            } else {
                break;
            }
        }

        if did_pop && *depth + 1 < dupe_sets.len() {
            dupe_sets.truncate(*depth + 1);
        }
        while dupe_sets.len() <= *depth {
            dupe_sets.push(HashSet::new());
        }

        let name_key = name.to_lowercase();
        if dupe_sets[*depth].contains(&name_key) {
            warnings.push(format!(
                "Line {}: duplicate genre \"{}\" at this level.",
                line_number, name
            ));
        } else {
            dupe_sets[*depth].insert(name_key);
        }

        stack.push((*depth, new_node));
    }

    while let Some(last) = stack.pop() {
        if stack.is_empty() {
            roots.push(last.1);
        } else {
            let parent = stack.last_mut().unwrap();
            parent.1.children.push(last.1);
        }
    }

    (roots, warnings)
}

use std::collections::HashSet;

// --- Sanitization ---

/// Strip null bytes, C0 control characters (0x00–0x1F except tab 0x09),
/// DEL (0x7F), and C1 control characters (0x80–0x9F). Preserves all Unicode.
fn strip_control_characters(input: &str) -> String {
    input
        .chars()
        .filter(|&c| {
            let v = c as u32;
            if v == 0x09 {
                return true;
            }
            if v <= 0x1F {
                return false;
            }
            if v == 0x7F {
                return false;
            }
            if (0x80..=0x9F).contains(&v) {
                return false;
            }
            true
        })
        .collect()
}

// --- JSON Output ---

#[derive(Serialize)]
struct GenreFileJson {
    genres: Vec<GenreRawJson>,
}

#[derive(Serialize)]
struct GenreRawJson {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    short_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<GenreRawJson>>,
}

fn node_to_json(node: &ParseNode) -> GenreRawJson {
    GenreRawJson {
        name: node.name.clone(),
        short_summary: node.description.clone(),
        children: if node.children.is_empty() {
            None
        } else {
            Some(node.children.iter().map(node_to_json).collect())
        },
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genre::mapper::GenreMapper;

    fn parse_and_load(text: &str) -> (GenreMapper, Vec<String>) {
        let (data, warnings) = CustomGenreParser::parse(text).unwrap();
        let mapper = GenreMapper::from_json_bytes(&data).unwrap();
        (mapper, warnings)
    }

    // --- Happy Path ---

    #[test]
    fn test_basic_hierarchy() {
        let text = "Rock\n  Alternative Rock\n    Shoegaze\n  Punk Rock\nElectronic\n  Ambient";
        let (mapper, warnings) = parse_and_load(text);
        assert!(warnings.is_empty());
        assert_eq!(mapper.root_nodes.len(), 2);
        assert_eq!(mapper.root_nodes[0].name, "Rock");
        assert_eq!(mapper.root_nodes[0].children.as_ref().unwrap().len(), 2);
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0].name,
            "Alternative Rock"
        );
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0]
                .children
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0]
                .children
                .as_ref()
                .unwrap()[0]
                .name,
            "Shoegaze"
        );
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[1].name,
            "Punk Rock"
        );
        assert_eq!(mapper.root_nodes[1].name, "Electronic");
        assert_eq!(mapper.root_nodes[1].children.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_tab_indentation() {
        let text = "Rock\n\tAlternative Rock\n\t\tShoegaze\n\tPunk Rock";
        let (mapper, warnings) = parse_and_load(text);
        assert!(warnings.is_empty());
        assert_eq!(mapper.root_nodes.len(), 1);
        assert_eq!(mapper.root_nodes[0].children.as_ref().unwrap().len(), 2);
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0]
                .children
                .as_ref()
                .unwrap()[0]
                .name,
            "Shoegaze"
        );
    }

    #[test]
    fn test_four_space_indentation() {
        let text = "Rock\n    Alternative Rock\n        Shoegaze";
        let (mapper, warnings) = parse_and_load(text);
        assert!(warnings.is_empty());
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0]
                .children
                .as_ref()
                .unwrap()[0]
                .name,
            "Shoegaze"
        );
    }

    // --- Descriptions ---

    #[test]
    fn test_optional_descriptions() {
        let text =
            "Rock[Guitar-driven music]\n  Shoegaze[Wall of sound with ethereal vocals]\n  Punk Rock\nJazz";
        let (mapper, warnings) = parse_and_load(text);
        assert!(warnings.is_empty());
        assert_eq!(
            mapper.root_nodes[0].short_summary.as_deref(),
            Some("Guitar-driven music")
        );
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0]
                .short_summary
                .as_deref(),
            Some("Wall of sound with ethereal vocals")
        );
        assert!(mapper.root_nodes[0].children.as_ref().unwrap()[1]
            .short_summary
            .is_none());
        assert!(mapper.root_nodes[1].short_summary.is_none());
    }

    #[test]
    fn test_empty_brackets_no_description() {
        let text = "Rock[]\n  Punk Rock";
        let (mapper, _) = parse_and_load(text);
        assert!(mapper.root_nodes[0].short_summary.is_none());
    }

    #[test]
    fn test_leaf_nodes_have_none_children() {
        let text = "Rock\n  Shoegaze";
        let (mapper, _) = parse_and_load(text);
        let shoegaze = &mapper.root_nodes[0].children.as_ref().unwrap()[0];
        assert!(shoegaze.children.is_none());
    }

    // --- Validation Errors ---

    #[test]
    fn test_empty_file() {
        let err = CustomGenreParser::parse("").unwrap_err();
        assert_eq!(err, CustomGenreParseError::EmptyFile);
    }

    #[test]
    fn test_whitespace_only_file() {
        let err = CustomGenreParser::parse("   \n\n  \n").unwrap_err();
        assert_eq!(err, CustomGenreParseError::EmptyFile);
    }

    #[test]
    fn test_file_too_large() {
        let big = "A".repeat(MAX_FILE_SIZE + 1);
        let err = CustomGenreParser::parse(&big).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::FileTooLarge(_)));
    }

    #[test]
    fn test_too_many_lines() {
        let lines: Vec<String> = (1..=50_001).map(|i| format!("Genre{}", i)).collect();
        let text = lines.join("\n");
        let err = CustomGenreParser::parse(&text).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::TooManyLines(_)));
    }

    #[test]
    fn test_indentation_jump() {
        let text = "Rock\n        Deep Nested"; // 8 spaces → 4-space unit → depth 2, jump from 0 to 2
        let err = CustomGenreParser::parse(text).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::IndentationJump(_, _, _)));
    }

    #[test]
    fn test_unmatched_bracket() {
        let text = "Rock[missing close bracket";
        let err = CustomGenreParser::parse(text).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::UnmatchedBracket(_)));
    }

    #[test]
    fn test_name_too_long() {
        let long_name = "A".repeat(201);
        let err = CustomGenreParser::parse(&long_name).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::NameTooLong(_, _)));
    }

    #[test]
    fn test_depth_too_deep_rejected() {
        // 33 levels of nesting (one above the cap) at one space per level.
        let mut text = String::from("Root");
        for i in 1..=MAX_DEPTH + 1 {
            text.push('\n');
            text.push_str(&" ".repeat(i));
            text.push_str(&format!("L{}", i));
        }
        let err = CustomGenreParser::parse(&text).unwrap_err();
        assert!(matches!(err, CustomGenreParseError::DepthTooDeep(_, _, _)));
    }

    #[test]
    fn test_depth_at_cap_accepted() {
        // Exactly MAX_DEPTH levels of nesting is allowed.
        let mut text = String::from("Root");
        for i in 1..=MAX_DEPTH {
            text.push('\n');
            text.push_str(&" ".repeat(i));
            text.push_str(&format!("L{}", i));
        }
        assert!(CustomGenreParser::parse(&text).is_ok());
    }

    #[test]
    fn test_json_input_rejected() {
        let json = "{\n  \"genres\": [\n    { \"name\": \"Rock\" }\n  ]\n}";
        let err = CustomGenreParser::parse(json).unwrap_err();
        assert_eq!(err, CustomGenreParseError::NotPlainText);
    }

    #[test]
    fn test_json_array_input_rejected() {
        let json = "[{\"name\": \"Rock\"}]";
        let err = CustomGenreParser::parse(json).unwrap_err();
        assert_eq!(err, CustomGenreParseError::NotPlainText);
    }

    #[test]
    fn test_no_root_genres() {
        let text = "  Indented Only\n  Another Indented";
        let err = CustomGenreParser::parse(text).unwrap_err();
        assert_eq!(err, CustomGenreParseError::NoRootGenresFound);
    }

    // --- Warnings ---

    #[test]
    fn test_duplicate_name_warning() {
        let text = "Rock\nJazz\nRock";
        let (_, warnings) = CustomGenreParser::parse(text).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("duplicate"));
        assert!(warnings[0].contains("Rock"));
    }

    #[test]
    fn test_duplicate_case_insensitive() {
        let text = "Rock\nrock";
        let (_, warnings) = CustomGenreParser::parse(text).unwrap();
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn test_duplicates_allowed_across_parents() {
        let text = "Electronic\n  Funk\nR&B\n  Funk";
        let (_, warnings) = CustomGenreParser::parse(text).unwrap();
        assert!(
            warnings.is_empty(),
            "Same name under different parents should not warn"
        );
    }

    // --- Edge Cases ---

    #[test]
    fn test_blank_lines_ignored() {
        let text = "Rock\n\n  Shoegaze\n\n\nJazz";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes.len(), 2);
    }

    #[test]
    fn test_c0_control_characters_stripped() {
        let text = "Rock\u{00}\u{01}\n  Shoegaze";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes[0].name, "Rock");
    }

    #[test]
    fn test_c1_control_characters_stripped() {
        let text = "Ro\u{0080}ck\u{008B}\u{0090}\n  Shoe\u{009F}gaze";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes[0].name, "Rock");
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0].name,
            "Shoegaze"
        );
    }

    #[test]
    fn test_unicode_preserved() {
        let text = "Música Brasileira\n  Bossa Nova\nJazz café";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes[0].name, "Música Brasileira");
        assert_eq!(mapper.root_nodes[1].name, "Jazz Café");
    }

    #[test]
    fn test_mixed_tabs_and_spaces_uses_detected_unit() {
        let text = "Rock\n\tAlternative Rock\n  Punk Rock";
        let (mapper, _) = parse_and_load(text);
        // First indented line uses tab → indent unit locks to tab.
        // Space-indented line gets depth 0 (treated as root).
        assert_eq!(
            mapper.root_nodes.len(),
            2,
            "Space-indented line becomes root when unit is tabs"
        );
        assert_eq!(mapper.root_nodes[0].name, "Rock");
        assert_eq!(mapper.root_nodes[0].children.as_ref().unwrap().len(), 1);
        assert_eq!(mapper.root_nodes[1].name, "Punk Rock");
    }

    #[test]
    fn test_single_root_genre() {
        let text = "Rock";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes.len(), 1);
        assert_eq!(mapper.root_nodes[0].name, "Rock");
        assert!(mapper.root_nodes[0].children.is_none());
    }

    #[test]
    fn test_deeply_nested() {
        let mut text = "Level0".to_string();
        for i in 1..=8 {
            text.push('\n');
            text.push_str(&"  ".repeat(i));
            text.push_str(&format!("Level{}", i));
        }
        let (mapper, _) = parse_and_load(&text);
        let mut node = &mapper.root_nodes[0];
        for i in 1..=8 {
            assert_eq!(node.children.as_ref().unwrap().len(), 1);
            node = &node.children.as_ref().unwrap()[0];
            assert_eq!(node.name, format!("Level{}", i));
        }
    }

    #[test]
    fn test_description_with_brackets_inside() {
        let text = "Rock[includes [sub]genres]";
        let (mapper, _) = parse_and_load(text);
        assert_eq!(mapper.root_nodes[0].name, "Rock");
        assert_eq!(
            mapper.root_nodes[0].short_summary.as_deref(),
            Some("includes [sub]genres")
        );
    }

    #[test]
    fn test_description_only_line_skipped() {
        let text = "Rock\n  [just a description]\n  Punk Rock";
        let (mapper, warnings) = parse_and_load(text);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no genre name"));
        assert_eq!(mapper.root_nodes[0].children.as_ref().unwrap().len(), 1);
        assert_eq!(
            mapper.root_nodes[0].children.as_ref().unwrap()[0].name,
            "Punk Rock"
        );
    }

    #[test]
    fn test_same_child_name_under_different_parents_no_warning() {
        let text = "Rock\n  Alternative Rock\n  Punk Rock\nJazz\n  Alternative Rock";
        let (_, warnings) = CustomGenreParser::parse(text).unwrap();
        assert!(
            warnings.is_empty(),
            "Same child under different parents should not warn"
        );
    }

    #[test]
    fn test_root_duplicate_still_detected() {
        let text = "Rock\n  Punk\nJazz\nRock";
        let (_, warnings) = CustomGenreParser::parse(text).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Rock"));
    }

    #[test]
    fn test_round_trip_through_genre_mapper() {
        let text = "Metal[Heavy guitar music]\n  Thrash Metal[Fast and aggressive]\n    Crossover Thrash\n  Death Metal\n  Black Metal\nRock\n  Progressive Rock";
        let (data, _) = CustomGenreParser::parse(text).unwrap();
        let mapper = GenreMapper::from_json_bytes(&data).unwrap();

        // Verify the mapper can match genres
        assert!(mapper.match_genre("thrash metal").is_some());
        assert!(mapper.match_genre("Progressive Rock").is_some());
        assert_eq!(
            mapper.match_genre("crossover thrash").unwrap().name,
            "Crossover Thrash"
        );

        // Verify display tree works
        let mut sets = std::collections::HashMap::new();
        sets.insert("Death Metal".into(), [1i64, 2].into());
        sets.insert("Rock".into(), [3i64].into());
        let tree = mapper.build_display_tree(&sets);
        assert!(!tree.is_empty());
    }
}
