//! Any URL opens a page, the browser or a diff without panicking, and a
//! page's own URL leads back to it. And any wiki text renders, its
//! `[[links]]` included, with every link it makes followable.
#![no_main]

use ghtui::route::Target;
use ghtui_api::browse::WikiPage;
use ghtui_api::model::RepoId;
use ghtui_ui::page::Page;
use libfuzzer_sys::fuzz_target;

fn follow(url: &str) {
    if let Target::Page(route) = Target::from_url(url) {
        assert_eq!(
            Target::from_url(&route.url()),
            Target::Page(route.clone()),
            "{url} -> {}",
            route.url()
        );
    }
}

fuzz_target!(|input: (&str, &str)| {
    let (url, wiki) = input;
    follow(url);
    for prefix in ["https://github.com/", "https://github.com/o/r/"] {
        follow(&format!("{prefix}{url}"));
    }
    let page = WikiPage {
        title: Some("Home".into()),
        text: Some(wiki.to_owned()),
        markdown: true,
        pages: vec!["Home".into()],
        sidebar: None,
    };
    let mut built = Page::new(80);
    ghtui_ui::pages::wiki(&mut built, &RepoId::new("o", "r"), &page);
    for link in built.links.iter().filter_map(|l| l.url()) {
        follow(link);
    }
});
