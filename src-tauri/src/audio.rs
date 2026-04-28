//! Notification chime playback.
//!
//! The MP3 lives next to the Tauri crate as `src-tauri/chime.mp3` and
//! is baked into the binary via `include_bytes!`, so there's no runtime
//! file lookup and no install-time asset bundling to worry about.
//!
//! `rodio` provides the audio sink (cpal under the hood) and uses
//! `symphonia-mp3` as its decoder backend — the user-facing requirement
//! to "use symphonia for playback" is fulfilled by enabling that
//! feature in `Cargo.toml`. We never touch symphonia's API directly
//! because rodio's `Decoder` already wraps it cleanly.
//!
//! Each `play_chime` call spawns a detached thread that owns the
//! `OutputStream` (which is `!Send`) and blocks until playback ends.
//! Threads are cheap, the chime is short, and this avoids the
//! complexity of holding a long-lived audio device in shared state.

use std::io::Cursor;
use std::thread;

const CHIME_BYTES: &[u8] = include_bytes!("../chime.mp3");

/// Play the embedded chime. Returns immediately; playback runs on a
/// background thread. Errors are logged to stderr (visible in the
/// `cargo tauri dev` terminal) so a missing audio device or decoder
/// panic shows up without taking down the chat client.
pub fn play() {
    thread::spawn(|| {
        eprintln!("audio: play_chime invoked, opening default output");

        let (stream, handle) = match rodio::OutputStream::try_default() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("audio: no default output device: {e}");
                return;
            }
        };

        let sink = match rodio::Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("audio: sink: {e}");
                return;
            }
        };

        let source = match rodio::Decoder::new(Cursor::new(CHIME_BYTES)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("audio: decode: {e}");
                return;
            }
        };

        sink.append(source);
        sink.sleep_until_end();

        // Keep `stream` alive until playback finishes — dropping it
        // earlier would cut audio mid-chime.
        drop(stream);
        eprintln!("audio: chime finished");
    });
}

#[tauri::command]
pub fn play_chime() {
    play();
}
