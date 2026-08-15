//! Generated playlists ("crates") — selection rules Plex's own smart-playlist
//! grammar can't express.
//!
//! Two rules motivate this module. **Hot tracks**: Plex ships a crowd
//! popularity figure per track (`ratingCount`) but exposes no filter or sort
//! for it, so "the standout track from each album" cannot be asked of the
//! server. **Sub-genre expansion**: library tags are specific and
//! hierarchical, so a rule matching the tag `Metal` alone matches almost
//! nothing — the useful query is `Metal` plus everything beneath it in the
//! genre tree.
//!
//! Because neither survives a round trip through a smart filter, a crate is
//! materialised as an ordinary playlist and its recipe is stored in the
//! playlist's `summary` field. That keeps the recipe on the server object
//! rather than in per-device settings, so any install can regenerate a crate
//! it did not create.
//!
//! Selection here is pure: callers resolve genres to a candidate track list
//! (which needs the database and the genre tree) and pass it in.

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::models::Track;
use crate::util::{percent_decode, percent_encode};

/// Marker prefix for the machine-readable recipe line inside a playlist
/// summary. Deliberately the last line, after a human sentence, so the
/// summary still reads as a description in clients that show it.
const CRATE_MARKER: &str = "ramus-crate:";

/// Recipe wire version. Bump only for a change a v1 parser would
/// misinterpret; unknown keys are already ignored, so added fields don't
/// need one.
const CRATE_VERSION: u32 = 1;

/// A crate's rules. Round-trips through a playlist `summary` via
/// [`CrateRecipe::to_summary`] / [`CrateRecipe::from_summary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateRecipe {
    /// Genre names as the user picked them (canonical display names, not tag
    /// ids — the crate resolves them against the library at generation time,
    /// so a recipe stays valid across servers and re-syncs).
    pub genres: Vec<String>,
    /// Match every genre beneath the picked ones in the tree as well.
    pub include_subgenres: bool,
    /// Keep only the top N tracks of each album by popularity. `0` disables
    /// the rule and lets every candidate through.
    pub hot_per_album: u8,
    /// Drop tracks that have been played at least once as of the last sync.
    pub unplayed_only: bool,
    /// How many tracks the finished playlist holds. `0` means no limit.
    pub count: u32,
}

impl Default for CrateRecipe {
    fn default() -> Self {
        Self {
            genres: Vec::new(),
            include_subgenres: true,
            hot_per_album: 2,
            unplayed_only: true,
            count: 25,
        }
    }
}

impl CrateRecipe {
    /// Render the full summary text: a human sentence, a blank line, then the
    /// machine-readable recipe. Clients that display summaries show something
    /// meaningful, and [`Self::from_summary`] only ever reads the last line.
    pub fn to_summary(&self) -> String {
        format!("{}\n\n{}", self.describe(), self.encode_line())
    }

    /// One-sentence description of the recipe, for the summary's first line
    /// and for confirmation UI.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.count > 0 {
            parts.push(self.count.to_string());
        }
        if self.unplayed_only {
            parts.push("unplayed".to_string());
        }
        if self.hot_per_album > 0 {
            parts.push("hot".to_string());
        }
        parts.push(if self.count == 1 { "track" } else { "tracks" }.to_string());

        let genres = if self.genres.is_empty() {
            "the whole library".to_string()
        } else {
            self.genres.join(", ")
        };
        let subs = if self.include_subgenres && !self.genres.is_empty() {
            " (+sub-genres)"
        } else {
            ""
        };
        format!("{} — {}{}", parts.join(" "), genres, subs)
    }

    /// The machine-readable line. Values are percent-encoded individually so
    /// a genre containing a comma, space or `=` can't corrupt the parse.
    fn encode_line(&self) -> String {
        let genres = self
            .genres
            .iter()
            .map(|g| percent_encode(g))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{}{} genres={} subs={} hot={} unplayed={} count={}",
            CRATE_MARKER,
            CRATE_VERSION,
            genres,
            u8::from(self.include_subgenres),
            self.hot_per_album,
            u8::from(self.unplayed_only),
            self.count,
        )
    }

    /// Recover a recipe from a playlist summary, or `None` if the summary
    /// carries no recipe (an ordinary playlist, or one whose summary a user
    /// has since rewritten). Callers surface `None` as "can't regenerate"
    /// rather than guessing.
    pub fn from_summary(summary: &str) -> Option<Self> {
        // Scan from the end: the recipe is written last, and a summary that
        // somehow holds two should honour the most recent.
        let line = summary
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with(CRATE_MARKER))?
            .trim();

        let rest = line.strip_prefix(CRATE_MARKER)?;
        let (version, params) = rest.split_once(' ').unwrap_or((rest, ""));
        if version.parse::<u32>().ok()? != CRATE_VERSION {
            return None;
        }

        let mut recipe = Self {
            genres: Vec::new(),
            include_subgenres: false,
            hot_per_album: 0,
            unplayed_only: false,
            count: 0,
        };
        for pair in params.split_whitespace() {
            let (key, value) = match pair.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            match key {
                "genres" => {
                    recipe.genres = value
                        .split(',')
                        .filter(|v| !v.is_empty())
                        .map(percent_decode)
                        .collect()
                }
                "subs" => recipe.include_subgenres = value == "1",
                "hot" => recipe.hot_per_album = value.parse().unwrap_or(0),
                "unplayed" => recipe.unplayed_only = value == "1",
                "count" => recipe.count = value.parse().unwrap_or(0),
                // Unknown keys are ignored so a newer writer's extra fields
                // degrade to defaults instead of failing the whole parse.
                _ => {}
            }
        }
        Some(recipe)
    }
}

/// Apply a recipe to an already genre-filtered candidate list.
///
/// Genre resolution lives in the caller because it needs the database and the
/// genre tree; everything downstream of that is decided here so the rules are
/// testable in isolation.
pub fn select_crate_tracks<R: Rng>(
    candidates: Vec<Track>,
    recipe: &CrateRecipe,
    rng: &mut R,
) -> Vec<Track> {
    let pool = eligible_tracks(candidates, recipe);

    let mut selected = balanced_shuffle(
        pool,
        |t| {
            t.track_artist
                .as_deref()
                .unwrap_or(&t.artist_name)
                .to_lowercase()
        },
        rng,
    );

    if recipe.count > 0 && selected.len() > recipe.count as usize {
        selected.truncate(recipe.count as usize);
    }
    selected
}

/// The tracks a recipe's rules admit, before shuffling or capping to
/// `count` — i.e. the pool the crate is drawn from.
///
/// Split out from [`select_crate_tracks`] so a preview can size that pool
/// without paying for a shuffle, and so the figure it shows actually
/// responds to the rules rather than reporting the raw genre match.
pub fn eligible_tracks(candidates: Vec<Track>, recipe: &CrateRecipe) -> Vec<Track> {
    let pool: Vec<Track> = if recipe.unplayed_only {
        candidates.into_iter().filter(Track::is_unplayed).collect()
    } else {
        candidates
    };

    if recipe.hot_per_album > 0 {
        keep_hottest_per_album(pool, recipe.hot_per_album as usize)
    } else {
        pool
    }
}

/// Keep each album's `n` most popular tracks.
///
/// An album whose tracks carry no popularity data at all is dropped rather
/// than contributing an arbitrary pick — the whole point of the rule is to
/// avoid album filler, and with no signal any choice would be filler at
/// random. Roughly 3% of a typical library falls in this bucket.
fn keep_hottest_per_album(tracks: Vec<Track>, n: usize) -> Vec<Track> {
    use std::collections::HashMap;

    let mut by_album: HashMap<String, Vec<Track>> = HashMap::new();
    for track in tracks {
        // Tracks reaching here come from a join against `albums`, so the key
        // is always present in practice; keyless ones share one bucket rather
        // than each counting as an album of one.
        let key = track.album_key.clone().unwrap_or_default();
        by_album.entry(key).or_default().push(track);
    }

    let mut out = Vec::new();
    for (_, mut group) in by_album {
        if group.iter().all(|t| t.rating_count.unwrap_or(0) == 0) {
            continue;
        }
        // Descending popularity, with track order then rating key as
        // tiebreakers so an album with equal counts yields a stable pick
        // rather than one that changes between regenerations.
        group.sort_by(|a, b| {
            b.rating_count
                .unwrap_or(0)
                .cmp(&a.rating_count.unwrap_or(0))
                .then_with(|| a.index.unwrap_or(i32::MAX).cmp(&b.index.unwrap_or(i32::MAX)))
                .then_with(|| a.rating_key.cmp(&b.rating_key))
        });
        group.truncate(n);
        out.append(&mut group);
    }
    out
}

/// Stratified shuffle: spreads each key-group's items at ~1/K average spacing.
///
/// A plain Fisher–Yates is uniformly random, but uniform randomness clusters —
/// a large same-key group lands back-to-back often enough to read as "not
/// shuffled". Each group is shuffled internally, then placed at jittered
/// evenly-spaced positions, so a group of K members lands roughly every 1/K of
/// the list while single-member groups stay fully random. Mirrors the
/// frontend's `balancedShuffle`.
/// Takes the RNG by argument rather than reaching for the thread RNG, so a
/// test can seed it and assert on the resulting order.
pub fn balanced_shuffle<T, K, F, R: Rng>(items: Vec<T>, key_of: F, rng: &mut R) -> Vec<T>
where
    F: Fn(&T) -> K,
    K: std::hash::Hash + Eq,
{
    use std::collections::HashMap;

    let mut groups: HashMap<K, Vec<T>> = HashMap::new();
    for item in items {
        groups.entry(key_of(&item)).or_default().push(item);
    }

    let mut placed: Vec<(f64, T)> = Vec::new();
    for (_, mut group) in groups {
        group.shuffle(rng);
        let len = group.len() as f64;
        for (i, item) in group.into_iter().enumerate() {
            placed.push(((i as f64 + rng.gen::<f64>()) / len, item));
        }
    }

    placed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    placed.into_iter().map(|(_, item)| item).collect()
}

// The shuffle inside `select_crate_tracks` needs a seedable RNG in tests, so it
// takes one; this keeps the public entry point ergonomic for callers that
// don't care.
impl CrateRecipe {
    /// Apply this recipe using the thread RNG.
    pub fn select(&self, candidates: Vec<Track>) -> Vec<Track> {
        select_crate_tracks(candidates, self, &mut rand::thread_rng())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn track(key: &str, album: &str, artist: &str, rating_count: Option<i64>, plays: i64) -> Track {
        Track {
            rating_key: key.into(),
            title: format!("Track {key}"),
            artist_name: artist.into(),
            track_artist: None,
            album_title: album.into(),
            album_key: Some(album.into()),
            index: Some(key.parse().unwrap_or(1)),
            duration: 200.0,
            codec: None,
            part_key: None,
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count,
            view_count: Some(plays),
            last_viewed_at: None,
        }
    }

    fn rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    // -- summary round trip --

    #[test]
    fn test_recipe_round_trips_through_a_summary() {
        let recipe = CrateRecipe {
            genres: vec!["Post-Hardcore".into(), "Midwest Emo".into()],
            include_subgenres: true,
            hot_per_album: 2,
            unplayed_only: true,
            count: 25,
        };
        let summary = recipe.to_summary();
        assert_eq!(CrateRecipe::from_summary(&summary), Some(recipe));
    }

    #[test]
    fn test_summary_leads_with_a_human_readable_line() {
        let recipe = CrateRecipe {
            genres: vec!["Post-Hardcore".into()],
            ..Default::default()
        };
        let summary = recipe.to_summary();
        let first = summary.lines().next().unwrap();
        assert_eq!(first, "25 unplayed hot tracks — Post-Hardcore (+sub-genres)");
        assert!(!first.contains(CRATE_MARKER));
    }

    #[test]
    fn test_genre_names_with_separators_survive_the_round_trip() {
        // A raw name carrying the pair or list separator would corrupt the
        // parse if it were not encoded before transport.
        let recipe = CrateRecipe {
            genres: vec!["Rock, Paper = Scissors".into(), "Hip Hop".into()],
            ..Default::default()
        };
        let parsed = CrateRecipe::from_summary(&recipe.to_summary()).unwrap();
        assert_eq!(parsed.genres, recipe.genres);
    }

    #[test]
    fn test_summary_without_a_recipe_yields_none() {
        assert_eq!(CrateRecipe::from_summary("Just my favourite songs."), None);
        assert_eq!(CrateRecipe::from_summary(""), None);
    }

    #[test]
    fn test_unknown_version_is_refused_rather_than_misread() {
        let line = format!("{}99 genres=Metal count=10", CRATE_MARKER);
        assert_eq!(CrateRecipe::from_summary(&line), None);
    }

    #[test]
    fn test_unknown_keys_are_ignored() {
        let line = format!("{}1 genres=Metal count=10 mood=angry", CRATE_MARKER);
        let parsed = CrateRecipe::from_summary(&line).unwrap();
        assert_eq!(parsed.count, 10);
        assert_eq!(parsed.genres, vec!["Metal".to_string()]);
    }

    #[test]
    fn test_a_user_edited_summary_keeps_the_recipe_while_the_marker_survives() {
        let recipe = CrateRecipe::default();
        let edited = format!("My own description\n{}", recipe.encode_line());
        assert_eq!(CrateRecipe::from_summary(&edited), Some(recipe));
    }

    // -- hot selection --

    #[test]
    fn test_hot_keeps_only_the_top_n_of_each_album() {
        let candidates = vec![
            track("1", "albumA", "Band", Some(100), 0),
            track("2", "albumA", "Band", Some(900), 0),
            track("3", "albumA", "Band", Some(500), 0),
            track("4", "albumB", "Other", Some(50), 0),
        ];
        let recipe = CrateRecipe {
            hot_per_album: 2,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        let picked = select_crate_tracks(candidates, &recipe, &mut rng());
        let mut keys: Vec<&str> = picked.iter().map(|t| t.rating_key.as_str()).collect();
        keys.sort_unstable();
        // Album A contributes its two most popular (2, 3) and not the least
        // popular (1); album B contributes its only track.
        assert_eq!(keys, vec!["2", "3", "4"]);
    }

    #[test]
    fn test_album_with_no_popularity_data_is_dropped_not_picked_at_random() {
        let candidates = vec![
            track("1", "known", "Band", Some(400), 0),
            track("2", "blank", "Other", None, 0),
            track("3", "blank", "Other", Some(0), 0),
        ];
        let recipe = CrateRecipe {
            hot_per_album: 1,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        let picked = select_crate_tracks(candidates, &recipe, &mut rng());
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].rating_key, "1");
    }

    #[test]
    fn test_equal_popularity_picks_the_same_track_every_time() {
        // Regeneration should not reshuffle which track represents an album
        // just because its counts tie.
        let build = || {
            vec![
                track("3", "album", "Band", Some(500), 0),
                track("1", "album", "Band", Some(500), 0),
                track("2", "album", "Band", Some(500), 0),
            ]
        };
        let recipe = CrateRecipe {
            hot_per_album: 1,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        let first = select_crate_tracks(build(), &recipe, &mut StdRng::seed_from_u64(1));
        let second = select_crate_tracks(build(), &recipe, &mut StdRng::seed_from_u64(9));
        assert_eq!(first[0].rating_key, "1");
        assert_eq!(second[0].rating_key, "1");
    }

    #[test]
    fn test_eligible_pool_scales_with_tracks_per_album() {
        // The builder's preview reports this figure, so it has to move when
        // the rule moves — a pool that reads the same for "best 1" and
        // "best 3" makes the control look inert.
        let candidates: Vec<Track> = (0..5)
            .flat_map(|album| {
                (0..6).map(move |i| {
                    track(
                        &format!("{album}{i}"),
                        &format!("album{album}"),
                        "Band",
                        Some(1000 - i as i64),
                        0,
                    )
                })
            })
            .collect();

        let pool_for = |hot: u8| {
            let recipe = CrateRecipe {
                hot_per_album: hot,
                unplayed_only: false,
                count: 0,
                ..Default::default()
            };
            eligible_tracks(candidates.clone(), &recipe).len()
        };

        assert_eq!(pool_for(1), 5);
        assert_eq!(pool_for(2), 10);
        assert_eq!(pool_for(3), 15);
        assert_eq!(pool_for(0), 30);
    }

    #[test]
    fn test_eligible_pool_is_what_selection_draws_from() {
        let candidates: Vec<Track> = (0..4)
            .flat_map(|album| {
                (0..5).map(move |i| {
                    track(
                        &format!("{album}{i}"),
                        &format!("album{album}"),
                        "Band",
                        Some(500 - i as i64),
                        if i == 0 { 3 } else { 0 },
                    )
                })
            })
            .collect();
        let recipe = CrateRecipe {
            hot_per_album: 2,
            unplayed_only: true,
            count: 0,
            ..Default::default()
        };
        let pool = eligible_tracks(candidates.clone(), &recipe).len();
        let picked = select_crate_tracks(candidates, &recipe, &mut rng()).len();
        assert_eq!(pool, picked, "an uncapped crate should hold the whole pool");
    }

    #[test]
    fn test_hot_disabled_lets_every_candidate_through() {
        let candidates = vec![
            track("1", "albumA", "Band", Some(10), 0),
            track("2", "albumA", "Band", Some(20), 0),
            track("3", "albumA", "Band", None, 0),
        ];
        let recipe = CrateRecipe {
            hot_per_album: 0,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        assert_eq!(select_crate_tracks(candidates, &recipe, &mut rng()).len(), 3);
    }

    // -- unplayed --

    #[test]
    fn test_unplayed_only_drops_played_tracks() {
        let candidates = vec![
            track("1", "albumA", "Band", Some(100), 0),
            track("2", "albumB", "Band", Some(100), 3),
        ];
        let recipe = CrateRecipe {
            hot_per_album: 0,
            unplayed_only: true,
            count: 0,
            ..Default::default()
        };
        let picked = select_crate_tracks(candidates, &recipe, &mut rng());
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].rating_key, "1");
    }

    #[test]
    fn test_never_synced_play_state_counts_as_unplayed() {
        // Tracks synced before the play-state columns existed report None.
        // Treating that as "played" would empty the crate for anyone who
        // hasn't resynced.
        let mut t = track("1", "albumA", "Band", Some(100), 0);
        t.view_count = None;
        let recipe = CrateRecipe {
            hot_per_album: 0,
            unplayed_only: true,
            count: 0,
            ..Default::default()
        };
        assert_eq!(select_crate_tracks(vec![t], &recipe, &mut rng()).len(), 1);
    }

    #[test]
    fn test_unplayed_is_applied_before_hot_so_an_album_survives_on_its_deeper_cuts() {
        // The album's most popular track is already played. The rule should
        // still offer its next-best unplayed track rather than dropping the
        // album for having no unplayed hit.
        let candidates = vec![
            track("1", "album", "Band", Some(900), 5),
            track("2", "album", "Band", Some(400), 0),
        ];
        let recipe = CrateRecipe {
            hot_per_album: 1,
            unplayed_only: true,
            count: 0,
            ..Default::default()
        };
        let picked = select_crate_tracks(candidates, &recipe, &mut rng());
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].rating_key, "2");
    }

    // -- count + shuffle --

    #[test]
    fn test_count_caps_the_result() {
        let candidates: Vec<Track> = (0..50)
            .map(|i| track(&i.to_string(), &format!("album{i}"), "Band", Some(100), 0))
            .collect();
        let recipe = CrateRecipe {
            hot_per_album: 1,
            unplayed_only: false,
            count: 25,
            ..Default::default()
        };
        assert_eq!(select_crate_tracks(candidates, &recipe, &mut rng()).len(), 25);
    }

    #[test]
    fn test_count_of_zero_means_no_limit() {
        let candidates: Vec<Track> = (0..30)
            .map(|i| track(&i.to_string(), &format!("album{i}"), "Band", Some(100), 0))
            .collect();
        let recipe = CrateRecipe {
            hot_per_album: 1,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        assert_eq!(select_crate_tracks(candidates, &recipe, &mut rng()).len(), 30);
    }

    #[test]
    fn test_shuffle_spreads_one_artist_instead_of_clumping() {
        // 10 tracks by one artist and 10 single-track artists: a stratified
        // shuffle should not leave the big group in one run.
        let mut candidates: Vec<Track> = (0..10)
            .map(|i| track(&format!("b{i}"), &format!("big{i}"), "Prolific", Some(100), 0))
            .collect();
        candidates.extend(
            (0..10).map(|i| track(&format!("s{i}"), &format!("sm{i}"), &format!("Solo{i}"), Some(100), 0)),
        );
        let recipe = CrateRecipe {
            hot_per_album: 0,
            unplayed_only: false,
            count: 0,
            ..Default::default()
        };
        let picked = select_crate_tracks(candidates, &recipe, &mut rng());

        let longest_run = picked
            .iter()
            .fold((0, 0, String::new()), |(best, run, prev), t| {
                let run = if t.artist_name == prev { run + 1 } else { 1 };
                (best.max(run), run, t.artist_name.clone())
            })
            .0;
        assert!(
            longest_run <= 4,
            "one artist clumped into a run of {longest_run}"
        );
    }

    #[test]
    fn test_selection_is_stable_when_nothing_survives_the_filters() {
        let recipe = CrateRecipe::default();
        assert!(select_crate_tracks(Vec::new(), &recipe, &mut rng()).is_empty());
    }
}
