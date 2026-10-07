//! Background thread: reacts to media events and renders wallpapers.

use std::error::Error;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};

use crate::media::{MediaWatcher, Sources};
use crate::render::{Gradient, corner_cover};
use crate::settings::{Mode, Settings};
use crate::wallpaper::Wallpaper;
use crate::{Event, Shared, Status, screen_size, trim_memory};

/// Media sessions fire bursts of events per track change (title, artwork,
/// playback state); they are coalesced into one refresh.
const DEBOUNCE: Duration = Duration::from_millis(350);

type AnyResult<T> = Result<T, Box<dyn Error>>;

pub fn run(shared: Arc<Shared>, events: Receiver<Event>, notify: Sender<Event>, wallpaper_dir: PathBuf) {
    // Background mode lowers CPU, I/O and memory priority so rendering never
    // competes with games or other foreground work.
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }

    let report = |error: &dyn Error| shared.set_status(Status::error(format!("Ошибка запуска: {error}")));
    // Declared first so COM is released after every COM object is dropped.
    let _com = match ComApartment::enter() {
        Ok(com) => com,
        Err(error) => return report(&error),
    };
    match Worker::new(shared.clone(), notify, wallpaper_dir) {
        Ok(mut worker) => worker.run(&events),
        Err(error) => report(error.as_ref()),
    }
}

struct ComApartment;

impl ComApartment {
    fn enter() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[derive(Default)]
struct Batch {
    refresh: bool,
    resubscribe: bool,
    restyle: bool,
    shutdown: bool,
}

impl Batch {
    fn add(&mut self, event: Event) {
        match event {
            Event::MediaChanged => self.refresh = true,
            Event::SessionsChanged => self.resubscribe = true,
            Event::SettingsChanged => self.restyle = true,
            Event::Shutdown => self.shutdown = true,
        }
    }

    /// Blocks for the next event, then gathers the rest of its burst.
    fn collect(events: &Receiver<Event>) -> Self {
        let mut batch = Self::default();
        match events.recv() {
            Ok(event) => batch.add(event),
            Err(_) => batch.shutdown = true,
        }
        while !batch.shutdown {
            match events.recv_timeout(DEBOUNCE) {
                Ok(event) => batch.add(event),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => batch.shutdown = true,
            }
        }
        batch
    }
}

struct Worker {
    shared: Arc<Shared>,
    wallpaper: Wallpaper,
    watcher: MediaWatcher,
    cover_hash: Option<u64>,
    custom_shown: bool,
}

impl Worker {
    fn new(shared: Arc<Shared>, notify: Sender<Event>, wallpaper_dir: PathBuf) -> AnyResult<Self> {
        fs::create_dir_all(&wallpaper_dir)?;
        Ok(Self {
            shared,
            wallpaper: Wallpaper::new(wallpaper_dir)?,
            watcher: MediaWatcher::new(notify)?,
            cover_hash: None,
            custom_shown: false,
        })
    }

    fn run(&mut self, events: &Receiver<Event>) {
        let mut batch = Batch { refresh: true, ..Batch::default() };
        while !batch.shutdown {
            if batch.resubscribe
                && let Err(error) = self.watcher.resubscribe()
            {
                self.report(error.into());
            }
            if (batch.refresh || batch.resubscribe || batch.restyle)
                && let Err(error) = self.refresh(batch.restyle)
            {
                self.report(error);
            }
            trim_memory();
            batch = Batch::collect(events);
        }

        // A custom gradient is the wallpaper the user chose to keep, so it
        // survives exit, logoff and reboot.
        let settings = self.shared.settings();
        if settings.mode == Mode::Music && settings.restore_on_exit {
            let _ = self.wallpaper.restore();
        }
    }

    fn refresh(&mut self, restyle: bool) -> AnyResult<()> {
        let settings = self.shared.settings();
        if settings.mode == Mode::Custom {
            return self.show_custom(&settings, restyle);
        }
        self.custom_shown = false;
        let sources = Sources { spotify: settings.spotify, browsers: settings.browsers };

        let cover_changed = match self.watcher.current_track(sources)? {
            Some(track) => {
                let hash = hash(&track.cover);
                let changed = self.cover_hash != Some(hash);
                if changed {
                    self.cover_hash = Some(hash);
                    self.shared.set_cover(Some(track.cover.as_slice().into()));
                }
                self.shared.set_status(Status::playing(track.label()));
                changed
            }
            None => {
                self.shared.set_status(Status::idle("Ожидание воспроизведения"));
                false
            }
        };

        // Albums share artwork: the next track of the same album keeps the
        // current wallpaper instead of rewriting an identical file.
        if !cover_changed && !restyle {
            return Ok(());
        }
        match self.shared.cover() {
            Some(cover) => self.render(&cover, &settings),
            None => Ok(()),
        }
    }

    /// Media events never touch a custom gradient; only settings changes do.
    fn show_custom(&mut self, settings: &Settings, restyle: bool) -> AnyResult<()> {
        if self.custom_shown && !restyle {
            return Ok(());
        }
        self.render(&corner_cover(settings.colors), settings)?;
        self.custom_shown = true;
        self.shared.set_status(Status::custom("Свой градиент установлен"));
        Ok(())
    }

    fn render(&mut self, cover: &[u8], settings: &Settings) -> AnyResult<()> {
        let (width, height) = screen_size();
        let gradient = Gradient::new(cover, &settings.look(), width, height);
        let path = self.wallpaper.next_path();
        gradient.write_bmp(width, height, &path)?;
        self.wallpaper.apply(&path)?;
        Ok(())
    }

    fn report(&self, error: Box<dyn Error>) {
        self.shared.set_status(Status::error(format!("Ошибка: {error}")));
    }
}

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
