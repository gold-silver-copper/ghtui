//! Real GitHub responses, recorded (see `tests/common`), replayed through
//! ghtui's client offline: every case decodes, and every answer adds up.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests: failing loudly is the point"
)]

mod common;

use ghtui_api::GitHub;
use ghtui_api::auth::Token;
use ghtui_store::Store;

#[tokio::test]
async fn the_recorded_corpus_decodes_and_adds_up() {
    let base = common::replay(common::corpus_dir()).await;
    let gh = GitHub::with_base_uri(Token::new("t"), Store::disabled(), Some(&base));
    common::run(&gh).await;
}
