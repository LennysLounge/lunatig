use std::{
    path::Path,
    sync::mpsc::{Receiver, Sender},
    thread::{self, JoinHandle},
    time::Instant,
};

use error_stack::{Report, ResultExt};
use git2::{ObjectType, Oid, Repository, StatusOptions};
use thiserror::Error;
use tracing::{debug, error, info, warn};

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
    Close,
    GetStatuses,
    StageFile { path: String },
    ResetStagedFile { path: String },
    Commit { message: String },
}

pub fn start_backend(
    repo_path: String,
    command_rx: Receiver<Command>,
    event_tx: Sender<Event>,
) -> JoinHandle<()> {
    thread::spawn(|| {
        Backend::new(repo_path, command_rx, event_tx).run();
    })
}

#[derive(Error, Debug)]
enum BackendError {
    #[error("Git error")]
    Git,
    #[error("Cannot send event")]
    ChannelSend,
}

struct Backend {
    repo_path: String,
    command_rx: Receiver<Command>,
    event_tx: Sender<Event>,
    repo: Repository,
    running: bool,
}
impl Backend {
    fn new(repo_path: String, command_rx: Receiver<Command>, event_tx: Sender<Event>) -> Self {
        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                panic!("Cannot open repository at {repo_path}, error: {e}");
            }
        };
        info!("Opened repository at {repo_path}");
        Backend {
            repo_path,
            command_rx,
            event_tx,
            repo,
            running: true,
        }
    }
    fn run(mut self) {
        while self.running {
            if let Err(e) = self.process_command() {
                error!("{e:?}");
            }
        }
    }
    fn process_command(&mut self) -> Result<(), Report<BackendError>> {
        let command = match self.command_rx.recv() {
            Ok(c) => c,
            Err(e) => {
                error!("Command channel closed, error: {e}. Stopping backend");
                self.running = false;
                return Ok(());
            }
        };
        match command {
            Command::Close => {
                info!("Close backend for {}", self.repo_path);
                self.running = false;
            }
            Command::GetStatuses => {
                info!("Refreshing statuses for work tree and index");
                self.send_statuses()?;
                info!("Refresh done");
            }
            Command::StageFile { path } => {
                let mut index = self.repo.index().change_context(BackendError::Git)?;
                index
                    .add_path(Path::new(&path))
                    .change_context(BackendError::Git)?;
                index.write().change_context(BackendError::Git)?;

                self.send_statuses()?;
            }
            Command::ResetStagedFile { path } => {
                {
                    let head = self
                        .repo
                        .head()
                        .change_context(BackendError::Git)?
                        .peel(ObjectType::Commit)
                        .change_context(BackendError::Git)?;
                    self.repo
                        .reset_default(Some(&head), [Path::new(&path)])
                        .change_context(BackendError::Git)?;
                }
                self.send_statuses()?;
            }
            Command::Commit { message } => {
                let commit_oid = self.commit(&message)?;
                info!("Created commit: {:?}", commit_oid);

                self.send_statuses()?;
            }
        }
        Ok(())
    }

    fn send_statuses(&mut self) -> Result<(), Report<BackendError>> {
        self.send_staged_statuses()?;
        self.send_unstaged_statuses()?;
        Ok(())
    }

    fn send_unstaged_statuses(&mut self) -> Result<(), Report<BackendError>> {
        let t1 = Instant::now();
        let unstaged_statuses = self
            .repo
            .statuses(Some(
                StatusOptions::new()
                    .show(git2::StatusShow::Workdir)
                    .include_ignored(false)
                    .include_untracked(true)
                    .recurse_untracked_dirs(true),
            ))
            .change_context(BackendError::Git)?;
        let mut unstaged_files = Vec::new();
        for file in unstaged_statuses.iter() {
            let path = file.path().change_context(BackendError::Git)?.to_owned();
            let status_type = match file.status() {
                s if s.is_wt_deleted() => FileStatusStatus::Deleted,
                s if s.is_wt_new() => FileStatusStatus::New,
                s if s.is_wt_modified() => FileStatusStatus::Modified,
                s if s.is_wt_typechange() => FileStatusStatus::TypeChanged,
                s => {
                    warn!("unknown status in worktree file {}: {:?}", &path, s);
                    break;
                }
            };
            unstaged_files.push(FileStatus { path, status_type })
        }
        self.event_tx
            .send(Event::UnstagedFiles(unstaged_files))
            .change_context(BackendError::ChannelSend)?;

        debug!(
            "Refreshing worktree statuses took: {} ms",
            Instant::now().duration_since(t1).as_millis()
        );
        Ok(())
    }

    fn send_staged_statuses(&mut self) -> Result<(), Report<BackendError>> {
        let t1 = Instant::now();
        let staged_statuses = self
            .repo
            .statuses(Some(
                StatusOptions::new()
                    .show(git2::StatusShow::Index)
                    .include_ignored(false)
                    .include_untracked(true)
                    .recurse_untracked_dirs(true),
            ))
            .change_context(BackendError::Git)?;
        let mut staged_files = Vec::new();
        for file in staged_statuses.iter() {
            let path = file.path().change_context(BackendError::Git)?.to_owned();
            let status_type = match file.status() {
                s if s.is_index_deleted() => FileStatusStatus::Deleted,
                s if s.is_index_new() => FileStatusStatus::New,
                s if s.is_index_modified() => FileStatusStatus::Modified,
                s if s.is_index_typechange() => FileStatusStatus::TypeChanged,
                s => {
                    warn!("unknown status in index file {}: {:?}", &path, s);
                    println!("unknown status: {:?}", s);
                    break;
                }
            };
            staged_files.push(FileStatus { path, status_type })
        }
        self.event_tx
            .send(Event::StagedFiles(staged_files))
            .change_context(BackendError::ChannelSend)?;
        debug!(
            "Refreshing index statuses took: {} ms",
            Instant::now().duration_since(t1).as_millis()
        );
        Ok(())
    }

    fn commit(&mut self, message: &str) -> Result<Oid, Report<BackendError>> {
        let mut index = self.repo.index().change_context(BackendError::Git)?;
        let tree_oid = index.write_tree().change_context(BackendError::Git)?;
        let tree = self
            .repo
            .find_tree(tree_oid)
            .change_context(BackendError::Git)?;

        let head = self.repo.head().change_context(BackendError::Git)?;
        let parent_commit = head.peel_to_commit().change_context(BackendError::Git)?;

        let commit_oid = self
            .repo
            .commit(
                Some("HEAD"),
                &self.repo.signature().change_context(BackendError::Git)?,
                &self.repo.signature().change_context(BackendError::Git)?,
                &message,
                &tree,
                &[&parent_commit],
            )
            .change_context(BackendError::Git)?;
        Ok(commit_oid)
    }
}
