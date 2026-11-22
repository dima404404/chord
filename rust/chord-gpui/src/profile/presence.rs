//! A bounded Spotify projection of member activity data.
#![forbid(unsafe_code)]

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chord_codec::etf::EtfValue;
use serde_json::Value;

use super::{
    spotify_mpris::{
        ACTIVITY_FLAG_SYNC, ACTIVITY_NAME_SPOTIFY, ACTIVITY_TYPE_LISTENING, ASSET_IMAGE_PREFIX,
        PARTY_ID_PREFIX, extract_spotify_track_id, format_spotify_details,
        normalize_spotify_art_asset,
    },
    spotify_widget::{SpotifyProfileWidget, SpotifyTrack},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotifyActivity {
    track: SpotifyTrack,
    start_ms: u64,
    end_ms: u64,
}

impl SpotifyActivity {
    pub fn from_etf_activities(activities: &[EtfValue]) -> Option<Self> {
        activities.iter().find_map(Self::from_activity)
    }

    pub fn from_json_activities(activities: &[Value]) -> Option<Self> {
        activities.iter().find_map(Self::from_activity)
    }

    fn from_activity(activity: &impl ActivityField) -> Option<Self> {
        if activity.get("type")?.number()? != u64::from(ACTIVITY_TYPE_LISTENING)
            || activity.get("name")?.text()? != ACTIVITY_NAME_SPOTIFY
        {
            return None;
        }

        let title = format_spotify_details(activity.get("details")?.text()?);
        if title.is_empty() {
            return None;
        }
        let artist = activity
            .get("state")
            .and_then(ActivityField::text)
            .unwrap_or("");
        let artwork_url = activity
            .get("assets")
            .and_then(|assets| assets.get("large_image"))
            .and_then(ActivityField::text)
            .and_then(spotify_artwork_url);
        let track_id = sync_track_id(activity);
        let timestamps = activity.get("timestamps");
        Some(Self {
            track: SpotifyTrack {
                title,
                artist: artist.chars().take(256).collect(),
                artwork_url,
                id: track_id,
            },
            start_ms: timestamp(timestamps, "start"),
            end_ms: timestamp(timestamps, "end"),
        })
    }

    pub fn widget_at(self, now_ms: u64) -> SpotifyProfileWidget {
        let duration = self.end_ms.saturating_sub(self.start_ms);
        let is_playing = self.start_ms > 0 && (self.start_ms..self.end_ms).contains(&now_ms);
        SpotifyProfileWidget::new(
            self.track,
            now_ms.saturating_sub(self.start_ms),
            if self.start_ms > 0 { duration } else { 0 },
            is_playing,
        )
    }
}

/// Normalize a Spotify asset to a fixed HTTPS host before rendering remote profile data.
pub(crate) fn spotify_artwork_url(asset: &str) -> Option<String> {
    let normalized = normalize_spotify_art_asset(asset)?;
    let id = normalized.strip_prefix(ASSET_IMAGE_PREFIX)?;
    (id.len() <= 128).then(|| format!("https://i.scdn.co/image/{id}"))
}

fn sync_track_id(activity: &impl ActivityField) -> Option<String> {
    let flags = activity.get("flags")?.number()?;
    let party_id = activity.get("party")?.get("id")?.text()?;
    if flags & u64::from(ACTIVITY_FLAG_SYNC) == 0 || !party_id.starts_with(PARTY_ID_PREFIX) {
        return None;
    }
    extract_spotify_track_id(activity.get("sync_id")?.text()?)
}

fn timestamp<T: ActivityField>(timestamps: Option<&T>, key: &str) -> u64 {
    timestamps
        .and_then(|timestamps| timestamps.get(key))
        .and_then(ActivityField::number)
        .unwrap_or_default()
}

trait ActivityField {
    fn get(&self, key: &str) -> Option<&Self>;
    fn text(&self) -> Option<&str>;
    fn number(&self) -> Option<u64>;
}

impl ActivityField for Value {
    fn get(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }

    fn text(&self) -> Option<&str> {
        self.as_str()
    }

    fn number(&self) -> Option<u64> {
        self.as_u64()
    }
}

impl ActivityField for EtfValue {
    fn get(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }

    fn text(&self) -> Option<&str> {
        crate::dispatch::etf_text(self)
    }

    fn number(&self) -> Option<u64> {
        crate::dispatch::etf_parse_u64(self)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SpotifySnapshot {
    widget: SpotifyProfileWidget,
    observed_at: Instant,
}

impl SpotifySnapshot {
    pub(crate) fn new(activity: SpotifyActivity, observed_at: Instant, now_ms: u64) -> Self {
        Self {
            widget: activity.widget_at(now_ms),
            observed_at,
        }
    }

    pub(crate) fn replace(&mut self, activity: SpotifyActivity, observed_at: Instant, now_ms: u64) {
        self.widget.advance_by(milliseconds(
            observed_at.saturating_duration_since(self.observed_at),
        ));
        self.widget.synchronize(activity.widget_at(now_ms));
        self.observed_at = observed_at;
    }

    pub(crate) fn widget_at(&self, now: Instant) -> SpotifyProfileWidget {
        let mut widget = self.widget.clone();
        widget.advance_by(milliseconds(
            now.saturating_duration_since(self.observed_at),
        ));
        widget
    }
}

pub(crate) fn unix_ms() -> u64 {
    milliseconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

pub(crate) fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
pub(crate) struct ListenAlongStatus {
    pub(crate) host_id: String,
    pub(crate) track_id: String,
    pub(crate) requested_at: Instant,
    pub(crate) result: Option<Result<(), String>>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_only_spotify_listening_and_uses_safe_album_art() {
        let activities = [
            json!({ "type": 0, "name": "Spotify", "details": "A game" }),
            json!({
                "type": 2, "name": "Spotify", "details": "Night Train", "state": "Four Rails",
                "assets": { "large_image": "spotify:ab123" },
                "timestamps": { "start": 10_000, "end": 20_000 },
                "flags": 48, "party": { "id": "spotify:42" },
                "sync_id": "4cOdK2wGLETKBW3PvgPWqT"
            }),
        ];
        let widget = SpotifyActivity::from_json_activities(&activities)
            .unwrap()
            .widget_at(12_500);
        assert_eq!(widget.track.title, "Night Train");
        assert_eq!(widget.track.artist, "Four Rails");
        assert_eq!(
            widget.track.artwork_url.as_deref(),
            Some("https://i.scdn.co/image/ab123")
        );
        assert_eq!(widget.progress_ratio(), 0.25);
        assert_eq!(widget.listen_along_command().unwrap().position_ms, 2_500);
        assert_eq!(spotify_artwork_url("https://localhost/secret"), None);
    }

    #[test]
    fn parses_typed_wire_activity_and_stops_at_the_end() {
        let text = |value: &str| EtfValue::Str(value.into());
        let map = |fields: Vec<(&str, EtfValue)>| {
            EtfValue::Map(
                fields
                    .into_iter()
                    .map(|(key, value)| (text(key), value))
                    .collect(),
            )
        };
        let activity = map(vec![
            ("type", EtfValue::Int(2)),
            ("name", text("Spotify")),
            ("details", text("Night Train")),
            (
                "timestamps",
                map(vec![
                    ("start", EtfValue::Int(10_000)),
                    ("end", EtfValue::Int(20_000)),
                ]),
            ),
        ]);
        let activity = SpotifyActivity::from_etf_activities(&[activity]).unwrap();
        let observed_at = Instant::now();
        let snapshot = SpotifySnapshot::new(activity, observed_at, 19_000);
        let start = snapshot.widget_at(observed_at);
        let ended = snapshot.widget_at(observed_at + Duration::from_secs(2));
        assert!(start.is_advancing());
        assert_eq!(start.elapsed_ms(), 9_000);
        assert_eq!(ended.progress_ratio(), 1.0);
        assert!(!ended.is_advancing());
    }

    #[test]
    fn ignores_sync_when_activity_lacks_opt_in() {
        let activities = [json!({
            "type": 2, "name": "Spotify", "details": "Night Train",
            "sync_id": "4cOdK2wGLETKBW3PvgPWqT",
            "timestamps": { "start": 10_000, "end": 20_000 }
        })];
        let widget = SpotifyActivity::from_json_activities(&activities)
            .unwrap()
            .widget_at(12_500);
        assert_eq!(widget.progress_ratio(), 0.25);
        assert_eq!(widget.listen_along_command(), None);
    }
}
