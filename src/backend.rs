use std::{
    path::Path,
    sync::mpsc::{Receiver, Sender},
    thread::{self, JoinHandle},
    time::Instant,
};

use eyre::{Report, eyre};
use git2::{
    Cred, FetchOptions, ObjectType, Oid, PushOptions, RemoteCallbacks, Repository, StatusOptions,
    build::CheckoutBuilder,
};
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

#[derive(Debug)]
pub enum Command {
    Close,
    GetStatuses,
    StageFile { path: String },
    ResetStagedFile { path: String },
    Commit { message: String },
    Push,
    Fetch,
    Pull,
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
            let command = match self.command_rx.recv() {
                Ok(c) => c,
                Err(e) => {
                    error!("Command channel closed, error: {e}. Stopping backend");
                    self.running = false;
                    break;
                }
            };
            if let Err(e) = self.process_command(&command) {
                error!("Error processing command.\nCommand:\n\t{command:?}\n\nError:\n\t{e:?}");
            }
        }
    }
    fn process_command(&mut self, command: &Command) -> Result<(), Report> {
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
                let path = Path::new(path);
                let mut index = self.repo.index()?;
                if std::fs::exists(path)? {
                    index.add_path(Path::new(&path))?;
                } else {
                    index.remove_path(path)?;
                }
                index.write()?;

                self.send_statuses()?;
            }
            Command::ResetStagedFile { path } => {
                {
                    let head = self.repo.head()?.peel(ObjectType::Commit)?;
                    self.repo.reset_default(Some(&head), [Path::new(&path)])?;
                }
                self.send_statuses()?;
            }
            Command::Commit { message } => {
                let commit_oid = self.commit(&message)?;
                info!("Created commit: {:?}", commit_oid);

                self.send_statuses()?;
            }
            Command::Push => {
                let mut remote = self.repo.find_remote("origin")?;
                let mut callbacks = RemoteCallbacks::new();
                callbacks.credentials(|_url, username_from_url, _allowed_types| {
                    Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
                });
                callbacks.pack_progress(|stage, current, total| {
                    info!("pack progress state: {stage:?}, current: {current}, total: {total}");
                });
                callbacks.push_negotiation(|_| {
                    info!("pack negotioation");
                    Ok(())
                });
                callbacks.push_transfer_progress(|current, total, bytes| {
                    info!(
                        "push transfer progrss current: {current}, total: {total}, bytes: {bytes}"
                    );
                });
                callbacks.sideband_progress(|bytes| {
                    if let Ok(str) = std::str::from_utf8(bytes) {
                        info!("sideband progress: {str}");
                    } else {
                        info!("sideband progress not utf8");
                    }
                    true
                });
                callbacks.transfer_progress(|progress| {
                    info!(
                        "transfer progres received bytes: {}",
                        progress.received_bytes()
                    );
                    true
                });
                callbacks.certificate_check(|_cert, _host| {
                    info!("certificate check");
                    Ok(git2::CertificateCheckStatus::CertificateOk)
                });

                let mut push_options = PushOptions::new();
                push_options.remote_callbacks(callbacks);

                info!("pre push");
                remote.push(
                    &["refs/heads/main:refs/heads/main"],
                    Some(&mut push_options),
                )?;
                info!("Pushed branch to remote");
            }
            Command::Fetch => {
                self.fetch()?;
            }
            Command::Pull => {
                info!("starting pull");

                self.fetch()?;

                let mut head = self.repo.head()?;
                let remote_commit = self.repo.reference_to_annotated_commit(&head)?;

                let (merge_analysis, _merge_preference) =
                    self.repo.merge_analysis(&[&remote_commit])?;
                if merge_analysis.is_none() {
                    return Err(eyre!("No merge possible"));
                } else if merge_analysis.is_normal() {
                    return Err(eyre!("Normal merge not implemented"));
                } else if merge_analysis.is_up_to_date() {
                    info!("Up to date")
                } else if merge_analysis.is_fast_forward() {
                    return Err(eyre!("Unborn merge not implemented"));
                } else if merge_analysis.is_fast_forward() {
                    let msg = format!(
                        "Fast forward branch {} to id: {}",
                        head.name()?,
                        remote_commit.id()
                    );
                    info!(msg);
                    head.set_target(remote_commit.id(), &msg)?;
                    self.repo
                        .checkout_head(Some(CheckoutBuilder::default().force()))?;
                }

                info!("pull done");
            }
        }
        Ok(())
    }

    fn send_statuses(&mut self) -> Result<(), Report> {
        self.send_staged_statuses()?;
        self.send_unstaged_statuses()?;
        Ok(())
    }

    fn send_unstaged_statuses(&mut self) -> Result<(), Report> {
        let t1 = Instant::now();
        let unstaged_statuses = self.repo.statuses(Some(
            StatusOptions::new()
                .show(git2::StatusShow::Workdir)
                .include_ignored(false)
                .include_untracked(true)
                .recurse_untracked_dirs(true),
        ))?;
        let mut unstaged_files = Vec::new();
        for file in unstaged_statuses.iter() {
            let path = file.path()?.to_owned();
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
        self.event_tx.send(Event::UnstagedFiles(unstaged_files))?;

        debug!(
            "Refreshing worktree statuses took: {} ms",
            Instant::now().duration_since(t1).as_millis()
        );
        Ok(())
    }

    fn send_staged_statuses(&mut self) -> Result<(), Report> {
        let t1 = Instant::now();
        let staged_statuses = self.repo.statuses(Some(
            StatusOptions::new()
                .show(git2::StatusShow::Index)
                .include_ignored(false)
                .include_untracked(true)
                .recurse_untracked_dirs(true),
        ))?;
        let mut staged_files = Vec::new();
        for file in staged_statuses.iter() {
            let path = file.path()?.to_owned();
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
        self.event_tx.send(Event::StagedFiles(staged_files))?;
        debug!(
            "Refreshing index statuses took: {} ms",
            Instant::now().duration_since(t1).as_millis()
        );
        Ok(())
    }

    fn commit(&mut self, message: &str) -> Result<Oid, Report> {
        let mut index = self.repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;

        let head = self.repo.head()?;
        let parent_commit = head.peel_to_commit()?;

        let commit_oid = self.repo.commit(
            Some("HEAD"),
            &self.repo.signature()?,
            &self.repo.signature()?,
            &message,
            &tree,
            &[&parent_commit],
        )?;
        Ok(commit_oid)
    }

    fn fetch(&mut self) -> Result<(), Report> {
        let remote_name = "origin";
        let mut remote = self.repo.find_remote("origin")?;

        let mut refspecs = Vec::new();
        for (i, refspec) in remote.fetch_refspecs()?.iter().enumerate() {
            match refspec {
                Ok(Some(refspec)) => refspecs.push(refspec.to_owned()),
                Ok(_) => (), // how?
                Err(_) => {
                    warn!("Refspec {i} for remote {remote_name} was invalid utf-8 and is ignored",);
                }
            }
        }

        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
        });
        callbacks.pack_progress(|stage, current, total| {
            info!("pack progress state: {stage:?}, current: {current}, total: {total}");
        });
        callbacks.push_negotiation(|_| {
            info!("pack negotioation");
            Ok(())
        });
        callbacks.push_transfer_progress(|current, total, bytes| {
            info!("push transfer progrss current: {current}, total: {total}, bytes: {bytes}");
        });
        callbacks.sideband_progress(|bytes| {
            if let Ok(str) = std::str::from_utf8(bytes) {
                info!("sideband progress: {str}");
            } else {
                info!("sideband progress not utf8");
            }
            true
        });
        callbacks.transfer_progress(|progress| {
            info!(
                "transfer progres received bytes: {}",
                progress.received_bytes()
            );
            true
        });
        callbacks.certificate_check(|_cert, _host| {
            info!("certificate check");
            Ok(git2::CertificateCheckStatus::CertificateOk)
        });

        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        info!("fetching from remote: {remote_name} with refspecs: {refspecs:?}");
        remote.fetch(&refspecs, Some(&mut fetch_options), Some("fetch"))?;

        // libgit2 wont error if a refspec has a reference that does not exist on the remote
        // side. To get an error out of it we would have to iterate all known refs of the
        // remote and see if any of the refspecs matches against it.

        info!("Fetched branches from remote");
        Ok(())
    }
}
