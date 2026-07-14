mod repo;

use std::sync::mpsc::TryRecvError;

use eframe::{
    App,
    egui::{self, ThemePreference, Widget},
};

use crate::repo::{Command, Repo, open_repository};

fn main() -> eframe::Result<()> {
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
    let repo_2 = open_repository("./");
    repo_2.command_tx.send(Command::GetStatuses).unwrap();

    Lunatig {
        commit_message: String::new(),
        commit_ammend: false,
        repo_2,
    }
}

struct Lunatig {
    commit_message: String,
    commit_ammend: bool,
    repo_2: Repo,
}
impl App for Lunatig {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        loop {
            let event = match self.repo_2.event_rx.try_recv() {
                Ok(e) => e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    panic!("Channel disconnected");
                }
            };

            match event {
                repo::Event::UnstagedFiles(unstaged_files) => {
                    self.repo_2.unstaged_files = unstaged_files;
                }
                repo::Event::StagedFiles(staged_files) => {
                    self.repo_2.staged_files = staged_files;
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Unstaged:");
                for status in self.repo_2.unstaged_files.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("stage").clicked() {
                            self.repo_2
                                .command_tx
                                .send(Command::StageFile {
                                    path: status.path.clone(),
                                })
                                .unwrap();
                        }
                        match status.status_type {
                            repo::FileStatusStatus::New => ui.monospace("NEW"),
                            repo::FileStatusStatus::Modified => ui.monospace("MOD"),
                            repo::FileStatusStatus::Deleted => ui.monospace("DEL"),
                            repo::FileStatusStatus::TypeChanged => ui.monospace("TYP"),
                        };
                        ui.label(&status.path);
                    });
                }
                ui.separator();
                ui.heading("Staged:");
                for status in self.repo_2.staged_files.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("unstage").clicked() {
                            self.repo_2
                                .command_tx
                                .send(Command::ResetStagedFile {
                                    path: status.path.clone(),
                                })
                                .unwrap();
                        }
                        match status.status_type {
                            repo::FileStatusStatus::New => ui.monospace("NEW"),
                            repo::FileStatusStatus::Modified => ui.monospace("MOD"),
                            repo::FileStatusStatus::Deleted => ui.monospace("DEL"),
                            repo::FileStatusStatus::TypeChanged => ui.monospace("TYP"),
                        };
                        ui.label(&status.path);
                    });
                }
                ui.separator();
                egui::text_edit::TextEdit::multiline(&mut self.commit_message)
                    .desired_width(ui.available_width())
                    .ui(ui);
                egui::containers::Sides::new().show(
                    ui,
                    |ui| {
                        // todo
                        ui.checkbox(&mut self.commit_ammend, "ammend");
                    },
                    |ui| {
                        if ui.button("commit").clicked() {
                            self.repo_2
                                .command_tx
                                .send(Command::Commit {
                                    message: self.commit_message.clone(),
                                })
                                .unwrap();
                        }
                    },
                )
            });
        });
    }
}
