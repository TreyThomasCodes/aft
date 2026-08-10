//! Internal generator entry point for the governed dual-mode edit schema artifact.

fn main() {
    let artifact = aft::hashline::integration::regenerate_governed_edit_artifacts();
    println!(
        "{}",
        serde_json::to_string_pretty(&artifact)
            .expect("governed hashline edit schema artifact must serialize")
    );
}
