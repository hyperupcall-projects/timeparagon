//! Non-destructive native sync compatibility probe. Does not push or execute DDL.
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let _ = rustls::crypto::ring::default_provider().install_default();
	let remote = std::env::args().nth(1).ok_or("usage: sync_preflight URL")?;
	let directory = tempfile::tempdir()?;
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()?;
	runtime.block_on(async {
		let path = directory.path().join("probe.db");
		let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
			.with_remote_url(&remote)
			.build()
			.await?;
		let conn = db.connect().await?;
		let mut rows = conn
			.query("SELECT count(*) FROM sqlite_master WHERE type='table'", ())
			.await?;
		let row = rows.next().await?.ok_or("missing result")?;
		println!(
			"Native bootstrap succeeded; {} tables in the local replica",
			row.get::<i64>(0)?
		);
		drop(rows);
		drop(conn);
		db.pull().await?;
		db.checkpoint().await?;
		println!("Native pull and checkpoint succeeded; remote unchanged");
		Ok::<_, Box<dyn std::error::Error>>(())
	})
}
