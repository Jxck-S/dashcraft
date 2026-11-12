//! Pushed external changes (M8.2) for `dashcraft serve` and `dashcraft mcp`:
//! every interval, each open document's project files are scanned as
//! `doc.externalChanges` does (sizes and timestamps; PBIP projects only),
//! and a change of the set of files changed on disk is announced as an
//! [`ExternalChangedEvent`]. A scan takes its turn among the document's
//! mutating requests, so a save, reload or refresh of this server is never
//! scanned halfway (its own writes are not reported); at most one scan per
//! document is pending.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::commands::edit::FileChangeOut;
use crate::rpc::DocQueue;
use crate::workspace::{Workspace, lock};

/// Default interval between two scans of the open documents.
pub const DEFAULT_EXTERNAL_POLL: Duration = Duration::from_millis(1500);

/// The files another program changed under an open document, sent when
/// they differ from the last event of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChangedEvent {
    /// The document id.
    pub doc_id: String,
    /// Every project file added, changed or removed on disk since the open
    /// or last save, by path (what `doc.externalChanges` returns); empty
    /// when none is left (after a save or reload).
    pub changes: Vec<FileChangeOut>,
}

/// Starts the scans of `ws`'s open documents every `interval` until `stop`
/// is cancelled; `notify` receives each change and returns whether it was
/// delivered (an undelivered set is offered again). Await the handle after
/// cancelling to let the running scans finish.
pub(crate) fn spawn_external_watch<F>(
    ws: Arc<Workspace>,
    queue: Arc<DocQueue>,
    interval: Duration,
    stop: CancellationToken,
    notify: F,
) -> JoinHandle<()>
where
    F: Fn(ExternalChangedEvent) -> bool + Send + Sync + 'static,
{
    let notify = Arc::new(notify);
    tokio::spawn(async move {
        let last: Arc<Mutex<HashMap<String, Vec<FileChangeOut>>>> = Arc::default();
        let pending: Arc<Mutex<HashSet<String>>> = Arc::default();
        let mut scans = JoinSet::new();
        let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(10)));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                _ = ticker.tick() => {}
            }
            while scans.try_join_next().is_some() {}
            let documents = ws.documents();
            {
                let open: HashSet<&str> = documents.iter().map(|e| e.id()).collect();
                lock(&last).retain(|id, _| open.contains(id.as_str()));
            }
            for entry in documents {
                let id = entry.id().to_owned();
                if !lock(&pending).insert(id.clone()) {
                    continue;
                }
                let mut turn = queue.enter(Some(id.clone()));
                let (last, pending, notify, stop) = (
                    Arc::clone(&last),
                    Arc::clone(&pending),
                    Arc::clone(&notify),
                    stop.clone(),
                );
                scans.spawn(async move {
                    if turn.wait(&stop).await {
                        match entry.session().external_changes().await {
                            Ok(Some(changes)) => {
                                let changes: Vec<FileChangeOut> =
                                    changes.into_iter().map(FileChangeOut::from).collect();
                                let changed = lock(&last).get(&id).map_or(&[][..], Vec::as_slice)
                                    != changes.as_slice();
                                if changed
                                    && notify(ExternalChangedEvent {
                                        doc_id: id.clone(),
                                        changes: changes.clone(),
                                    })
                                {
                                    lock(&last).insert(id.clone(), changes);
                                }
                            }
                            Ok(None) => {}
                            Err(e) => tracing::debug!("external change scan of {id}: {e}"),
                        }
                    }
                    drop(turn);
                    lock(&pending).remove(&id);
                });
            }
        }
        while scans.join_next().await.is_some() {}
    })
}
