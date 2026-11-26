//! Driving sync from a desktop app (docs/roadmap/per-node-sync.md §6): network calls run on
//! two threads of their own, one pushing and one pulling, so a pull waiting on the backend
//! (long polling) never holds up an edit going out. The engine and the workspace stay on the
//! thread that owns them: [`SyncDriver::pump`] takes what the threads finished and hands them
//! the next calls. The threads call a wake function when they finish one, for the app to
//! schedule a pump; the app also pumps after each edit it saves.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use flowie_sync::{Backend, ChangeBatch, Engine, Envelope, RecordStore, SaveResult};

use crate::Workspace;

/// Records per save or fetch.
const BATCH: usize = 200;
/// The longest wait after a failed call, which doubles from a second up to this.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Where sync stands, for the app to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncPhase {
    /// Sending or fetching, or not caught up with the backend yet.
    Syncing,
    /// Everything here is saved and nothing new is waiting on the backend.
    Synced,
    /// The last call failed (no network, the server down, a bad token); it is tried again.
    Offline(String),
}

/// Network calls the threads make.
enum Call {
    Save(Vec<Envelope>),
    Fetch { cursor: Option<String>, wait: Duration },
}

/// What they answer.
enum Answer {
    Saved(Result<Vec<SaveResult>, String>, Vec<String>),
    Fetched(Result<ChangeBatch, String>),
}

/// Sync with one backend for an open workspace.
pub struct SyncDriver {
    engine: Engine,
    saves: Sender<(Duration, Call)>,
    fetches: Sender<(Duration, Call)>,
    answers: Receiver<Answer>,
    saving: bool,
    fetching: bool,
    /// The last fetch had nothing more waiting after it, so the next one can wait.
    caught_up: bool,
    save_failures: u32,
    fetch_failures: u32,
    error: Option<String>,
    /// Saves wait until the first fetch has caught up (see `hold_saves`).
    holding: bool,
    /// Records fetched so far.
    received: usize,
    /// How long a fetch waits on the backend once caught up.
    pub long_poll: Duration,
}

/// The wait after `failures` failed calls in a row.
fn backoff(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        n => Duration::from_secs(1 << (n - 1).min(6)).min(MAX_BACKOFF),
    }
}

fn spawn_lane(
    mut backend: Box<dyn Backend + Send>,
    answers: Sender<Answer>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Sender<(Duration, Call)> {
    let (tx, rx) = channel::<(Duration, Call)>();
    std::thread::Builder::new()
        .name("flowie-sync".into())
        .spawn(move || {
            // Ends when the driver is dropped (its sender goes) or nobody listens any more.
            while let Ok((delay, call)) = rx.recv() {
                std::thread::sleep(delay);
                let answer = match call {
                    Call::Save(saves) => {
                        let ids = saves.iter().map(|e| e.id.clone()).collect();
                        Answer::Saved(backend.modify(saves), ids)
                    }
                    Call::Fetch { cursor, wait } => {
                        backend.set_wait(wait);
                        Answer::Fetched(backend.changes(cursor.as_deref(), BATCH))
                    }
                };
                if answers.send(answer).is_err() {
                    break;
                }
                wake();
            }
        })
        .expect("a thread for sync");
    tx
}

impl SyncDriver {
    /// Start syncing through backends from `connect` (one per thread). `wake` is called from
    /// those threads whenever an answer is ready to [`pump`](Self::pump).
    pub fn start(connect: impl Fn() -> Box<dyn Backend + Send>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let (answer_tx, answers) = channel();
        SyncDriver {
            engine: Engine::new(),
            saves: spawn_lane(connect(), answer_tx.clone(), wake.clone()),
            fetches: spawn_lane(connect(), answer_tx, wake),
            answers,
            saving: false,
            fetching: false,
            caught_up: false,
            save_failures: 0,
            fetch_failures: 0,
            error: None,
            holding: false,
            received: 0,
            long_poll: Duration::from_secs(30),
        }
    }

    /// Take the answers that came in, then start the next save and fetch. Call it when woken,
    /// and after saving edits (`Workspace::persist`) so they go out at once. Returns whether
    /// anything from the backend changed the tree.
    pub fn pump(&mut self, ws: &mut Workspace) -> bool {
        let revision = ws.tree.revision();
        while let Ok(answer) = self.answers.try_recv() {
            match answer {
                Answer::Saved(Ok(results), _) => {
                    self.saving = false;
                    self.save_failures = 0;
                    self.engine.on_saved(ws, results);
                }
                Answer::Saved(Err(error), ids) => {
                    self.saving = false;
                    self.save_failures += 1;
                    let failed = ids.into_iter().map(|id| SaveResult::Failed { id, error: error.clone() }).collect();
                    self.engine.on_saved(ws, failed);
                    self.error = Some(error);
                }
                Answer::Fetched(Ok(batch)) => {
                    self.fetching = false;
                    self.fetch_failures = 0;
                    self.caught_up = !batch.more;
                    self.received += batch.records.len();
                    self.engine.on_fetched(ws, batch);
                }
                Answer::Fetched(Err(error)) => {
                    self.fetching = false;
                    self.fetch_failures += 1;
                    self.error = Some(error);
                }
            }
        }
        if self.save_failures == 0 && self.fetch_failures == 0 {
            self.error = None;
        }
        if !self.saving && !self.holding {
            let saves = self.engine.next_saves(ws, BATCH);
            if !saves.is_empty() && self.saves.send((backoff(self.save_failures), Call::Save(saves))).is_ok() {
                self.saving = true;
            }
        }
        if !self.fetching {
            let wait = if self.caught_up { self.long_poll } else { Duration::ZERO };
            let call = Call::Fetch { cursor: ws.cursor(), wait };
            if self.fetches.send((backoff(self.fetch_failures), call)).is_ok() {
                self.fetching = true;
            }
        }
        ws.tree.revision() != revision
    }

    /// Send nothing until the first fetch has caught up with the backend and
    /// [`release_saves`](Self::release_saves) is called: for a first sync, which looks at what
    /// the backend has before deciding what to send (an untouched tutorial isn't).
    pub fn hold_saves(&mut self) {
        self.holding = true;
    }

    pub fn release_saves(&mut self) {
        self.holding = false;
    }

    /// The last fetch had nothing more waiting after it.
    pub fn caught_up(&self) -> bool {
        self.caught_up
    }

    /// How many records came from the backend since the driver started.
    pub fn received(&self) -> usize {
        self.received
    }

    /// Where sync stands now.
    pub fn phase(&self, ws: &Workspace) -> SyncPhase {
        match &self.error {
            Some(error) => SyncPhase::Offline(error.clone()),
            None if self.saving || self.holding || !self.caught_up || ws.sync.has_unsent() => SyncPhase::Syncing,
            None => SyncPhase::Synced,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_calls_back_off_up_to_a_minute() {
        let waits: Vec<u64> = (0..10).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [0, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }
}
