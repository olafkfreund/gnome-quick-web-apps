//! CEF subprocess helper. Chromium's multi-process model re-launches this
//! binary for its render/GPU/utility subprocesses; it just hands control to
//! CEF and never initializes the browser itself.

use cef::*;

fn main() {
    let args = args::Args::new();
    let _ = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
}
