mod initial_network_dynamic;
mod initial_network_metadata;

fn c4(bytes: impl AsRef<[u8]>) -> clonk_engine::LegacyCString {
    clonk_engine::LegacyCString::from_bytes(bytes.as_ref().to_vec())
        .expect("fixture contains no NUL")
}
