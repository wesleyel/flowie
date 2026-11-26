//! Sync with a self-hosted server (Settings → Sync, docs/roadmap/per-node-sync.md §6): when
//! the driver runs, how the outline feeds it, and what the window shows of it.

use std::time::Duration;

use flowie_doc::{SyncDriver, SyncPhase};
use flowie_sync::http::HttpBackend;
use flowie_sync::RecordStore;
use gpui::{Context, Task};
use serde::{Deserialize, Serialize};

use flowie_core::{NodeId, ROOT};

use super::{Field, Outline, Target};

/// The outline's name on the server. One outline per server for now.
const ZONE: &str = "outline";
/// A tutorial with no write later than this after it was added is untouched.
const SEEDING: u64 = 10_000;
/// How often sync looks in while nothing wakes it: to apply what was held for the row being
/// edited once editing ends, and to send what an agent saved.
const IDLE_PUMP: Duration = Duration::from_secs(2);

/// Sync as Settings → Sync sets it up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    /// The server's address (`https://sync.example.com`).
    #[serde(default)]
    pub server: String,
    /// This device's token, from `flowie-server token add`.
    #[serde(default)]
    pub token: String,
}

impl SyncConfig {
    fn ready(&self) -> bool {
        self.enabled && !self.server.trim().is_empty() && !self.token.trim().is_empty()
    }

    /// The name sync keeps its place under, so another server starts afresh.
    fn remote(&self) -> String {
        format!("{}#{ZONE}", self.server.trim().trim_end_matches('/'))
    }
}

/// Sync running for this window.
pub struct SyncSession {
    config: SyncConfig,
    driver: SyncDriver,
    phase: SyncPhase,
    /// Never synced with this server before: saves wait for the first fetch (`pump_sync`).
    first: bool,
    /// The untouched tutorial this device had before the first fetch.
    tutorial: Option<NodeId>,
    /// When everything was last in sync (local time, for Settings).
    pub last_synced: Option<chrono::DateTime<chrono::Local>>,
    _pump: Task<()>,
}

impl SyncSession {
    pub fn phase(&self) -> &SyncPhase {
        &self.phase
    }
}

impl Outline {
    /// Start, restart or stop sync to match the settings. Not while a sync field is being
    /// typed in: that would connect with half an address.
    pub(crate) fn apply_sync_config(&mut self, cx: &mut Context<Self>) {
        if matches!(self.edit_target(), Some(Target::Field(Field::SyncServer | Field::SyncToken))) {
            return;
        }
        let config = self.config.sync.clone();
        if self.sync.as_ref().is_some_and(|s| s.config == config) || (self.sync.is_none() && !config.ready()) {
            return;
        }
        self.sync = None;
        cx.notify();
        if !config.ready() || self.ws.writer.is_none() {
            return;
        }
        self.ws.set_sync_remote(&config.remote());
        let first = self.ws.cursor().is_none();
        if first {
            // Never synced with this server: keep the outline as it was before merging.
            if let Err(error) = self.snapshot_before_sync() {
                self.toast(crate::app::i18n::t!("sync.snapshot_failed", error = error));
                return;
            }
        }
        let (wake, mut woken) = futures::channel::mpsc::unbounded::<()>();
        let (server, token) = (config.server.trim().to_string(), config.token.trim().to_string());
        let tutorial = if first { self.untouched_tutorial() } else { None };
        let mut driver = SyncDriver::start(
            move || Box::new(HttpBackend::new(&server, ZONE, &token)),
            move || {
                let _ = wake.unbounded_send(());
            },
        );
        if first {
            driver.hold_saves();
        }
        let pump = cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            loop {
                let next = woken.next();
                let idle = cx.background_executor().timer(IDLE_PUMP);
                futures::pin_mut!(next);
                if let futures::future::Either::Left((None, _)) = futures::future::select(next, idle).await {
                    break;
                }
                if this.update(cx, |this, cx| this.pump_sync(cx)).is_err() {
                    break;
                }
            }
        });
        self.sync = Some(SyncSession { config, driver, phase: SyncPhase::Syncing, first, tutorial, last_synced: None, _pump: pump });
        self.pump_sync(cx);
    }

    /// Hand sync what changed here and take what came in. Called when sync wakes, every
    /// `IDLE_PUMP`, and after each save so edits go out at once.
    pub(crate) fn pump_sync(&mut self, cx: &mut Context<Self>) {
        if self.sync.is_none() {
            return;
        }
        // What comes in for the row being edited waits until editing ends.
        let editing = self.editing_node().map(|id| self.ws.tree.source(id));
        let mut changed = match editing {
            Some(id) => !self.ws.begin_edit(id).is_empty(),
            None => !self.ws.end_edit().is_empty(),
        };
        let Some(session) = self.sync.as_mut() else { return };
        changed |= session.driver.pump(&mut self.ws);
        if session.first && session.driver.caught_up() {
            // The first fetch is in: a fresh device joining an outline doesn't add its own
            // copy of the tutorial to it.
            session.first = false;
            let joined = session.driver.received() > 0;
            let tutorial = session.tutorial.take().filter(|&root| joined && self.untouched(root));
            if let Some(tutorial) = tutorial {
                self.ws.discard_unsent(tutorial);
                changed = true;
            }
            let Some(session) = self.sync.as_mut() else { return };
            session.driver.release_saves();
            session.driver.pump(&mut self.ws);
        }
        let Some(session) = self.sync.as_mut() else { return };
        let phase = session.driver.phase(&self.ws);
        if phase == SyncPhase::Synced {
            session.last_synced = Some(chrono::Local::now());
        }
        let shown = std::mem::replace(&mut session.phase, phase) != session.phase;
        if changed {
            self.refresh(false);
        }
        if changed || shown {
            cx.notify();
        }
    }

    /// The tutorial this device added, if it is all the outline holds and nobody has touched
    /// it: no write to it later than `SEEDING` after it was added, and nothing in the Trash.
    fn untouched_tutorial(&self) -> Option<NodeId> {
        let root = self.tutorial?;
        (self.ws.tree.children(ROOT) == [root] && self.ws.tree.trashed().is_empty() && self.untouched(root)).then_some(root)
    }

    /// Nothing in the subtree at `root` was written later than `SEEDING` after it was added.
    fn untouched(&self, root: NodeId) -> bool {
        let tree = &self.ws.tree;
        let Some(added) = tree.record(root).map(|n| n.sync.clocks.created.wall) else { return false };
        let mut ids = tree.descendants(root);
        ids.push(root);
        ids.iter().all(|&id| tree.record(id).is_some_and(|n| n.sync.clocks.max().wall <= added + SEEDING))
    }

    /// Copy the database beside itself before the first sync with a server.
    fn snapshot_before_sync(&mut self) -> Result<(), String> {
        let Some(db) = self.db_path.clone() else { return Ok(()) };
        self.ws.flush()?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!("{}.before-sync-{stamp}", db.file_name().and_then(|n| n.to_str()).unwrap_or("flowie.sqlite"));
        flowie_core::store::snapshot(&db, &db.with_file_name(name)).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_starts_only_with_an_address_and_a_token() {
        let mut config = SyncConfig { enabled: true, server: " https://sync.example.com/ ".into(), token: "t".into() };
        assert!(config.ready());
        assert_eq!(config.remote(), "https://sync.example.com#outline");
        config.token = "  ".into();
        assert!(!config.ready());
        config = SyncConfig { enabled: false, ..SyncConfig { enabled: true, server: "s".into(), token: "t".into() } };
        assert!(!config.ready());
    }
}
