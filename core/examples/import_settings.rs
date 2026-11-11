//! Imports a `config.toml` into a database, as the first start of this
//! version does with the device's own. Meant for trying it on copies first:
//!
//! ```sh
//! cargo run -p timeslay-core --example import_settings -- copy.sqlite3 copy-config.toml
//! ```
//!
//! The file is renamed `<file>.imported` once it is in. A database that has
//! imported a file before imports nothing.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [database, file] = args.as_slice() else {
        eprintln!("usage: import_settings <database> <config.toml>");
        std::process::exit(2);
    };
    let imported = timeslay_core::settings::import_file(Path::new(database), Path::new(file))?;
    if imported {
        println!("Imported {file} into {database}");
    } else {
        println!(
            "Nothing imported: the database has imported a file before, or {file} could not be read"
        );
    }
    Ok(())
}
