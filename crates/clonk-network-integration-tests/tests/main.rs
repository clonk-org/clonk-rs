// The load probe validates transport admission against the engine player registry.
// Keep this integration separate so ordinary network tests do not build simulation.
#[path = "../../clonk-network/tests/network_load_24.rs"]
mod network_load_24;

fn c4(bytes: impl AsRef<[u8]>) -> clonk_protocol::LegacyCString {
    clonk_protocol::LegacyCString::from_bytes(bytes.as_ref().to_vec())
        .expect("fixture contains no NUL")
}
