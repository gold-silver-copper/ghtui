//! A release's assets, beyond listing them on its page (with their
//! sizes and how often they were downloaded): `↵` on one saves it where
//! you say, never over a file without asking (as a run's artifacts
//! are); `,U` uploads a file from a path; `,D` on one deletes it, after
//! asking. The page follows GitHub until it shows an upload or a
//! deletion.

use std::path::{Path, PathBuf};

use crate::api::GitHub;
use crate::api::browse::{Asset, Release};
use crate::api::model::RepoId;
use crate::browse::DataKey;
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Cmd, Screen, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, WriteLink};
use crate::writes::{Answer, Chosen, Expected, Pick, Work as Writes, item};

/// An asset of a release, by REST's ID for the release and the asset's
/// name (unique on a release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Of {
    pub repo: RepoId,
    pub tag: String,
    pub id: u64,
    pub name: String,
}

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Save {
        of: Of,
        path: PathBuf,
        replace: bool,
    },
    Upload {
        of: Of,
        path: PathBuf,
    },
    Delete(Of),
}

#[derive(Debug)]
pub enum Done {
    Saved(Of, PathBuf, Result<u64, String>),
    Uploaded(Of, Result<(), String>),
    Deleted(Of, Result<(), String>),
}

impl Done {
    pub fn wrote(&self) -> bool {
        match self {
            Done::Saved(..) => false,
            Done::Uploaded(_, r) | Done::Deleted(_, r) => r.is_ok(),
        }
    }
}

impl Work {
    pub fn failed(self, why: String) -> Done {
        match self {
            Work::Save { of, path, .. } => Done::Saved(of, path, Err(why)),
            Work::Upload { of, .. } => Done::Uploaded(of, Err(why)),
            Work::Delete(of) => Done::Deleted(of, Err(why)),
        }
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        let why = |err: crate::api::ApiError| crate::writes::reason(&err);
        match self {
            Work::Save { of, path, replace } => {
                let saved = gh
                    .download_asset(&of.repo, of.id, &of.name, &path, replace)
                    .await;
                Done::Saved(of, path, saved.map_err(why))
            }
            Work::Upload { of, path } => {
                let sent = gh.upload_asset(&of.repo, of.id, &path, &of.name).await;
                Done::Uploaded(of, sent.map_err(why))
            }
            Work::Delete(of) => {
                let done = gh.delete_asset(&of.repo, of.id, &of.name).await;
                Done::Deleted(of, done.map_err(why))
            }
        }
    }
}

/// The release on screen's asset named `name`, as loaded.
fn asset<'a>(r: &'a Release, name: &str) -> Option<&'a Asset> {
    r.assets.iter().find(|a| a.name == name)
}

/// The asset `name` of the release on screen.
fn of(state: &State, name: &str) -> Option<Of> {
    let (repo, tag, r) = super::shown(state)?;
    Some(Of {
        repo,
        tag,
        id: r.id?,
        name: name.to_owned(),
    })
}

/// The asset selected on the release's page, by its name.
pub fn selected(state: &State) -> Option<String> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    match p.selected_link()? {
        Link::Write(WriteLink::Asset(name)) => Some(name.clone()),
        _ => None,
    }
}

/// `↵` on an asset: where to save it.
#[must_use]
pub fn open(state: &mut State, name: &str) -> Vec<Cmd> {
    let Some(of) = of(state, name) else {
        state.info("The release hasn't loaded yet");
        return Vec::new();
    };
    crate::writes::open_picker(state, Pick::SaveAsset(of))
}

/// `,U`: a file to upload.
#[must_use]
pub fn open_upload(state: &mut State) -> Vec<Cmd> {
    if !matches!(state.route(), Some(Route::Release { .. })) {
        state.info("Assets are uploaded to a release: open its page");
        return Vec::new();
    }
    if let Some(why) = super::unchangeable(state) {
        state.info(why);
        return Vec::new();
    }
    let Some(of) = of(state, "") else {
        return Vec::new();
    };
    crate::writes::open_picker(state, Pick::Upload(of))
}

/// Where an asset `name` is saved for what's typed: in the current
/// folder by default, in a folder named, or as the file named.
fn save_path(typed: &str, name: &str) -> PathBuf {
    let typed = typed.trim();
    if typed.is_empty() {
        return Path::new(".").join(name);
    }
    let path = PathBuf::from(typed);
    if typed.ends_with('/') || path.is_dir() {
        path.join(name)
    } else {
        path
    }
}

/// The saving picker's row: saving where it's typed, or (where a file
/// is) replacing it, which choosing is the asking.
pub fn save_rows(of: &Of, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let path = save_path(q, &of.name);
    let shown = path.display().to_string();
    let chosen = |replace| {
        Some(Chosen::SaveAsset {
            of: of.clone(),
            path: path.clone(),
            replace,
        })
    };
    if path.is_dir() {
        return vec![item(format!("{shown} is a folder"), "", None)];
    }
    if path.exists() {
        vec![item(format!("Replace {shown}"), "it exists", chosen(true))]
    } else {
        vec![item(format!("Save to {shown}"), "", chosen(false))]
    }
}

/// How many files the upload picker lists of a folder.
const LISTED: usize = 50;

/// The upload picker's rows: the file typed, else the files in the
/// folder typed (or beside what's typed) whose names start as typed.
pub fn upload_rows(state: &State, of: &Of, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let typed = q.trim();
    let release = (super::shown(state)).map(|(_, _, r)| r);
    let row = |path: &Path| {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        let shown = path.display().to_string();
        if release.is_some_and(|r| asset(r, &name).is_some()) {
            let delete = state.first_key(Action::DeleteComment);
            return item(
                format!("{name} is on the release already"),
                format!("{delete} on it deletes it"),
                None,
            );
        }
        let up = Of { name, ..of.clone() };
        item(
            format!("Upload {shown}"),
            crate::ui::text::size(size),
            Some(Chosen::Upload {
                of: up,
                path: path.to_owned(),
            }),
        )
    };
    let path = PathBuf::from(if typed.is_empty() { "." } else { typed });
    if path.is_file() {
        return vec![row(&path)];
    }
    let (dir, start) = if typed.is_empty() || typed.ends_with('/') || path.is_dir() {
        (path.clone(), String::new())
    } else {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_owned);
        let start = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (dir, start)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![item(format!("No file or folder {typed}"), "", None)];
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&start))
        })
        .collect();
    files.sort();
    let more = files.len().saturating_sub(LISTED);
    let mut rows: Vec<_> = files.iter().take(LISTED).map(|p| row(p)).collect();
    if rows.is_empty() {
        rows.push(item(
            format!("No files in {} start with “{start}”", dir.display()),
            "",
            None,
        ));
    }
    if more > 0 {
        rows.push(item(
            format!("{more} more: type more of the name"),
            "",
            None,
        ));
    }
    rows
}

/// Choosing a row: saving, or uploading.
#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    let work = match chosen {
        Chosen::SaveAsset { of, path, replace } => {
            state.info(format!("Saving {} to {}…", of.name, path.display()));
            Work::Save { of, path, replace }
        }
        Chosen::Upload { of, path } => {
            state.info(format!("Uploading {}…", path.display()));
            Work::Upload { of, path }
        }
        _ => return Vec::new(),
    };
    vec![Cmd::Write(Writes::Release(super::Work::Asset(work)))]
}

/// `,D` on an asset: asks first, as GitHub does.
#[must_use]
pub fn ask_delete(state: &mut State, name: &str) -> Vec<Cmd> {
    let Some((repo, tag, r)) = super::shown(state) else {
        return Vec::new();
    };
    let Some(a) = asset(r, name) else {
        return Vec::new();
    };
    let facts = vec![(
        format!(
            "{}, downloaded {}: it can't be brought back",
            crate::ui::text::size(a.size),
            if a.downloads == 1 {
                "once".to_owned()
            } else {
                format!("{} times", a.downloads)
            }
        ),
        true,
    )];
    crate::writes::ask(
        state,
        Route::Release { repo, tag },
        format!("Delete {name}?"),
        facts,
        vec![("Delete asset".into(), Answer::DeleteAsset(name.to_owned()))],
    );
    Vec::new()
}

/// Deletes the asset asked about.
#[must_use]
pub fn delete(state: &mut State, name: &str) -> Vec<Cmd> {
    let Some(of) = of(state, name) else {
        return Vec::new();
    };
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Release(super::Work::Asset(
        Work::Delete(of),
    )))]
}

/// Work's answers: a saved file is said; the release's page follows an
/// upload or a deletion.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Saved(of, path, Ok(size)) => {
            let size = crate::ui::text::size(size);
            state.info(format!("Saved {} to {} ({size})", of.name, path.display()));
            Vec::new()
        }
        Done::Saved(of, _, Err(why)) => {
            state.error(format!("Couldn't save {}: {why}", of.name));
            Vec::new()
        }
        Done::Uploaded(of, Ok(())) => {
            state.info(format!("Uploaded {}", of.name));
            follow(state, of, true)
        }
        Done::Uploaded(of, Err(why)) => {
            state.error(format!("GitHub didn't take {}: {why}", of.name));
            Vec::new()
        }
        Done::Deleted(of, Ok(())) => {
            let route = Route::Release {
                repo: of.repo.clone(),
                tag: of.tag.clone(),
            };
            crate::writes::close_sender(state, &route);
            state.info(format!("Deleted {}", of.name));
            follow(state, of, false)
        }
        Done::Deleted(of, Err(why)) => {
            crate::writes::say(state, format!("GitHub didn't delete {}: {why}", of.name));
            Vec::new()
        }
    }
}

/// The release's page, if it's on screen, follows GitHub until it shows
/// the asset there (or gone).
fn follow(state: &mut State, of: Of, there: bool) -> Vec<Cmd> {
    let route = Route::Release {
        repo: of.repo.clone(),
        tag: of.tag.clone(),
    };
    if state.route() != Some(&route) {
        return Vec::new();
    }
    let what = Expected::Asset {
        repo: of.repo,
        tag: of.tag,
        name: of.name,
        there,
    };
    crate::writes::expect(state, route.clone(), what);
    state.ensure_route(&route, true)
}

/// Whether the release's page shows asset `name` there (or gone).
pub fn shows(state: &State, repo: &RepoId, tag: &str, name: &str, there: bool) -> bool {
    let key = DataKey::Release(repo.clone(), tag.to_owned());
    state
        .picked::<Release>(&key)
        .is_some_and(|r| asset(r, name).is_some() == there)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An asset is saved by its own name in the current folder, in a
    /// folder named, or as the file named.
    #[test]
    fn an_asset_is_saved_where_its_typed() {
        let name = "ghtui.tar.gz";
        assert_eq!(save_path("", name), Path::new(".").join(name));
        assert_eq!(save_path("out/", name), Path::new("out/").join(name));
        assert_eq!(save_path("x.tgz", name), PathBuf::from("x.tgz"));
    }
}
