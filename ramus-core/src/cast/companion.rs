//! The controller side of the Plex Companion protocol: commands a player
//! over HTTP on its own port (`/player/playback/*`) and reads back what it
//! plays by polling its timeline (`/player/timeline/poll`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use url::Url;

use super::players::CastPlayer;
use super::timeline::{
    parse_player_resources, parse_timeline, MusicTimeline, PlayerInfo, XmlError,
};
use crate::plex::client::join_path;
use crate::util::redact_urls;

/// The identity a controller presents to players.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerIdentity {
    pub client_identifier: String,
    pub platform: &'static str,
    pub device: &'static str,
}

/// The Plex Media Server a cast plays from, as a player is told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub machine_identifier: String,
    pub protocol: String,
    pub address: String,
    pub port: u16,
}

impl ServerAddress {
    /// The server at `url`: its scheme, host, and port (80/443 by scheme
    /// when the URL names none).
    pub fn from_url(machine_identifier: &str, url: &Url) -> Option<Self> {
        Some(Self {
            machine_identifier: machine_identifier.to_string(),
            protocol: url.scheme().to_string(),
            address: url.host_str()?.to_string(),
            port: url.port_or_known_default()?,
        })
    }
}

/// Start a server play queue on the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayMedia {
    pub rating_key: String,
    pub offset_ms: u64,
    pub paused: bool,
    pub play_queue_id: i64,
    pub server: ServerAddress,
    /// A transient delegation token for the server. Never the long-lived
    /// token: the command travels over plain HTTP.
    pub token: String,
}

/// A transport command for the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerCommand {
    Play,
    Pause,
    Stop,
    SkipNext,
    SkipPrevious,
    SeekTo {
        offset_ms: u64,
    },
    /// Jump to a play-queue item. The item ID goes in both `key` and
    /// `playQueueItemID`: receivers differ in which they read, and a library
    /// key would be ambiguous when a track is queued twice.
    SkipTo {
        item_id: i64,
    },
    /// Reload the play queue after it was edited on the server.
    RefreshPlayQueue {
        play_queue_id: i64,
    },
}

impl PlayerCommand {
    fn path(&self) -> &'static str {
        match self {
            Self::Play => "player/playback/play",
            Self::Pause => "player/playback/pause",
            Self::Stop => "player/playback/stop",
            Self::SkipNext => "player/playback/skipNext",
            Self::SkipPrevious => "player/playback/skipPrevious",
            Self::SeekTo { .. } => "player/playback/seekTo",
            Self::SkipTo { .. } => "player/playback/skipTo",
            Self::RefreshPlayQueue { .. } => "player/playback/refreshPlayQueue",
        }
    }

    fn params(&self) -> Vec<(&'static str, String)> {
        let mut params = match self {
            Self::SeekTo { offset_ms } => vec![("offset", offset_ms.to_string())],
            Self::SkipTo { item_id } => vec![
                ("key", item_id.to_string()),
                ("playQueueItemID", item_id.to_string()),
            ],
            Self::RefreshPlayQueue { play_queue_id } => {
                vec![("playQueueID", play_queue_id.to_string())]
            }
            _ => Vec::new(),
        };
        params.push(("type", "music".to_string()));
        params
    }
}

fn endpoint(base: &Url, path: &str) -> Url {
    join_path(base, path).expect("a constant relative path joins onto a parsed base URL")
}

fn player_url(base: &Url, path: &str, params: &[(&'static str, String)], command_id: u64) -> Url {
    let mut url = endpoint(base, path);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in params {
            query.append_pair(key, value);
        }
        query.append_pair("commandID", &command_id.to_string());
    }
    url
}

pub fn command_url(base: &Url, command: &PlayerCommand, command_id: u64) -> Url {
    player_url(base, command.path(), &command.params(), command_id)
}

pub fn play_media_url(base: &Url, play: &PlayMedia, command_id: u64) -> Url {
    let params = [
        (
            "providerIdentifier",
            "com.plexapp.plugins.library".to_string(),
        ),
        ("machineIdentifier", play.server.machine_identifier.clone()),
        ("protocol", play.server.protocol.clone()),
        ("address", play.server.address.clone()),
        ("port", play.server.port.to_string()),
        ("offset", play.offset_ms.to_string()),
        ("key", format!("/library/metadata/{}", play.rating_key)),
        ("type", "music".to_string()),
        (
            "containerKey",
            format!("/playQueues/{}?own=1", play.play_queue_id),
        ),
        ("paused", if play.paused { "1" } else { "0" }.to_string()),
        ("token", play.token.clone()),
    ];
    player_url(base, "player/playback/playMedia", &params, command_id)
}

pub fn poll_url(base: &Url, command_id: u64) -> Url {
    let mut url = endpoint(base, "player/timeline/poll");
    url.query_pairs_mut()
        .append_pair("wait", "0")
        .append_pair("commandID", &command_id.to_string());
    url
}

pub fn resources_url(base: &Url) -> Url {
    endpoint(base, "resources")
}

/// How long one command or poll may take. A player on the LAN answers in
/// milliseconds; anything slower counts as unreachable.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompanionError {
    /// No answer. The message has had its URLs (and so any token) removed.
    #[error("unreachable: {0}")]
    Unreachable(String),
    #[error("HTTP {0}")]
    Http(u16),
    #[error("unexpected reply")]
    InvalidResponse,
    #[error("another device answers at this address")]
    WrongPlayer,
    #[error("the player can't play a Plex play queue")]
    Unsupported,
}

impl From<XmlError> for CompanionError {
    fn from(_: XmlError) -> Self {
        Self::InvalidResponse
    }
}

/// One timeline poll's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Timeline(MusicTimeline),
    /// The player answered but plays no music.
    NoMusicTimeline,
    Failed,
}

/// Commands one player. `commandID` rises by one per request, polls
/// included, as receivers expect from a controller.
pub struct CompanionClient {
    base: Url,
    target: String,
    identity: ControllerIdentity,
    http: reqwest::Client,
    command_id: AtomicU64,
}

impl CompanionClient {
    pub fn new(base: Url, target: String, identity: ControllerIdentity) -> Self {
        Self::with_timeout(base, target, identity, COMMAND_TIMEOUT)
    }

    pub fn with_timeout(
        base: Url,
        target: String,
        identity: ControllerIdentity,
        timeout: Duration,
    ) -> Self {
        // No redirects: following one would carry a command's token to
        // another origin.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(3)))
            .timeout(timeout)
            .build()
            .expect("reqwest client init");
        Self {
            base,
            target,
            identity,
            http,
            command_id: AtomicU64::new(0),
        }
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    fn next_command_id(&self) -> u64 {
        self.command_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    async fn get(&self, url: Url) -> Result<String, CompanionError> {
        let resp = self
            .http
            .get(url)
            .header("X-Plex-Client-Identifier", &self.identity.client_identifier)
            .header("X-Plex-Target-Client-Identifier", &self.target)
            .header("X-Plex-Product", "ramus")
            .header("X-Plex-Platform", self.identity.platform)
            .header("X-Plex-Device", self.identity.device)
            .header("X-Plex-Device-Name", "ramus")
            .send()
            .await
            .map_err(|e| CompanionError::Unreachable(redact_urls(&e.to_string())))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(CompanionError::Http(status));
        }
        resp.text()
            .await
            .map_err(|_| CompanionError::InvalidResponse)
    }

    /// Checks the player at this address is the one named, and can take a cast.
    pub async fn probe(&self) -> Result<PlayerInfo, CompanionError> {
        let body = self.get(resources_url(&self.base)).await?;
        let info = parse_player_resources(&body)?;
        if info.machine_identifier != self.target {
            return Err(CompanionError::WrongPlayer);
        }
        if !info.can_cast() {
            return Err(CompanionError::Unsupported);
        }
        Ok(info)
    }

    pub async fn play_media(&self, play: &PlayMedia) -> Result<(), CompanionError> {
        let url = play_media_url(&self.base, play, self.next_command_id());
        self.get(url).await.map(|_| ())
    }

    pub async fn send(&self, command: &PlayerCommand) -> Result<(), CompanionError> {
        let url = command_url(&self.base, command, self.next_command_id());
        self.get(url).await.map(|_| ())
    }

    pub async fn poll(&self) -> PollOutcome {
        match self.get(poll_url(&self.base, self.next_command_id())).await {
            Ok(body) => match parse_timeline(&body) {
                Ok(Some(timeline)) => PollOutcome::Timeline(timeline),
                Ok(None) => PollOutcome::NoMusicTimeline,
                Err(_) => PollOutcome::Failed,
            },
            Err(_) => PollOutcome::Failed,
        }
    }
}

/// The first of `player`'s connections that answers a probe, in order.
pub async fn find_reachable(
    player: &CastPlayer,
    identity: &ControllerIdentity,
    timeout: Duration,
) -> Option<(Url, PlayerInfo)> {
    for uri in &player.connections {
        let Ok(url) = Url::parse(uri) else { continue };
        let client = CompanionClient::with_timeout(
            url.clone(),
            player.id.clone(),
            identity.clone(),
            timeout,
        );
        if let Ok(info) = client.probe().await {
            return Some((url, info));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const PLAY_MEDIA: &str = include_str!("fixtures/companion-request-playMedia.txt");
    const SKIP_TO: &str = include_str!("fixtures/companion-request-skipTo.txt");

    fn base() -> Url {
        Url::parse("http://192.168.1.50:32500").unwrap()
    }

    /// The query of a captured request's first line, decoded.
    fn fixture_query(request: &str) -> HashMap<String, String> {
        let line = request.lines().next().unwrap();
        let target = line.split(' ').nth(1).unwrap();
        let query = target.split_once('?').unwrap().1;
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect()
    }

    fn query(url: &Url) -> HashMap<String, String> {
        url.query_pairs().into_owned().collect()
    }

    fn fixture_play_media() -> PlayMedia {
        PlayMedia {
            rating_key: "2103".into(),
            offset_ms: 15_000,
            paused: false,
            play_queue_id: 9001,
            server: ServerAddress {
                machine_identifier: "server-mid".into(),
                protocol: "http".into(),
                address: "192.168.1.20".into(),
                port: 32400,
            },
            token: "transient-token".into(),
        }
    }

    #[test]
    fn play_media_matches_what_a_receiver_parses() {
        let url = play_media_url(&base(), &fixture_play_media(), 5);
        assert_eq!(url.path(), "/player/playback/playMedia");
        let ours = query(&url);
        let theirs = fixture_query(PLAY_MEDIA);
        for (key, value) in &theirs {
            if key == "containerKey" {
                continue;
            }
            assert_eq!(ours.get(key), Some(value), "parameter {key}");
        }
        assert!(theirs["containerKey"].starts_with("/playQueues/9001"));
        assert_eq!(ours["containerKey"], "/playQueues/9001?own=1");
        assert_eq!(ours["paused"], "0");
    }

    #[test]
    fn play_media_percent_encodes_paths() {
        let url = play_media_url(&base(), &fixture_play_media(), 5);
        let raw = url.query().unwrap();
        assert!(raw.contains("key=%2Flibrary%2Fmetadata%2F2103"), "{raw}");
        assert!(
            raw.contains("containerKey=%2FplayQueues%2F9001%3Fown%3D1"),
            "{raw}"
        );
    }

    #[test]
    fn skip_to_carries_the_item_id_both_ways() {
        let url = command_url(&base(), &PlayerCommand::SkipTo { item_id: 5003 }, 6);
        assert_eq!(url.path(), "/player/playback/skipTo");
        let ours = query(&url);
        for (key, value) in fixture_query(SKIP_TO) {
            assert_eq!(ours.get(&key), Some(&value), "parameter {key}");
        }
        assert_eq!(ours["playQueueItemID"], "5003");
    }

    #[test]
    fn simple_commands_carry_type_and_command_id() {
        let url = command_url(&base(), &PlayerCommand::SeekTo { offset_ms: 60_000 }, 9);
        assert_eq!(url.path(), "/player/playback/seekTo");
        let q = query(&url);
        assert_eq!(
            (
                q["offset"].as_str(),
                q["type"].as_str(),
                q["commandID"].as_str()
            ),
            ("60000", "music", "9")
        );

        let url = command_url(
            &base(),
            &PlayerCommand::RefreshPlayQueue {
                play_queue_id: 9001,
            },
            10,
        );
        assert_eq!(url.path(), "/player/playback/refreshPlayQueue");
        assert_eq!(query(&url)["playQueueID"], "9001");
    }

    #[test]
    fn poll_and_resources_urls() {
        assert_eq!(
            poll_url(&base(), 7).as_str(),
            "http://192.168.1.50:32500/player/timeline/poll?wait=0&commandID=7"
        );
        assert_eq!(
            resources_url(&base()).as_str(),
            "http://192.168.1.50:32500/resources"
        );
    }

    #[test]
    fn server_address_defaults_the_port_by_scheme() {
        let https = Url::parse("https://1-2-3-4.abc.plex.direct:32400").unwrap();
        let a = ServerAddress::from_url("mid", &https).unwrap();
        assert_eq!(
            (a.protocol.as_str(), a.address.as_str(), a.port),
            ("https", "1-2-3-4.abc.plex.direct", 32400)
        );
        let bare = Url::parse("https://plex.example.org").unwrap();
        assert_eq!(ServerAddress::from_url("mid", &bare).unwrap().port, 443);
        let http = Url::parse("http://10.0.0.5").unwrap();
        assert_eq!(ServerAddress::from_url("mid", &http).unwrap().port, 80);
    }

    use crate::cast::players::CastPlayer;
    use wiremock::matchers::{header, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RESOURCES: &str = include_str!("fixtures/companion-resources.xml");
    const PLAYING: &str = include_str!("fixtures/companion-timeline-playing.xml");

    fn identity() -> ControllerIdentity {
        ControllerIdentity {
            client_identifier: "controller-1".into(),
            platform: "macOS",
            device: "Mac",
        }
    }

    fn client(server: &MockServer, target: &str) -> CompanionClient {
        CompanionClient::new(
            Url::parse(&server.uri()).unwrap(),
            target.into(),
            identity(),
        )
    }

    #[tokio::test]
    async fn probe_accepts_the_named_player() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resources"))
            .and(header("X-Plex-Target-Client-Identifier", "ramus-client-id"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RESOURCES))
            .mount(&server)
            .await;
        let info = client(&server, "ramus-client-id").probe().await.unwrap();
        assert_eq!(info.machine_identifier, "ramus-client-id");
    }

    #[tokio::test]
    async fn probe_refuses_a_different_device() {
        let server = MockServer::start().await;
        Mock::given(path("/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RESOURCES))
            .mount(&server)
            .await;
        assert_eq!(
            client(&server, "someone-else").probe().await,
            Err(CompanionError::WrongPlayer)
        );
    }

    #[tokio::test]
    async fn probe_refuses_a_player_without_play_queues() {
        let server = MockServer::start().await;
        Mock::given(path("/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Player machineIdentifier="p" protocolCapabilities="timeline,playback"/></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        assert_eq!(
            client(&server, "p").probe().await,
            Err(CompanionError::Unsupported)
        );
    }

    #[tokio::test]
    async fn commands_carry_rising_command_ids_and_the_controller_headers() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r"^/player/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYING))
            .mount(&server)
            .await;
        let c = client(&server, "tv");
        c.send(&PlayerCommand::Play).await.unwrap();
        c.send(&PlayerCommand::Pause).await.unwrap();
        let _ = c.poll().await;

        let requests = server.received_requests().await.unwrap();
        let ids: Vec<String> = requests
            .iter()
            .map(|r| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == "commandID")
                    .unwrap()
                    .1
                    .into_owned()
            })
            .collect();
        assert_eq!(ids, ["1", "2", "3"]);
        for r in &requests {
            assert_eq!(
                r.headers.get("X-Plex-Client-Identifier").unwrap(),
                "controller-1"
            );
            assert_eq!(
                r.headers.get("X-Plex-Target-Client-Identifier").unwrap(),
                "tv"
            );
            assert_eq!(r.headers.get("X-Plex-Product").unwrap(), "ramus");
        }
    }

    #[tokio::test]
    async fn poll_parses_the_music_timeline() {
        let server = MockServer::start().await;
        Mock::given(path("/player/timeline/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYING))
            .mount(&server)
            .await;
        match client(&server, "tv").poll().await {
            PollOutcome::Timeline(t) => assert_eq!(t.play_queue_item_id, Some(5002)),
            other => panic!("expected a timeline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failing_poll_is_failed() {
        let server = MockServer::start().await;
        Mock::given(path("/player/timeline/poll"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        assert_eq!(client(&server, "tv").poll().await, PollOutcome::Failed);
    }

    #[tokio::test]
    async fn errors_never_carry_the_token() {
        let dead = CompanionClient::with_timeout(
            Url::parse("http://127.0.0.1:1").unwrap(),
            "tv".into(),
            identity(),
            Duration::from_secs(1),
        );
        match dead.play_media(&fixture_play_media()).await {
            Err(CompanionError::Unreachable(message)) => {
                assert!(!message.contains("transient-token"), "{message}")
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn find_reachable_skips_dead_connections() {
        let server = MockServer::start().await;
        Mock::given(path("/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RESOURCES))
            .mount(&server)
            .await;
        let player = CastPlayer {
            id: "ramus-client-id".into(),
            name: "ramus".into(),
            product: None,
            connections: vec!["http://127.0.0.1:1".into(), server.uri()],
        };
        let (url, _) = find_reachable(&player, &identity(), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(url, Url::parse(&server.uri()).unwrap());
    }
}
