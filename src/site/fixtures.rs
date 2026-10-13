//! These pages' data as GitHub (or git) might answer, for tests.

use crate::api::explore::{Topic, Topics};
use crate::api::insights::{
    Activity, Author, Counted, Counter, Dependencies, Dependency, Manifest, Popular, Traffic,
};
use crate::api::lists::{ListPage, Mine, MyLists, StarList};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::api::packages::{
    Kind, Package, PackageDetail, PackageList, PackageRef, PackagesOf, Version,
};
use crate::api::replies::SavedReply;
use crate::browse::Data;

use super::{Key, Value};

/// Sunday 2026-04-12, 00:00 UTC.
pub const START: u64 = 1_775_952_000;

/// 26 weeks of commits by three authors.
pub fn activity() -> Activity {
    let alice: Vec<u32> = (0..26).map(|w| [3, 0, 5, 8, 1, 0, 2][w % 7]).collect();
    let bob: Vec<u32> = (0..26).map(|w| u32::from(w % 4 == 0)).collect();
    let carol: Vec<u32> = (0..26).map(|w| u32::from(w >= 20) * 4).collect();
    let weeks: Vec<u32> = (0..26)
        .map(|w| alice.get(w).unwrap_or(&0) + bob.get(w).unwrap_or(&0) + carol.get(w).unwrap_or(&0))
        .collect();
    let sum = |w: &[u32]| w.iter().map(|&n| u64::from(n)).sum::<u64>();
    let author = |name: &str, login: Option<&str>, weeks: Vec<u32>| Author {
        name: name.into(),
        login: login.map(str::to_owned),
        commits: sum(&weeks),
        weeks,
    };
    Activity {
        start: START,
        total: sum(&weeks),
        weeks,
        weekdays: [4, 20, 25, 18, 22, 9, 2],
        authors: vec![
            author("Alice Liddell", Some("alice"), alice),
            author("carol", Some("carol"), carol),
            author("Bob Builder", None, bob),
        ],
        authors_left_out: 0,
        cut: None,
        by: Counter::Git,
    }
}


pub fn traffic() -> Traffic {
    let day = |d: u32, n: u64, u: u64| (format!("2026-10-{d:02}T00:00:00Z"), n, u);
    Traffic {
        views: Counted {
            count: 54,
            uniques: 12,
            days: vec![day(8, 10, 3), day(9, 40, 8), day(10, 4, 1)],
        },
        clones: Counted {
            count: 7,
            uniques: 2,
            days: vec![day(9, 6, 1), day(10, 1, 1)],
        },
        referrers: vec![Popular {
            name: "news.ycombinator.com".into(),
            title: None,
            count: 30,
            uniques: 9,
        }],
        paths: vec![Popular {
            name: "/gold-silver-copper/ghtui".into(),
            title: Some("A keyboard-driven terminal client for GitHub".into()),
            count: 41,
            uniques: 11,
        }],
    }
}

pub fn dependencies() -> Dependencies {
    let dep = |package: &str, requirements: &str, repo: Option<&str>| Dependency {
        package: package.into(),
        manager: "RUST".into(),
        requirements: requirements.into(),
        repo: repo.and_then(RepoId::parse),
    };
    Dependencies {
        manifests: Capped::new(
            vec![
                Manifest {
                    path: "Cargo.lock".into(),
                    dependencies: Capped::new(
                        vec![
                            dep("ratatui", "= 0.30.0", Some("ratatui/ratatui")),
                            dep("tokio", "= 1.47.1", Some("tokio-rs/tokio")),
                        ],
                        212,
                    ),
                    unparseable: false,
                    too_big: false,
                },
                Manifest {
                    path: "xtask/Cargo.toml".into(),
                    dependencies: Capped::new(vec![dep("duct", ">= 1.0.0, < 2.0.0", None)], 1),
                    unparseable: false,
                    too_big: false,
                },
            ],
            2,
        ),
    }
}

fn topic(name: &str, title: &str, about: &str) -> Topic {
    Topic {
        name: name.into(),
        display_name: Some(title.into()),
        short_description: Some(about.into()),
        featured: true,
        curated: true,
        ..Topic::default()
    }
}

pub fn featured() -> Topics {
    Topics {
        topics: vec![
            topic("rust", "Rust", "Rust is a systems programming language."),
            topic("terminal", "Terminal", "Programs that run in a terminal."),
        ],
        total: 2,
    }
}

pub fn rust() -> Topic {
    Topic {
        description: Some("Rust is a systems programming language created by Mozilla.".into()),
        created_by: Some("Graydon Hoare".into()),
        released: Some("July 7, 2010".into()),
        stars: Some(1200),
        related: vec!["cargo".into(), "webassembly".into()],
        ..topic("rust", "Rust", "Rust is a systems programming language.")
    }
}

fn list(name: &str, slug: &str, count: u64, private: bool) -> StarList {
    StarList {
        id: NodeId::new(format!("UL_{slug}")),
        name: name.into(),
        slug: slug.into(),
        description: (!private).then(|| "Tools I use every day".to_owned()),
        private,
        count,
        last_added: Some("2026-10-01T10:00:00Z".into()),
    }
}

pub fn star_lists() -> Capped<StarList> {
    Capped::new(
        vec![
            list("Terminal tools", "terminal-tools", 2, false),
            list("To read", "to-read", 0, true),
        ],
        2,
    )
}

pub fn list_page() -> ListPage {
    let mut repos = crate::fixtures::profile_repos();
    repos.items.truncate(2);
    repos.total = 2;
    repos.next = None;
    ListPage {
        list: list("Terminal tools", "terminal-tools", 2, false),
        repos,
    }
}

pub fn my_lists() -> MyLists {
    let mine = |name: &str, slug: &str, has| Mine {
        id: NodeId::new(format!("UL_{slug}")),
        name: name.into(),
        private: false,
        count: 2,
        has: Some(has),
    };
    MyLists {
        repo: NodeId::new("R_ghtui"),
        starred: true,
        viewer: "octocat".into(),
        lists: Capped::new(
            vec![
                mine("Terminal tools", "terminal-tools", true),
                mine("To read", "to-read", false),
            ],
            2,
        ),
    }
}

pub fn package_ref() -> PackageRef {
    PackageRef {
        owner: "gold-silver-copper".into(),
        kind: Kind::Container,
        name: "ghtui".into(),
    }
}

fn package() -> Package {
    Package {
        at: package_ref(),
        repo: RepoId::parse("gold-silver-copper/ghtui"),
        visibility: "public".into(),
        versions: 2,
        updated_at: Some("2026-10-01T10:00:00Z".into()),
    }
}

pub fn packages() -> PackageList {
    PackageList {
        items: vec![
            package(),
            Package {
                at: PackageRef {
                    kind: Kind::Npm,
                    name: "ghtui-theme".into(),
                    ..package_ref()
                },
                visibility: "private".into(),
                versions: 5,
                ..package()
            },
        ],
        cut: Vec::new(),
    }
}

pub fn package_detail() -> PackageDetail {
    let version = |id, name: &str, tags: &[&str]| Version {
        id,
        name: name.into(),
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        created_at: Some("2026-10-01T10:00:00Z".into()),
    };
    PackageDetail {
        package: package(),
        versions: crate::api::browse::Results {
            total: 2,
            items: vec![
                version(102, "sha256:9f1c0d2e", &["latest", "0.2.0"]),
                version(101, "sha256:3a7b55e1", &[]),
            ],
            next: None,
        },
    }
}

pub fn saved_replies() -> Capped<SavedReply> {
    Capped::from(vec![SavedReply {
        title: "Thanks".into(),
        body: "Thanks for the report!".into(),
    }])
}

/// What GitHub might answer for `key`.
pub fn sample(key: &Key) -> Data {
    let v = match key {
        Key::Activity(_) => Value::Activity(activity()),
        Key::Traffic(_) => Value::Traffic(traffic()),
        Key::Dependencies(_) => Value::Dependencies(dependencies()),
        Key::Featured => Value::Topics(featured()),
        Key::Topic(_) => Value::Topic(rust()),
        Key::StarLists(_) => Value::StarLists(star_lists()),
        Key::StarList(..) => Value::StarList(list_page()),
        Key::MyLists(_) => Value::MyLists(my_lists()),
        Key::Packages(PackagesOf::Owner(_) | PackagesOf::Repo(_)) => Value::Packages(packages()),
        Key::Package(_) => Value::Package(package_detail()),
        Key::SavedReplies => Value::SavedReplies(saved_replies()),
    };
    Data::Site(Box::new(v))
}
