//! Plex players on the account, as plex.tv lists them.

use crate::plex::models::PlexResourceResponse;

/// A Plex player registered on the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastPlayer {
    /// The player's client identifier: what `X-Plex-Target-Client-Identifier`
    /// names and what its `/resources` reply must echo.
    pub id: String,
    pub name: String,
    pub product: Option<String>,
    /// Addresses to try, local ones first. Relay connections are left out:
    /// a player's Companion port isn't reachable through the relay.
    pub connections: Vec<String>,
}

/// The players in a plex.tv resource list: entries that provide `player`,
/// other than ramus itself, with at least one direct connection.
pub fn players_from_resources(
    resources: Vec<PlexResourceResponse>,
    own_client_identifier: &str,
) -> Vec<CastPlayer> {
    resources
        .into_iter()
        .filter(|r| r.provides.split(',').any(|p| p.trim() == "player"))
        .filter(|r| r.client_identifier != own_client_identifier)
        .filter_map(|r| {
            let mut connections: Vec<_> = r
                .connections
                .unwrap_or_default()
                .into_iter()
                .filter(|c| !c.relay.unwrap_or(false))
                .collect();
            // Stable: plex.tv's order is kept within each group.
            connections.sort_by_key(|c| !c.local.unwrap_or(false));
            let connections: Vec<String> = connections.into_iter().map(|c| c.uri).collect();
            if connections.is_empty() {
                return None;
            }
            Some(CastPlayer {
                id: r.client_identifier,
                name: r.name,
                product: r.product,
                connections,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(json: serde_json::Value) -> Vec<PlexResourceResponse> {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn keeps_players_and_drops_servers_itself_and_dead_entries() {
        let list = resources(serde_json::json!([
            {"name": "Home Server", "product": "Plex Media Server", "provides": "server",
             "clientIdentifier": "srv", "accessToken": "t", "owned": true,
             "connections": [{"uri": "https://a.plex.direct:32400", "local": true}]},
            {"name": "Living Room", "product": "ramusTV", "provides": "client,player,pubsub-player",
             "clientIdentifier": "tv",
             "connections": [{"uri": "http://10.0.0.9:32500", "local": true}]},
            {"name": "Old Browser", "product": "Plex Web", "provides": "client,player,pubsub-player",
             "clientIdentifier": "web", "connections": []},
            {"name": "This Mac", "product": "ramus", "provides": "client,player",
             "clientIdentifier": "me",
             "connections": [{"uri": "http://10.0.0.2:1", "local": true}]}
        ]));
        let players = players_from_resources(list, "me");
        assert_eq!(
            players,
            vec![CastPlayer {
                id: "tv".into(),
                name: "Living Room".into(),
                product: Some("ramusTV".into()),
                connections: vec!["http://10.0.0.9:32500".into()],
            }]
        );
    }

    #[test]
    fn orders_local_connections_first_and_drops_relays() {
        let list = resources(serde_json::json!([
            {"name": "Kitchen", "provides": "player", "clientIdentifier": "pi",
             "connections": [
                {"uri": "https://relay.plex.direct:8443", "local": false, "relay": true},
                {"uri": "http://82.1.2.3:32500", "local": false},
                {"uri": "http://10.0.0.7:32500", "local": true}
             ]}
        ]));
        let players = players_from_resources(list, "me");
        assert_eq!(
            players[0].connections,
            vec![
                "http://10.0.0.7:32500".to_string(),
                "http://82.1.2.3:32500".to_string()
            ]
        );
        assert_eq!(players[0].product, None);
    }

    #[test]
    fn pubsub_player_alone_is_not_a_player() {
        let list = resources(serde_json::json!([
            {"name": "Odd", "provides": "client,pubsub-player", "clientIdentifier": "x",
             "connections": [{"uri": "http://10.0.0.8:32500", "local": true}]}
        ]));
        assert!(players_from_resources(list, "me").is_empty());
    }
}
