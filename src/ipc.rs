use std::sync::mpsc::{self, Receiver};
use std::thread;

use niri_ipc::socket::Socket;
use niri_ipc::{Action, Event, Reply, Request, Response};

use crate::error::{NiriAutostartError, Result};

#[derive(Debug)]
pub enum EventMessage {
    Event(Event),
    Closed(String),
}

pub struct CommandClient {
    socket: Socket,
}

pub struct EventStream {
    pub rx: Receiver<EventMessage>,
    _reader: thread::JoinHandle<()>,
}

impl CommandClient {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            socket: Socket::connect()?,
        })
    }

    pub fn action(&mut self, action: Action) -> Result<()> {
        let reply = self.socket.send(Request::Action(action))?;
        match reply {
            Ok(Response::Handled) => Ok(()),
            Ok(_) => Err(NiriAutostartError::UnexpectedReply { context: "action" }),
            Err(message) => Err(NiriAutostartError::Niri(message)),
        }
    }

}

impl EventStream {
    pub fn connect() -> Result<Self> {
        let mut socket = Socket::connect()?;
        let reply: Reply = socket.send(Request::EventStream)?;
        match reply {
            Ok(Response::Handled) => {}
            Ok(_) => return Err(NiriAutostartError::UnexpectedReply { context: "event-stream" }),
            Err(message) => return Err(NiriAutostartError::Niri(message)),
        }

        let (tx, rx) = mpsc::channel();
        let mut read_event = socket.read_events();
        let reader = thread::spawn(move || loop {
            match read_event() {
                Ok(event) => {
                    if tx.send(EventMessage::Event(event)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(EventMessage::Closed(err.to_string()));
                    break;
                }
            }
        });

        Ok(Self {
            rx,
            _reader: reader,
        })
    }
}
