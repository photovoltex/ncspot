use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;
use librespot_playback::audio_backend::SinkBuilder;
use librespot_playback::config::PlayerConfig;
use librespot_playback::mixer::softmixer::SoftMixer;
use librespot_playback::mixer::MixerConfig;
use log::{debug, error, info};

use librespot_playback::audio_backend;
use librespot_playback::config::Bitrate;

use futures::channel::oneshot;
use tokio::sync::mpsc;

use url::Url;

use librespot_connect::spirc::SpircLoadCommand;
use librespot_protocol::spirc::TrackRef;
use std::env;
use std::str::FromStr;
use std::sync::{Arc, LockResult, RwLock};
use std::time::{Duration, SystemTime};

use crate::application::ASYNC_RUNTIME;
use crate::config;
use crate::events::{Event, EventManager};
use crate::spotify_api::WebApi;
use crate::spotify_connect::Connect;
use crate::spotify_worker::{Worker, WorkerCommand};

pub const VOLUME_PERCENT: u16 = ((u16::MAX as f64) * 1.0 / 100.0) as u16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum PlayerEvent {
    Playing(SystemTime),
    Paused(Duration),
    Stopped,
    FinishedTrack,
}

// TODO: Rename or document this as it isn't immediately clear what it represents/does from the
// name.
#[derive(Clone)]
pub struct Spotify {
    events: EventManager,
    credentials: Credentials,
    cfg: Arc<config::Config>,
    status: Arc<RwLock<PlayerEvent>>,
    pub api: WebApi,
    elapsed: Arc<RwLock<Option<Duration>>>,
    since: Arc<RwLock<Option<SystemTime>>>,
    channel: Arc<RwLock<Option<mpsc::UnboundedSender<WorkerCommand>>>>,
    pub user: Option<String>,
}

impl Spotify {
    pub fn new(
        events: EventManager,
        credentials: Credentials,
        cfg: Arc<config::Config>,
    ) -> Spotify {
        let mut spotify = Spotify {
            events,
            credentials,
            cfg: cfg.clone(),
            status: Arc::new(RwLock::new(PlayerEvent::Stopped)),
            api: WebApi::new(),
            elapsed: Arc::new(RwLock::new(None)),
            since: Arc::new(RwLock::new(None)),
            channel: Arc::new(RwLock::new(None)),
            user: None,
        };

        let (user_tx, user_rx) = oneshot::channel();
        spotify.start_worker(Some(user_tx));
        spotify.user = ASYNC_RUNTIME.block_on(user_rx).ok();
        let volume = cfg.state().volume;
        spotify.set_volume(volume);

        spotify.api.set_worker_channel(spotify.channel.clone());
        spotify.api.update_token();

        spotify.api.set_user(spotify.user.clone());

        spotify
    }

    pub fn start_worker(&self, user_tx: Option<oneshot::Sender<String>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self
            .channel
            .write()
            .expect("can't writelock worker channel") = Some(tx);
        {
            let worker_channel = self.channel.clone();
            let cfg = self.cfg.clone();
            let events = self.events.clone();
            let volume = self.volume();
            let credentials = self.credentials.clone();
            ASYNC_RUNTIME.spawn(Self::worker(
                worker_channel,
                events,
                rx,
                cfg,
                credentials,
                user_tx,
                volume,
            ));
        }
    }

    pub fn session_config() -> SessionConfig {
        let mut session_config = SessionConfig {
            client_id: config::CLIENT_ID.to_string(),
            ..Default::default()
        };
        match env::var("http_proxy") {
            Ok(proxy) => {
                info!("Setting HTTP proxy {}", proxy);
                session_config.proxy = Url::parse(&proxy).ok();
            }
            Err(_) => debug!("No HTTP proxy set"),
        }
        session_config
    }

    pub fn test_credentials(credentials: Credentials) -> Result<Session, librespot_core::Error> {
        let config = Self::session_config();
        let _guard = ASYNC_RUNTIME.enter();
        let session = Session::new(config, None);
        ASYNC_RUNTIME
            .block_on(session.connect(credentials, true))
            .map(|_| session)
    }

    fn create_session(cfg: &config::Config) -> Session {
        let librespot_cache_path = config::cache_path("librespot");
        let audio_cache_path = match cfg.values().audio_cache.unwrap_or(true) {
            true => Some(librespot_cache_path.join("files")),
            false => None,
        };
        let cache = Cache::new(
            Some(librespot_cache_path.clone()),
            Some(librespot_cache_path.join("volume")),
            audio_cache_path,
            cfg.values()
                .audio_cache_size
                .map(|size| size as u64 * 1048576),
        )
        .expect("Could not create cache");

        let session_config = Self::session_config();
        Session::new(session_config, Some(cache))
    }

    fn init_backend(desired_backend: Option<String>) -> Option<SinkBuilder> {
        let backend = if let Some(name) = desired_backend {
            audio_backend::BACKENDS
                .iter()
                .find(|backend| name == backend.0)
        } else {
            audio_backend::BACKENDS.first()
        }?;

        let backend_name = backend.0;

        info!("Initializing audio backend {}", backend_name);
        if backend_name == "pulseaudio" {
            env::set_var("PULSE_PROP_application.name", "ncspot");
            env::set_var("PULSE_PROP_stream.description", "ncurses Spotify client");
            env::set_var("PULSE_PROP_media.role", "music");
        }

        Some(backend.1)
    }

    async fn worker(
        worker_channel: Arc<RwLock<Option<mpsc::UnboundedSender<WorkerCommand>>>>,
        events: EventManager,
        commands: mpsc::UnboundedReceiver<WorkerCommand>,
        cfg: Arc<config::Config>,
        credentials: Credentials,
        user_tx: Option<oneshot::Sender<String>>,
        volume: u16,
    ) {
        let bitrate_str = cfg.values().bitrate.unwrap_or(320).to_string();
        let bitrate = Bitrate::from_str(&bitrate_str);
        if bitrate.is_err() {
            error!("invalid bitrate, will use 320 instead")
        }

        let player_config = PlayerConfig {
            gapless: cfg.values().gapless.unwrap_or(true),
            bitrate: bitrate.unwrap_or(Bitrate::Bitrate320),
            normalisation: cfg.values().volnorm.unwrap_or(false),
            normalisation_pregain_db: cfg.values().volnorm_pregain.unwrap_or(0.0),
            ..Default::default()
        };

        let session = Self::create_session(&cfg);
        user_tx.map(|tx| tx.send(session.username()));

        let create_mixer = librespot_playback::mixer::find(Some(SoftMixer::NAME))
            .expect("could not create softvol mixer");
        let mixer = create_mixer(MixerConfig::default());
        mixer.set_volume(volume);

        let backend_name = cfg.values().backend.clone();
        let backend =
            Self::init_backend(backend_name).expect("Could not find an audio playback backend");
        let device = cfg.values().backend_device.clone();
        let connect = Connect::new(
            credentials,
            session,
            player_config,
            create_mixer,
            backend,
            device,
        );
        let mut worker = Worker::new(events.clone(), commands, connect);

        debug!("worker thread ready.");
        worker.run_loop().await;

        error!("worker thread died, requesting restart");
        *worker_channel
            .write()
            .expect("can't writelock worker channel") = None;
        events.send(Event::SessionDied)
    }

    pub fn get_current_status(&self) -> PlayerEvent {
        let status = self
            .status
            .read()
            .expect("could not acquire read lock on playback status");
        (*status).clone()
    }

    pub fn get_current_progress(&self) -> Duration {
        self.get_elapsed().unwrap_or_else(|| Duration::from_secs(0))
            + self
                .get_since()
                .map(|t| t.elapsed().unwrap())
                .unwrap_or_else(|| Duration::from_secs(0))
    }

    fn set_elapsed(&self, new_elapsed: Option<Duration>) {
        match self.elapsed.write() {
            Ok(mut current) => *current = new_elapsed,
            Err(_) => error!("could not acquire write lock on elapsed time")
        }
    }

    fn get_elapsed(&self) -> Option<Duration> {
        let elapsed = self
            .elapsed
            .read()
            .expect("could not acquire read lock on elapsed time");
        *elapsed
    }

    fn set_since(&self, new_since: Option<SystemTime>) {
        match self.since.write() {
            Ok(mut current) => *current = new_since,
            Err(_) => error!("could not acquire write lock on since time")
        }
    }

    fn get_since(&self) -> Option<SystemTime> {
        let since = self
            .since
            .read()
            .expect("could not acquire read lock on since time");
        *since
    }

    pub fn load(
        &self,
        context_uri: String,
        tracks: Vec<TrackRef>,
        current_index: usize,
        start_playing: bool,
        position_ms: u32,
        shuffle: bool,
        repeat: bool,
    ) {
        info!("loading track: {:?}", tracks.get(current_index));
        let cmd = SpircLoadCommand {
            context_uri,
            start_playing,
            shuffle,
            repeat,
            playing_track_index: current_index as u32,
            tracks,
        };

        self.send_worker(WorkerCommand::Load(cmd, position_ms));
    }

    pub fn update_status(&self, new_status: PlayerEvent) {
        match new_status {
            PlayerEvent::Paused(position) => {
                self.set_elapsed(Some(position));
                self.set_since(None);
            }
            PlayerEvent::Playing(playback_start) => {
                self.set_since(Some(playback_start));
                self.set_elapsed(None);
            }
            PlayerEvent::Stopped | PlayerEvent::FinishedTrack => {
                self.set_elapsed(None);
                self.set_since(None);
            }
        }

        let mut status = self
            .status
            .write()
            .expect("could not acquire write lock on player status");
        *status = new_status;
    }
    
    pub fn update_position(&self, position: Duration) {
        self.set_elapsed(Some(position));
        self.set_since(None);
    }

    pub fn update_track(&self) {
        self.set_elapsed(None);
        self.set_since(None);
    }

    pub fn play(&self) {
        info!("play()");
        self.send_worker(WorkerCommand::Play);
    }

    pub fn toggleplayback(&self) {
        match self.get_current_status() {
            PlayerEvent::Playing(_) => self.pause(),
            PlayerEvent::Paused(_) => self.play(),
            _ => (),
        }
    }

    fn send_worker(&self, cmd: WorkerCommand) {
        let channel = self.channel.read().expect("can't readlock worker channel");
        match channel.as_ref() {
            Some(channel) => channel.send(cmd).expect("can't send message to worker"),
            None => error!("no channel to worker available"),
        }
    }

    pub fn pause(&self) {
        info!("pause()");
        self.send_worker(WorkerCommand::Pause);
    }

    pub fn stop(&self) {
        info!("stop()");
        self.send_worker(WorkerCommand::Stop);
    }

    pub fn seek(&self, position_ms: u32) {
        self.send_worker(WorkerCommand::Seek(position_ms));
    }

    pub fn seek_relative(&self, delta: i32) {
        let progress = self.get_current_progress();
        let new = (progress.as_secs() * 1000) as i32 + progress.subsec_millis() as i32 + delta;
        self.seek(std::cmp::max(0, new) as u32);
    }

    pub fn volume(&self) -> u16 {
        self.cfg.state().volume
    }

    pub fn set_volume(&self, volume: u16) {
        info!("setting volume to {}", volume);
        self.cfg.with_state_mut(|mut s| s.volume = volume);
        self.send_worker(WorkerCommand::SetVolume(volume));
    }

    pub fn shutdown(&self) {
        self.send_worker(WorkerCommand::Shutdown);
    }
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum UriType {
    Album,
    Artist,
    Track,
    Playlist,
    Show,
    Episode,
}

impl UriType {
    pub fn from_uri(s: &str) -> Option<UriType> {
        if s.starts_with("spotify:album:") {
            Some(UriType::Album)
        } else if s.starts_with("spotify:artist:") {
            Some(UriType::Artist)
        } else if s.starts_with("spotify:track:") {
            Some(UriType::Track)
        } else if s.starts_with("spotify:") && s.contains(":playlist:") {
            Some(UriType::Playlist)
        } else if s.starts_with("spotify:show:") {
            Some(UriType::Show)
        } else if s.starts_with("spotify:episode:") {
            Some(UriType::Episode)
        } else {
            None
        }
    }
}
