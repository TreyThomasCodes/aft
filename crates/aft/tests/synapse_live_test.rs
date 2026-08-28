use std::path::PathBuf;
use std::time::{Duration, Instant};

use aft::config::{SemanticBackend, SemanticBackendConfig};
use aft::synapse_embed::SynapseEmbeddingClient;

#[test]
fn synapse_live_probe() {
    if std::env::var("AFT_SYNAPSE_LIVE").as_deref() != Ok("1") {
        eprintln!("synapse live probe skipped; set AFT_SYNAPSE_LIVE=1");
        return;
    }

    let connection_file = std::env::var_os("AFT_SYNAPSE_CONNECTION_FILE")
        .map(PathBuf::from)
        .expect("AFT_SYNAPSE_CONNECTION_FILE must name the configured SubC connection file");
    let model = std::env::var("AFT_SYNAPSE_MODEL")
        .expect("AFT_SYNAPSE_MODEL must match semantic.model in user config");
    let root = std::env::current_dir().expect("resolve probe project root");
    let config = SemanticBackendConfig {
        backend: SemanticBackend::Synapse,
        model,
        timeout_ms: 120_000,
        query_timeout_ms: 3_000,
        subc_connection_file: Some(connection_file),
        route_project_root: Some(root),
        route_harness: Some("runner".to_string()),
        ..SemanticBackendConfig::default()
    };

    let mut client = SynapseEmbeddingClient::from_config(&config)
        .expect("discover configured model through Synapse models.list");
    if let Some(path) = std::env::var_os("AFT_SYNAPSE_FIXTURE_OUT") {
        std::fs::write(path, client.models_list_envelope())
            .expect("write captured models.list envelope");
    }
    let metadata = client.metadata().clone();
    assert!(metadata.recommended_rows > 0);
    assert!(metadata.recommended_token_budget > 0);

    let interactive = [
        "semantic search",
        "SubC management surface",
        "content hash belt",
    ];
    let started = Instant::now();
    let query_vectors = interactive
        .iter()
        .map(|text| client.embed_query(text, Duration::from_millis(config.query_timeout_ms)))
        .collect::<Result<Vec<_>, _>>()
        .expect("embed interactive corpus with embed.query");
    let elapsed = started.elapsed();
    let dims = query_vectors[0].len();
    assert!(dims > 0);
    assert!(query_vectors.iter().all(|vector| vector.len() == dims));

    let bulk = interactive
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    let batch_vectors = client
        .embed_batch(&bulk)
        .expect("embed small bulk corpus with embed.batch");
    assert_eq!(batch_vectors.len(), bulk.len());
    assert!(batch_vectors.iter().all(|vector| vector.len() == dims));

    println!(
        "AFT_SYNAPSE_LIVE fingerprint={} table_epoch={} dims={} interactive_total_ms={} recommended_batch_rows={} recommended_token_budget={}",
        client.identity().fingerprint,
        client.identity().table_epoch,
        dims,
        elapsed.as_millis(),
        metadata.recommended_rows,
        metadata.recommended_token_budget,
    );
}
