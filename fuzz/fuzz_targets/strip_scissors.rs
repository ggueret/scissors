#![no_main]

use libfuzzer_sys::fuzz_target;
use scissors::strip_scissors;

// Fuzz strip_scissors with arbitrary UTF-8 input. It should never panic;
// the worst legitimate outcome is an empty string when the input is the
// scissors line or starts with it.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = strip_scissors(s);
    }
});
