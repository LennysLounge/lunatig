use std::ops::Mul;

use eframe::{
    App,
    egui::{self, ThemePreference},
};
use git2::Repository;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
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

    let repo = Repository::open("../egui_ltreeview/").expect("Cannot open repository");
    let mut revwalk = repo.revwalk().unwrap();
    revwalk.push_head().unwrap();

    let mut commits = Vec::new();
    for oid in revwalk{
        let oid = oid.unwrap();
        let commit = repo.find_commit(oid).unwrap();
        commits.push(commit.message().unwrap().to_owned());
    }


    MyApp {
        name: "Arthur".to_owned(),
        age: 42,
        repo,
        commits
    }
}

struct MyApp {
    name: String,
    age: i32,
    repo: Repository,
    commits: Vec<String>,
}
impl App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::MenuBar::default().ui(ui, |ui| {
            ui.label("hi");
            _ = ui.button("file");
            ui.menu_button("file", |ui| {
                ui.label("hi");
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Commits:");
            for msg in self.commits.iter(){
                ui.label(msg);
                ui.separator();
                // if !ui.clip_rect().intersects(ui.cursor()){
                //     break;
                // }
            }
            // ui.heading("My egui Application");
            // ui.horizontal(|ui| {
            //     let name_label = ui.label("Your name: ");
            //     ui.text_edit_singleline(&mut self.name)
            //         .labelled_by(name_label.id);
            // });
            // ui.add(egui::Slider::new(&mut self.age, 0..=120).text("age"));
            // if ui.button("Increment").clicked() {
            //     self.age += 1;
            // }
            // ui.label(format!("Hello '{}', age {}", self.name, self.age));
        });
    }
}
