//! Database snapshot helper. Never launches application initialization.
use std::path::Path;
use timeslay_core::database_migration;
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	let result = match args.as_slice() {
        [command, source] if command == "inspect" => {
            println!("{}", serde_json::to_string_pretty(&database_migration::inspect(Path::new(source))?)?);
            return Ok(());
        }
        [command, source, destination] if command == "backup" => database_migration::backup(Path::new(source), Path::new(destination))?,
        [command, directory] if command == "verify" => database_migration::verify(Path::new(directory))?,
        _ => return Err("usage: database_backup inspect DATABASE | backup DATABASE DIRECTORY | verify DIRECTORY".into()),
    };
	println!("{}", serde_json::to_string_pretty(&result)?);
	Ok(())
}
