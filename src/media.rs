//! Now-playing detection through Windows media sessions (GSMTC).
//!
//! Spotify and Chromium/Firefox browsers publish their sessions with cover
//! art, so one event-driven API covers both and no polling is needed.

use std::sync::mpsc::Sender;

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::Win32::Foundation::E_UNEXPECTED;
use windows::core::{Error, Result};

use crate::Event;
use crate::render::{COVER_BYTES, COVER_SIZE};

const SPOTIFY_APPS: &[&str] = &["spotify"];
/// Firefox registers its sessions under the install hash `308046B0AF4A39CB`.
const BROWSER_APPS: &[&str] = &[
    "chrome",
    "msedge",
    "firefox",
    "308046b0af4a39cb",
    "opera",
    "brave",
    "vivaldi",
    "yandex",
    "thebrowsercompany.arc",
];

#[derive(Clone, Copy, Debug)]
pub struct Sources {
    pub spotify: bool,
    pub browsers: bool,
}

impl Sources {
    fn accepts(&self, app_id: &str) -> bool {
        let app_id = app_id.to_ascii_lowercase();
        let listed = |apps: &[&str]| apps.iter().any(|app| app_id.contains(app));
        (self.spotify && listed(SPOTIFY_APPS)) || (self.browsers && listed(BROWSER_APPS))
    }
}

pub struct Track {
    pub title: String,
    pub artist: String,
    /// `COVER_SIZE`x`COVER_SIZE` BGRA.
    pub cover: Vec<u8>,
}

impl Track {
    pub fn label(&self) -> String {
        if self.artist.is_empty() { self.title.clone() } else { format!("{} - {}", self.artist, self.title) }
    }
}

struct Subscription {
    session: Session,
    media_token: i64,
    playback_token: i64,
}

pub struct MediaWatcher {
    manager: SessionManager,
    sessions_token: i64,
    subscriptions: Vec<Subscription>,
    notify: Sender<Event>,
}

impl MediaWatcher {
    pub fn new(notify: Sender<Event>) -> Result<Self> {
        let manager = SessionManager::RequestAsync()?.join()?;
        let sessions_token = manager.SessionsChanged(&handler(&notify, Event::SessionsChanged))?;
        let mut watcher = Self { manager, sessions_token, subscriptions: Vec::new(), notify };
        watcher.resubscribe()?;
        Ok(watcher)
    }

    /// Re-attaches change handlers after sessions appear or disappear.
    pub fn resubscribe(&mut self) -> Result<()> {
        self.unsubscribe();
        for session in self.manager.GetSessions()? {
            let media_token = session.MediaPropertiesChanged(&handler(&self.notify, Event::MediaChanged))?;
            let playback_token = session.PlaybackInfoChanged(&handler(&self.notify, Event::MediaChanged))?;
            self.subscriptions.push(Subscription { session, media_token, playback_token });
        }
        Ok(())
    }

    /// The playing track from an accepted source, preferring the session
    /// Windows considers current. Paused sessions are ignored.
    pub fn current_track(&self, sources: Sources) -> Result<Option<Track>> {
        let current = self.manager.GetCurrentSession().ok();
        let sessions = current.iter().chain(self.subscriptions.iter().map(|s| &s.session));
        for session in sessions {
            if !is_playing_from(session, sources) {
                continue;
            }
            let properties = session.TryGetMediaPropertiesAsync()?.join()?;
            // Spotify publishes the title before the artwork; the next
            // property change event brings the thumbnail.
            let Ok(thumbnail) = properties.Thumbnail() else {
                continue;
            };
            return Ok(Some(Track {
                title: properties.Title()?.to_string(),
                artist: properties.Artist()?.to_string(),
                cover: decode_cover(&thumbnail)?,
            }));
        }
        Ok(None)
    }

    fn unsubscribe(&mut self) {
        for sub in self.subscriptions.drain(..) {
            let _ = sub.session.RemoveMediaPropertiesChanged(sub.media_token);
            let _ = sub.session.RemovePlaybackInfoChanged(sub.playback_token);
        }
    }
}

impl Drop for MediaWatcher {
    fn drop(&mut self) {
        self.unsubscribe();
        let _ = self.manager.RemoveSessionsChanged(self.sessions_token);
    }
}

fn handler<S, A>(notify: &Sender<Event>, event: Event) -> TypedEventHandler<S, A>
where
    S: windows::core::RuntimeType + 'static,
    A: windows::core::RuntimeType + 'static,
{
    let notify = notify.clone();
    TypedEventHandler::new(move |_, _| {
        let _ = notify.send(event);
        Ok(())
    })
}

/// Sessions can vanish between enumeration and the query; such a session is
/// simply skipped.
fn is_playing_from(session: &Session, sources: Sources) -> bool {
    let accepted = session.SourceAppUserModelId().is_ok_and(|id| sources.accepts(&id.to_string()));
    accepted
        && session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .is_ok_and(|status| status == PlaybackStatus::Playing)
}

/// Decodes the artwork straight to a center-cropped `COVER_SIZE` square.
/// WIC's Fant filter averages the downscale, so no extra resampling is needed.
fn decode_cover(thumbnail: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let stream = thumbnail.OpenReadAsync()?.join()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (width, height) = (decoder.PixelWidth()?.max(1), decoder.PixelHeight()?.max(1));
    let short = width.min(height) as u64;
    let scale = |side: u32| (side as u64 * COVER_SIZE as u64).div_ceil(short).max(COVER_SIZE as u64) as u32;
    let (scaled_width, scaled_height) = (scale(width), scale(height));

    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(scaled_width)?;
    transform.SetScaledHeight(scaled_height)?;
    transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    transform.SetBounds(BitmapBounds {
        X: (scaled_width - COVER_SIZE) / 2,
        Y: (scaled_height - COVER_SIZE) / 2,
        Width: COVER_SIZE,
        Height: COVER_SIZE,
    })?;

    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Ignore,
            &transform,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    let pixels = data.DetachPixelData()?.to_vec();
    if pixels.len() != COVER_BYTES {
        return Err(Error::new(E_UNEXPECTED, "unexpected cover pixel buffer size"));
    }
    Ok(pixels)
}
