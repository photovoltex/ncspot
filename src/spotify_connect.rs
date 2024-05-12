use std::future::Future;
use std::pin::Pin;

use librespot_connect::config::ConnectConfig;
use librespot_connect::spirc::{Spirc, SpircEventChannel};
use librespot_core::authentication::Credentials;
use librespot_core::config::DeviceType;
use librespot_core::Session;
use librespot_playback::audio_backend::SinkBuilder;
use librespot_playback::config::{AudioFormat, PlayerConfig};
use librespot_playback::mixer::{MixerConfig, MixerFn};
use librespot_playback::player::{Player, PlayerEventChannel};

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
