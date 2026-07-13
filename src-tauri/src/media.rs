use log::{debug, info};
use std::time::{Duration, Instant};
use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

/// Cached media session information
#[derive(Clone, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app_name: String,
    pub is_playing: bool,
}

/// Reads media information from Windows Media Session
pub struct MediaReader {
    cached_info: MediaInfo,
    last_read: Option<Instant>,
    manager: Option<GlobalSystemMediaTransportControlsSessionManager>,
    initialized: bool,
}

impl MediaReader {
    /// Create a new MediaReader
    /// Note: Initialization is deferred because the Windows API is async
    pub fn new() -> Self {
        Self {
            cached_info: MediaInfo::default(),
            last_read: None,
            manager: None,
            initialized: false,
        }
    }

    /// Clone the current cached media info. The daemon publishes this into shared
    /// state each tick so the GUI preview can seed a reader via `with_cached_info`.
    pub fn snapshot(&self) -> MediaInfo {
        self.cached_info.clone()
    }

    /// Initialize the media session manager (blocking)
    /// Should be called once during app initialization
    pub fn initialize(&mut self) -> Result<(), anyhow::Error> {
        if self.initialized {
            return Ok(());
        }

        // Request the media session manager and wait for it
        let async_op = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .map_err(|e| anyhow::anyhow!("Failed to request media session manager: {}", e))?;

        // Use get() to block and wait for the async operation to complete
        let manager = async_op
            .get()
            .map_err(|e| anyhow::anyhow!("Failed to get media session manager: {}", e))?;

        self.manager = Some(manager);
        self.initialized = true;
        info!("MediaReader initialized successfully");
        Ok(())
    }

    /// Get media info for a specific field
    /// Returns None if no media is playing (sensor should be hidden)
    /// Uses 5-second cache to avoid excessive API calls
    pub fn get_media_field(&mut self, field: MediaField) -> Option<String> {
        // Check cache expiration (5 seconds - faster than mouse battery since media changes more)
        if let Some(last_read) = self.last_read {
            if last_read.elapsed() < Duration::from_secs(5) {
                return self.get_cached_field(field);
            }
        }

        // Refresh cache
        self.refresh_media_info();

        self.get_cached_field(field)
    }

    fn get_cached_field(&self, field: MediaField) -> Option<String> {
        if !self.cached_info.is_playing {
            return None; // Hide sensor when nothing is playing
        }

        let value = match field {
            MediaField::Title => &self.cached_info.title,
            MediaField::Artist => &self.cached_info.artist,
            MediaField::Album => &self.cached_info.album,
            MediaField::App => &self.cached_info.app_name,
        };

        if value.is_empty() {
            None // Hide sensor if field is empty
        } else {
            Some(value.clone())
        }
    }

    fn refresh_media_info(&mut self) {
        self.last_read = Some(Instant::now());

        let manager = match &self.manager {
            Some(m) => m,
            None => {
                debug!("MediaReader not initialized");
                self.cached_info = MediaInfo::default();
                return;
            }
        };

        // Get current session
        let session = match manager.GetCurrentSession() {
            Ok(session) => session,
            Err(e) => {
                debug!("No current media session: {}", e);
                self.cached_info = MediaInfo::default();
                return;
            }
        };

        // Get app name from session
        let app_name = session
            .SourceAppUserModelId()
            .map(|s| Self::extract_app_name(&s.to_string()))
            .unwrap_or_default();

        // Get media properties (blocking)
        let async_op = match session.TryGetMediaPropertiesAsync() {
            Ok(op) => op,
            Err(e) => {
                debug!("Failed to request media properties: {}", e);
                self.cached_info = MediaInfo::default();
                return;
            }
        };

        let properties = match async_op.get() {
            Ok(props) => props,
            Err(e) => {
                debug!("Failed to get media properties: {}", e);
                self.cached_info = MediaInfo::default();
                return;
            }
        };

        // Extract media info
        self.cached_info = MediaInfo {
            title: properties
                .Title()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            artist: properties
                .Artist()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            album: properties
                .AlbumTitle()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            app_name,
            is_playing: true, // If we got here, something is playing
        };

        debug!(
            "Media info refreshed: {} - {} ({})",
            self.cached_info.title, self.cached_info.artist, self.cached_info.app_name
        );
    }

    /// Extract friendly app name from AppUserModelId
    /// e.g., "Spotify.exe" -> "Spotify", "Microsoft.ZuneMusic_..." -> "Groove"
    fn extract_app_name(app_id: &str) -> String {
        // Common app name mappings
        let known_apps = [
            ("Spotify", "Spotify"),
            ("Microsoft.ZuneMusic", "Groove"),
            ("chrome", "Chrome"),
            ("firefox", "Firefox"),
            ("msedge", "Edge"),
            ("VLC", "VLC"),
            ("foobar2000", "foobar"),
            ("AIMP", "AIMP"),
            ("musicbee", "MusicBee"),
            ("iTunes", "iTunes"),
            ("Deezer", "Deezer"),
            ("Tidal", "Tidal"),
            ("Amazon Music", "Amazon"),
            ("YouTube", "YouTube"),
        ];

        for (pattern, name) in known_apps {
            if app_id.to_lowercase().contains(&pattern.to_lowercase()) {
                return name.to_string();
            }
        }

        // Fallback: extract from path or use as-is
        app_id
            .split(&['\\', '/', '!', '_'][..])
            .find(|s| !s.is_empty() && !s.starts_with("Microsoft."))
            .map(|s| s.trim_end_matches(".exe").to_string())
            .unwrap_or_else(|| app_id.to_string())
    }
}

impl Default for MediaReader {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaReader {
    /// Constructor with a pre-populated cache and no live manager. `last_read` is
    /// set to now so `get_media_field` serves the seeded data from cache without
    /// touching the (absent) Windows API. Used by the settings preview to render
    /// `MEDIA_*` sensors with the daemon's live data.
    pub fn with_cached_info(info: MediaInfo) -> Self {
        Self {
            cached_info: info,
            last_read: Some(Instant::now()),
            manager: None,
            initialized: false,
        }
    }
}

/// Media field types for sensor configuration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaField {
    Title,
    Artist,
    Album,
    App,
}

impl MediaField {
    pub fn from_sensor_name(name: &str) -> Option<Self> {
        match name {
            "MEDIA_TITLE" => Some(MediaField::Title),
            "MEDIA_ARTIST" => Some(MediaField::Artist),
            "MEDIA_ALBUM" => Some(MediaField::Album),
            "MEDIA_APP" => Some(MediaField::App),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_field_from_sensor_name() {
        assert_eq!(
            MediaField::from_sensor_name("MEDIA_TITLE"),
            Some(MediaField::Title)
        );
        assert_eq!(
            MediaField::from_sensor_name("MEDIA_ARTIST"),
            Some(MediaField::Artist)
        );
        assert_eq!(
            MediaField::from_sensor_name("MEDIA_ALBUM"),
            Some(MediaField::Album)
        );
        assert_eq!(
            MediaField::from_sensor_name("MEDIA_APP"),
            Some(MediaField::App)
        );
        assert_eq!(MediaField::from_sensor_name("INVALID"), None);
        assert_eq!(MediaField::from_sensor_name("CLOCK"), None);
        assert_eq!(MediaField::from_sensor_name("BLANK"), None);
    }

    #[test]
    fn test_extract_app_name_spotify() {
        assert_eq!(MediaReader::extract_app_name("Spotify.exe"), "Spotify");
        assert_eq!(MediaReader::extract_app_name("spotify"), "Spotify");
    }

    #[test]
    fn test_extract_app_name_groove() {
        assert_eq!(
            MediaReader::extract_app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Groove"
        );
    }

    #[test]
    fn test_extract_app_name_chrome() {
        assert_eq!(MediaReader::extract_app_name("chrome.exe"), "Chrome");
        assert_eq!(MediaReader::extract_app_name("Google Chrome"), "Chrome");
    }

    #[test]
    fn test_extract_app_name_unknown() {
        // Unknown apps should extract a reasonable name
        let result = MediaReader::extract_app_name("SomeUnknownApp.exe");
        assert!(!result.is_empty());
        assert!(!result.contains(".exe"));
    }

    #[test]
    fn test_new_reader_defaults() {
        let reader = MediaReader::new();
        assert!(!reader.initialized);
        assert!(reader.manager.is_none());
        assert!(reader.last_read.is_none());
    }

    #[test]
    fn test_cached_field_no_media() {
        let reader = MediaReader::new();
        // Default MediaInfo has is_playing = false
        assert_eq!(reader.get_cached_field(MediaField::Title), None);
        assert_eq!(reader.get_cached_field(MediaField::Artist), None);
    }

    fn playing(title: &str, artist: &str, album: &str, app: &str) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            app_name: app.into(),
            is_playing: true,
        }
    }

    #[test]
    fn test_cached_field_returns_title_when_playing() {
        let r = MediaReader::with_cached_info(playing("Song", "Artist", "Album", "App"));
        assert_eq!(r.get_cached_field(MediaField::Title), Some("Song".into()));
    }

    #[test]
    fn test_cached_field_returns_artist_when_playing() {
        let r = MediaReader::with_cached_info(playing("T", "Artist", "Alb", "A"));
        assert_eq!(
            r.get_cached_field(MediaField::Artist),
            Some("Artist".into())
        );
    }

    #[test]
    fn test_cached_field_returns_album_when_playing() {
        let r = MediaReader::with_cached_info(playing("T", "Ar", "Album", "A"));
        assert_eq!(r.get_cached_field(MediaField::Album), Some("Album".into()));
    }

    #[test]
    fn test_cached_field_returns_app_when_playing() {
        let r = MediaReader::with_cached_info(playing("T", "Ar", "Al", "MyApp"));
        assert_eq!(r.get_cached_field(MediaField::App), Some("MyApp".into()));
    }

    #[test]
    fn test_cached_field_returns_none_for_empty_field_when_playing() {
        // is_playing=true but title is empty → field hidden (None)
        let r = MediaReader::with_cached_info(playing("", "Ar", "Al", "App"));
        assert_eq!(r.get_cached_field(MediaField::Title), None);
    }

    #[test]
    fn test_get_media_field_uses_cache_within_5s() {
        // last_read is set to "now" by with_cached_info → cache short-circuit hit
        let mut r = MediaReader::with_cached_info(playing("CachedTitle", "", "", ""));
        assert_eq!(
            r.get_media_field(MediaField::Title),
            Some("CachedTitle".into())
        );
    }

    #[test]
    fn test_get_media_field_refresh_with_no_manager_returns_none() {
        // No cache, no manager → refresh_media_info hits early return, defaults stay
        let mut r = MediaReader::new();
        assert_eq!(r.get_media_field(MediaField::Title), None);
    }

    #[test]
    fn test_refresh_media_info_resets_to_default_when_no_manager() {
        let mut r = MediaReader::new();
        r.cached_info = playing("stale", "stale", "stale", "stale");
        r.refresh_media_info();
        assert!(!r.cached_info.is_playing);
        assert!(r.cached_info.title.is_empty());
        assert!(r.last_read.is_some());
    }

    #[test]
    fn test_refresh_media_info_with_real_manager_does_not_panic() {
        // Best-effort: initialize the COM-backed manager. If initialize() succeeds, refresh()
        // exercises the manager-Some path (which may return early via GetCurrentSession Err
        // when nothing is playing). If initialize() fails, manager stays None and we hit the
        // None branch (already covered elsewhere — this test is just for the success-init case).
        let mut r = MediaReader::new();
        if r.initialize().is_ok() {
            r.refresh_media_info(); // exercises lines 95+ in the manager-Some branch
        }
    }

    #[test]
    fn test_default_constructs_uninitialized_reader() {
        let r = MediaReader::default();
        assert!(!r.initialized);
        assert!(r.manager.is_none());
        assert!(r.last_read.is_none());
    }

    #[test]
    fn test_extract_app_name_remaining_known_apps() {
        // Cover more rows of the known_apps table
        assert_eq!(MediaReader::extract_app_name("firefox.exe"), "Firefox");
        assert_eq!(MediaReader::extract_app_name("msedge.exe"), "Edge");
        assert_eq!(MediaReader::extract_app_name("VLC media player"), "VLC");
        assert_eq!(MediaReader::extract_app_name("foobar2000"), "foobar");
        assert_eq!(MediaReader::extract_app_name("AIMP"), "AIMP");
        assert_eq!(MediaReader::extract_app_name("musicbee"), "MusicBee");
        assert_eq!(MediaReader::extract_app_name("iTunes"), "iTunes");
        assert_eq!(MediaReader::extract_app_name("Deezer"), "Deezer");
        assert_eq!(MediaReader::extract_app_name("Tidal Music"), "Tidal");
        assert_eq!(MediaReader::extract_app_name("Amazon Music"), "Amazon");
        assert_eq!(MediaReader::extract_app_name("YouTube Music"), "YouTube");
    }

    #[test]
    fn test_get_media_field_expired_cache_triggers_refresh() {
        // Stale last_read (older than 5s) → falls through to refresh path.
        let mut r = MediaReader::with_cached_info(playing("stale", "", "", ""));
        let Some(stale) = Instant::now().checked_sub(Duration::from_secs(10)) else {
            return;
        };
        r.last_read = Some(stale);
        // No manager → refresh resets to default → None.
        let val = r.get_media_field(MediaField::Title);
        assert_eq!(val, None);
    }

    #[test]
    fn test_initialize_attempts_com_initialization() {
        // Initialize the COM-backed session manager. In some CI envs the call works,
        // in others (no Windows session) it errors. Both outcomes are acceptable;
        // we just want to exercise the function body.
        let mut r = MediaReader::new();
        let _ = r.initialize();
    }

    #[test]
    fn test_initialize_is_idempotent_when_already_initialized() {
        let mut r = MediaReader::new();
        r.initialized = true; // pretend init already happened
                              // Second call short-circuits at the early return → Ok(())
        assert!(r.initialize().is_ok());
    }

    #[test]
    fn test_extract_app_name_skips_microsoft_prefix() {
        // Microsoft.* prefix is filtered when no known match
        let result = MediaReader::extract_app_name("Microsoft.Unknown_8wekyb!Some");
        assert!(!result.starts_with("Microsoft."));
        assert!(!result.is_empty());
    }
}
