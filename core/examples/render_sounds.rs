//! Renders TimeSlay's built-in tones as WAV files.
//!
//! `scripts/build-android.sh` runs this into the generated project's
//! `res/raw`, where notification channels can play them:
//!
//! ```sh
//! cargo run -p timeslay-core --example render_sounds -- path/to/res/raw
//! ```

use std::path::PathBuf;

use timeslay_core::sounds::{Tone, wav};

fn main() -> std::io::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("usage: render_sounds <directory>"))?;
    std::fs::create_dir_all(&directory)?;
    for tone in Tone::ALL {
        let path = directory.join(format!("{}.wav", tone.android_resource()));
        std::fs::write(&path, wav(&tone.render()))?;
        println!("{}", path.display());
    }
    Ok(())
}
