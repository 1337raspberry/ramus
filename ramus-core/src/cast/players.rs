//! Plex players on the account, as plex.tv lists them.

use quick_xml::events::Event;
use quick_xml::Reader;
use url::{Host, Url};

use super::timeline::{attributes, get, XmlError};
use crate::plex::models::PlexResourceResponse;

/// A Plex player registered on the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastPlayer {
    /// The player's client identifier: what `X-Plex-Target-Client-Identifier`
    /// names and what its `/resources` reply must echo.
    pub id: String,
    pub name: String,
    pub product: Option<String>,
    /// Addresses to try, local ones first. Relay connections are left out
    /// (a player's Companion port isn't reachable through the relay), and so
    /// are plain-HTTP addresses that aren't private (see
    /// [`is_private_address`]).
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
        .filter(|r| provides_player(&r.provides))
        .filter(|r| r.client_identifier != own_client_identifier)
        .filter_map(|r| {
            let mut connections: Vec<_> = r
                .connections
                .unwrap_or_default()
                .into_iter()
                .filter(|c| !c.relay.unwrap_or(false) && usable(&c.uri))
                .collect();
            // Stable: plex.tv's order is kept within each group. Its `local`
            // flag (same public address as this device) ranks first, then
            // private addresses (a VPN overlay, say), then the rest.
            connections.sort_by_key(|c| (!c.local.unwrap_or(false), !is_private_address(&c.uri)));
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

/// Whether `uri` names an address plain HTTP can reach without crossing the
/// public internet: a private, shared (100.64.0.0/10, used by VPN overlays)
/// or link-local IPv4 address, a unique-local or link-local IPv6 address, or
/// a `.local` name.
pub fn is_private_address(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    match url.host() {
        Some(Host::Ipv4(ip)) => {
            let [a, b, ..] = ip.octets();
            ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        Some(Host::Ipv6(ip)) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Some(Host::Domain(name)) => name.ends_with(".local"),
        None => false,
    }
}

/// Commands carry a server token, so plain HTTP is used only where it stays
/// off the internet.
fn usable(uri: &str) -> bool {
    uri.starts_with("https://") || is_private_address(uri)
}

fn provides_player(provides: &str) -> bool {
    provides.split(',').any(|p| p.trim() == "player")
}

/// The players in plex.tv's device list (`/devices.xml`). It also holds
/// players that publish their address there but are missing from the
/// resource list. The per-device tokens it carries are never read.
pub fn players_from_devices(
    xml: &str,
    own_client_identifier: &str,
) -> Result<Vec<CastPlayer>, XmlError> {
    let mut reader = Reader::from_str(xml);
    let mut players = Vec::new();
    // The `Device` being read, when it is a player.
    let mut current: Option<CastPlayer> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"Device" => {
                let attrs = attributes(&e)?;
                let id = get(&attrs, "clientIdentifier").unwrap_or_default();
                current = (provides_player(get(&attrs, "provides").unwrap_or_default())
                    && !id.is_empty()
                    && id != own_client_identifier)
                    .then(|| CastPlayer {
                        id: id.to_string(),
                        name: get(&attrs, "name").unwrap_or(id).to_string(),
                        product: get(&attrs, "product")
                            .filter(|p| !p.is_empty())
                            .map(str::to_string),
                        connections: Vec::new(),
                    });
            }
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"Connection" => {
                let Some(player) = current.as_mut() else {
                    continue;
                };
                let attrs = attributes(&e)?;
                if let Some(uri) = get(&attrs, "uri").filter(|u| usable(u)) {
                    if !player.connections.iter().any(|c| c == uri) {
                        player.connections.push(uri.to_string());
                    }
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"Device" => {
                if let Some(mut player) = current.take() {
                    // Stable: the listed order is kept within each group.
                    player.connections.sort_by_key(|c| !is_private_address(c));
                    if !player.connections.is_empty() {
                        players.push(player);
                    }
                }
            }
            Ok(Event::Eof) => return Ok(players),
            Err(_) => return Err(XmlError::Malformed),
            _ => {}
        }
    }
}

/// `primary` in order, then the players of `extra` it lacks. A player in
/// both keeps its entry and gains any addresses only `extra` knows.
pub fn merge_players(mut primary: Vec<CastPlayer>, extra: Vec<CastPlayer>) -> Vec<CastPlayer> {
    for player in extra {
        match primary.iter_mut().find(|p| p.id == player.id) {
            Some(known) => {
                for uri in player.connections {
                    if !known.connections.contains(&uri) {
                        known.connections.push(uri);
                    }
                }
            }
            None => primary.push(player),
        }
    }
    primary
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
            vec!["http://10.0.0.7:32500".to_string()]
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

    #[test]
    fn a_remote_connection_needs_https() {
        // Commands carry a server token, so they only cross the internet
        // encrypted; a plain-HTTP address is kept only on the local network.
        let list = resources(serde_json::json!([
            {"name": "Cabin", "provides": "player", "clientIdentifier": "c",
             "connections": [
                {"uri": "http://82.1.2.3:32500", "local": false},
                {"uri": "https://82-1-2-3.abc.plex.direct:32500", "local": false},
                {"uri": "http://10.0.0.7:32500"}
             ]}
        ]));
        assert_eq!(
            players_from_resources(list, "me")[0].connections,
            vec![
                "http://10.0.0.7:32500".to_string(),
                "https://82-1-2-3.abc.plex.direct:32500".to_string()
            ]
        );
    }

    #[test]
    fn plain_http_follows_the_address_not_the_local_flag() {
        // plex.tv's `local` means "same public IP as the asker"; whether
        // plain HTTP stays off the internet depends on the address itself.
        let list = resources(serde_json::json!([
            {"name": "Tower", "provides": "player", "clientIdentifier": "t",
             "connections": [
                {"uri": "http://100.118.73.36:32500", "local": false},
                {"uri": "http://82.1.2.3:32500", "local": true},
                {"uri": "http://192.168.0.128:32500", "local": true}
             ]}
        ]));
        assert_eq!(
            players_from_resources(list, "me")[0].connections,
            vec![
                "http://192.168.0.128:32500".to_string(),
                "http://100.118.73.36:32500".to_string()
            ]
        );
    }

    #[test]
    fn private_addresses() {
        for lan in [
            "http://10.1.2.3:1",
            "http://172.16.0.1:1",
            "http://172.31.255.255:1",
            "http://192.168.1.1:1",
            "http://100.64.0.1:1",
            "http://169.254.1.1:1",
            "http://[fd12::1]:1",
            "http://[fe80::1]:1",
            "http://appletv.local:1",
        ] {
            assert!(is_private_address(lan), "{lan}");
        }
        for wan in [
            "http://172.32.0.1:1",
            "http://100.128.0.1:1",
            "http://8.8.8.8:1",
            "http://[2001:db8::1]:1",
            "http://example.com:1",
            "not a url",
        ] {
            assert!(!is_private_address(wan), "{wan}");
        }
    }

    const DEVICES: &str = r#"<MediaContainer publicAddress="82.30.73.36">
<Device name="Living Room" publicAddress="82.30.73.36" product="ramusTV" platform="tvOS" provides="client,player,pubsub-player" clientIdentifier="tv-1" token="device-secret" createdAt="1" lastSeenAt="2">
<Connection uri="http://192.168.0.153:32500"/>
<Connection uri="http://82.30.73.36:32500"/>
</Device>
<Device name="Mac" product="ramus" provides="" clientIdentifier="desk" token="device-secret"/>
<Device name="Apple TV" product="Plex for Apple TV" provides="client" clientIdentifier="atv"><Connection uri="http://192.168.0.90:32500"/></Device>
<Device name="Dash" product="Plex Dash" provides="client,player,pubsub-player" clientIdentifier="dash"/>
<Device name="This Mac" product="ramus" provides="client,player" clientIdentifier="me"><Connection uri="http://192.168.0.2:1"/></Device>
</MediaContainer>"#;

    #[test]
    fn devices_xml_lists_registered_players() {
        assert_eq!(
            players_from_devices(DEVICES, "me").unwrap(),
            vec![CastPlayer {
                id: "tv-1".into(),
                name: "Living Room".into(),
                product: Some("ramusTV".into()),
                connections: vec!["http://192.168.0.153:32500".into()],
            }]
        );
        assert!(players_from_devices("<MediaContainer><Device", "me").is_err());
    }

    fn player(id: &str, connections: &[&str]) -> CastPlayer {
        CastPlayer {
            id: id.into(),
            name: id.into(),
            product: None,
            connections: connections.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn devices_add_players_the_resource_list_lacks() {
        let merged = merge_players(
            vec![player("tower", &["http://192.168.0.128:32500"])],
            vec![
                player(
                    "tower",
                    &["http://192.168.0.128:32500", "http://10.0.0.5:32500"],
                ),
                player("tv", &["http://192.168.0.153:32500"]),
            ],
        );
        assert_eq!(
            merged,
            vec![
                player(
                    "tower",
                    &["http://192.168.0.128:32500", "http://10.0.0.5:32500"]
                ),
                player("tv", &["http://192.168.0.153:32500"]),
            ]
        );
    }
}
