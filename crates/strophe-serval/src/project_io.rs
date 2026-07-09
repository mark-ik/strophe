//! Background project save/open work for the Serval host.
//!
//! The host kernel keeps the live audio engine and session state. This actor
//! receives cloned save snapshots or a project path, performs Redb I/O away from
//! the kernel thread, then reports a typed result for the next frame to apply.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use armillary::{ActorHandle, Emitter, Wake, spawn};
use muniment::RedbBackend;
use strophe_engine::media::InMemoryStore;
use strophe_engine::project_store::{LoadedProject, ProjectStore};
use strophe_model::{NodeId, ProjectBundle};

pub enum ProjectCommand {
    Save {
        path: PathBuf,
        bundle: ProjectBundle,
        media: InMemoryStore,
        saved_head: NodeId,
    },
    Open {
        path: PathBuf,
    },
}

pub enum ProjectUpdate {
    Saved {
        path: PathBuf,
        saved_head: NodeId,
    },
    Opened {
        path: PathBuf,
        loaded: LoadedProject,
    },
    Failed {
        action: &'static str,
        message: String,
    },
}

pub fn spawn_project_worker(
    wake: Wake,
) -> (ActorHandle<ProjectCommand>, Receiver<ProjectUpdate>) {
    spawn(wake, |commands, updates| {
        while let Ok(command) = commands.recv() {
            run_command(command, &updates);
        }
    })
}

fn run_command(command: ProjectCommand, updates: &Emitter<ProjectUpdate>) {
    match command {
        ProjectCommand::Save {
            path,
            bundle,
            media,
            saved_head,
        } => {
            let result = RedbBackend::open(&path)
                .map_err(|error| error.to_string())
                .and_then(|backend| {
                    pollster::block_on(ProjectStore::new(backend).save(&bundle, &media))
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(()) => updates.emit(ProjectUpdate::Saved { path, saved_head }),
                Err(message) => updates.emit(ProjectUpdate::Failed {
                    action: "save",
                    message,
                }),
            }
        }
        ProjectCommand::Open { path } => {
            let result = RedbBackend::open(&path)
                .map_err(|error| error.to_string())
                .and_then(|backend| {
                    pollster::block_on(ProjectStore::new(backend).load())
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(loaded) => updates.emit(ProjectUpdate::Opened { path, loaded }),
                Err(message) => updates.emit(ProjectUpdate::Failed {
                    action: "open",
                    message,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use strophe_model::{History, Session};

    #[test]
    fn worker_saves_then_opens_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("practice.strophe");
        let wake: Wake = Arc::new(|| {});
        let (worker, updates) = spawn_project_worker(wake);
        let bundle = ProjectBundle::new(Session::new_default(), History::new());
        let saved_head = bundle.history.head;

        assert!(worker.command(ProjectCommand::Save {
            path: path.clone(),
            bundle: bundle.clone(),
            media: InMemoryStore::new(),
            saved_head,
        }));
        match updates.recv_timeout(Duration::from_secs(5)).unwrap() {
            ProjectUpdate::Saved { path: saved, saved_head: head } => {
                assert_eq!(saved, path);
                assert_eq!(head, saved_head);
            }
            _ => panic!("expected save result"),
        }

        assert!(worker.command(ProjectCommand::Open { path }));
        match updates.recv_timeout(Duration::from_secs(5)).unwrap() {
            ProjectUpdate::Opened { loaded, .. } => assert_eq!(loaded.bundle, bundle),
            _ => panic!("expected open result"),
        }
        worker.join();
    }
}
