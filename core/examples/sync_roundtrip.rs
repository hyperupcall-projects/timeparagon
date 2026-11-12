//! Isolated native replication test. Creates and removes its own scratch table.
use std::time::Duration;
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let _ = rustls::crypto::ring::default_provider().install_default();
	let url = std::env::args().nth(1).ok_or("usage: sync_roundtrip URL")?;
	let mut random = [0u8; 8];
	getrandom::fill(&mut random)?;
	let name = format!("timeparagon_probe_{}", u64::from_le_bytes(random));
	let directory = tempfile::tempdir()?;
	tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        tokio::time::timeout(Duration::from_secs(120),async {
            let a_path=directory.path().join("a.db");let b_path=directory.path().join("b.db");
            let a=turso::sync::Builder::new_remote(a_path.to_str().unwrap()).with_remote_url(&url).build().await?;
            let ac=a.connect().await?;
            ac.execute(&format!("CREATE TABLE {name}(id TEXT PRIMARY KEY,value TEXT NOT NULL)"),()).await?;
            a.push().await?;
            let b=turso::sync::Builder::new_remote(b_path.to_str().unwrap()).with_remote_url(&url).build().await?;
            let bc=b.connect().await?;
            // Both clients commit locally before either pushes.
            ac.execute(&format!("INSERT INTO {name} VALUES('desktop','offline A')"),()).await?;
            bc.execute(&format!("INSERT INTO {name} VALUES('phone','offline B')"),()).await?;
            a.push().await?;b.push().await?;a.pull().await?;b.pull().await?;
            for conn in [&ac,&bc] {
                let mut rows=conn.query(&format!("SELECT count(*) FROM {name}"),()).await?;
                assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?,2);
            }
            a.checkpoint().await?;b.checkpoint().await?;
            drop(ac);drop(a);
            let a=turso::sync::Builder::new_remote(a_path.to_str().unwrap()).with_remote_url(&url).build().await?;
            let ac=a.connect().await?;a.pull().await?;
            let mut rows=ac.query(&format!("SELECT count(*) FROM {name}"),()).await?;
            assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?,2);drop(rows);
            ac.execute(&format!("DROP TABLE {name}"),()).await?;a.push().await?;
            println!("PASS: two offline writers converge; checkpoint and reopen retain both records; scratch table removed");
            Ok::<_,Box<dyn std::error::Error>>(())
        }).await?
    })
}
