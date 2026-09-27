//! Listing players, starting a cast, and the ways a cast ends.

use std::time::Duration;

use tauri::{AppHandle, Manager};
use url::Url;

use ramus_core::cast::companion::{
    find_reachable, CompanionClient, CompanionError, PlayMedia, PlayerCommand, PollOutcome,
    ServerAddress,
};
use ramus_core::cast::play_queue::{upload_plan, PlayQueue};
use ramus_core::cast::players::{merge_players, players_from_devices, players_from_resources};
use ramus_core::cast::record::{self, CastRecord};
use ramus_core::cast::session::{CastQueue, CastState, Link, LocalResume};
use ramus_core::models::{PlaybackStatus, Track};
use ramus_core::playback::media_keys::MediaKeyHandler;
use ramus_core::plex::client::{build_library_uri, PlexClientError};
use ramus_core::plex::token_store::{TokenKey, TokenStore};

use crate::events::{
    emit_playback_position, emit_playback_state, PlaybackPositionPayload, PlaybackStatePayload,
};
use crate::state::AppState;

use super::poll::{self, Ending};
use super::{ActiveCast, CastPlayerRef, ListedPlayer};

/// How long a player in the picker gets to answer its probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long handing back waits on the player, for its position and for the
/// stop; a LAN player answers in milliseconds.
const HAND_BACK_WAIT: Duration = Duration::from_secs(1);

/// What a cast starts from: the local queue, or the cast being replaced.
pub(crate) struct Source {
    pub tracks: Vec<Track>,
    pub index: usize,
    pub position: f64,
    pub paused: bool,
}

fn unreachable_message(name: &str) -> String {
    let hint = if cfg!(target_os = "ios") {
        ", and that ramus has Local Network access in Settings"
    } else {
        ""
    };
    format!("Can't reach {name}. Check it's on and on the same network{hint}")
}

fn probe_message(name: &str, error: &CompanionError) -> String {
    match error {
        CompanionError::WrongPlayer => {
            format!("Another device answers at {name}'s address. Refresh the list and try again")
        }
        CompanionError::Unsupported => format!("{name} can't play a Plex queue"),
        _ => unreachable_message(name),
    }
}

pub(crate) fn server_message(error: PlexClientError) -> String {
    format!("The Plex server refused the change ({error})")
}

/// The players on the account that answer a probe now, in plex.tv's order.
/// Registrations that don't answer (players switched off, uninstalled, or
/// long gone) are left out.
pub async fn list_players(state: &AppState) -> Result<Vec<CastPlayerRef>, String> {
    let auth_token = TokenStore::new()
        .ok()
        .and_then(|store| store.read(TokenKey::AuthToken))
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "Sign in with Plex to cast to other players".to_string())?;
    let resources = state
        .client
        .fetch_resources(&auth_token)
        .await
        .map_err(|_| "Couldn't reach plex.tv".to_string())?;
    let own = &state.client.client_identifier;
    // Some players publish their address to the device list only; the
    // resource list leaves them out.
    let registered = match state.client.fetch_devices(&auth_token).await {
        Ok(xml) => players_from_devices(&xml, own).unwrap_or_else(|_| {
            log::warn!("cast: plex.tv device list didn't parse");
            Vec::new()
        }),
        Err(e) => {
            log::warn!("cast: plex.tv device list unavailable: {e}");
            Vec::new()
        }
    };
    let players = merge_players(players_from_resources(resources, own), registered);

    let identity = super::identity(&state.client);
    let mut probes = tokio::task::JoinSet::new();
    for (order, player) in players.into_iter().enumerate() {
        let identity = identity.clone();
        probes.spawn(async move {
            let uri = find_reachable(&player, &identity, PROBE_TIMEOUT)
                .await
                .map(|(uri, _)| uri);
            (order, player, uri)
        });
    }
    let mut found = Vec::new();
    while let Some(joined) = probes.join_next().await {
        if let Ok((order, player, Some(uri))) = joined {
            found.push((order, player, uri));
        }
    }
    found.sort_by_key(|(order, ..)| *order);

    let listed: Vec<ListedPlayer> = found
        .into_iter()
        .map(|(_, p, uri)| ListedPlayer {
            player: CastPlayerRef {
                id: p.id,
                name: p.name,
                product: p.product,
            },
            uri,
        })
        .collect();
    let players = listed.iter().map(|l| l.player.clone()).collect();
    state.cast.set_listed(listed);
    Ok(players)
}

pub(crate) fn server_address(state: &AppState) -> Result<ServerAddress, String> {
    let machine_identifier = crate::commands::playlists::get_machine_identifier()?;
    let url = state
        .client
        .server_url()
        .ok_or("Not connected to a Plex server")?;
    ServerAddress::from_url(&machine_identifier, &url)
        .ok_or_else(|| "The Plex server's address can't be passed to a player".to_string())
}

fn local_source(state: &AppState) -> Source {
    let ps = state.player.state();
    Source {
        tracks: ps.queue,
        index: ps.queue_index,
        position: state.player.position(),
        paused: ps.status != PlaybackStatus::Playing,
    }
}

fn mirror_source(cast: &ActiveCast) -> Source {
    let (index, position, paused) = match cast.state.local_resume(&cast.queue) {
        Some(LocalResume::Play { index, position }) => (index, position, false),
        Some(LocalResume::Paused { index, position }) => (index, position, true),
        None => (0, 0.0, true),
    };
    Source {
        tracks: cast.queue.tracks(),
        index,
        position,
        paused,
    }
}

/// Sends `source` to the server as a new play queue and starts it on the
/// player, which is handed a delegation token (never the long-lived one).
pub(crate) async fn send_queue(
    state: &AppState,
    client: &CompanionClient,
    server: &ServerAddress,
    source: &Source,
) -> Result<PlayQueue, String> {
    let keys: Vec<String> = source.tracks.iter().map(|t| t.rating_key.clone()).collect();
    let plan = upload_plan(&keys, source.index).ok_or("Nothing to play")?;
    let mid = &server.machine_identifier;
    let token = state
        .client
        .delegation_token()
        .await
        .map_err(|e| format!("The Plex server didn't issue a token for the player ({e})"))?;
    let key = format!("/library/metadata/{}", plan.key);
    let queue = state
        .client
        .create_play_queue(&build_library_uri(mid, &plan.first), &key)
        .await
        .map_err(server_message)?;
    for chunk in &plan.rest {
        state
            .client
            .add_to_play_queue(queue.id, &build_library_uri(mid, chunk), false)
            .await
            .map_err(server_message)?;
    }
    let play = PlayMedia {
        rating_key: plan.key,
        offset_ms: (source.position.max(0.0) * 1000.0) as u64,
        paused: source.paused,
        play_queue_id: queue.id,
        server: server.clone(),
        token,
    };
    client
        .play_media(&play)
        .await
        .map_err(|e| format!("The player didn't start playback ({e})"))?;
    Ok(queue)
}

/// Mirrors a freshly sent play queue and remembers the cast for relaunch.
/// Until the player reports the queue, the cast shows (and would resume
/// from) the point `source` was sent from.
pub(crate) async fn adopt_queue(
    app: &AppHandle,
    state: &AppState,
    cast: &mut ActiveCast,
    sent: &PlayQueue,
    source: &Source,
) {
    // The creation reply holds one window of the queue; the mirror needs all of it.
    let selected = match state.client.play_queue(sent.id).await {
        Ok(full) => {
            poll::set_queue(state, cast, &full);
            full.selected_item_id.or(sent.selected_item_id)
        }
        Err(e) => {
            log::warn!("cast: play queue fetch failed, using the creation reply: {e}");
            poll::set_queue(state, cast, sent);
            sent.selected_item_id
        }
    };
    let pending = selected
        .and_then(|item| cast.queue.index_of_item(item))
        .map(|index| {
            let position = source.position.max(0.0);
            if source.paused {
                LocalResume::Paused { index, position }
            } else {
                LocalResume::Play { index, position }
            }
        });
    cast.state.restart(pending);
    let record = CastRecord {
        player_id: cast.player.id.clone(),
        player_name: cast.player.name.clone(),
        product: cast.player.product.clone(),
        uri: cast.client.base().to_string(),
        play_queue_id: cast.queue.play_queue_id,
    };
    if let Err(e) = record::save(&record) {
        log::warn!("cast: could not remember the cast: {e}");
    }
    super::emit_status(app, state, None);
}

/// Moves playback to the player `player_id` from the last listing. The
/// queue comes from local playback, or from the cast it replaces.
pub async fn start(app: &AppHandle, state: &AppState, player_id: &str) -> Result<(), String> {
    if state.cast.view().player.is_some_and(|p| p.id == player_id) {
        return Ok(());
    }
    let listed = match state.cast.listed(player_id) {
        Some(listed) => listed,
        None => {
            list_players(state).await?;
            state
                .cast
                .listed(player_id)
                .ok_or_else(|| unreachable_message("that player"))?
        }
    };
    let name = listed.player.name.clone();
    let uri = listed.uri.clone();
    let client = CompanionClient::new(
        uri,
        listed.player.id.clone(),
        super::identity(&state.client),
    );
    client.probe().await.map_err(|e| probe_message(&name, &e))?;
    let server = server_address(state)?;

    let mut guard = state.cast.session.lock().await;
    let previous = guard.take();
    let source = match &previous {
        Some(prev) => mirror_source(prev),
        None => local_source(state),
    };
    if source.tracks.is_empty() {
        *guard = previous;
        return Err("Play something first, then pick a player".into());
    }
    let local_was_playing = previous.is_none() && state.player.status() == PlaybackStatus::Playing;
    if local_was_playing {
        state.player.pause();
    }

    let queue = match send_queue(state, &client, &server, &source).await {
        Ok(queue) => queue,
        Err(e) => {
            if local_was_playing {
                state.player.resume();
            }
            *guard = previous;
            return Err(e);
        }
    };
    if let Some(prev) = &previous {
        // Best effort: the replaced player may already be gone.
        let _ = prev.client.send(&PlayerCommand::Stop).await;
    }

    let generation = state.cast.activate(listed.player.clone());
    if previous.is_none() {
        crate::commands::playback::stop_local_playback(state);
    }
    let mut cast = ActiveCast {
        player: listed.player,
        client,
        server,
        queue: CastQueue::default(),
        state: CastState::new(),
    };
    adopt_queue(app, state, &mut cast, &queue, &source).await;
    *guard = Some(cast);
    poll::settle(app, state, &mut guard).await;
    drop(guard);
    poll::spawn(app.clone(), generation);
    Ok(())
}

/// Takes the cast's track off the OS now-playing card.
fn clear_os_controls(state: &AppState) {
    if let Some(ref mc) = *state.media_controls.lock() {
        mc.clear();
    }
}

pub(crate) fn emit_stopped(app: &AppHandle, state: &AppState) {
    clear_os_controls(state);
    emit_playback_state(
        app,
        PlaybackStatePayload {
            status: "stopped".to_string(),
            current_track: None,
            queue_index: 0,
        },
    );
}

/// Loads the cast's queue into the local player where the cast left off.
pub(crate) fn resume_locally(
    app: &AppHandle,
    state: &AppState,
    tracks: Vec<Track>,
    resume: LocalResume,
) {
    match resume {
        LocalResume::Play { index, position } => {
            let resume_at = (position > 0.5).then_some(position);
            crate::commands::playback::start_local_queue(app, state, tracks, index, resume_at);
        }
        LocalResume::Paused { index, position } => {
            // Like a restored queue: no OS card until playback starts.
            clear_os_controls(state);
            state.player.restore_queue(tracks, index, position);
            let ps = state.player.state();
            emit_playback_state(
                app,
                PlaybackStatePayload {
                    status: "paused".to_string(),
                    current_track: ps.current_track.clone(),
                    queue_index: ps.queue_index,
                },
            );
            emit_playback_position(
                app,
                PlaybackPositionPayload {
                    position: state.player.position(),
                    duration: state.player.duration(),
                },
            );
            crate::queue_persist::save_soon(app);
        }
    }
}

/// The cast ended on the player's side: another app, or another ramus, now
/// drives it, or it never started the queue. Playback comes back here,
/// paused where this cast last saw it.
pub(crate) fn finish_takeover(app: &AppHandle, state: &AppState, cast: ActiveCast, ending: Ending) {
    let resume = cast
        .state
        .local_resume(&cast.queue)
        .map(LocalResume::paused);
    let tracks = cast.queue.tracks();
    state.cast.deactivate();
    let name = &cast.player.name;
    let notice = match ending {
        Ending::TakenOver => format!("{name} is playing something else now"),
        Ending::NotStarted => {
            format!("{name} didn't start playing. Check it can reach your Plex server")
        }
    };
    super::emit_status(app, state, Some(notice));
    match resume {
        Some(resume) => resume_locally(app, state, tracks, resume),
        None => emit_stopped(app, state),
    }
}

/// "This device": the player stops and playback continues here from where
/// it was: playing if it was playing, else paused. A player that has
/// stopped answering is left alone, and the last position seen is used.
pub async fn hand_back(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let mut guard = state.cast.session.lock().await;
    let Some(mut cast) = guard.take() else {
        return Ok(());
    };
    if cast.state.answering() {
        if let Ok(outcome) = tokio::time::timeout(HAND_BACK_WAIT, cast.client.poll()).await {
            let _ = cast.state.apply(&outcome, &cast.queue);
        }
    }
    let resume = cast.state.local_resume(&cast.queue);
    let tracks = cast.queue.tracks();
    state.cast.deactivate();
    drop(guard);
    if cast.state.link() == Link::Connected {
        let _ = tokio::time::timeout(HAND_BACK_WAIT, cast.client.send(&PlayerCommand::Stop)).await;
    }
    super::emit_status(app, state, None);
    match resume {
        Some(resume) => resume_locally(app, state, tracks, resume),
        None => emit_stopped(app, state),
    }
    Ok(())
}

/// Clear Queue while casting: the player stops and the cast ends, leaving
/// nothing queued here either. The surfaces that hold the cast button close
/// with an empty queue, so a cast can't be left running out of reach.
pub async fn end_on_clear(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.take() else {
        return Ok(());
    };
    state.cast.deactivate();
    drop(guard);
    let _ = cast.client.send(&PlayerCommand::Stop).await;
    super::emit_status(app, state, None);
    crate::queue_persist::forget();
    emit_stopped(app, state);
    Ok(())
}

/// Forgets the cast without touching the player: on sign-out, and when
/// onboarding connects a server.
pub async fn drop_silently(app: &AppHandle, state: &AppState) {
    let mut guard = state.cast.session.lock().await;
    let was_casting = guard.take().is_some();
    drop(guard);
    if was_casting {
        state.cast.deactivate();
        super::emit_status(app, state, None);
    } else {
        record::clear();
    }
}

/// Picks up a cast that was active when ramus last quit, provided the player
/// still plays the queue this cast sent. Otherwise the cast is forgotten and
/// the ordinary local restore (the cast's queue, paused) stands.
pub async fn resume_on_launch(app: AppHandle) {
    let Some(record) = record::load() else { return };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match reconnect(&app, &state, &record).await {
        Ok(true) => log::info!("cast: reconnected to {}", record.player_name),
        Ok(false) => {}
        Err(reason) => {
            log::info!(
                "cast: not resuming the cast to {}: {reason}",
                record.player_name
            );
            record::clear();
        }
    }
}

async fn reconnect(app: &AppHandle, state: &AppState, record: &CastRecord) -> Result<bool, String> {
    let uri = Url::parse(&record.uri).map_err(|e| e.to_string())?;
    let client = CompanionClient::new(
        uri,
        record.player_id.clone(),
        super::identity(&state.client),
    );
    client.probe().await.map_err(|e| e.to_string())?;
    let PollOutcome::Timeline(timeline) = client.poll().await else {
        return Err("the player reports no music".into());
    };
    if timeline.play_queue_id != Some(record.play_queue_id) {
        return Err("the player moved on to another queue".into());
    }
    let server = server_address(state)?;
    let queue = state
        .client
        .play_queue(record.play_queue_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut guard = state.cast.session.lock().await;
    // A cast picked while this was connecting wins.
    if guard.is_some() {
        return Ok(false);
    }
    let player = CastPlayerRef {
        id: record.player_id.clone(),
        name: record.player_name.clone(),
        product: record.product.clone(),
    };
    let generation = state.cast.activate(player.clone());
    // The restored local queue was never handed to mpv, so this stop is
    // silent; its events are ignored now that the cast is active anyway.
    state.player.stop();
    let mut cast = ActiveCast {
        player,
        client,
        server,
        queue: CastQueue::default(),
        state: CastState::new(),
    };
    poll::set_queue(state, &mut cast, &queue);
    *guard = Some(cast);
    poll::settle(app, state, &mut guard).await;
    drop(guard);
    super::emit_status(app, state, None);
    poll::spawn(app.clone(), generation);
    Ok(true)
}
