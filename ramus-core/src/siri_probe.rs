//! Read-only library probe for the platform voice-assistant / App Intents
//! surface.
//!
//! Voice intents can run outside the app's normal command path — potentially in
//! a freshly launched process with no initialised runtime state — so this module
//! deliberately answers a small set of natural-language-style questions by
//! opening its own connection to the on-disk cache. It only ever issues
//! `SELECT`s, so it is safe to run alongside the live app under WAL.
//!
//! This is intentionally minimal (a proof that library data can cross into the
//! assistant surface); the richer parameterised query set is layered on top of
//! the same seam later.

use rusqlite::Connection;
use serde::Serialize;

/// Number of recently-played artists to surface in a spoken answer.
const RECENT_LIMIT: usize = 5;

/// Structured result serialised to JSON for the native intent layer. The
/// assistant only needs `ok` + `spoken`; the remaining fields are for
/// on-device diagnostics.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProbeResult {
    pub ok: bool,
    /// A ready-to-speak natural-language sentence.
    pub spoken: String,
    /// Echoes the resolved genre filter (if any).
    pub genre: Option<String>,
    pub total_albums: i64,
    pub genre_album_count: i64,
    pub recent_artists: Vec<String>,
    pub error: Option<String>,
}

impl ProbeResult {
    fn failure(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            spoken: message.clone(),
            genre: None,
            total_albums: 0,
            genre_album_count: 0,
            recent_artists: Vec::new(),
            error: Some(message),
        }
    }
}

/// Resolve the on-disk cache, run the probe, and return a JSON string. Never
/// panics — any failure is reported as a spoken sentence with `ok: false`.
pub fn probe_json(genre: Option<&str>) -> String {
    let result = probe(genre);
    serde_json::to_string(&result).unwrap_or_else(|e| {
        format!("{{\"ok\":false,\"spoken\":\"ramus could not read your library.\",\"error\":\"serialize: {e}\"}}")
    })
}

fn probe(genre: Option<&str>) -> ProbeResult {
    let db_path = match crate::plex::token_store::config_dir() {
        Ok(dir) => dir.join("library_cache.db"),
        Err(e) => return ProbeResult::failure(format!("ramus could not find its library ({e}).")),
    };
    if !db_path.exists() {
        return ProbeResult::failure("Open ramus and sync your library first, then try again.");
    }
    match Connection::open(&db_path) {
        Ok(conn) => query_probe(&conn, genre),
        Err(e) => ProbeResult::failure(format!("ramus could not open its library ({e}).")),
    }
}

/// The pure query core, separated so it can be exercised against an in-memory
/// database in tests.
fn query_probe(conn: &Connection, genre: Option<&str>) -> ProbeResult {
    let total_albums: i64 = conn
        .query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))
        .unwrap_or(0);

    let (genre_album_count, recent_artists) = match genre {
        Some(g) => {
            let like = format!("%{g}%");
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT a.id) FROM albums a \
                     JOIN album_genres ag ON ag.albumId = a.id \
                     JOIN genres g ON g.id = ag.genreId \
                     WHERE g.name = ?1 COLLATE NOCASE OR g.name LIKE ?2 COLLATE NOCASE",
                    rusqlite::params![g, like],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let artists = query_artists(
                conn,
                "SELECT ar.name, MAX(a.lastViewedAt) AS lastv FROM albums a \
                 JOIN artists ar ON ar.id = a.artistId \
                 JOIN album_genres ag ON ag.albumId = a.id \
                 JOIN genres g ON g.id = ag.genreId \
                 WHERE (g.name = ?1 COLLATE NOCASE OR g.name LIKE ?2 COLLATE NOCASE) \
                   AND a.viewCount IS NOT NULL AND a.viewCount > 0 \
                 GROUP BY ar.id ORDER BY lastv DESC LIMIT ?3",
                rusqlite::params![g, like, RECENT_LIMIT as i64],
            );
            (count, artists)
        }
        None => {
            let artists = query_artists(
                conn,
                "SELECT ar.name, MAX(a.lastViewedAt) AS lastv FROM albums a \
                 JOIN artists ar ON ar.id = a.artistId \
                 WHERE a.viewCount IS NOT NULL AND a.viewCount > 0 \
                 GROUP BY ar.id ORDER BY lastv DESC LIMIT ?1",
                rusqlite::params![RECENT_LIMIT as i64],
            );
            (0, artists)
        }
    };

    let spoken = build_spoken(genre, total_albums, genre_album_count, &recent_artists);
    ProbeResult {
        ok: true,
        spoken,
        genre: genre.map(str::to_owned),
        total_albums,
        genre_album_count,
        recent_artists,
        error: None,
    }
}

/// Run a `SELECT name, …` statement and collect the first column. Any error
/// (e.g. a not-yet-created table) yields an empty list rather than failing the
/// whole probe.
fn query_artists(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map(params, |r| r.get::<_, String>(0)) {
            out.extend(rows.flatten());
        }
    }
    out
}

fn build_spoken(genre: Option<&str>, total: i64, genre_count: i64, artists: &[String]) -> String {
    match genre {
        Some(g) if genre_count == 0 => {
            format!("I couldn't find any {g} albums in your ramus library.")
        }
        Some(g) if artists.is_empty() => {
            format!("You have {genre_count} {g} albums in ramus, but you haven't played any yet.")
        }
        Some(g) => format!(
            "You have {genre_count} {g} albums in ramus. Recently you've listened to {}.",
            human_list(artists)
        ),
        None if artists.is_empty() => {
            format!("Your ramus library has {total} albums, but nothing has been played yet.")
        }
        None => format!(
            "Your ramus library has {total} albums. Recently you've listened to {}.",
            human_list(artists)
        ),
    }
}

/// Join names into a spoken list: "A", "A and B", "A, B, and C".
fn human_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE artists (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE albums (id INTEGER PRIMARY KEY, title TEXT, artistId INTEGER, \
                viewCount INTEGER, lastViewedAt INTEGER);
             CREATE TABLE genres (id INTEGER PRIMARY KEY, name TEXT NOT NULL COLLATE NOCASE);
             CREATE TABLE album_genres (albumId INTEGER, genreId INTEGER);

             INSERT INTO artists (id, name) VALUES (1,'Touché Amoré'),(2,'Defeater'),(3,'Cave In');
             INSERT INTO genres (id, name) VALUES (1,'Post-Hardcore'),(2,'Metal');
             -- Two played post-hardcore albums (Touché Amoré most recent), one unplayed metal.
             INSERT INTO albums (id,title,artistId,viewCount,lastViewedAt) VALUES
                (10,'Stage Four',1,7,2000),
                (11,'Empty Days',2,3,1000),
                (12,'Heavy Pendulum',3,NULL,NULL);
             INSERT INTO album_genres (albumId,genreId) VALUES (10,1),(11,1),(12,2);",
        )
        .unwrap();
    }

    #[test]
    fn genre_query_lists_recently_played_artists_most_recent_first() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let r = query_probe(&conn, Some("post-hardcore"));
        assert!(r.ok);
        assert_eq!(r.genre_album_count, 2);
        assert_eq!(r.recent_artists, vec!["Touché Amoré", "Defeater"]);
        assert_eq!(r.total_albums, 3);
        assert!(r.spoken.contains("2 post-hardcore albums"));
        assert!(r.spoken.contains("Touché Amoré and Defeater"));
    }

    #[test]
    fn genre_match_is_case_insensitive_and_substring() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // "hardcore" is a substring of the stored "Post-Hardcore".
        assert_eq!(query_probe(&conn, Some("HARDCORE")).genre_album_count, 2);
    }

    #[test]
    fn unknown_genre_reports_none_found() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let r = query_probe(&conn, Some("polka"));
        assert_eq!(r.genre_album_count, 0);
        assert!(r.recent_artists.is_empty());
        assert!(r.spoken.contains("couldn't find any polka"));
    }

    #[test]
    fn overall_query_ignores_unplayed_albums() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let r = query_probe(&conn, None);
        assert_eq!(r.total_albums, 3);
        // Cave In (unplayed) is excluded; Touché Amoré is most recent.
        assert_eq!(r.recent_artists, vec!["Touché Amoré", "Defeater"]);
    }

    #[test]
    fn human_list_grammar() {
        assert_eq!(human_list(&["A".into()]), "A");
        assert_eq!(human_list(&["A".into(), "B".into()]), "A and B");
        assert_eq!(human_list(&["A".into(), "B".into(), "C".into()]), "A, B, and C");
    }
}
