use std::path::Path;

use eframe::{
    App,
    egui::{self, ThemePreference, Widget},
};
use git2::{ObjectType, Repository, StatusOptions};

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

fn new_app() -> MyApp {
    let repo = Repository::open("./").expect("Cannot open repository");

    println!("head is branch: {}", repo.head().unwrap().is_branch());
    println!("head detached: {}", repo.head_detached().unwrap());

    MyApp {
        repo,
        commit_message: String::new(),
        commit_ammend: false,
    }
}

struct MyApp {
    repo: Repository,
    commit_message: String,
    commit_ammend: bool,
}
impl App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                
                ui.heading("Unstaged:");
                let statuses = self
                    .repo
                    .statuses(Some(
                        &mut StatusOptions::new()
                            .show(git2::StatusShow::Workdir)
                            .include_ignored(false)
                            .include_untracked(true)
                            .recurse_untracked_dirs(true),
                    ))
                    .unwrap();
                for status in statuses.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("stage").clicked() {
                            let mut index = self.repo.index().unwrap();
                            index.add_path(Path::new(status.path().unwrap())).unwrap();
                            index.write().unwrap();
                        }
                        if status.status().is_wt_new() {
                            ui.monospace("NEW");
                        }
                        if status.status().is_wt_modified() {
                            ui.monospace("MOD");
                        }
                        if status.status().is_wt_deleted() {
                            ui.monospace("DEL");
                        }
                        if status.status().is_wt_typechange() {
                            ui.monospace("TYP");
                        }
                        ui.label(status.path().unwrap());
                    });
                }
                ui.separator();
                ui.heading("Staged:");
                let statuses = self
                    .repo
                    .statuses(Some(
                        &mut StatusOptions::new()
                            .show(git2::StatusShow::Index)
                            .include_ignored(false)
                            .include_untracked(true),
                    ))
                    .unwrap();
                for status in statuses.iter() {
                    ui.horizontal(|ui| {
                        if ui.button("unstage").clicked() {
                            let head = self.repo.head().unwrap().peel(ObjectType::Commit).unwrap();
                            self.repo
                                .reset_default(Some(&head), [Path::new(status.path().unwrap())])
                                .unwrap();
                        }
                        if status.status().is_index_new() {
                            ui.monospace("NEW");
                        }
                        if status.status().is_index_modified() {
                            ui.monospace("MOD");
                        }
                        if status.status().is_index_deleted() {
                            ui.monospace("DEL");
                        }
                        if status.status().is_index_typechange() {
                            ui.monospace("TYP");
                        }
                        ui.label(status.path().unwrap());
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
                            let mut index = self.repo.index().unwrap();
                            let tree_oid = index.write_tree().unwrap();
                            let tree = self.repo.find_tree(tree_oid).unwrap();

                            let head = self.repo.head().unwrap();
                            let parent_commit = head.peel_to_commit().unwrap();

                            let commit_oid = self.repo.commit(
                                Some("HEAD"),
                                &self.repo.signature().unwrap(),
                                &self.repo.signature().unwrap(),
                                &self.commit_message,
                                &tree,
                                &[&parent_commit],
                            ).unwrap();
                            println!("Created commit: {:?}", commit_oid);
                        }
                    },
                )
            });
        });
    }
}
