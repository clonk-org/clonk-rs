//! Display-only release identity. Reports travel only over links that announced
//! RELEASE_DIAGNOSTICS; the host supplies the authoritative client ID and relays
//! reports to other participants. Release strings never participate in admission.

pub const PID_PORT_CLIENT_RELEASE: u8 = 0x76;
const MAX_VERSION_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRelease {
    pub client_id: crate::ClientId,
    pub version: String,
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= MAX_VERSION_BYTES
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
}

pub(crate) fn encode_client_release(report: &ClientRelease) -> Option<Vec<u8>> {
    let client_id = i32::try_from(report.client_id).ok()?;
    if !valid_version(&report.version) {
        return None;
    }
    let mut wire = vec![PID_PORT_CLIENT_RELEASE];
    wire.extend_from_slice(&client_id.to_le_bytes());
    wire.push(report.version.len() as u8);
    wire.extend_from_slice(report.version.as_bytes());
    Some(wire)
}

pub(crate) fn decode_client_release(wire: &[u8]) -> Option<ClientRelease> {
    if wire.first().copied()? != PID_PORT_CLIENT_RELEASE {
        return None;
    }
    let client_id = i32::from_le_bytes(wire.get(1..5)?.try_into().ok()?);
    let length = usize::from(*wire.get(5)?);
    if wire.len() != 6 + length {
        return None;
    }
    let version = std::str::from_utf8(wire.get(6..)?).ok()?;
    valid_version(version)
        .then(|| ClientRelease {
            client_id: client_id as u32,
            version: version.to_string(),
        })
        .filter(|_| client_id >= 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_report_preserves_the_client_and_prerelease_version() {
        let report = ClientRelease {
            client_id: 17,
            version: "1.5.0-dev+abcdef".into(),
        };
        let wire = encode_client_release(&report).unwrap();
        assert_eq!(&wire[..6], &[0x76, 17, 0, 0, 0, 16]);
        assert_eq!(decode_client_release(&wire), Some(report));
    }

    #[test]
    fn malformed_release_reports_cannot_supply_display_text() {
        let report = ClientRelease {
            client_id: 7,
            version: "1.5.0".into(),
        };
        let wire = encode_client_release(&report).unwrap();
        for length in 0..wire.len() {
            assert_eq!(decode_client_release(&wire[..length]), None);
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert_eq!(decode_client_release(&trailing), None);
        let mut negative_id = wire;
        negative_id[1..5].copy_from_slice(&(-1_i32).to_le_bytes());
        assert_eq!(decode_client_release(&negative_id), None);
        for version in [
            "",
            "<c ff0000>fake",
            "1.0\nHost",
            "1.0\0",
            "é",
            &"a".repeat(MAX_VERSION_BYTES + 1),
        ] {
            assert_eq!(
                encode_client_release(&ClientRelease {
                    client_id: 7,
                    version: version.into()
                }),
                None
            );
            let mut malformed = vec![PID_PORT_CLIENT_RELEASE, 7, 0, 0, 0, version.len() as u8];
            malformed.extend_from_slice(version.as_bytes());
            assert_eq!(decode_client_release(&malformed), None);
        }
        assert_eq!(
            encode_client_release(&ClientRelease {
                client_id: u32::MAX,
                version: "1.5.0".into()
            }),
            None
        );
    }
}
