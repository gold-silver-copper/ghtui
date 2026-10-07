//! Any JSON, decoded as one of GitHub's wire shapes, makes ghtui's model
//! of it without panicking.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u8, &[u8])| {
    let (shape, json) = input;
    ghtui_api::fuzz::decode(shape, json);
});
