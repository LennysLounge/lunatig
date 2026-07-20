mod backend;

use std::sync::mpsc::TryRecvError;

use eframe::{
    App,
    egui::{self, Layout, ThemePreference, Widget},
};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};

use crate::backend::{Command, Event, FileStatus, FileStatusStatus};

fn main() -> eframe::Result<()> {
    // 1. Start with your hardcoded, baseline rules
    let mut filter = EnvFilter::new("warn")
        .add_directive("wgpu_hal=error".parse().unwrap())
        .add_directive("egui_wgpu=error".parse().unwrap())
        .add_directive("lunatig=debug".parse().unwrap());

    // 2. If RUST_LOG is set, merge its directives *on top* of the baseline
    if let Ok(rust_log) = std::env::var("RUST_LOG") {
        for directive in rust_log.split(',') {
            match directive.parse() {
                Ok(parsed) => filter = filter.add_directive(parsed),
                Err(e) => eprintln!("Could not parse logging directive: '{directive}', error: {e}"),
            }
        }
    }

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "My egui App",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(ThemePreference::Dark);
            Ok(Box::new(new_app()))
        }),
    )
}

fn new_app() -> Lunatig {
    let repo = open_repository("./");
    repo.command_tx.send(Command::GetStatuses).unwrap();

    Lunatig { repo }
}

fn open_repository(path: &str) -> Repo {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let repo_path = path.to_owned();
    let backend_thread = backend::start_backend(repo_path, command_rx, event_tx);
    Repo {
        unstaged_files: Vec::new(),
        staged_files: Vec::new(),
        backend_handle: Some(backend_thread),
        command_tx,
        event_rx,
        commit_message: String::new(),
        commit_ammend: false,
    }
}

struct Repo {
    unstaged_files: Vec<FileStatus>,
    staged_files: Vec<FileStatus>,
    backend_handle: Option<JoinHandle<()>>,
    command_tx: Sender<Command>,
    event_rx: Receiver<Event>,
    commit_message: String,
    commit_ammend: bool,
}

impl Repo {
    fn send_command(&self, command: Command) {
        if let Err(e) = self.command_tx.send(command) {
            error!("Error sending command: {e}");
        }
    }
    fn receive_event(&mut self) -> Option<Event> {
        match self.event_rx.try_recv() {
            Ok(e) => Some(e),
            Err(TryRecvError::Empty) => None,
            Err(e) => {
                error!("Error receiving event, error: {e}. Close backend");
                self.send_command(Command::Close);
                self.backend_handle = None;
                None
            }
        }
    }
    fn is_backend_alive(&self) -> bool {
        if let Some(handle) = &self.backend_handle {
            !handle.is_finished()
        } else {
            false
        }
    }
}

struct Lunatig {
    repo: Repo,
}
impl App for Lunatig {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.repo.is_backend_alive() {
            while let Some(event) = self.repo.receive_event() {
                match event {
                    Event::UnstagedFiles(unstaged_files) => {
                        self.repo.unstaged_files = unstaged_files;
                    }
                    Event::StagedFiles(staged_files) => {
                        self.repo.staged_files = staged_files;
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if !self.repo.is_backend_alive() {
                    ui.horizontal(|ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("Backend offline");
                        });
                    });
                }
                ui.horizontal_top(|ui| {
                    if ui.button("push").clicked() {
                        self.repo.send_command(Command::Push);
                    }
                    if ui.button("fetch").clicked() {
                        self.repo.send_command(Command::Fetch);
                    }
                });
                ui.heading("Unstaged:");
                for status in self.repo.unstaged_files.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("stage").clicked() {
                            self.repo.send_command(Command::StageFile {
                                path: status.path.clone(),
                            });
                        }
                        match status.status_type {
                            FileStatusStatus::New => ui.monospace("NEW"),
                            FileStatusStatus::Modified => ui.monospace("MOD"),
                            FileStatusStatus::Deleted => ui.monospace("DEL"),
                            FileStatusStatus::TypeChanged => ui.monospace("TYP"),
                        };
                        ui.label(&status.path);
                    });
                }
                ui.separator();
                ui.heading("Staged:");
                for status in self.repo.staged_files.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("unstage").clicked() {
                            self.repo.send_command(Command::ResetStagedFile {
                                path: status.path.clone(),
                            });
                        }
                        match status.status_type {
                            FileStatusStatus::New => ui.monospace("NEW"),
                            FileStatusStatus::Modified => ui.monospace("MOD"),
                            FileStatusStatus::Deleted => ui.monospace("DEL"),
                            FileStatusStatus::TypeChanged => ui.monospace("TYP"),
                        };
                        ui.label(&status.path);
                    });
                }
                ui.separator();
                egui::text_edit::TextEdit::multiline(&mut self.repo.commit_message)
                    .desired_width(ui.available_width())
                    .ui(ui);

                ui.horizontal(|ui| {
                    // todo
                    ui.checkbox(&mut self.repo.commit_ammend, "ammend");
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("commit").clicked() {
                            self.repo.send_command(Command::Commit {
                                message: self.repo.commit_message.clone(),
                            });
                        }
                    });
                });
            });
        });
    }
}
