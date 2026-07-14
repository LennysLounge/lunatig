use std::{
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use git2::{ObjectType, Oid, Repository, StatusOptions};

pub struct Repo {
    pub unstaged_files: Vec<FileStatus>,
    pub staged_files: Vec<FileStatus>,
    #[allow(unused)]
    pub backend: Option<JoinHandle<()>>,
    pub command_tx: Sender<Command>,
    pub event_rx: Receiver<Event>,
}

pub enum FileStatusStatus {
    New,
    Modified,
    Deleted,
    TypeChanged,
}

pub struct FileStatus {
    pub path: String,
    pub status_type: FileStatusStatus,
}

pub enum Event {
    UnstagedFiles(Vec<FileStatus>),
    StagedFiles(Vec<FileStatus>),
}

pub enum Command {
    #[allow(unused)]
    Close,
    GetStatuses,
    StageFile {
        path: String,
    },
    ResetStagedFile {
        path: String,
    },
    Commit {
        message: String,
    },
}

pub fn open_repository(path: &str) -> Repo {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let repo_path = path.to_owned();
    let backend = thread::spawn(|| {
        Backend::new(repo_path, command_rx, event_tx).run();
    });

    Repo {
        unstaged_files: Vec::new(),
        staged_files: Vec::new(),
        backend: Some(backend),
        command_tx,
        event_rx,
    }
}

struct Backend {
    repo_path: String,
    command_rx: Receiver<Command>,
    event_tx: Sender<Event>,
    repo: Repository,
}
impl Backend {
    fn new(repo_path: String, command_rx: Receiver<Command>, event_tx: Sender<Event>) -> Self {
        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                panic!("Cannot open repository at {repo_path}, error: {e}");
            }
        };
        Backend {
            repo_path,
            command_rx,
            event_tx,
            repo,
        }
    }
    fn run(mut self) {
        loop {
            let command = match self.command_rx.recv() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Command channel closed, error: {e}");
                    break;
                }
            };
            match command {
                Command::Close => {
                    println!("Close backend for {}", self.repo_path);
                    break;
                }
                Command::GetStatuses => {
                    println!("Refreshing statuses");
                    self.send_statuses().unwrap();
                }
                Command::StageFile { path } => {
                    let mut index = self.repo.index().unwrap();
                    index.add_path(Path::new(&path)).unwrap();
                    index.write().unwrap();

                    self.send_statuses().unwrap();
                }
                Command::ResetStagedFile { path } => {
                    {
                        let head = self.repo.head().unwrap().peel(ObjectType::Commit).unwrap();
                        self.repo
                            .reset_default(Some(&head), [Path::new(&path)])
                            .unwrap();
                    }
                    self.send_statuses().unwrap();
                }
                Command::Commit { message } => {
                    let commit_oid = self.commit(&message).unwrap();
                    println!("Created commit: {:?}", commit_oid);

                    self.send_statuses().unwrap();
                }
            }
        }
    }

    fn send_statuses(&mut self) -> anyhow::Result<()> {
        self.send_staged_statuses()?;
        self.send_unstaged_statuses()?;
        Ok(())
    }

    fn send_unstaged_statuses(&mut self) -> anyhow::Result<()> {
        let unstaged_statuses = self.repo.statuses(Some(
            StatusOptions::new()
                .show(git2::StatusShow::Workdir)
                .include_ignored(false)
                .include_untracked(true)
                .recurse_untracked_dirs(true),
        ))?;
        let mut unstaged_files = Vec::new();
        for file in unstaged_statuses.iter() {
            let status_type = match file.status() {
                s if s.is_wt_deleted() => FileStatusStatus::Deleted,
                s if s.is_wt_new() => FileStatusStatus::New,
                s if s.is_wt_modified() => FileStatusStatus::Modified,
                s if s.is_wt_typechange() => FileStatusStatus::TypeChanged,
                s => {
                    println!("unknown status: {:?}", s);
                    break;
                }
            };
            unstaged_files.push(FileStatus {
                path: file.path().unwrap().to_owned(),
                status_type,
            })
        }
        self.event_tx.send(Event::UnstagedFiles(unstaged_files))?;
        Ok(())
    }

    fn send_staged_statuses(&mut self) -> anyhow::Result<()> {
        let staged_statuses = self.repo.statuses(Some(
            StatusOptions::new()
                .show(git2::StatusShow::Index)
                .include_ignored(false)
                .include_untracked(true)
                .recurse_untracked_dirs(true),
        ))?;
        let mut staged_files = Vec::new();
        for file in staged_statuses.iter() {
            let status_type = match file.status() {
                s if s.is_index_deleted() => FileStatusStatus::Deleted,
                s if s.is_index_new() => FileStatusStatus::New,
                s if s.is_index_modified() => FileStatusStatus::Modified,
                s if s.is_index_typechange() => FileStatusStatus::TypeChanged,
                s => {
                    println!("unknown status: {:?}", s);
                    break;
                }
            };
            staged_files.push(FileStatus {
                path: file.path().unwrap().to_owned(),
                status_type,
            })
        }
        self.event_tx.send(Event::StagedFiles(staged_files))?;
        Ok(())
    }

    fn commit(&mut self, message: &str) -> anyhow::Result<Oid> {
        let mut index = self.repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;

        let head = self.repo.head()?;
        let parent_commit = head.peel_to_commit()?;

        let commit_oid = self.repo.commit(
            Some("HEAD"),
            &self.repo.signature().unwrap(),
            &self.repo.signature().unwrap(),
            &message,
            &tree,
            &[&parent_commit],
        )?;
        Ok(commit_oid)
    }
}
