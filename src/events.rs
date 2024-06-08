use crossbeam_channel::{unbounded, Receiver, Sender, TryIter};
use cursive::{CbSink, Cursive};

use crate::spotify::PlayerEvent;
use crate::spotify_connect::{ConnectEvent, DeviceEvent};

pub enum Event {
    Connect(ConnectEvent),
    Device(DeviceEvent),
    Player(PlayerEvent),
    SessionDied,
    IpcInput(String),
}

pub type EventSender = Sender<Event>;

#[derive(Clone)]
pub struct EventManager {
    tx: EventSender,
    rx: Receiver<Event>,
    cursive_sink: CbSink,
}

impl EventManager {
    pub fn new(cursive_sink: CbSink) -> EventManager {
        let (tx, rx) = unbounded();

        EventManager {
            tx,
            rx,
            cursive_sink,
        }
    }

    pub fn msg_iter(&self) -> TryIter<Event> {
        self.rx.try_iter()
    }

    pub fn send(&self, event: Event) {
        self.tx.send(event).expect("could not send event");
        self.trigger();
    }

    pub fn trigger(&self) {
        // send a no-op to trigger event loop processing
        self.cursive_sink
            .send(Box::new(Cursive::noop))
            .expect("could not send no-op event to cursive");
    }
}
