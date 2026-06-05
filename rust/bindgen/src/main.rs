//! In-workspace uniffi-bindgen wrapper.
//!
//! Pinned to the same `uniffi` version as `dashdown-core` so library-mode
//! generation reads matching metadata. Run via:
//!   cargo run -p dashdown-bindgen --bin uniffi-bindgen -- generate \
//!     --library <lib> --language kotlin|swift --out-dir <dir>
fn main() {
    uniffi::uniffi_bindgen_main()
}
