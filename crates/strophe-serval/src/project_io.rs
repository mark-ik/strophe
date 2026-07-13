//! Background project save/open work for the Genet host.
//!
//! The host kernel keeps the live audio engine and session state. This actor
//! receives cloned save snapshots or a project path, performs Redb I/O away from
//! the kernel thread, then reports a typed result for the next frame to apply.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use armillary::{ActorHandle, Emitter, Wake, spawn};
use muniment::RedbBackend;
use std::collections::BTreeSet;
use strophe_engine::export::{ExportLength, render_mix, write_stereo_wav};
use strophe_engine::media::InMemoryStore;
use strophe_engine::project_store::{LoadedProject, ProjectStore};
use strophe_model::{NodeId, ProjectBundle, Session, TrackId};

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
    ExportMix {
        path: PathBuf,
        session: Session,
        media: InMemoryStore,
        solo: BTreeSet<TrackId>,
        length: ExportLength,
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
    Exported {
        path: PathBuf,
    },
    Failed {
        action: &'static str,
        message: String,
    },
}

pub fn spawn_project_worker(wake: Wake) -> (ActorHandle<ProjectCommand>, Receiver<ProjectUpdate>) {
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
        ProjectCommand::ExportMix {
            path,
            session,
            media,
            solo,
            length,
        } => match render_mix(&session, &media, &solo, length)
            .and_then(|mix| write_stereo_wav(&path, &mix))
        {
            Ok(()) => updates.emit(ProjectUpdate::Exported { path }),
            Err(error) => updates.emit(ProjectUpdate::Failed {
                action: "export",
                message: error.to_string(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use strophe_engine::media::MediaStore;
    use strophe_model::{History, Layer, Phrase, Session};

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
            ProjectUpdate::Saved {
                path: saved,
                saved_head: head,
            } => {
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
        drop(worker);
    }

    #[test]
    fn worker_exports_audible_mix_to_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("practice-mix.wav");
        let wake: Wake = Arc::new(|| {});
        let (worker, updates) = spawn_project_worker(wake);
        let mut session = Session::new_default();
        let mut media = InMemoryStore::new();
        let reference = media.put(&[0.25, -0.25], 48_000);
        let phrase = Phrase::new(reference, session.bars_per_phrase, session.bpm, 1);
        session.phrases.insert(phrase.id, phrase.clone());
        session.tracks[0].layers.push(Layer::new(phrase.id));

        assert!(worker.command(ProjectCommand::ExportMix {
            path: path.clone(),
            session,
            media,
            solo: BTreeSet::new(),
            length: ExportLength::OneCycle,
        }));
        match updates.recv_timeout(Duration::from_secs(5)).unwrap() {
            ProjectUpdate::Exported { path: exported } => assert_eq!(exported, path),
            _ => panic!("expected export result"),
        }
        assert!(path.is_file());
        drop(worker);
    }
}
