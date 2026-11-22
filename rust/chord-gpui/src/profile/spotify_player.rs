//! Linux playback transport for the Spotify profile.
#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui::BackgroundExecutor;
use zbus::{
    Proxy,
    connection::Builder as ConnectionBuilder,
    proxy::{Builder as ProxyBuilder, CacheProperties},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue},
};

use crate::dispatch::profile::{
    presence::milliseconds,
    spotify_mpris::{
        DBUS_BUS_NAME, DBUS_OBJECT_PATH, DBUS_PLAYER_INTERFACE, extract_spotify_track_id,
    },
    spotify_widget::{ListenAlongCommand, SPOTIFY_DESYNC_THRESHOLD_MS},
};

const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

pub(super) async fn listen_along(
    command: ListenAlongCommand,
    requested_at: Instant,
    executor: BackgroundExecutor,
    cancelled: async_channel::Receiver<()>,
) -> Result<(), String> {
    let timeout = executor.timer(Duration::from_secs(12));
    tokio::select! {
        biased;
        _ = cancelled.recv() => Err("Listen Along was cancelled.".into()),
        _ = timeout => Err("Spotify did not respond in time.".into()),
        result = synchronize_player(command, requested_at, executor) => result,
    }
}

async fn synchronize_player(
    command: ListenAlongCommand,
    requested_at: Instant,
    executor: BackgroundExecutor,
) -> Result<(), String> {
    let connection = ConnectionBuilder::session()
        .map_err(|error| format!("Couldn't connect to the desktop session: {error}"))?
        .method_timeout(Duration::from_secs(3))
        .build()
        .await
        .map_err(|error| format!("Couldn't connect to the desktop session: {error}"))?;
    let player: Proxy<'_> = ProxyBuilder::new(&connection)
        .destination(DBUS_BUS_NAME)
        .and_then(|builder| builder.path(DBUS_OBJECT_PATH))
        .and_then(|builder| builder.interface(DBUS_PLAYER_INTERFACE))
        .map_err(|error| format!("Couldn't find the Spotify player: {error}"))?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(|error| format!("Couldn't find the Spotify player: {error}"))?;

    let can_control = player
        .get_property::<bool>("CanControl")
        .await
        .map_err(|_| "Open Spotify on this desktop and retry.".to_owned())?;
    if !can_control {
        return Err("Spotify is not accepting playback controls on this desktop.".into());
    }

    let mut current_track = find_track(&player, &command.track_id).await?;
    if current_track.is_none() {
        check_uri_support(&connection).await?;
        let uri = format!("spotify:track:{}", command.track_id);
        player
            .call::<_, _, ()>("OpenUri", &(uri.as_str(),))
            .await
            .map_err(|error| format!("Spotify couldn't open this track: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while current_track.is_none() && Instant::now() < deadline {
            executor.timer(Duration::from_millis(150)).await;
            current_track = find_track(&player, &command.track_id).await?;
        }
    }
    let path =
        current_track.ok_or_else(|| "Spotify didn't load the requested track.".to_owned())?;
    let position_ms = command
        .position_ms
        .saturating_add(milliseconds(requested_at.elapsed()));
    let position_us = i64::try_from(position_ms.saturating_mul(1_000))
        .map_err(|_| "The track position is outside Spotify's supported range.".to_owned())?;
    align_playback(&player, &path, &command.track_id, position_us).await
}

async fn check_uri_support(connection: &zbus::Connection) -> Result<(), String> {
    let root = Proxy::new(
        connection,
        DBUS_BUS_NAME,
        DBUS_OBJECT_PATH,
        "org.mpris.MediaPlayer2",
    )
    .await
    .map_err(|error| format!("Couldn't read Spotify player capabilities: {error}"))?;
    let schemes: Vec<String> = root
        .get_property("SupportedUriSchemes")
        .await
        .map_err(|error| format!("Couldn't read Spotify player capabilities: {error}"))?;
    if !schemes.iter().any(|scheme| scheme == "spotify") {
        return Err(
            "Open this track in Spotify and retry. This player cannot open tracks via MPRIS."
                .into(),
        );
    }
    Ok(())
}

async fn find_track(
    player: &Proxy<'_>,
    expected_id: &str,
) -> Result<Option<OwnedObjectPath>, String> {
    let metadata: HashMap<String, OwnedValue> = player
        .get_property("Metadata")
        .await
        .map_err(|error| format!("Couldn't read Spotify playback metadata: {error}"))?;
    Ok(matching_track(&metadata, expected_id).map(|path| path.clone().into()))
}

fn matching_track<'a>(
    metadata: &'a HashMap<String, OwnedValue>,
    expected_id: &str,
) -> Option<&'a ObjectPath<'a>> {
    let path = <&ObjectPath<'_>>::try_from(metadata.get("mpris:trackid")?).ok()?;
    if path.as_str() == NO_TRACK {
        return None;
    }
    let url = metadata
        .get("xesam:url")
        .and_then(|value| <&str>::try_from(value).ok());
    let id = extract_spotify_track_id(path.as_str())
        .or_else(|| url.and_then(extract_spotify_track_id))?;
    (id == expected_id).then_some(path)
}

async fn align_playback(
    player: &Proxy<'_>,
    track: &OwnedObjectPath,
    track_id: &str,
    position_us: i64,
) -> Result<(), String> {
    let can_seek = player
        .get_property::<bool>("CanSeek")
        .await
        .map_err(|error| format!("Couldn't read Spotify seek capability: {error}"))?;
    if !can_seek {
        return Err("Spotify cannot seek within this track.".into());
    }
    player
        .call::<_, _, ()>("SetPosition", &(ObjectPath::from(track), position_us))
        .await
        .map_err(|error| format!("Spotify couldn't seek to the host's position: {error}"))?;
    player
        .call::<_, _, ()>("Play", &())
        .await
        .map_err(|error| format!("Spotify couldn't start playback: {error}"))?;

    let current_position: i64 = player
        .get_property("Position")
        .await
        .map_err(|error| format!("Couldn't verify Spotify's position: {error}"))?;
    let status: String = player
        .get_property("PlaybackStatus")
        .await
        .map_err(|error| format!("Couldn't verify Spotify's playback status: {error}"))?;
    if find_track(player, track_id).await?.as_ref() != Some(track)
        || status != "Playing"
        || current_position.abs_diff(position_us) > SPOTIFY_DESYNC_THRESHOLD_MS * 1_000
    {
        return Err("Spotify did not synchronize playback with the host.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn no_track_never_matches_a_stale_spotify_uri() {
        let track_id = "4cOdK2wGLETKBW3PvgPWqT";
        let url = format!("spotify:track:{track_id}");
        let object_path =
            |path| OwnedValue::try_from(Value::from(ObjectPath::try_from(path).unwrap())).unwrap();
        let mut metadata = HashMap::from([
            ("mpris:trackid".into(), object_path(NO_TRACK)),
            (
                "xesam:url".into(),
                OwnedValue::try_from(Value::from(url.as_str())).unwrap(),
            ),
        ]);
        assert_eq!(matching_track(&metadata, track_id), None);
        metadata.insert(
            "mpris:trackid".into(),
            object_path("/com/spotify/track/2takcwOaAZWiXQijPHIx7B"),
        );
        assert_eq!(matching_track(&metadata, track_id), None);
        let path = "/com/spotify/track/4cOdK2wGLETKBW3PvgPWqT";
        metadata.insert("mpris:trackid".into(), object_path(path));
        assert_eq!(
            matching_track(&metadata, track_id).map(ObjectPath::as_str),
            Some(path)
        );
    }
}
