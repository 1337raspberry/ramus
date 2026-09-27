//! The XML a Companion player answers with: its timeline (what it plays)
//! and its `/resources` description.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// A player's transport state, as its music timeline reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteState {
    Playing,
    Paused,
    Buffering,
    Stopped,
}

/// The `type="music"` timeline of a poll reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicTimeline {
    pub state: RemoteState,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub rating_key: Option<String>,
    pub play_queue_id: Option<i64>,
    pub play_queue_item_id: Option<i64>,
    pub play_queue_version: Option<i64>,
}

/// A player's `/resources` description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub machine_identifier: String,
    pub title: Option<String>,
    pub capabilities: Vec<String>,
}

impl PlayerInfo {
    /// Whether the player can take a cast: it reports timelines, takes
    /// playback commands and plays server play queues.
    pub fn can_cast(&self) -> bool {
        ["timeline", "playback", "playqueues"]
            .iter()
            .all(|needed| self.capabilities.iter().any(|c| c == needed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum XmlError {
    #[error("malformed XML")]
    Malformed,
    #[error("no Player element")]
    NoPlayer,
}

pub(super) type Attributes = Vec<(String, String)>;

pub(super) fn attributes(element: &BytesStart) -> Result<Attributes, XmlError> {
    let mut out = Vec::new();
    for attr in element.attributes() {
        let attr = attr.map_err(|_| XmlError::Malformed)?;
        let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| XmlError::Malformed)?;
        let raw = std::str::from_utf8(&attr.value).map_err(|_| XmlError::Malformed)?;
        let value = quick_xml::escape::unescape(raw).map_err(|_| XmlError::Malformed)?;
        out.push((key.to_string(), value.into_owned()));
    }
    Ok(out)
}

/// The attributes of the first `name` element that `matches` accepts.
fn find_element(
    xml: &str,
    name: &[u8],
    matches: impl Fn(&Attributes) -> bool,
) -> Result<Option<Attributes>, XmlError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == name => {
                let attrs = attributes(&e)?;
                if matches(&attrs) {
                    return Ok(Some(attrs));
                }
            }
            Ok(Event::Eof) => return Ok(None),
            Err(_) => return Err(XmlError::Malformed),
            _ => {}
        }
    }
}

pub(super) fn get<'a>(attrs: &'a Attributes, key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn number(attrs: &Attributes, key: &str) -> Option<i64> {
    get(attrs, key).and_then(|v| v.parse().ok())
}

/// The music timeline of a poll reply; `None` when the reply has none.
pub fn parse_timeline(xml: &str) -> Result<Option<MusicTimeline>, XmlError> {
    let Some(attrs) = find_element(xml, b"Timeline", |a| get(a, "type") == Some("music"))? else {
        return Ok(None);
    };
    let state = match get(&attrs, "state") {
        Some("playing") => RemoteState::Playing,
        Some("paused") => RemoteState::Paused,
        Some("buffering") => RemoteState::Buffering,
        _ => RemoteState::Stopped,
    };
    Ok(Some(MusicTimeline {
        state,
        time_ms: number(&attrs, "time").unwrap_or(0).max(0) as u64,
        duration_ms: number(&attrs, "duration").unwrap_or(0).max(0) as u64,
        rating_key: get(&attrs, "ratingKey")
            .filter(|k| !k.is_empty())
            .map(str::to_string),
        play_queue_id: number(&attrs, "playQueueID"),
        play_queue_item_id: number(&attrs, "playQueueItemID"),
        play_queue_version: number(&attrs, "playQueueVersion"),
    }))
}

/// The `Player` element of a `/resources` reply.
pub fn parse_player_resources(xml: &str) -> Result<PlayerInfo, XmlError> {
    let attrs = find_element(xml, b"Player", |_| true)?.ok_or(XmlError::NoPlayer)?;
    let machine_identifier = get(&attrs, "machineIdentifier")
        .filter(|m| !m.is_empty())
        .ok_or(XmlError::NoPlayer)?
        .to_string();
    Ok(PlayerInfo {
        machine_identifier,
        title: get(&attrs, "title").map(str::to_string),
        capabilities: get(&attrs, "protocolCapabilities")
            .unwrap_or_default()
            .split(',')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: &str = include_str!("fixtures/companion-timeline-idle.xml");
    const PLAYING: &str = include_str!("fixtures/companion-timeline-playing.xml");
    const PAUSED: &str = include_str!("fixtures/companion-timeline-paused.xml");
    const RESOURCES: &str = include_str!("fixtures/companion-resources.xml");

    #[test]
    fn parses_a_playing_timeline() {
        assert_eq!(
            parse_timeline(PLAYING).unwrap(),
            Some(MusicTimeline {
                state: RemoteState::Playing,
                time_ms: 61_250,
                duration_ms: 245_123,
                rating_key: Some("2102".into()),
                play_queue_id: Some(9001),
                play_queue_item_id: Some(5002),
                play_queue_version: Some(3),
            })
        );
    }

    #[test]
    fn parses_a_paused_timeline_without_a_play_queue() {
        let t = parse_timeline(PAUSED).unwrap().unwrap();
        assert_eq!(t.state, RemoteState::Paused);
        assert_eq!((t.time_ms, t.duration_ms), (5_000, 180_000));
        assert_eq!(t.rating_key.as_deref(), Some("77"));
        assert_eq!((t.play_queue_id, t.play_queue_item_id), (None, None));
    }

    #[test]
    fn an_idle_timeline_is_stopped_with_no_track() {
        let t = parse_timeline(IDLE).unwrap().unwrap();
        assert_eq!(t.state, RemoteState::Stopped);
        assert_eq!(t.rating_key, None);
        assert_eq!(t.time_ms, 0);
    }

    #[test]
    fn buffering_is_its_own_state() {
        let xml = r#"<MediaContainer><Timeline type="music" state="buffering" time="10"/></MediaContainer>"#;
        assert_eq!(
            parse_timeline(xml).unwrap().unwrap().state,
            RemoteState::Buffering
        );
    }

    #[test]
    fn a_container_without_a_music_timeline_is_none() {
        let xml = r#"<MediaContainer><Timeline type="video" state="playing"/></MediaContainer>"#;
        assert_eq!(parse_timeline(xml), Ok(None));
        assert_eq!(parse_timeline(""), Ok(None));
    }

    #[test]
    fn escaped_attribute_values_are_unescaped() {
        let xml = r#"<MediaContainer><Timeline type="music" state="paused" ratingKey="a&amp;b"/></MediaContainer>"#;
        assert_eq!(
            parse_timeline(xml).unwrap().unwrap().rating_key.as_deref(),
            Some("a&b")
        );
    }

    #[test]
    fn an_unclosed_tag_is_malformed() {
        assert_eq!(
            parse_timeline(r#"<MediaContainer><Timeline type="music" state="playing"#),
            Err(XmlError::Malformed)
        );
    }

    #[test]
    fn parses_player_resources() {
        let info = parse_player_resources(RESOURCES).unwrap();
        assert_eq!(info.machine_identifier, "ramus-client-id");
        assert_eq!(info.title.as_deref(), Some("ramus"));
        assert!(info.can_cast());
    }

    #[test]
    fn a_player_without_play_queue_support_cannot_cast() {
        let xml = r#"<MediaContainer><Player machineIdentifier="x" protocolCapabilities="timeline,playback"/></MediaContainer>"#;
        assert!(!parse_player_resources(xml).unwrap().can_cast());
    }

    #[test]
    fn resources_without_a_player_are_refused() {
        assert_eq!(
            parse_player_resources("<MediaContainer/>"),
            Err(XmlError::NoPlayer)
        );
    }
}
