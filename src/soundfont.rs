//! SoundFont handling module

use once_cell::sync::OnceCell;
use rustysynth::SoundFont;
use std::io::Cursor;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

// Global singleton to hold the loaded SoundFont as an Arc.
static GLOBAL_SF: OnceCell<Arc<SoundFont>> = OnceCell::new();

/// Load a SoundFont from raw bytes (e.g. fetched in JavaScript).
///
/// This function is exported to JavaScript via `wasm-bindgen`.
/// The caller should pass a `Uint8Array` containing the .sf2 file.
#[wasm_bindgen]
pub fn load_soundfont(data: &[u8]) -> Result<(), JsValue> {
    let mut cursor = Cursor::new(data);
    let sf = SoundFont::new(&mut cursor)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse SoundFont: {}", e)))?;

    GLOBAL_SF
        .set(Arc::new(sf))
        .map_err(|_| JsValue::from_str("SoundFont already loaded"))
}

/// Retrieve a reference to the loaded SoundFont, if any.
/// Returns `None` when no SoundFont has been loaded yet.
pub fn get_soundfont() -> Option<Arc<SoundFont>> {
    GLOBAL_SF.get().cloned()
}
