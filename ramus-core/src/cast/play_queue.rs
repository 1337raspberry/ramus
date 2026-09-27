//! Server play queues (`/playQueues`): the queue a cast plays, held on the
//! Plex Media Server so the player fetches its tracks from there.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Rating keys per request when a queue is sent to the server; keeps the
/// `uri` parameter to a few kilobytes.
pub const UPLOAD_CHUNK: usize = 200;
/// Tracks before the current one sent along, so "previous" works on the player.
pub const CAST_HISTORY_MAX: usize = 50;
/// Items per side of the centre in a play-queue fetch.
pub const SLICE_WINDOW: usize = 200;

/// PMS sends some numeric fields as JSON strings.
fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn flex_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(value_i64(&Value::deserialize(d)?))
}

/// One occurrence in a play queue. A track queued twice has two items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayQueueItem {
    pub item_id: i64,
    pub rating_key: String,
}

/// The loaded part of a server play queue, in queue order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayQueue {
    pub id: i64,
    pub version: i64,
    pub selected_item_id: Option<i64>,
    /// The selected item's position in the whole queue.
    pub selected_item_offset: Option<i64>,
    /// Absent for queues that grow as they play.
    pub total_count: Option<i64>,
    pub items: Vec<PlayQueueItem>,
    /// Set once a following slice brought nothing new.
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlayQueueError {
    #[error("not a play queue")]
    NotAPlayQueue,
    #[error("malformed play queue")]
    Malformed,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "MediaContainer")]
    container: Container,
}

#[derive(Deserialize)]
struct Container {
    #[serde(rename = "playQueueID", default, deserialize_with = "flex_i64")]
    id: Option<i64>,
    #[serde(rename = "playQueueVersion", default, deserialize_with = "flex_i64")]
    version: Option<i64>,
    #[serde(
        rename = "playQueueSelectedItemID",
        default,
        deserialize_with = "flex_i64"
    )]
    selected_item_id: Option<i64>,
    #[serde(
        rename = "playQueueSelectedItemOffset",
        default,
        deserialize_with = "flex_i64"
    )]
    selected_item_offset: Option<i64>,
    #[serde(rename = "playQueueTotalCount", default, deserialize_with = "flex_i64")]
    total_count: Option<i64>,
    #[serde(rename = "Metadata", default)]
    metadata: Vec<Value>,
}

impl PlayQueue {
    /// Decodes a `/playQueues` reply. Items without an ID or rating key are
    /// skipped, so one odd item never fails the queue.
    pub fn decode(bytes: &[u8]) -> Result<Self, PlayQueueError> {
        let container = serde_json::from_slice::<Response>(bytes)
            .map_err(|_| PlayQueueError::Malformed)?
            .container;
        let id = container.id.ok_or(PlayQueueError::NotAPlayQueue)?;
        let items = container
            .metadata
            .iter()
            .filter_map(|m| {
                let item_id = m.get("playQueueItemID").and_then(value_i64)?;
                let rating_key = match m.get("ratingKey")? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => return None,
                };
                Some(PlayQueueItem {
                    item_id,
                    rating_key,
                })
            })
            .collect();
        Ok(Self {
            id,
            version: container.version.unwrap_or(1),
            selected_item_id: container.selected_item_id,
            selected_item_offset: container.selected_item_offset,
            total_count: container.total_count,
            items,
            exhausted: false,
        })
    }

    fn index_of_item(&self, item_id: i64) -> Option<usize> {
        self.items.iter().position(|i| i.item_id == item_id)
    }

    /// The selected item's index here: by ID, else its offset when that lies
    /// inside this window, else 0.
    pub fn selected_index(&self) -> usize {
        if let Some(index) = self.selected_item_id.and_then(|id| self.index_of_item(id)) {
            return index;
        }
        match self.selected_item_offset {
            Some(offset) if offset >= 0 && (offset as usize) < self.items.len() => offset as usize,
            _ => 0,
        }
    }

    /// The whole-queue position of `items[0]`, when the selection places it.
    fn window_start(&self) -> Option<i64> {
        let offset = self.selected_item_offset?;
        let index = self.index_of_item(self.selected_item_id?)?;
        Some(offset - index as i64)
    }

    /// Whether the server holds items after the loaded ones.
    pub fn has_more_after(&self) -> bool {
        if self.exhausted || self.items.is_empty() {
            return false;
        }
        match (self.total_count, self.window_start()) {
            (Some(total), Some(start)) => start + (self.items.len() as i64) < total,
            _ => true,
        }
    }

    /// The item to centre the next slice on (the last loaded), while more follow.
    pub fn next_slice_center(&self) -> Option<i64> {
        if self.has_more_after() {
            self.items.last().map(|i| i.item_id)
        } else {
            None
        }
    }

    /// Adds a following slice's new items; returns how many. A slice that
    /// brings nothing new marks the queue exhausted.
    pub fn append_slice(&mut self, slice: PlayQueue) -> usize {
        let before = self.items.len();
        for item in slice.items {
            if self.index_of_item(item.item_id).is_none() {
                self.items.push(item);
            }
        }
        let added = self.items.len() - before;
        if added == 0 {
            self.exhausted = true;
        }
        if slice.total_count.is_some() {
            self.total_count = slice.total_count;
        }
        self.version = self.version.max(slice.version);
        added
    }
}

/// How a queue goes to the server: one request that creates the play queue
/// (`first`, selecting `key`), then appends of `rest` in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    pub first: Vec<String>,
    /// The rating key of the track to start on.
    pub key: String,
    pub rest: Vec<Vec<String>>,
}

/// Splits `rating_keys` for upload, starting at `index`. Up to
/// [`CAST_HISTORY_MAX`] earlier tracks go along, unless the current track
/// also occurs among them: the server selects the first matching item, so
/// history would put the selection on the wrong copy.
pub fn upload_plan(rating_keys: &[String], index: usize) -> Option<UploadPlan> {
    let current = rating_keys.get(index)?;
    let history_start = index.saturating_sub(CAST_HISTORY_MAX);
    let start = if rating_keys[history_start..index].contains(current) {
        index
    } else {
        history_start
    };
    let first_end = (start + UPLOAD_CHUNK).min(rating_keys.len());
    Some(UploadPlan {
        first: rating_keys[start..first_end].to_vec(),
        key: current.clone(),
        rest: rating_keys[first_end..]
            .chunks(UPLOAD_CHUNK)
            .map(<[String]>::to_vec)
            .collect(),
    })
}

/// Chunks for a "play next" insert. Each insert lands directly after the
/// current track, so the last chunk goes first and the order survives.
pub fn insert_chunks(rating_keys: &[String]) -> Vec<Vec<String>> {
    rating_keys
        .chunks(UPLOAD_CHUNK)
        .rev()
        .map(<[String]>::to_vec)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PlayQueue {
        let json = match name {
            "album" => include_str!("fixtures/companion-playqueue-album.json"),
            "long" => include_str!("fixtures/companion-playqueue-long.json"),
            "long-slice" => include_str!("fixtures/companion-playqueue-long-slice.json"),
            "station" => include_str!("fixtures/companion-playqueue-station.json"),
            _ => unreachable!(),
        };
        PlayQueue::decode(json.as_bytes()).unwrap()
    }

    fn ids(q: &PlayQueue) -> Vec<i64> {
        q.items.iter().map(|i| i.item_id).collect()
    }

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| i.to_string()).collect()
    }

    #[test]
    fn decodes_an_album_queue() {
        let q = fixture("album");
        assert_eq!((q.id, q.version, q.total_count), (9001, 3, Some(3)));
        assert_eq!(ids(&q), [5001, 5002, 5003]);
        assert_eq!(q.items[1].rating_key, "2102");
        assert_eq!(q.selected_index(), 1);
        assert!(!q.has_more_after());
    }

    #[test]
    fn a_window_short_of_the_total_has_more_after_it() {
        let mut q = fixture("long");
        assert!(q.has_more_after());
        assert_eq!(q.next_slice_center(), Some(6003));
        assert_eq!(q.append_slice(fixture("long-slice")), 2);
        assert_eq!(ids(&q), [6001, 6002, 6003, 6004, 6005]);
        assert!(!q.has_more_after());
    }

    #[test]
    fn a_queue_without_a_total_stops_when_a_slice_adds_nothing() {
        let mut q = fixture("station");
        assert_eq!((q.id, q.version, q.total_count), (9100, 1, None));
        assert_eq!(ids(&q), [7001, 7002]);
        assert!(q.has_more_after());
        assert_eq!(q.append_slice(fixture("station")), 0);
        assert!(q.exhausted);
        assert!(!q.has_more_after());
    }

    #[test]
    fn selected_index_falls_back_to_the_offset_then_the_start() {
        let mut q = fixture("album");
        q.selected_item_id = Some(99);
        q.selected_item_offset = Some(2);
        assert_eq!(q.selected_index(), 2);
        q.selected_item_offset = Some(7);
        assert_eq!(q.selected_index(), 0);
    }

    #[test]
    fn items_without_an_id_or_key_are_skipped() {
        let json = r#"{"MediaContainer":{"playQueueID":1,"playQueueVersion":1,"Metadata":[
            {"playQueueItemID":1,"ratingKey":"1"},
            {"playQueueItemID":2},
            {"ratingKey":"3"},
            {"playQueueItemID":"4","ratingKey":4}
        ]}}"#;
        let q = PlayQueue::decode(json.as_bytes()).unwrap();
        assert_eq!(
            q.items,
            vec![
                PlayQueueItem {
                    item_id: 1,
                    rating_key: "1".into()
                },
                PlayQueueItem {
                    item_id: 4,
                    rating_key: "4".into()
                },
            ]
        );
    }

    #[test]
    fn a_container_that_is_not_a_play_queue_is_refused() {
        assert_eq!(
            PlayQueue::decode(br#"{"MediaContainer":{"size":0}}"#),
            Err(PlayQueueError::NotAPlayQueue)
        );
        assert_eq!(
            PlayQueue::decode(b"not json"),
            Err(PlayQueueError::Malformed)
        );
    }

    #[test]
    fn a_short_queue_goes_in_one_request() {
        let plan = upload_plan(&keys(10), 4).unwrap();
        assert_eq!(plan.first, keys(10));
        assert_eq!(plan.key, "4");
        assert!(plan.rest.is_empty());
    }

    #[test]
    fn history_is_capped() {
        let all = keys(300);
        let plan = upload_plan(&all, 120).unwrap();
        assert_eq!(plan.first, all[70..270].to_vec());
        assert_eq!(plan.rest, vec![all[270..300].to_vec()]);
    }

    #[test]
    fn a_long_tail_is_appended_in_chunks() {
        let all = keys(1200);
        let plan = upload_plan(&all, 0).unwrap();
        assert_eq!(plan.first, all[0..200].to_vec());
        assert_eq!(plan.rest.len(), 5);
        assert!(plan.rest.iter().all(|c| c.len() == 200));
        assert_eq!(plan.rest[4].last().unwrap(), "1199");
    }

    #[test]
    fn a_current_track_repeated_in_history_drops_the_history() {
        let all: Vec<String> = ["a", "b", "a", "c"].iter().map(|s| s.to_string()).collect();
        let plan = upload_plan(&all, 2).unwrap();
        assert_eq!(plan.first, ["a", "c"]);
        assert_eq!(plan.key, "a");
    }

    #[test]
    fn an_index_past_the_end_has_no_plan() {
        assert_eq!(upload_plan(&keys(3), 3), None);
        assert_eq!(upload_plan(&[], 0), None);
    }

    #[test]
    fn insert_chunks_run_last_first() {
        let all = keys(450);
        let chunks = insert_chunks(&all);
        assert_eq!(
            chunks,
            vec![
                all[400..450].to_vec(),
                all[200..400].to_vec(),
                all[0..200].to_vec()
            ]
        );
    }
}
