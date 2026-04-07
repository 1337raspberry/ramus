import SwiftUI

/// Shared section header style: uppercase, caption-weight, tertiary colour.
struct SectionHeaderLabel: View {
    let title: String

    var body: some View {
        Text(title.uppercased())
            .font(.caption)
            .fontWeight(.semibold)
            .foregroundStyle(.tertiary)
    }
}
