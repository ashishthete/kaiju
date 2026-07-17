use eframe::egui;

use crate::model::Status;
use crate::poller::{UiCmd, WorkerMsg};
use crate::reducer::{ReducerOutput, VisualState};

pub struct PetApp {
    cmd_tx: std::sync::mpsc::Sender<UiCmd>,
    msg_rx: std::sync::mpsc::Receiver<WorkerMsg>,
    prefs: crate::prefs::Prefs,
    out: ReducerOutput,
    expanded: bool,
    selected: Option<String>,
    // The selection we have already fetched logs and marked-read for, so those
    // one-shot commands do not fire on every repaint while the bubble is open.
    shown_for: Option<String>,
    reply_text: String,
    send_in_flight: bool,
    status_line: String,
    log_lines: Vec<String>,
}

impl PetApp {
    pub fn new(
        cmd_tx: std::sync::mpsc::Sender<UiCmd>,
        msg_rx: std::sync::mpsc::Receiver<WorkerMsg>,
        prefs: crate::prefs::Prefs,
    ) -> Self {
        PetApp {
            cmd_tx,
            msg_rx,
            prefs,
            out: ReducerOutput {
                visual: VisualState::Idle,
                ordered_attention: Vec::new(),
                new_events: Vec::new(),
                unread_count: 0,
            },
            expanded: false,
            selected: None,
            shown_for: None,
            reply_text: String::new(),
            send_in_flight: false,
            status_line: String::new(),
            log_lines: Vec::new(),
        }
    }

    fn drain_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                WorkerMsg::Snapshot(out) => {
                    // Per the spec's default policy the cue fires for waiting/stuck/
                    // error transitions but NOT limited or completed. `new_events`
                    // only ever carries attention statuses.
                    let wants_sound = self.prefs.sound
                        && out.new_events.iter().any(|e| {
                            matches!(
                                e.status,
                                Status::WaitingForInput | Status::Stuck | Status::Error
                            )
                        });
                    if wants_sound {
                        crate::window::play_cue();
                    }
                    // Keep a valid selection; default to the top attention agent.
                    if self.selected.is_none() {
                        self.selected = out.ordered_attention.first().map(|a| a.id.clone());
                    }
                    self.out = out;
                }
                WorkerMsg::LogTail { id: _, lines } => {
                    self.log_lines = lines.unwrap_or_else(|_| vec!["Context unavailable".into()]);
                }
                WorkerMsg::SendResult { id: _, result } => {
                    self.send_in_flight = false;
                    match result {
                        Ok(()) => {
                            self.reply_text.clear();
                            self.status_line = "Sent".into();
                        }
                        Err(e) => self.status_line = format!("{e}"),
                    }
                }
            }
        }
    }

    fn label_for(v: VisualState) -> &'static str {
        match v {
            VisualState::Disconnected => "Disconnected",
            VisualState::Idle => "All quiet",
            VisualState::Working => "Working",
            VisualState::Waiting => "Needs you",
            VisualState::Stuck => "Stuck",
            VisualState::Limited => "Rate-limited",
            VisualState::Error => "Error",
            VisualState::Celebrate => "Done!",
        }
    }

    fn draw_bubble(&mut self, ui: &mut egui::Ui) {
        let attention = self.out.ordered_attention.clone();
        ui.label(format!(
            "{} · {} need you",
            Self::label_for(self.out.visual),
            self.out.unread_count
        ));

        // Drop a selection that has left the attention list, then fall back to the
        // top attention agent. Without this the bubble stays stuck on a departed
        // agent and — since the combo only renders with >1 agent — the operator
        // cannot re-select the last remaining one (spec: "Agent disappears:
        // remove it from selection").
        if let Some(sel) = &self.selected {
            if !attention.iter().any(|a| &a.id == sel) {
                self.selected = None;
            }
        }
        if self.selected.is_none() {
            self.selected = attention.first().map(|a| a.id.clone());
        }

        if attention.len() > 1 {
            egui::ComboBox::from_label("Agent")
                .selected_text(self.selected.clone().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for a in &attention {
                        ui.selectable_value(&mut self.selected, Some(a.id.clone()), &a.id);
                    }
                });
        }

        let Some(sel_id) = self.selected.clone() else {
            return;
        };
        let Some(agent) = attention.iter().find(|a| a.id == sel_id).cloned() else {
            return;
        };

        // Once per selection: clear the previous reply (spec/security: reply text
        // is cleared when switching agents, so a message typed for A can never be
        // sent to B), mark the event read, and load logs.
        if self.shown_for.as_deref() != Some(sel_id.as_str()) {
            self.shown_for = Some(sel_id.clone());
            self.reply_text.clear();
            self.status_line.clear();
            self.log_lines.clear();
            self.cmd_tx.send(UiCmd::MarkRead(sel_id.clone())).ok();
            self.cmd_tx.send(UiCmd::FetchLogs(sel_id.clone())).ok();
        }

        ui.separator();
        ui.label(format!(
            "{} · {} · {:?}",
            &agent.id, &agent.agent_type, agent.status
        ));
        if let Some(p) = &agent.prompt {
            ui.label(egui::RichText::new(truncate_two_lines(p)).weak());
        }
        for line in &self.log_lines {
            ui.small(line);
        }

        ui.separator();
        let terminal = agent.status.is_terminal() || self.out.visual == VisualState::Disconnected;
        if terminal {
            if ui.button("Open dashboard").clicked() {
                let _ = open_dashboard(&self.prefs.daemon_url);
            }
            return;
        }

        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                !self.send_in_flight,
                egui::TextEdit::singleline(&mut self.reply_text).desired_width(150.0),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let send = ui
                .add_enabled(!self.send_in_flight, egui::Button::new("Send"))
                .clicked();
            if enter || send {
                match crate::validate::validate_reply(&self.reply_text) {
                    Ok(text) => {
                        self.send_in_flight = true;
                        self.status_line.clear();
                        self.cmd_tx
                            .send(UiCmd::SendReply {
                                id: sel_id.clone(),
                                text,
                            })
                            .ok();
                    }
                    Err(_) => self.status_line = "Enter one line of text".into(),
                }
            }
        });
        if !self.status_line.is_empty() {
            ui.small(&self.status_line);
        }
    }
}

impl eframe::App for PetApp {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent
    }

    // eframe 0.35 splits the old `update` into `logic` (non-UI state, still runs
    // when the window is hidden but a repaint was requested) and `ui` (painting,
    // handed a `Ui` for the root — no CentralPanel needed).
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let badge = if self.out.unread_count > 0 {
            format!("  ({})", self.out.unread_count)
        } else {
            String::new()
        };
        let pet = ui.button(format!("🤖 {}{}", Self::label_for(self.out.visual), badge));
        if pet.clicked() {
            self.expanded = !self.expanded;
            if !self.expanded {
                // Re-open should re-fetch logs and re-mark the selection.
                self.shown_for = None;
            }
        }
        if self.expanded {
            self.draw_bubble(ui);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.expanded = false;
                self.shown_for = None;
            }
        }
    }
}

/// Truncate a prompt to roughly two lines without splitting a UTF-8 char.
fn truncate_two_lines(s: &str) -> String {
    const MAX: usize = 140;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX).collect();
    format!("{truncated}…")
}

fn open_dashboard(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Ok(())
    }
}
