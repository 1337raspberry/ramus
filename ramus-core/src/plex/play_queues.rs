//! Server play-queue calls (see `cast::play_queue`) and the delegation
//! token a player is handed.

use serde::Deserialize;

use super::client::{PlexClient, PlexClientError};
use crate::cast::play_queue::{PlayQueue, SLICE_WINDOW};

/// Slices fetched at most for one queue (about 10,000 items): a stop for a
/// server that never reports the end.
const MAX_SLICES: usize = 50;

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(rename = "MediaContainer")]
    container: TokenContainer,
}

#[derive(Deserialize)]
struct TokenContainer {
    token: Option<String>,
}

impl PlexClient {
    /// Creates an audio play queue from a library `uri` (see
    /// [`super::client::build_library_uri`]), selecting the item for `key`.
    pub async fn create_play_queue(
        &self,
        uri: &str,
        key: &str,
    ) -> Result<PlayQueue, PlexClientError> {
        let body = self
            .post(
                "playQueues",
                &[
                    ("type", "audio"),
                    ("uri", uri),
                    ("key", key),
                    ("shuffle", "0"),
                    ("repeat", "0"),
                    ("continuous", "0"),
                ],
            )
            .await?;
        PlayQueue::decode(&body).map_err(|_| PlexClientError::InvalidResponse)
    }

    /// The whole play queue: the window around its selection, then following
    /// slices until the end.
    pub async fn play_queue(&self, id: i64) -> Result<PlayQueue, PlexClientError> {
        let path = format!("playQueues/{id}");
        let window = SLICE_WINDOW.to_string();
        let body = self.get(&path, &[("window", window.as_str())]).await?;
        let mut queue = PlayQueue::decode(&body).map_err(|_| PlexClientError::InvalidResponse)?;
        for _ in 0..MAX_SLICES {
            let Some(center) = queue.next_slice_center() else {
                break;
            };
            let center = center.to_string();
            let body = self
                .get(
                    &path,
                    &[
                        ("center", center.as_str()),
                        ("includeBefore", "0"),
                        ("window", window.as_str()),
                    ],
                )
                .await?;
            let slice = PlayQueue::decode(&body).map_err(|_| PlexClientError::InvalidResponse)?;
            queue.append_slice(slice);
        }
        Ok(queue)
    }

    /// Adds the tracks addressed by `uri`: at the end, or straight after the
    /// current item when `next`.
    pub async fn add_to_play_queue(
        &self,
        id: i64,
        uri: &str,
        next: bool,
    ) -> Result<(), PlexClientError> {
        let path = format!("playQueues/{id}");
        if next {
            self.put(&path, &[("uri", uri), ("next", "1")]).await
        } else {
            self.put(&path, &[("uri", uri)]).await
        }
    }

    pub async fn remove_play_queue_item(
        &self,
        id: i64,
        item_id: i64,
    ) -> Result<(), PlexClientError> {
        self.delete(&format!("playQueues/{id}/items/{item_id}"), &[])
            .await
    }

    /// Moves an item after `after`, or to the top when `None`.
    pub async fn move_play_queue_item(
        &self,
        id: i64,
        item_id: i64,
        after: Option<i64>,
    ) -> Result<(), PlexClientError> {
        let path = format!("playQueues/{id}/items/{item_id}/move");
        match after {
            Some(after) => {
                let after = after.to_string();
                self.put(&path, &[("after", after.as_str())]).await
            }
            None => self.put(&path, &[]).await,
        }
    }

    /// A transient token for handing to a player, so the long-lived token
    /// never leaves this device.
    pub async fn delegation_token(&self) -> Result<String, PlexClientError> {
        let body = self
            .get(
                "security/token",
                &[("type", "delegation"), ("scope", "all")],
            )
            .await?;
        let reply: TokenResponse =
            serde_json::from_slice(&body).map_err(|_| PlexClientError::InvalidResponse)?;
        reply
            .container
            .token
            .filter(|t| !t.is_empty())
            .ok_or(PlexClientError::InvalidResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ALBUM: &str = include_str!("../cast/fixtures/companion-playqueue-album.json");
    const LONG: &str = include_str!("../cast/fixtures/companion-playqueue-long.json");
    const LONG_SLICE: &str = include_str!("../cast/fixtures/companion-playqueue-long-slice.json");

    fn client(server: &MockServer) -> PlexClient {
        let client = PlexClient::new("test-client-id".into());
        client.set_server_url(Some(Url::parse(&server.uri()).unwrap()));
        client.set_token(Some("test-token".into()));
        client
    }

    #[tokio::test]
    async fn create_posts_the_uri_and_selection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/playQueues"))
            .and(query_param("type", "audio"))
            .and(query_param(
                "uri",
                "server://mid/com.plexapp.plugins.library/library/metadata/1,2",
            ))
            .and(query_param("key", "/library/metadata/2"))
            .and(query_param("shuffle", "0"))
            .and(query_param("repeat", "0"))
            .and(query_param("continuous", "0"))
            .and(header("X-Plex-Token", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ALBUM))
            .expect(1)
            .mount(&server)
            .await;
        let q = client(&server)
            .create_play_queue(
                "server://mid/com.plexapp.plugins.library/library/metadata/1,2",
                "/library/metadata/2",
            )
            .await
            .unwrap();
        assert_eq!(q.id, 9001);
    }

    #[tokio::test]
    async fn play_queue_pages_until_the_total() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/playQueues/9002"))
            .and(query_param("center", "6003"))
            .and(query_param("includeBefore", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LONG_SLICE))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/playQueues/9002"))
            .and(query_param("window", "200"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LONG))
            .mount(&server)
            .await;
        let q = client(&server).play_queue(9002).await.unwrap();
        let ids: Vec<i64> = q.items.iter().map(|i| i.item_id).collect();
        assert_eq!(ids, [6001, 6002, 6003, 6004, 6005]);
    }

    #[tokio::test]
    async fn add_sends_next_only_for_play_next() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/playQueues/9001"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let c = client(&server);
        c.add_to_play_queue(9001, "server://mid/x/library/metadata/7", true)
            .await
            .unwrap();
        c.add_to_play_queue(9001, "server://mid/x/library/metadata/8", false)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let next: Vec<Option<String>> = requests
            .iter()
            .map(|r| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == "next")
                    .map(|(_, v)| v.into_owned())
            })
            .collect();
        assert_eq!(next, [Some("1".to_string()), None]);
    }

    #[tokio::test]
    async fn remove_deletes_the_item() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/playQueues/9001/items/5002"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .remove_play_queue_item(9001, 5002)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn move_names_the_item_to_follow_or_none_for_the_top() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/playQueues/9001/items/5003/move"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let c = client(&server);
        c.move_play_queue_item(9001, 5003, Some(5001))
            .await
            .unwrap();
        c.move_play_queue_item(9001, 5003, None).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let after: Vec<Option<String>> = requests
            .iter()
            .map(|r| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == "after")
                    .map(|(_, v)| v.into_owned())
            })
            .collect();
        assert_eq!(after, [Some("5001".to_string()), None]);
    }

    #[tokio::test]
    async fn delegation_token_reads_the_container_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/security/token"))
            .and(query_param("type", "delegation"))
            .and(query_param("scope", "all"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"MediaContainer":{"size":0,"token":"transient-abc"}}"#),
            )
            .mount(&server)
            .await;
        assert_eq!(
            client(&server).delegation_token().await.unwrap(),
            "transient-abc"
        );
    }

    #[tokio::test]
    async fn a_reply_without_a_token_is_invalid() {
        let server = MockServer::start().await;
        Mock::given(path("/security/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"MediaContainer":{"size":0}}"#),
            )
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server).delegation_token().await,
            Err(PlexClientError::InvalidResponse)
        ));
    }
}
