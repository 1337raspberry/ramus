import Foundation
import GRDB

extension CacheDatabase {

    // MARK: - Rating Updates

    /// Update album rating (for optimistic UI after favourite toggle).
    public func updateAlbumRating(sourceId: String, rating: Double?) throws {
        try dbPool.write { db in
            try db.execute(
                sql: "UPDATE albums SET rating = ? WHERE sourceId = ?",
                arguments: [rating, sourceId]
            )
        }
    }

    /// Update album lastViewedAt timestamp (called on local playback).
    public func updateAlbumLastViewed(sourceId: String, at timestamp: Int) throws {
        try dbPool.write { db in
            try db.execute(
                sql: "UPDATE albums SET lastViewedAt = ? WHERE sourceId = ?",
                arguments: [timestamp, sourceId]
            )
        }
    }

    /// Update track rating (for optimistic UI after favourite toggle).
    public func updateTrackRating(sourceId: String, userRating: Double?) throws {
        try dbPool.write { db in
            try db.execute(
                sql: "UPDATE tracks SET userRating = ? WHERE sourceId = ?",
                arguments: [userRating, sourceId]
            )
        }
    }
}
