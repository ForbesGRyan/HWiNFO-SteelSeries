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
        let manager = async_op.get()
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
            title: properties.Title().map(|s| s.to_string()).unwrap_or_default(),
            artist: properties.Artist().map(|s| s.to_string()).unwrap_or_default(),
            album: properties.AlbumTitle().map(|s| s.to_string()).unwrap_or_default(),
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
        assert_eq!(MediaField::from_sensor_name("MEDIA_TITLE"), Some(MediaField::Title));
        assert_eq!(MediaField::from_sensor_name("MEDIA_ARTIST"), Some(MediaField::Artist));
        assert_eq!(MediaField::from_sensor_name("MEDIA_ALBUM"), Some(MediaField::Album));
        assert_eq!(MediaField::from_sensor_name("MEDIA_APP"), Some(MediaField::App));
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
}
