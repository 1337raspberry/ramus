import Foundation

// MARK: - Errors

/// Errors from parsing a custom genre text file.
public enum CustomGenreParseError: Error, LocalizedError, Sendable {
    case emptyFile
    case fileTooLarge(bytes: Int)
    case tooManyLines(count: Int)
    case nameTooLong(line: Int, name: String)
    case unmatchedBracket(line: Int)
    case indentationJump(line: Int, fromLevel: Int, toLevel: Int)
    case noRootGenresFound
    case notPlainText

    public var errorDescription: String? {
        switch self {
        case .emptyFile:
            "The file is empty."
        case .notPlainText:
            "This looks like JSON or another structured format. Please use a plain text file with indented genre names instead."
        case .fileTooLarge(let bytes):
            "File is too large (\(bytes / 1024) KB). Maximum is 1 MB."
        case .tooManyLines(let count):
            "File has \(count) lines. Maximum is 50,000."
        case .nameTooLong(let line, let name):
            "Line \(line): genre name is too long (\(name.prefix(40))…). Maximum is 200 characters."
        case .unmatchedBracket(let line):
            "Line \(line): opening '[' without a closing ']'."
        case .indentationJump(let line, let fromLevel, let toLevel):
            "Line \(line): indentation jumps from level \(fromLevel) to \(toLevel). Expected at most level \(fromLevel + 1)."
        case .noRootGenresFound:
            "No root-level genres found. Check that at least one line has no leading indentation."
        }
    }
}

// MARK: - Parser

/// Parses an indented text file into a genre hierarchy JSON blob
/// compatible with the JSON format that `GenreMapper` consumes.
public struct CustomGenreParser: Sendable {

    /// Maximum file size in bytes (1 MB).
    public static let maxFileSize = 1_048_576
    /// Maximum number of lines.
    public static let maxLineCount = 50_000
    /// Maximum genre name length in characters.
    public static let maxNameLength = 200

    /// Parse indented text and return JSON data in GenreMapper JSON format.
    /// Throws `CustomGenreParseError` on hard failures.
    /// Returns the JSON data and any non-fatal warnings.
    public static func parse(_ text: String) throws -> (jsonData: Data, warnings: [String]) {
        // Size check (UTF-8 byte count)
        let byteCount = text.utf8.count
        guard byteCount > 0 else { throw CustomGenreParseError.emptyFile }
        guard byteCount <= maxFileSize else { throw CustomGenreParseError.fileTooLarge(bytes: byteCount) }

        // Split into lines, strip control characters
        let rawLines = text.components(separatedBy: .newlines)
        let lines = rawLines.map { stripControlCharacters($0) }
        guard lines.count <= maxLineCount else { throw CustomGenreParseError.tooManyLines(count: lines.count) }

        // Filter to non-empty lines (keep original indices for error reporting)
        let indexedLines: [(lineNumber: Int, text: String)] = lines.enumerated().compactMap { i, line in
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty else { return nil }
            return (i + 1, line) // 1-based line numbers
        }
        guard !indexedLines.isEmpty else { throw CustomGenreParseError.emptyFile }

        // Detect JSON input and give a clear error
        if let firstChar = indexedLines.first?.text.trimmingCharacters(in: .whitespaces).first, firstChar == "{" || firstChar == "[" {
            throw CustomGenreParseError.notPlainText
        }

        // Detect indent unit from first indented line
        let indentUnit = detectIndentUnit(indexedLines)

        // Parse lines into a flat list of (depth, name, description, lineNumber)
        var entries: [(depth: Int, name: String, description: String?, lineNumber: Int)] = []
        var warnings: [String] = []
        var previousDepth = 0

        for (lineNumber, lineText) in indexedLines {
            let (depth, content) = measureIndent(lineText, unit: indentUnit)

            // Parse name and optional [description]
            let (name, desc) = try parseLine(content, lineNumber: lineNumber)

            // Skip lines with no genre name (e.g. "[description only]")
            guard !name.isEmpty else {
                warnings.append("Line \(lineNumber): skipped — no genre name found.")
                continue
            }

            // Validate indentation doesn't jump (only for lines that contribute to the tree)
            if depth > previousDepth + 1 {
                throw CustomGenreParseError.indentationJump(
                    line: lineNumber, fromLevel: previousDepth, toLevel: depth
                )
            }

            // Validate name length
            guard name.count <= maxNameLength else {
                throw CustomGenreParseError.nameTooLong(line: lineNumber, name: name)
            }

            previousDepth = depth
            entries.append((depth, name, desc, lineNumber))
        }

        // Must have at least one root-level genre
        guard entries.contains(where: { $0.depth == 0 }) else {
            throw CustomGenreParseError.noRootGenresFound
        }

        // Build tree using a stack
        let (roots, dupeWarnings) = buildTree(from: entries)
        warnings.append(contentsOf: dupeWarnings)

        // Serialize to JSON
        let file = GenreFileJSON(genres: roots.map { nodeToJSON($0) })
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(file)

        return (data, warnings)
    }

    // MARK: - Indent Detection

    /// Detect the indent unit from the first indented line.
    /// Returns the indent string (e.g. "\t", "  ", "    ").
    /// Defaults to 2 spaces if no indented lines found.
    private static func detectIndentUnit(_ lines: [(lineNumber: Int, text: String)]) -> IndentUnit {
        for (_, text) in lines {
            guard let first = text.first, first == "\t" || first == " " else { continue }
            if first == "\t" {
                return .tab
            }
            // Count leading spaces
            let spaceCount = text.prefix(while: { $0 == " " }).count
            if spaceCount >= 4 {
                return .spaces(4)
            } else if spaceCount >= 2 {
                return .spaces(2)
            } else {
                return .spaces(1)
            }
        }
        return .spaces(2) // default
    }

    private enum IndentUnit {
        case tab
        case spaces(Int)
    }

    /// Measure the indent depth and return the content after stripping indentation.
    private static func measureIndent(_ line: String, unit: IndentUnit) -> (depth: Int, content: String) {
        switch unit {
        case .tab:
            let tabCount = line.prefix(while: { $0 == "\t" }).count
            let content = String(line.dropFirst(tabCount))
            return (tabCount, content.trimmingCharacters(in: .whitespaces))
        case .spaces(let size):
            let spaceCount = line.prefix(while: { $0 == " " }).count
            let depth = spaceCount / size
            let content = String(line.dropFirst(depth * size))
            return (depth, content.trimmingCharacters(in: .whitespaces))
        }
    }

    // MARK: - Line Parsing

    /// Parse a content string into (name, optional description).
    /// `content` has already had its indentation stripped.
    private static func parseLine(_ content: String, lineNumber: Int) throws -> (name: String, description: String?) {
        guard let bracketStart = content.firstIndex(of: "[") else {
            // No description — entire content is the name
            let name = content.trimmingCharacters(in: .whitespaces)
            return (name, nil)
        }

        // Has a bracket — extract name before it and description inside
        let name = String(content[content.startIndex..<bracketStart]).trimmingCharacters(in: .whitespaces)
        let afterBracket = content[content.index(after: bracketStart)...]

        guard let bracketEnd = afterBracket.lastIndex(of: "]") else {
            throw CustomGenreParseError.unmatchedBracket(line: lineNumber)
        }

        let rawDescription = String(afterBracket[afterBracket.startIndex..<bracketEnd]).trimmingCharacters(in: .whitespaces)
        let description = rawDescription.isEmpty ? nil : String(rawDescription.prefix(500))
        return (name, description)
    }

    // MARK: - Tree Building

    private struct ParseNode {
        let name: String
        let description: String?
        var children: [ParseNode]
    }

    /// Build a tree from flat depth-tagged entries using a stack.
    /// Returns root nodes and duplicate-name warnings.
    private static func buildTree(from entries: [(depth: Int, name: String, description: String?, lineNumber: Int)]) -> ([ParseNode], [String]) {
        var warnings: [String] = []
        // Track duplicates per parent level using a stack of sets
        var dupeSets: [Set<String>] = [Set<String>()] // level 0

        // Stack of (depth, node) — we accumulate children as we go
        var stack: [(depth: Int, node: ParseNode)] = []
        var roots: [ParseNode] = []

        for (depth, name, desc, lineNumber) in entries {
            let newNode = ParseNode(name: name, description: desc, children: [])

            // Pop stack entries deeper than current depth — they're done,
            // attach them as children of the entry above
            var didPop = false
            while let last = stack.last, last.depth >= depth {
                let popped = stack.removeLast()
                if stack.isEmpty {
                    roots.append(popped.node)
                } else {
                    stack[stack.count - 1].node.children.append(popped.node)
                }
                didPop = true
            }

            // Duplicate check: only detect siblings under the same parent.
            // When we popped back up (parent changed), clear child dupe sets
            // so names under a new parent don't false-positive.
            if didPop && depth + 1 < dupeSets.count {
                dupeSets.removeSubrange((depth + 1)...)
            }
            while dupeSets.count <= depth {
                dupeSets.append(Set<String>())
            }

            let nameKey = name.lowercased()
            if dupeSets[depth].contains(nameKey) {
                warnings.append("Line \(lineNumber): duplicate genre \"\(name)\" at this level.")
            } else {
                dupeSets[depth].insert(nameKey)
            }

            stack.append((depth, newNode))
        }

        // Drain remaining stack
        while let last = stack.popLast() {
            if stack.isEmpty {
                roots.append(last.node)
            } else {
                stack[stack.count - 1].node.children.append(last.node)
            }
        }

        return (roots, warnings)
    }

    // MARK: - Sanitization

    /// Strip null bytes, C0 control characters (0x00–0x1F except tab 0x09),
    /// DEL (0x7F), and C1 control characters (0x80–0x9F). Preserves all Unicode.
    private static func stripControlCharacters(_ string: String) -> String {
        String(string.unicodeScalars.filter { scalar in
            if scalar.value == 0x09 { return true } // keep tab
            if scalar.value <= 0x1F { return false } // strip C0
            if scalar.value == 0x7F { return false } // strip DEL
            if scalar.value >= 0x80 && scalar.value <= 0x9F { return false } // strip C1
            return true
        })
    }

    // MARK: - JSON Output

    private struct GenreFileJSON: Encodable {
        let genres: [GenreRawJSON]
    }

    private struct GenreRawJSON: Encodable {
        let name: String
        let short_summary: String?
        let children: [GenreRawJSON]?
    }

    private static func nodeToJSON(_ node: ParseNode) -> GenreRawJSON {
        GenreRawJSON(
            name: node.name,
            short_summary: node.description,
            children: node.children.isEmpty ? nil : node.children.map { nodeToJSON($0) }
        )
    }
}
