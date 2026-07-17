use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::client::Client;
use crate::error::PetError;
use crate::reducer::{Reducer, ReducerOutput};

pub enum WorkerMsg {
    Snapshot(ReducerOutput),
    LogTail {
        id: String,
        lines: Result<Vec<String>, PetError>,
    },
    SendResult {
        id: String,
        result: Result<(), PetError>,
    },
}

pub enum UiCmd {
    FetchLogs(String),
    MarkRead(String),
    SendReply { id: String, text: String },
    RetryNow,
    Shutdown,
}

const BACKOFF: [u64; 4] = [2, 5, 10, 30];

/// Start the daemon worker without depending on any GUI framework.
///
/// `wake` is called after each message is queued. eframe uses it to request a
/// repaint; the Tauri shell uses it only as a lightweight notification because
/// a bridge thread emits the queued messages to the webview.
pub fn spawn_worker(
    client: Client,
    wake: impl Fn() + Send + Sync + 'static,
) -> (Sender<UiCmd>, Receiver<WorkerMsg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<WorkerMsg>();
    let wake = Arc::new(wake);

    std::thread::spawn(move || {
        let mut reducer = Reducer::new();
        let mut fail_streak: usize = 0;
        let mut next_poll = Instant::now();

        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    UiCmd::Shutdown => return,
                    UiCmd::RetryNow => next_poll = Instant::now(),
                    UiCmd::MarkRead(id) => reducer.mark_read(&id),
                    UiCmd::FetchLogs(id) => {
                        let lines = client.get_log_tail(&id, 5);
                        let _ = msg_tx.send(WorkerMsg::LogTail { id, lines });
                        wake();
                    }
                    UiCmd::SendReply { id, text } => {
                        let result = client.send_input(&id, &text);
                        let _ = msg_tx.send(WorkerMsg::SendResult { id, result });
                        wake();
                    }
                }
            }

            if Instant::now() >= next_poll {
                match client.get_agents() {
                    Ok(agents) => {
                        fail_streak = 0;
                        let out = reducer.on_snapshot(&agents);
                        let _ = msg_tx.send(WorkerMsg::Snapshot(out));
                        next_poll = Instant::now() + Duration::from_secs(2);
                    }
                    Err(PetError::Transport) => {
                        let out = reducer.on_disconnect();
                        let _ = msg_tx.send(WorkerMsg::Snapshot(out));
                        let idx = fail_streak.min(BACKOFF.len() - 1);
                        next_poll = Instant::now() + Duration::from_secs(BACKOFF[idx]);
                        fail_streak += 1;
                    }
                    Err(_) => {
                        // The daemon answered, so keep the last good snapshot
                        // and retry on the normal cadence. Replies are never
                        // retried by this worker.
                        next_poll = Instant::now() + Duration::from_secs(2);
                    }
                }
                wake();
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    });

    (cmd_tx, msg_rx)
}
