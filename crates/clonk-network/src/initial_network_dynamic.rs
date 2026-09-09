//! Serialized initial network group data, independent of scenario composition.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkDynamicEntry {
    pub name: &'static str,
    pub payload: Vec<u8>,
    pub contents_crc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkDynamic {
    pub group_filename: String,
    /// Exact C-string body retained by the packed C4Group header.
    pub maker: Vec<u8>,
    pub packed_bytes: Vec<u8>,
    pub file_size: u32,
    pub file_crc: u32,
    pub contents_crc: u32,
    /// Entries in final `C4FLS_Scenario` order.
    pub entries: Vec<InitialNetworkDynamicEntry>,
}
