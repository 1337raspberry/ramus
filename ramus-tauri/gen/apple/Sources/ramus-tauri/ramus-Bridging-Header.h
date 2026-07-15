#pragma once

// C ABI exposed by the Rust static library (libapp.a) for the App Intents /
// Siri integration. Implemented in ramus-tauri/src/siri_ffi.rs. Declared as
// plain C (no C++ namespace) so it can be imported through the Swift bridging
// header. The returned string is heap-allocated by Rust and must be released
// with ramus_siri_free.
char *ramus_siri_probe(const char *genre);
void ramus_siri_free(char *ptr);
