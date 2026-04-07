import SwiftUI
import GenreTree

/// Sidebar showing the genre hierarchy as an expandable tree.
/// Uses ScrollView + LazyVStack for full layout control (List + .sidebar
/// style imposes hard-coded margins that can't be overridden).
struct GenreTreeView: View {

    let genres: [GenreNode]
    let allExpandableIDs: Set<String>
    let verticalPadding: CGFloat
    @Binding var selection: GenreNode?
    @Binding var expandedIDs: Set<String>
    @Binding var scrollToID: String?
    /// When non-nil, shows a shuffle button on the "All" sentinel row.
    /// The view calls shuffleFavouriteTracks() directly — no closure needed.
    var shuffleVM: LibraryViewModel?

    @Environment(\.dynamicAccent) private var accentColor

    @State private var hStackSpacing: CGFloat = 6
    @State private var chevronWidth: CGFloat = 12
    @State private var depthIndent: CGFloat = 8
    @State private var trailingPad: CGFloat = 6
    @State private var outerHPad: CGFloat = 6
    #if DEBUG
    @State private var showDebug = false
    #endif

    var body: some View {
        VStack(spacing: 0) {
            #if DEBUG
            HStack {
                Spacer()
                Button(showDebug ? "Hide Debug" : "Debug") { showDebug.toggle() }
                    .font(.caption2).buttonStyle(.plain).foregroundStyle(.secondary)
                    .padding(.trailing, 8).padding(.top, 2)
            }
            if showDebug {
                VStack(spacing: 2) {
                    dSlider("HStack Spacing", $hStackSpacing, 0...16)
                    dSlider("Chevron W", $chevronWidth, 4...24)
                    dSlider("Depth Indent", $depthIndent, 4...32)
                    dSlider("Trail Pad", $trailingPad, 0...20)
                    dSlider("Outer H Pad", $outerHPad, 0...20)
                }.padding(.horizontal, 8).padding(.bottom, 4).font(.caption2)
            }
            #endif

        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(genres) { node in
                        GenreRowView(
                            node: node,
                            depth: 0,
                            allExpandableIDs: allExpandableIDs,
                            verticalPadding: verticalPadding,
                            hStackSpacing: hStackSpacing,
                            chevronWidth: chevronWidth,
                            depthIndent: depthIndent,
                            trailingPad: trailingPad,
                            selection: $selection,
                            expandedIDs: $expandedIDs,
                            accentColor: accentColor,
                            isSentinel: node.id.hasPrefix("__sentinel__"),
                            shuffleVM: node.id.hasPrefix("__sentinel__") ? shuffleVM : nil
                        )
                    }
                }
                .padding(.horizontal, outerHPad)
                .padding(.top, 4)
            }
            .animation(nil, value: expandedIDs)
            .onChange(of: scrollToID) { _, newID in
                if let newID {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                        proxy.scrollTo(newID, anchor: .center)
                        if scrollToID == newID { scrollToID = nil }
                    }
                }
            }
        }
        } // VStack
    }

    #if DEBUG
    private func dSlider(_ label: String, _ value: Binding<CGFloat>, _ range: ClosedRange<CGFloat>) -> some View {
        HStack(spacing: 4) {
            Text(label).frame(width: 90, alignment: .leading).lineLimit(1)
            Slider(value: value, in: range, step: 1)
            Text("\(Int(value.wrappedValue))").monospacedDigit().frame(width: 24, alignment: .trailing)
        }
    }
    #endif
}

/// Recursive row with manual chevron and indentation per depth level.
private struct GenreRowView: View {

    let node: GenreNode
    let depth: Int
    let allExpandableIDs: Set<String>
    let verticalPadding: CGFloat
    var hStackSpacing: CGFloat = 6
    var chevronWidth: CGFloat = 12
    var depthIndent: CGFloat = 8
    var trailingPad: CGFloat = 6
    @Binding var selection: GenreNode?
    @Binding var expandedIDs: Set<String>
    let accentColor: Color
    /// True only for the sentinel "All" row — chevron expands/collapses all groups.
    var isSentinel: Bool = false
    /// When non-nil, shows a shuffle button on this row.
    var shuffleVM: LibraryViewModel?

    private var isSelected: Bool { selection == node }
    private var hasChildren: Bool { node.children != nil && !(node.children?.isEmpty ?? true) }
    private var isExpanded: Bool {
        if isSentinel {
            return expandedIDs == allExpandableIDs
        }
        return expandedIDs.contains(node.id)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: hStackSpacing) {
                // Chevron for expandable nodes and the "All" sentinel row
                if hasChildren || isSentinel {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                        .frame(width: chevronWidth)
                        .contentShape(Rectangle())
                        .onTapGesture { toggleExpansion() }
                } else {
                    Spacer().frame(width: chevronWidth)
                }

                Text(node.name)
                    .lineLimit(1)

                Spacer(minLength: 0)

                if shuffleVM != nil {
                    Button {
                        shuffleVM?.shuffleFavouriteTracks()
                    } label: {
                        Image(systemName: "shuffle")
                            .font(.caption2)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.tint)
                }

                Text("\(node.deduplicatedTotalCount)")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
            }
            .padding(.leading, CGFloat(depth) * depthIndent)
            .padding(.trailing, trailingPad)
            .padding(.vertical, verticalPadding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isSelected ? accentColor.opacity(0.3) : Color.clear)
            .contentShape(Rectangle())
            .id(node.id)
            .onTapGesture { selection = node }
            .help(node.shortSummary ?? "")

            // Expanded children
            if hasChildren && isExpanded, let children = node.children {
                ForEach(children) { child in
                    GenreRowView(
                        node: child,
                        depth: depth + 1,
                        allExpandableIDs: allExpandableIDs,
                        verticalPadding: verticalPadding,
                        hStackSpacing: hStackSpacing,
                        chevronWidth: chevronWidth,
                        depthIndent: depthIndent,
                        trailingPad: trailingPad,
                        selection: $selection,
                        expandedIDs: $expandedIDs,
                        accentColor: accentColor
                    )
                }
            }
        }
    }

    private func toggleExpansion() {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            if isSentinel {
                // "All" row: expand all or collapse all
                expandedIDs = isExpanded ? [] : allExpandableIDs
            } else if isExpanded {
                expandedIDs.remove(node.id)
            } else {
                expandedIDs.insert(node.id)
            }
        }
    }
}
