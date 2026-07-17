use std::sync::mpsc::{self, Receiver, Sender};
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
    /// Mark an agent's attention event read (badge only) once it is shown in the
    /// expanded bubble. The reducer lives on this worker thread, so read state is
    /// applied here and reflected on the next snapshot.
    MarkRead(String),
    SendReply {
        id: String,
        text: String,
    },
    RetryNow,
    Shutdown,
}

const BACKOFF: [u64; 4] = [2, 5, 10, 30];

pub fn spawn_worker(client: Client, ctx: egui::Context) -> (Sender<UiCmd>, Receiver<WorkerMsg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<WorkerMsg>();

    std::thread::spawn(move || {
        let mut reducer = Reducer::new();
        let mut fail_streak: usize = 0;
        let mut next_poll = Instant::now();

        loop {
            // Drain UI commands (non-blocking) before deciding to poll.
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    UiCmd::Shutdown => return,
                    UiCmd::RetryNow => next_poll = Instant::now(),
                    UiCmd::MarkRead(id) => reducer.mark_read(&id),
                    UiCmd::FetchLogs(id) => {
                        let lines = client.get_log_tail(&id, 5);
                        let _ = msg_tx.send(WorkerMsg::LogTail { id, lines });
                        ctx.request_repaint();
                    }
                    UiCmd::SendReply { id, text } => {
                        let result = client.send_input(&id, &text);
                        let _ = msg_tx.send(WorkerMsg::SendResult { id, result });
                        ctx.request_repaint();
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
                        // The daemon is unreachable: show disconnected and back off.
                        let out = reducer.on_disconnect();
                        let _ = msg_tx.send(WorkerMsg::Snapshot(out));
                        let idx = fail_streak.min(BACKOFF.len() - 1);
                        next_poll = Instant::now() + Duration::from_secs(BACKOFF[idx]);
                        fail_streak += 1;
                    }
                    Err(_) => {
                        // A transient 5xx or a malformed body: the daemon IS
                        // reachable, so retain the last good snapshot (spec: "mark
                        // data stale, and retry") rather than wiping the view.
                        next_poll = Instant::now() + Duration::from_secs(2);
                    }
                }
                ctx.request_repaint();
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    });

    (cmd_tx, msg_rx)
}
