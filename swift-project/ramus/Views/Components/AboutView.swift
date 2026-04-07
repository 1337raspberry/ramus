import SwiftUI

/// About sheet: app info, keyboard shortcuts, licenses & acknowledgements.
struct AboutView: View {

    @Environment(\.dismiss) private var dismiss

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
    }

    var body: some View {
        VStack(spacing: 0) {
            // Close button
            HStack {
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.title2)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }

            ScrollView {
                VStack(spacing: 24) {
                    appIdentity
                    Divider()
                    shortcutsSection
                    Divider()
                    licensesSection
                }
                .padding(.bottom, 24)
            }
        }
        .padding(24)
        .frame(minWidth: 500, maxWidth: 600, minHeight: 400, maxHeight: 700)
    }

    // MARK: - App Identity

    private var appIdentity: some View {
        VStack(spacing: 8) {
            if let nsImage = NSImage(named: "AppIcon") {
                Image(nsImage: nsImage)
                    .resizable()
                    .frame(width: 80, height: 80)
                    .clipShape(RoundedRectangle(cornerRadius: 16))
            }
            Text("ramus")
                .font(.title.bold())
            Text("Version \(appVersion)")
                .font(.caption)
                .foregroundStyle(.secondary)
            Text("A highly opinionated, genre-first music player for Plex on macOS.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Link("github.com/1337raspberry/ramus", destination: URL(string: "https://github.com/1337raspberry/ramus")!)
                .font(.caption)
                .foregroundStyle(.tint)
            Text("ramus is not affiliated with Plex Inc.")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .padding(.top, 4)
        }
    }

    // MARK: - Keyboard Shortcuts

    private var shortcutsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionHeaderLabel(title: "Keyboard Shortcuts & Usage")
                .frame(maxWidth: .infinity, alignment: .leading)

            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                shortcutRow("Space", "Play / Pause")
                shortcutRow("Cmd+F", "Search")
                shortcutRow("/  @  !  %  $", "Genre / Artist / Track / Album / Year search")
                shortcutRow("Shift+Enter", "Load search results into album grid")
                shortcutRow("R", "Pick a new suggested album")
                shortcutRow("Double-click album", "Play album")
                shortcutRow("Force-touch / \u{2026} pill", "Browse album tracks")
                shortcutRow("Right-click sidebar tabs", "Change album sort order")
                shortcutRow("Cmd+Shift+Z", "Zoom window")
            }
            .font(.subheadline)
        }
    }

    @ViewBuilder
    private func shortcutRow(_ key: String, _ description: String) -> some View {
        GridRow {
            Text(key)
                .fontDesign(.monospaced)
                .foregroundStyle(.primary)
                .gridColumnAlignment(.trailing)
            Text(description)
                .foregroundStyle(.secondary)
                .gridColumnAlignment(.leading)
        }
    }

    // MARK: - Licenses

    private var licensesSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionHeaderLabel(title: "Licenses & Acknowledgements")
                .frame(maxWidth: .infinity, alignment: .leading)

            Text("ramus is licensed under CC BY-NC-SA 4.0")
                .font(.subheadline)
                .foregroundStyle(.secondary)

            DisclosureGroup {
                licenseText(Licenses.ccBySaNc40Summary)
            } label: {
                Text("ramus — CC BY-NC-SA 4.0")
                    .font(.subheadline)
            }

            DisclosureGroup {
                licenseText(Licenses.lgpl21)
            } label: {
                Text("MPVKit (mpv + FFmpeg) — LGPL 2.1+")
                    .font(.subheadline)
            }

            DisclosureGroup {
                licenseText(Licenses.grdb)
            } label: {
                Text("GRDB.swift — MIT")
                    .font(.subheadline)
            }

            DisclosureGroup {
                licenseText(Licenses.fuseSwift)
            } label: {
                Text("fuse-swift — MIT")
                    .font(.subheadline)
            }

            DisclosureGroup {
                licenseText(Licenses.nuke)
            } label: {
                Text("Nuke — MIT")
                    .font(.subheadline)
            }

            Text("Lyrics powered by LRCLIB (lrclib.net) — free, open-source synced lyrics API.")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .padding(.top, 4)

            Text("Genre hierarchy from Wikidata, licensed under CC0 1.0 Universal.")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }

    private func licenseText(_ text: String) -> some View {
        Text(text)
            .font(.caption2)
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

}

// MARK: - Embedded License Texts

private enum Licenses {

    static let ccBySaNc40Summary = """
    Copyright (c) 2025-2026 raspsoft (github.com/1337raspberry)

    This work is licensed under the Creative Commons Attribution-NonCommercial-ShareAlike \
    4.0 International License (CC BY-NC-SA 4.0).

    You are free to share and adapt this work under the following terms: you must give \
    appropriate credit, you may not use the material for commercial purposes, and if you \
    remix or build upon the material you must distribute your contributions under the same license.

    Full license: https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode
    """

    static let lgpl21 = """
    GNU LESSER GENERAL PUBLIC LICENSE
    Version 2.1, February 1999

    Copyright (C) 1991, 1999 Free Software Foundation, Inc. <http://fsf.org/>

    This library is free software; you can redistribute it and/or modify it under the \
    terms of the GNU Lesser General Public License as published by the Free Software \
    Foundation; either version 2.1 of the License, or (at your option) any later version.

    MPVKit is dynamically linked as xcframework bundles. The fork rebuilds mpv with \
    -Dlua=disabled to eliminate LuaJIT. No other modifications to mpv or FFmpeg source.

    Full license: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
    Source: https://github.com/1337raspberry/MPVKit (fork)
    """

    static let grdb = """
    The MIT License (MIT)

    Copyright (C) 2015-2025 Gwendal Roué

    Permission is hereby granted, free of charge, to any person obtaining a copy of this \
    software and associated documentation files (the "Software"), to deal in the Software \
    without restriction, including without limitation the rights to use, copy, modify, merge, \
    publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons \
    to whom the Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all copies or \
    substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, \
    INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR \
    PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE \
    FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR \
    OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER \
    DEALINGS IN THE SOFTWARE.
    """

    static let fuseSwift = """
    The MIT License (MIT)

    Copyright (c) 2017 Kirollos Risk <kirollos@gmail.com>

    Permission is hereby granted, free of charge, to any person obtaining a copy of this \
    software and associated documentation files (the "Software"), to deal in the Software \
    without restriction, including without limitation the rights to use, copy, modify, merge, \
    publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons \
    to whom the Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all copies or \
    substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, \
    INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR \
    PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE \
    FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR \
    OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER \
    DEALINGS IN THE SOFTWARE.
    """

    static let nuke = """
    The MIT License (MIT)

    Copyright (c) 2015-2026 Alexander Grebenyuk

    Permission is hereby granted, free of charge, to any person obtaining a copy of this \
    software and associated documentation files (the "Software"), to deal in the Software \
    without restriction, including without limitation the rights to use, copy, modify, merge, \
    publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons \
    to whom the Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all copies or \
    substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, \
    INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR \
    PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE \
    FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR \
    OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER \
    DEALINGS IN THE SOFTWARE.
    """
}
