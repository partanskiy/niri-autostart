use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use niri_ipc::Event;

use crate::error::{NiriAutostartError, Result};
use crate::ipc::{EventMessage, EventStream};
use crate::reducer::apply_event;
use crate::state::ActualState;

const HISTORY_LIMIT: usize = 512;

#[derive(Debug, Default)]
struct EventAdapterState {
    state: ActualState,
    seq: u64,
    saw_workspaces: bool,
    saw_windows: bool,
    closed: Option<String>,
    history: VecDeque<(u64, Event)>,
}

pub struct EventAdapter {
    inner: Arc<(Mutex<EventAdapterState>, Condvar)>,
    _applier: thread::JoinHandle<()>,
}

impl EventAdapter {
    pub fn connect(initial_timeout: Duration) -> Result<Self> {
        let stream = EventStream::connect()?;
        let rx = stream.into_receiver();
        let inner = Arc::new((Mutex::new(EventAdapterState::default()), Condvar::new()));
        let applier_inner = Arc::clone(&inner);

        let applier = thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                let (lock, cvar) = &*applier_inner;
                let mut guard = lock.lock().expect("event adapter lock poisoned");

                match message {
                    EventMessage::Event(event) => {
                        guard.seq += 1;
                        let seq = guard.seq;

                        if matches!(event, Event::WorkspacesChanged { .. }) {
                            guard.saw_workspaces = true;
                        }
                        if matches!(event, Event::WindowsChanged { .. }) {
                            guard.saw_windows = true;
                        }

                        apply_event(&mut guard.state, event.clone());
                        guard.history.push_back((seq, event));
                        while guard.history.len() > HISTORY_LIMIT {
                            guard.history.pop_front();
                        }
                    }
                    EventMessage::Closed(message) => {
                        guard.closed = Some(message);
                    }
                }

                cvar.notify_all();
            }
        });

        let adapter = Self {
            inner,
            _applier: applier,
        };
        adapter.wait_for_initial_state(initial_timeout)?;
        Ok(adapter)
    }

    pub fn state(&self) -> ActualState {
        self.inner
            .0
            .lock()
            .expect("event adapter lock poisoned")
            .state
            .clone()
    }

    pub fn wait_for<F>(&self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        self.wait_loop(timeout, what, |guard| predicate(&guard.state))
    }

    pub fn wait_for_stable<F>(
        &self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
        predicate: F,
    ) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;

            if predicate(&guard.state) {
                let wait_for = quiet_for.min(remaining);
                let stable_seq = guard.seq;
                let (next_guard, wait_result) = cvar
                    .wait_timeout(guard, wait_for)
                    .expect("event adapter lock poisoned");
                guard = next_guard;

                if wait_result.timed_out() && guard.seq == stable_seq && predicate(&guard.state) {
                    if wait_for == quiet_for {
                        return Ok(());
                    }

                    return Err(NiriAutostartError::Timeout {
                        what: what.clone(),
                        timeout,
                    });
                }

                continue;
            }

            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, remaining)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && !predicate(&guard.state) {
                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }

    pub fn wait_until_quiet(
        &self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
    ) -> Result<()> {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;
            let wait_for = quiet_for.min(remaining);
            let stable_seq = guard.seq;
            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, wait_for)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && guard.seq == stable_seq {
                if wait_for == quiet_for {
                    return Ok(());
                }

                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }

    fn wait_for_initial_state(&self, timeout: Duration) -> Result<()> {
        self.wait_loop(
            timeout,
            "initial niri event-stream state".to_string(),
            |guard| guard.saw_workspaces && guard.saw_windows,
        )
    }

    fn wait_loop<F>(&self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&EventAdapterState) -> bool,
    {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if predicate(&guard) {
                return Ok(());
            }

            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;
            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, remaining)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && !predicate(&guard) {
                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }
}
