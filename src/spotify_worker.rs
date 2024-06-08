use std::{pin::Pin, time::SystemTime};
use std::fmt::{Display, Formatter};
use std::time::Duration;

use futures::channel::oneshot;
use futures::Future;
use futures::FutureExt;
use librespot_connect::spirc::{SpircEvent, SpircLoadCommand};
use librespot_core::session::Session;
use librespot_core::token::Token;
use librespot_playback::player::PlayerEvent as LibrespotPlayerEvent;
use log::{debug, error, info, warn};
use tokio::sync::mpsc;
use tokio::time;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::events::{Event, EventManager};
use crate::spotify::PlayerEvent;
use crate::spotify_connect::{Connect, ConnectDevices, ConnectState, SpircHandle};

#[derive(Debug)]
pub(crate) enum WorkerCommand {
    Activate,
    Load(SpircLoadCommand, u32),
    Play,
    Pause,
    Stop,
    Seek(u32),
    SetVolume(u16),
    RequestToken(oneshot::Sender<Option<Token>>),
    Shutdown,
}

impl Display for WorkerCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let cmd = match self {
            WorkerCommand::Activate => "Activate",
            WorkerCommand::Load(_, _) => "Load",
            WorkerCommand::Play => "Play",
            WorkerCommand::Pause => "Pause",
            WorkerCommand::Stop => "Stop",
            WorkerCommand::Seek(_) => "Seek",
            WorkerCommand::SetVolume(_) => "SetVolume",
            WorkerCommand::RequestToken(_) => "RequestToken",
            WorkerCommand::Shutdown => "Shutdown",
        };

        write!(f, "{cmd}")
    }
}

pub struct Worker {
    events: EventManager,
    commands: UnboundedReceiverStream<WorkerCommand>,
    connect: Connect,
    token_task: Pin<Box<dyn Future<Output = ()> + Send>>,
    active: bool,
    last_state: ConnectState,
    devices: ConnectDevices,
}

impl Worker {
    pub(crate) fn new(
        events: EventManager,
        commands: mpsc::UnboundedReceiver<WorkerCommand>,
        connect: Connect,
    ) -> Worker {
        Worker {
            events,
            commands: UnboundedReceiverStream::new(commands),
            connect,
            token_task: Box::pin(futures::future::pending()),
            active: false,
            last_state: ConnectState::default(),
            devices: ConnectDevices::default(),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        debug!("Worker thread is shutting down, stopping player");
    }
}

impl Worker {
    async fn get_token(session: Session, sender: oneshot::Sender<Option<Token>>) {
        let scopes = "user-read-private,playlist-read-private,playlist-read-collaborative,playlist-modify-public,playlist-modify-private,user-follow-modify,user-follow-read,user-library-read,user-library-modify,user-top-read,user-read-recently-played";
        session
            .token_provider()
            .get_token(scopes)
            .map(|response| sender.send(response.ok()).expect("token channel is closed"))
            .await;
    }

    pub async fn run_loop(&mut self) {
        debug!("run loop");
        let handle_tuple = self.connect.create_new_handle(true, None).await;
        let (handle, mut spirc_task) = match handle_tuple {
            Ok(inner) => inner,
            Err(why) => {
                error!("Creating connection failed: {why}");
                return;
            }
        };

        let SpircHandle {
            spirc,
            mut player_events,
            // todo: use spirc_events and handle external changes
            mut spirc_events,
        } = handle;

        // spirc.activate().unwrap();

        debug!("created handle successfully");

        let mut ui_refresh = time::interval(Duration::from_millis(400));

        loop {
            let player_events = player_events.as_mut();

            if self.connect.session.is_invalid() {
                info!("Librespot session invalidated, terminating worker");
                self.events.send(Event::Player(PlayerEvent::Stopped));
                break;
            }

            tokio::select! {
                cmd = self.commands.next() => match cmd {
                    Some(WorkerCommand::Activate) => {
                        if self.active {
                            warn!("trying to activate already active device")
                        } else {
                            // todo: fix .unwrap()
                            spirc.activate().unwrap()
                        }
                    }
                    Some(WorkerCommand::Load(cmd, position_ms)) if self.active => {
                        // todo: fix .unwrap()
                        spirc.load(cmd).unwrap();
                        spirc.set_position_ms(position_ms).unwrap()
                    }
                    Some(WorkerCommand::Play) if self.active => {
                        // todo: fix .unwrap()
                        spirc.play().unwrap();
                    }
                    Some(WorkerCommand::Pause) if self.active => {
                        // todo: fix .unwrap()
                        spirc.pause().unwrap();
                    }
                    Some(WorkerCommand::Stop) if self.active => {
                        // todo: fix .unwrap()
                        spirc.load(SpircLoadCommand {
                            context_uri: "".to_string(),
                            start_playing: false,
                            shuffle: false,
                            repeat: false,
                            playing_track_index: 0,
                            tracks: vec![],
                        }).unwrap()
                    }
                    Some(WorkerCommand::Seek(pos)) if self.active => {
                        // todo: fix .unwrap()
                        spirc.set_position_ms(pos).unwrap();
                    }
                    Some(WorkerCommand::SetVolume(volume)) if self.active => {
                        // todo: fix .unwrap()
                        spirc.set_volume(volume).unwrap();
                    }
                    Some(WorkerCommand::RequestToken(sender)) => {
                        self.token_task = Box::pin(Self::get_token(self.connect.session.clone(), sender));
                    }
                    Some(WorkerCommand::Shutdown) => {
                        // todo: fix unused
                        spirc.shutdown().unwrap();
                    }
                    Some(cmd) => info!("not active, skipping: {cmd}"),
                    None => info!("empty stream")
                },
                event = player_events.unwrap().recv(), if player_events.is_some() => match event {
                    Some(LibrespotPlayerEvent::Playing {
                        play_request_id: _,
                        track_id: _,
                        position_ms,
                    }) => {
                        let position = Duration::from_millis(position_ms as u64);
                        let playback_start = SystemTime::now() - position;
                        self.events
                            .send(Event::Player(PlayerEvent::Playing(playback_start)));
                        self.active = true;
                    }
                    Some(LibrespotPlayerEvent::Paused {
                        play_request_id: _,
                        track_id: _,
                        position_ms,
                    }) => {
                        let position = Duration::from_millis(position_ms as u64);
                        self.events
                            .send(Event::Player(PlayerEvent::Paused(position)));
                        self.active = false;
                    }
                    Some(LibrespotPlayerEvent::Stopped { .. }) => {
                        self.events.send(Event::Player(PlayerEvent::Stopped));
                        self.active = false;
                    }
                    Some(LibrespotPlayerEvent::EndOfTrack { .. }) => {
                        self.events.send(Event::Player(PlayerEvent::FinishedTrack));
                    }
                    None => {
                        warn!("Librespot player event channel died, terminating worker");
                        break
                    },
                    unused_player_event => warn!("unused player event: {unused_player_event:?}")
                },
                event = spirc_events.recv() => match event {
                    Some(SpircEvent::Playback(state)) => self.last_state.update_state(state, &self.events),
                    Some(SpircEvent::Device(device_state)) => self.devices.update_devices(device_state, &self.events),
                    None => {
                        warn!("Librespot spirc event channel died, terminating worker");
                        break
                    }
                },
                // todo: maybe handle reconnecting
                _ = spirc_task.as_mut() => {
                    warn!("Spirc shut down unexpectedly, terminating worker");
                    self.connect.session.shutdown();
                    break
                },
                _ = ui_refresh.tick() => {
                    self.events.trigger();
                },
                _ = self.token_task.as_mut() => {
                    info!("token updated!");
                    self.token_task = Box::pin(futures::future::pending());
                },
            }
        }
    }
}
