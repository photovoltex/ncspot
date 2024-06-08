use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use librespot_connect::config::ConnectConfig;
use librespot_connect::spirc::{Spirc, SpircEventChannel};
use librespot_core::authentication::Credentials;
use librespot_core::config::DeviceType;
use librespot_core::Session;
use librespot_playback::audio_backend::SinkBuilder;
use librespot_playback::config::{AudioFormat, PlayerConfig};
use librespot_playback::mixer::{MixerConfig, MixerFn};
use librespot_playback::player::{Player, PlayerEventChannel};
use librespot_protocol::spirc::{DeviceState, PlayStatus, State, TrackRef};
use log::{debug, warn};
use crate::events::{Event, EventManager};
use crate::spotify::PlayerEvent;

const MEASURED_AT_TOLERATION: Duration = Duration::from_secs(2);

const WORKER_NAME: &str = "ncspot";
const OBSERVER_NAME: &str = "ncspotStateObserver";

pub struct Connect {
    credentials: Credentials,
    pub session: Session,
    player_config: PlayerConfig,
    create_mixer: MixerFn,
    backend: SinkBuilder,
    backend_device: Option<String>,
}

pub struct SpircHandle {
    pub spirc: Spirc,
    pub player_events: Option<PlayerEventChannel>,
    pub spirc_events: SpircEventChannel,
}

impl Connect {
    pub fn new(
        credentials: Credentials,
        session: Session,
        player_config: PlayerConfig,
        create_mixer: MixerFn,
        backend: SinkBuilder,
        backend_device: Option<String>,
    ) -> Self {
        Connect {
            credentials,
            session,
            player_config,
            create_mixer,
            backend,
            backend_device,
        }
    }

    pub async fn create_new_handle(
        &self,
        needs_player: bool,
        initial_volume: Option<u16>,
    ) -> Result<(SpircHandle, Pin<Box<impl Future<Output = ()>>>), librespot_core::Error> {
        let connect_config = ConnectConfig {
            name: if needs_player {
                WORKER_NAME
            } else {
                OBSERVER_NAME
            }
            .to_string(),
            initial_volume,
            device_type: if needs_player {
                DeviceType::Computer
            } else {
                DeviceType::Observer
            },
            has_volume_ctrl: needs_player,
            hidden: !needs_player,
        };

        let create_mixer = self.create_mixer;
        let mixer = create_mixer(MixerConfig::default());

        let soft_volume = mixer.get_soft_volume();
        let backend = self.backend;
        let device = self.backend_device.clone();

        let player = needs_player.then(|| {
            Player::new(
                self.player_config.clone(),
                self.session.clone(),
                soft_volume,
                move || backend(device, AudioFormat::default()),
            )
        });

        let player_events = player.as_ref().map(|p| p.get_player_event_channel());
        let (spirc, spirc_task) = Spirc::new(
            connect_config,
            self.session.clone(),
            self.credentials.clone(),
            player,
            mixer,
        )
        .await?;

        let spirc_events = spirc.get_remote_event_channel()?;
        let spirc_task = Box::pin(spirc_task);

        let handle = SpircHandle {
            spirc,
            player_events,
            spirc_events,
        };

        Ok((handle, spirc_task))
    }
}

#[derive(Default)]
pub struct ConnectState {
    shuffle: bool,
    repeat: bool,
    context_uri: String,
    position_ms: u32,
    position_measured_at: u64,
    index: u32,
    status: PlayStatus,
    track: Vec<TrackRef>,
}

#[derive(Debug)]
pub enum ConnectEvent {
    Shuffle(bool),
    Repeat(bool),
    Context(String),
    Index(usize),
    Position(u32),
    Queue(Vec<TrackRef>)
}

impl From<ConnectEvent> for Event {
    fn from(value: ConnectEvent) -> Self {
        Event::Connect(value)
    }
}

impl ConnectState {
    fn assign_if_new_value<T: Eq>(field: &mut T, new_value: T) -> bool {
        if new_value.ne(field) {
            *field = new_value;
            true
        } else {
            false
        }
    }
    
    fn should_update_position_ms(&self) -> bool {
        let time = SystemTime::now();
        let measured_at = UNIX_EPOCH + Duration::from_millis(self.position_measured_at);
        time.duration_since(measured_at)
            .map(|since| since < MEASURED_AT_TOLERATION)
            .unwrap_or(false)
    }
    
    pub fn update_state(&mut self, state: State, event_manager: &EventManager) {
        debug!("update state");
        if Self::assign_if_new_value(&mut self.shuffle, state.shuffle()) {
            event_manager.send(ConnectEvent::Shuffle(self.shuffle).into())
        }
        
        if Self::assign_if_new_value(&mut self.repeat, state.repeat()) {
            event_manager.send(ConnectEvent::Repeat(self.repeat).into())
        }

        if Self::assign_if_new_value(&mut self.context_uri, state.context_uri().to_string()) {
            event_manager.send(ConnectEvent::Context(self.context_uri.clone()).into())
        }

        if Self::assign_if_new_value(&mut self.index, state.index()) {
            let index = self.index.try_into().unwrap_or_default();
            event_manager.send(ConnectEvent::Index(index).into())
        }

        _ = Self::assign_if_new_value(&mut self.position_measured_at, state.position_measured_at());
            
        let should_update_position = self.should_update_position_ms();
        if Self::assign_if_new_value(&mut self.position_ms, state.position_ms()) && should_update_position {
            event_manager.send(ConnectEvent::Position(self.position_ms).into())
        }

        let status = Self::assign_if_new_value(&mut self.status, state.status()).then(|| match self.status {
            PlayStatus::kPlayStatusStop => Some(PlayerEvent::Stopped),
            PlayStatus::kPlayStatusPlay => Some(PlayerEvent::Playing(UNIX_EPOCH + Duration::from_millis(self.position_measured_at) - Duration::from_millis(self.position_ms.into()))),
            PlayStatus::kPlayStatusPause => Some(PlayerEvent::Paused(Duration::from_millis(self.position_ms.into()))),
            PlayStatus::kPlayStatusLoading => None
        }).flatten();
        
        if self.track.len() != state.track.len() {
            debug!("old: {}, new: {}", self.track.len(), state.track.len());
            self.track = state.track;

            event_manager.send(ConnectEvent::Queue(self.track.clone()).into());
        }

        if let Some(status) = status {
            event_manager.send(Event::Player(status))
        }

        // event_manager.send(Event::Connect(ConnectEvent::))
    }
}

#[derive(Default)]
pub struct ConnectDevices(HashMap<String, DeviceState>);

#[derive(Debug)]
pub enum DeviceEvent {
    Add(String, String, bool),
    Remove(String),
    Active(String)
}


impl ConnectDevices {
    pub fn update_devices(&mut self, device_state: DeviceState, event_manager: &EventManager) {
        let ConnectDevices(devices) = self;
        
        if device_state.name().is_empty() {
            warn!("device with no name received");
            return;
        }
        
        let name = device_state.name().to_string();
        debug!("updated device: '{name}'");
        devices.insert(name, device_state);

        // event_manager.send(DeviceEvent::Active())
    }

    fn get_active(&self) -> Option<&DeviceState> {
        self.0.iter().find_map(|(_, device)| device.is_active().then_some(device))
    }
    
    pub fn active_name(&self) -> Option<String> {
        self.get_active().map(|d| d.name().to_string())
    }
    
    pub fn active_volume(&self) -> Option<u32> {
        self.get_active().map(|d| d.volume())
    }

    pub fn is_active(&self) -> bool {
        self.0
            .get(WORKER_NAME)
            .map(|device| device.is_active())
            .unwrap_or(false)
    }
}
