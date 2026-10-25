//! Read-only folder sharing over iroh.

pub mod identity;
pub mod link;

#[cfg(test)]
pub(crate) async fn test_endpoint() -> iroh::Endpoint {
    use iroh::endpoint::presets;
    // Minimal preset: no relays, no discovery. Loopback only.
    iroh::Endpoint::builder(presets::Minimal)
        .alpns(vec![iroh_blobs::ALPN.to_vec()])
        .bind_addr("127.0.0.1:0")
        .expect("valid loopback address")
        .bind()
        .await
        .expect("bind loopback endpoint")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::{address_lookup::memory::MemoryLookup, protocol::Router};
    use iroh_blobs::{
        BlobFormat, BlobsProtocol, api::blobs::AddPathOptions, api::blobs::ImportMode,
        store::fs::FsStore,
    };
    use std::time::Duration;

    fn dir_size(p: &std::path::Path) -> u64 {
        std::fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let m = e.metadata().unwrap();
                if m.is_dir() {
                    dir_size(&e.path())
                } else {
                    m.len()
                }
            })
            .sum()
    }

    #[tokio::test]
    async fn a_file_round_trips_between_two_loopback_endpoints() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let tmp = tempfile::tempdir().unwrap();
            let data: Vec<u8> = (0..3_000_000u32)
                .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
                .collect();
            let src = tmp.path().join("big.bin");
            std::fs::write(&src, &data).unwrap();

            let store_dir = tmp.path().join("a-store");
            let store_a = FsStore::load(&store_dir).await.unwrap();
            let tag = store_a
                .blobs()
                .add_path_with_opts(AddPathOptions {
                    path: src.clone(),
                    format: BlobFormat::Raw,
                    mode: ImportMode::TryReference,
                })
                .await
                .unwrap();

            let ep_a = test_endpoint().await;
            let addr_a = ep_a.addr();
            let router = Router::builder(ep_a)
                .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store_a, None))
                .spawn();

            let store_b = FsStore::load(tmp.path().join("b-store")).await.unwrap();
            let ep_b = test_endpoint().await;
            // Discovery is off, so tell B where A is (direct addresses).
            let lookup = MemoryLookup::new();
            lookup.add_endpoint_info(addr_a.clone());
            ep_b.address_lookup().unwrap().add(lookup);
            store_b
                .downloader(&ep_b)
                .download(tag.hash, Some(addr_a.id))
                .await
                .unwrap();
            let out = tmp.path().join("out.bin");
            store_b.blobs().export(tag.hash, &out).await.unwrap();
            assert_eq!(std::fs::read(&out).unwrap(), data);

            let size = dir_size(&store_dir);
            println!("A store dir: {size} bytes");
            assert!(size < 1_500_000, "file was copied into the store: {size}");

            ep_b.close().await;
            router.shutdown().await.unwrap();
        })
        .await
        .expect("timed out");
    }
}
