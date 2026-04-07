extension SyncEngine.SyncProgress.Phase {
    /// Human-readable name for display in progress UI.
    public var displayName: String {
        switch self {
        case .artists: "Syncing artists..."
        case .albums: "Syncing albums..."
        case .tracks: "Syncing tracks..."
        case .deepGenres: "Fetching full genre data..."
        case .done: "Done!"
        }
    }
}
