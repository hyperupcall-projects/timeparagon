//! Export every version, including superseded values and deletions, privately.
use std::path::Path;
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	if args.len() != 2 {
		return Err("usage: database_history DATABASE OUTPUT.json".into());
	}
	let output = Path::new(&args[1]);
	if output.exists() {
		return Err("output already exists; choose a new filename".into());
	}
	let edits = timeparagon_core::database_sync::history(Path::new(&args[0]))?;
	timeparagon_core::fsutil::write_atomic(
		output,
		&serde_json::to_vec_pretty(&edits)?,
		timeparagon_core::fsutil::Kind::Private,
	)?;
	println!("Exported {} retained versions", edits.len());
	Ok(())
}
