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

/// Cap on the number of entities returned to the assistant when it builds a
/// picker or resolves a spoken name. The full library can hold thousands of
/// artists; a bounded, relevance-sorted slice keeps the suggestion list usable.
/// The cap only limits *suggestions* — playback resolves a name against the
/// whole library regardless.
const LIST_LIMIT: usize = 100;

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

    // When the term isn't a genre it might be an artist ("what My Chemical
    // Romance albums do I have"). Fall back to an artist answer before giving up.
    if let Some(term) = genre {
        if genre_album_count == 0 {
            if let Some((artist_name, album_count, played)) = artist_probe(conn, term) {
                return ProbeResult {
                    ok: true,
                    spoken: build_artist_spoken(&artist_name, album_count, played),
                    genre: Some(term.to_owned()),
                    total_albums,
                    genre_album_count: album_count,
                    recent_artists: Vec::new(),
                    error: None,
                };
            }
        }
    }

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

/// Resolve a probe term as an artist: the best-matching in-library artist's
/// actual name, its album count, and how many of those albums have been played.
/// Prefers an exact (case-insensitive) name, else the most-stocked substring
/// match. `None` when no artist matches, so the caller falls through to
/// "not found".
fn artist_probe(conn: &Connection, term: &str) -> Option<(String, i64, i64)> {
    let like = format!("%{term}%");
    let (name, album_count): (String, i64) = conn
        .query_row(
            "SELECT ar.name, COUNT(*) AS c FROM albums a \
             JOIN artists ar ON ar.id = a.artistId \
             WHERE ar.name = ?1 COLLATE NOCASE OR ar.name LIKE ?2 COLLATE NOCASE \
             GROUP BY ar.id ORDER BY (ar.name = ?1 COLLATE NOCASE) DESC, c DESC LIMIT 1",
            rusqlite::params![term, like],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let played: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM albums a JOIN artists ar ON ar.id = a.artistId \
             WHERE ar.name = ?1 COLLATE NOCASE \
               AND a.viewCount IS NOT NULL AND a.viewCount > 0",
            rusqlite::params![name],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Some((name, album_count, played))
}

fn build_artist_spoken(artist: &str, album_count: i64, played: i64) -> String {
    let album_word = if album_count == 1 { "album" } else { "albums" };
    if played == 0 {
        format!("You have {album_count} {artist} {album_word} in ramus, but haven't played any yet.")
    } else if played >= album_count {
        format!("You have {album_count} {artist} {album_word} in ramus.")
    } else {
        format!("You have {album_count} {artist} {album_word} in ramus, and you've played {played}.")
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

// ---------------------------------------------------------------------------
// Entity listing — backs the assistant's genre/artist vocabulary.
//
// The voice layer models genres and artists as resolvable "entities" so the
// assistant can turn a spoken word ("post-hardcore", "Touché Amoré") into a
// concrete library item. These helpers feed two assistant needs: the initial
// picker list (`query = None`) and spoken-name search (`query = Some(text)`).
// Both are read-only and open their own connection, so they answer even when
// the app isn't running.
// ---------------------------------------------------------------------------

/// A genre present in the library, with how many albums carry the tag (used to
/// surface the most-represented genres first).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GenreItem {
    pub name: String,
    pub album_count: i64,
}

/// An artist present in the library.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArtistItem {
    pub name: String,
}

/// An album present in the library. `source_id` is the stable Plex rating key —
/// the assistant plays an album by this id (titles aren't unique), while `title`
/// and `artist` are for display / disambiguation.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AlbumItem {
    pub source_id: String,
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Serialize)]
struct GenreList {
    ok: bool,
    items: Vec<GenreItem>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ArtistList {
    ok: bool,
    items: Vec<ArtistItem>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct AlbumList {
    ok: bool,
    items: Vec<AlbumItem>,
    error: Option<String>,
}

/// Open a read-only-style connection to the on-disk cache. Shares the probe's
/// philosophy: only ever issues `SELECT`s, so it is WAL-safe alongside the live
/// app. Errors are returned as short strings for the JSON `error` field.
fn open_cache() -> Result<Connection, String> {
    let db_path = crate::plex::token_store::config_dir()
        .map_err(|e| format!("config dir: {e}"))?
        .join("library_cache.db");
    if !db_path.exists() {
        return Err("library not synced yet".to_string());
    }
    Connection::open(&db_path).map_err(|e| format!("open: {e}"))
}

fn map_genre(r: &rusqlite::Row) -> rusqlite::Result<GenreItem> {
    Ok(GenreItem {
        name: r.get(0)?,
        album_count: r.get(1)?,
    })
}

fn map_artist(r: &rusqlite::Row) -> rusqlite::Result<ArtistItem> {
    Ok(ArtistItem { name: r.get(0)? })
}

fn map_album(r: &rusqlite::Row) -> rusqlite::Result<AlbumItem> {
    Ok(AlbumItem {
        source_id: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
    })
}

/// Run a genre `SELECT name, count` statement and collect the rows. Any error
/// yields an empty list rather than failing the caller.
fn run_genre_query(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Vec<GenreItem> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map(params, map_genre) {
            out.extend(rows.flatten());
        }
    }
    out
}

/// The genres actually present in the library, most-represented first. With a
/// `query`, an exact (case-insensitive) name match wins outright — so a spoken
/// genre ("metal") resolves to the single "Metal" tag rather than every genre
/// that merely contains the word (Metalcore, Death Metal, …), which the
/// assistant can't disambiguate. Only when nothing matches exactly does it fall
/// back to substring search, so a partial word ("hardcore") still surfaces
/// "Post-Hardcore". Separated from the JSON wrapper so tests exercise it directly.
fn query_genres(conn: &Connection, query: Option<&str>) -> Vec<GenreItem> {
    let genre_select = "SELECT g.name, COUNT(DISTINCT ag.albumId) AS c \
         FROM genres g JOIN album_genres ag ON ag.genreId = g.id";
    match query {
        Some(q) => {
            let exact = run_genre_query(
                conn,
                &format!(
                    "{genre_select} WHERE g.name = ?1 COLLATE NOCASE \
                     GROUP BY g.id ORDER BY c DESC, g.name ASC LIMIT ?2"
                ),
                rusqlite::params![q, LIST_LIMIT as i64],
            );
            if !exact.is_empty() {
                return exact;
            }
            let like = format!("%{q}%");
            run_genre_query(
                conn,
                &format!(
                    "{genre_select} WHERE g.name LIKE ?1 COLLATE NOCASE \
                     GROUP BY g.id ORDER BY c DESC, g.name ASC LIMIT ?2"
                ),
                rusqlite::params![like, LIST_LIMIT as i64],
            )
        }
        None => run_genre_query(
            conn,
            &format!("{genre_select} GROUP BY g.id ORDER BY c DESC, g.name ASC LIMIT ?1"),
            rusqlite::params![LIST_LIMIT as i64],
        ),
    }
}

/// Run an artist `SELECT name` statement and collect the rows. Any error yields
/// an empty list rather than failing the caller.
fn run_artist_query(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Vec<ArtistItem> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map(params, map_artist) {
            out.extend(rows.flatten());
        }
    }
    out
}

/// The artists present in the library, most-played first (by summed album
/// `viewCount`). With a `query`, an exact (case-insensitive) name match wins
/// outright (mirrors [`query_genres`]) so a spoken artist resolves to one
/// entity; otherwise it falls back to substring search.
fn query_artists_list(conn: &Connection, query: Option<&str>) -> Vec<ArtistItem> {
    let artist_select = "SELECT ar.name FROM artists ar JOIN albums a ON a.artistId = ar.id";
    let order = "GROUP BY ar.id ORDER BY COALESCE(SUM(a.viewCount), 0) DESC, ar.name ASC LIMIT ?2";
    match query {
        Some(q) => {
            let exact = run_artist_query(
                conn,
                &format!("{artist_select} WHERE ar.name = ?1 COLLATE NOCASE {order}"),
                rusqlite::params![q, LIST_LIMIT as i64],
            );
            if !exact.is_empty() {
                return exact;
            }
            let like = format!("%{q}%");
            run_artist_query(
                conn,
                &format!("{artist_select} WHERE ar.name LIKE ?1 COLLATE NOCASE {order}"),
                rusqlite::params![like, LIST_LIMIT as i64],
            )
        }
        None => run_artist_query(
            conn,
            &format!(
                "{artist_select} \
                 GROUP BY ar.id ORDER BY COALESCE(SUM(a.viewCount), 0) DESC, ar.name ASC LIMIT ?1"
            ),
            rusqlite::params![LIST_LIMIT as i64],
        ),
    }
}

/// Run an album `SELECT sourceId, title, artist` statement and collect the rows.
fn run_album_query(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Vec<AlbumItem> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map(params, map_album) {
            out.extend(rows.flatten());
        }
    }
    out
}

/// The albums present in the library, most-played first. With a `query`, an
/// exact (case-insensitive) title match wins outright (mirrors [`query_genres`])
/// so a spoken album resolves cleanly; otherwise it falls back to substring
/// search. Albums are keyed by `source_id` because titles aren't unique.
fn query_albums(conn: &Connection, query: Option<&str>) -> Vec<AlbumItem> {
    let album_select =
        "SELECT a.sourceId, a.title, ar.name FROM albums a JOIN artists ar ON ar.id = a.artistId";
    let order = "ORDER BY COALESCE(a.viewCount, 0) DESC, a.title ASC LIMIT ?2";
    match query {
        Some(q) => {
            let exact = run_album_query(
                conn,
                &format!("{album_select} WHERE a.title = ?1 COLLATE NOCASE {order}"),
                rusqlite::params![q, LIST_LIMIT as i64],
            );
            if !exact.is_empty() {
                return exact;
            }
            let like = format!("%{q}%");
            run_album_query(
                conn,
                &format!("{album_select} WHERE a.title LIKE ?1 COLLATE NOCASE {order}"),
                rusqlite::params![like, LIST_LIMIT as i64],
            )
        }
        None => run_album_query(
            conn,
            &format!(
                "{album_select} ORDER BY COALESCE(a.viewCount, 0) DESC, a.title ASC LIMIT ?1"
            ),
            rusqlite::params![LIST_LIMIT as i64],
        ),
    }
}

/// JSON list of in-library genres for the assistant. `query` is an optional
/// case-insensitive substring filter (null → the top suggestions).
pub fn list_genres_json(query: Option<&str>) -> String {
    let result = match open_cache() {
        Ok(conn) => GenreList {
            ok: true,
            items: query_genres(&conn, query),
            error: None,
        },
        Err(e) => GenreList {
            ok: false,
            items: Vec::new(),
            error: Some(e),
        },
    };
    serde_json::to_string(&result)
        .unwrap_or_else(|_| "{\"ok\":false,\"items\":[],\"error\":\"serialize\"}".to_string())
}

/// JSON list of in-library artists for the assistant. `query` is an optional
/// case-insensitive substring filter (null → the top suggestions).
pub fn list_artists_json(query: Option<&str>) -> String {
    let result = match open_cache() {
        Ok(conn) => ArtistList {
            ok: true,
            items: query_artists_list(&conn, query),
            error: None,
        },
        Err(e) => ArtistList {
            ok: false,
            items: Vec::new(),
            error: Some(e),
        },
    };
    serde_json::to_string(&result)
        .unwrap_or_else(|_| "{\"ok\":false,\"items\":[],\"error\":\"serialize\"}".to_string())
}

/// JSON list of in-library albums for the assistant. `query` is an optional
/// case-insensitive title filter (null → the top suggestions). Each item carries
/// `sourceId` (the stable play key), `title`, and `artist`.
pub fn list_albums_json(query: Option<&str>) -> String {
    let result = match open_cache() {
        Ok(conn) => AlbumList {
            ok: true,
            items: query_albums(&conn, query),
            error: None,
        },
        Err(e) => AlbumList {
            ok: false,
            items: Vec::new(),
            error: Some(e),
        },
    };
    serde_json::to_string(&result)
        .unwrap_or_else(|_| "{\"ok\":false,\"items\":[],\"error\":\"serialize\"}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE artists (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE albums (id INTEGER PRIMARY KEY, title TEXT, artistId INTEGER, \
                viewCount INTEGER, lastViewedAt INTEGER, sourceId TEXT);
             CREATE TABLE genres (id INTEGER PRIMARY KEY, name TEXT NOT NULL COLLATE NOCASE);
             CREATE TABLE album_genres (albumId INTEGER, genreId INTEGER);

             INSERT INTO artists (id, name) VALUES (1,'Touché Amoré'),(2,'Defeater'),(3,'Cave In');
             -- Metal + Metalcore both contain 'metal', to exercise exact-vs-substring.
             INSERT INTO genres (id, name) VALUES (1,'Post-Hardcore'),(2,'Metal'),(3,'Metalcore');
             -- Two played post-hardcore albums (Touché Amoré most recent), one unplayed
             -- album tagged both Metal and Metalcore.
             INSERT INTO albums (id,title,artistId,viewCount,lastViewedAt,sourceId) VALUES
                (10,'Stage Four',1,7,2000,'rk10'),
                (11,'Empty Days',2,3,1000,'rk11'),
                (12,'Heavy Pendulum',3,NULL,NULL,'rk12');
             INSERT INTO album_genres (albumId,genreId) VALUES (10,1),(11,1),(12,2),(12,3);",
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
        // "polka" is neither a genre nor an artist here.
        let r = query_probe(&conn, Some("polka"));
        assert_eq!(r.genre_album_count, 0);
        assert!(r.recent_artists.is_empty());
        assert!(r.spoken.contains("couldn't find any polka"));
    }

    #[test]
    fn probe_resolves_a_term_that_is_an_artist_not_a_genre() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // "Defeater" is an artist (1 played album), not a genre.
        let r = query_probe(&conn, Some("defeater"));
        assert!(r.ok);
        assert_eq!(r.genre_album_count, 1);
        assert!(r.spoken.contains("Defeater"), "spoken was: {}", r.spoken);
        assert!(r.spoken.contains("1 Defeater album"), "spoken was: {}", r.spoken);
    }

    #[test]
    fn probe_artist_with_no_plays_says_so() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // "Cave In" has one album (Heavy Pendulum) that has never been played.
        let r = query_probe(&conn, Some("Cave In"));
        assert!(r.spoken.contains("Cave In"), "spoken was: {}", r.spoken);
        assert!(r.spoken.contains("haven't played"), "spoken was: {}", r.spoken);
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

    #[test]
    fn genre_list_returns_in_library_sorted_by_album_count() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let items = query_genres(&conn, None);
        // Post-Hardcore (2 albums) ranks above Metal and Metalcore (1 each,
        // broken by name ASC).
        assert_eq!(
            items,
            vec![
                GenreItem {
                    name: "Post-Hardcore".into(),
                    album_count: 2
                },
                GenreItem {
                    name: "Metal".into(),
                    album_count: 1
                },
                GenreItem {
                    name: "Metalcore".into(),
                    album_count: 1
                },
            ]
        );
    }

    #[test]
    fn genre_query_exact_match_wins_over_substring_siblings() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // "metal" matches both Metal and Metalcore by substring, but an exact
        // (case-insensitive) hit must resolve to just "Metal" so the assistant
        // isn't handed an ambiguous pile of candidates.
        let items = query_genres(&conn, Some("METAL"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Metal");
    }

    #[test]
    fn genre_query_falls_back_to_substring_without_exact() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // No genre is exactly "core", so both "Post-Hardcore" and "Metalcore"
        // surface via the substring fallback.
        let names: Vec<String> = query_genres(&conn, Some("core"))
            .into_iter()
            .map(|g| g.name)
            .collect();
        assert!(names.contains(&"Post-Hardcore".to_string()));
        assert!(names.contains(&"Metalcore".to_string()));
    }

    #[test]
    fn artist_query_exact_match_resolves_single() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let items = query_artists_list(&conn, Some("defeater"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Defeater");
    }

    #[test]
    fn album_list_ranks_most_played_first() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let titles: Vec<String> = query_albums(&conn, None)
            .into_iter()
            .map(|a| a.title)
            .collect();
        // Stage Four (7 plays) > Empty Days (3) > Heavy Pendulum (unplayed → 0).
        assert_eq!(titles, vec!["Stage Four", "Empty Days", "Heavy Pendulum"]);
    }

    #[test]
    fn album_query_exact_title_resolves_to_source_id_and_artist() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let items = query_albums(&conn, Some("stage four"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_id, "rk10");
        assert_eq!(items[0].title, "Stage Four");
        assert_eq!(items[0].artist, "Touché Amoré");
    }

    #[test]
    fn album_query_falls_back_to_substring_without_exact() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        // No album is exactly "days"; the substring fallback finds "Empty Days".
        let items = query_albums(&conn, Some("days"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Empty Days");
    }

    #[test]
    fn artist_list_ranks_most_played_first() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let items = query_artists_list(&conn, None);
        // Touché Amoré (7 plays) > Defeater (3) > Cave In (unplayed → 0).
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Touché Amoré", "Defeater", "Cave In"]);
    }

    #[test]
    fn artist_list_substring_filter_matches_partial_name() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn);
        let items = query_artists_list(&conn, Some("def"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Defeater");
    }
}
