//! The profile playback timeline, independent of GPUI and the playback transport.
#![forbid(unsafe_code)]

pub const SPOTIFY_GREEN_HEX: &str = "#1db954";
pub const SPOTIFY_DESYNC_THRESHOLD_MS: u64 = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotifyTrack {
    pub title: String,
    pub artist: String,
    pub artwork_url: Option<String>,
    /// Validated Spotify track ID, when the activity supplies one.
    pub id: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ListenAlongCommand {
    pub track_id: String,
    pub position_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotifyProfileWidget {
    pub track: SpotifyTrack,
    elapsed_ms: u64,
    duration_ms: u64,
    is_playing: bool,
}

impl SpotifyProfileWidget {
    pub fn new(track: SpotifyTrack, elapsed_ms: u64, duration_ms: u64, is_playing: bool) -> Self {
        Self {
            track,
            elapsed_ms: elapsed_ms.min(duration_ms),
            duration_ms,
            is_playing,
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn is_advancing(&self) -> bool {
        self.is_playing && self.elapsed_ms < self.duration_ms
    }

    pub fn advance_by(&mut self, delta_ms: u64) {
        if !self.is_playing {
            return;
        }
        self.elapsed_ms = self
            .elapsed_ms
            .saturating_add(delta_ms)
            .min(self.duration_ms);
    }

    /// Refresh metadata and correct significant clock drift. Track and playback-state changes
    /// always use the newly observed position.
    pub fn synchronize(&mut self, observed: Self) {
        let same_track = match (&self.track.id, &observed.track.id) {
            (Some(current), Some(next)) => current == next,
            (None, None) => {
                self.track.title == observed.track.title
                    && self.track.artist == observed.track.artist
            }
            _ => false,
        };
        if same_track
            && self.duration_ms == observed.duration_ms
            && self.is_playing == observed.is_playing
            && self.elapsed_ms.abs_diff(observed.elapsed_ms) <= SPOTIFY_DESYNC_THRESHOLD_MS
        {
            self.track = observed.track;
        } else {
            *self = observed;
        }
    }

    pub fn progress_ratio(&self) -> f64 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        (self.elapsed_ms as f64 / self.duration_ms as f64).clamp(0.0, 1.0)
    }

    pub fn listen_along_command(&self) -> Option<ListenAlongCommand> {
        if !self.is_advancing() {
            return None;
        }
        let track_id = self.track.id.clone()?;
        Some(ListenAlongCommand {
            track_id,
            position_ms: self.elapsed_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(position: u64, duration: u64, playing: bool) -> SpotifyProfileWidget {
        SpotifyProfileWidget::new(
            SpotifyTrack {
                title: "Night Train".into(),
                artist: "Four Rails".into(),
                artwork_url: None,
                id: Some("4cOdK2wGLETKBW3PvgPWqT".into()),
            },
            position,
            duration,
            playing,
        )
    }

    #[test]
    fn uses_spotify_green() {
        assert_eq!(SPOTIFY_GREEN_HEX, "#1db954");
    }

    #[test]
    fn paused_playback_does_not_advance() {
        let mut paused = widget(1_000, 10_000, false);
        paused.advance_by(3_000);
        assert_eq!(paused.elapsed_ms(), 1_000);
    }

    #[test]
    fn playing_position_stops_at_track_end() {
        let mut playing = widget(1_000, 10_000, true);
        playing.advance_by(2_000);
        assert_eq!(playing.elapsed_ms(), 3_000);
        playing.advance_by(u64::MAX);
        assert_eq!(playing.elapsed_ms(), 10_000);
        assert!(!playing.is_advancing());
    }

    #[test]
    fn corrects_drift_over_two_seconds() {
        let mut playing = widget(3_000, 10_000, true);
        playing.synchronize(widget(5_000, 10_000, true));
        assert_eq!(playing.elapsed_ms(), 3_000);
        playing.synchronize(widget(5_001, 10_000, true));
        assert_eq!(playing.elapsed_ms(), 5_001);
        playing.synchronize(widget(2_500, 10_000, true));
        assert_eq!(playing.elapsed_ms(), 2_500);
    }

    #[test]
    fn a_pause_or_track_change_accepts_the_observed_position() {
        let mut playing = widget(4_000, 10_000, true);
        playing.synchronize(widget(4_250, 10_000, false));
        playing.advance_by(1_000);
        assert_eq!(playing.elapsed_ms(), 4_250);

        let mut next = widget(3_000, 10_000, false);
        next.track.id = Some("2takcwOaAZWiXQijPHIx7B".into());
        playing.synchronize(next);
        assert_eq!(playing.elapsed_ms(), 3_000);
    }

    #[test]
    fn progress_ratio_stays_in_range() {
        assert_eq!(widget(2_500, 10_000, true).progress_ratio(), 0.25);
        assert_eq!(widget(11_000, 10_000, true).progress_ratio(), 1.0);
        assert_eq!(widget(100, 0, true).progress_ratio(), 0.0);
    }

    #[test]
    fn active_track_produces_a_positioned_listen_along_command() {
        let mut playing = widget(2_000, 10_000, true);
        playing.advance_by(1_500);
        assert_eq!(
            playing.listen_along_command(),
            Some(ListenAlongCommand {
                track_id: "4cOdK2wGLETKBW3PvgPWqT".into(),
                position_ms: 3_500,
            })
        );
    }

    #[test]
    fn listen_along_requires_an_active_playable_track() {
        assert_eq!(widget(2_000, 10_000, false).listen_along_command(), None);
        assert_eq!(widget(10_000, 10_000, true).listen_along_command(), None);
        let mut unaddressable = widget(2_000, 10_000, true);
        unaddressable.track.id = None;
        assert_eq!(unaddressable.listen_along_command(), None);
    }
}
