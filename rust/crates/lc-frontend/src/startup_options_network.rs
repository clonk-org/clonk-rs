//! Retained model for the native Network page of `C4StartupOptionsDlg`.

use crate::classic_gui::IntRect;
use crate::GuiPoint;

pub const DEFAULT_TCP_PORT: u16 = 11_112;
pub const DEFAULT_UDP_PORT: u16 = 11_113;
pub const DEFAULT_REFERENCE_PORT: u16 = 11_111;
pub const DEFAULT_DISCOVERY_PORT: u16 = 11_114;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkPortId {
    Tcp,
    Udp,
    Reference,
    Discovery,
}

impl NetworkPortId {
    pub const ALL: [Self; 4] = [Self::Tcp, Self::Udp, Self::Reference, Self::Discovery];

    pub const fn index(self) -> usize {
        match self {
            Self::Tcp => 0,
            Self::Udp => 1,
            Self::Reference => 2,
            Self::Discovery => 3,
        }
    }

    pub const fn default_port(self) -> u16 {
        match self {
            Self::Tcp => DEFAULT_TCP_PORT,
            Self::Udp => DEFAULT_UDP_PORT,
            Self::Reference => DEFAULT_REFERENCE_PORT,
            Self::Discovery => DEFAULT_DISCOVERY_PORT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkTextField {
    Port(NetworkPortId),
    AlternateServerAddress,
    LocalName,
    Nick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkCheckboxId {
    UseAlternateServer,
    AutomaticUpdate,
    EnableUpnp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkValidationError {
    TcpReferenceConflict,
    UdpDiscoveryConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPortState {
    pub enabled: bool,
    pub port: u16,
}

impl NetworkPortState {
    pub const fn from_config(value: i32, default: u16) -> Self {
        if value > 0 && value <= u16::MAX as i32 {
            Self {
                enabled: true,
                port: value as u16,
            }
        } else {
            Self {
                enabled: false,
                port: default,
            }
        }
    }

    pub const fn config_value(&self) -> i32 {
        if self.enabled {
            self.port as i32
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSheetState {
    ports: [NetworkPortState; 4],
    pub use_alternate_server: bool,
    pub alternate_server_address: String,
    pub automatic_update: bool,
    pub enable_upnp: bool,
    pub local_name: String,
    /// The edit displays `LocalName` when the stored Nick is empty. Saving
    /// clears Nick again when both edits contain identical text.
    pub nick: String,
    pub hide_no_official_league_notice: bool,
}

impl Default for NetworkSheetState {
    fn default() -> Self {
        Self::new(
            [
                DEFAULT_TCP_PORT as i32,
                DEFAULT_UDP_PORT as i32,
                DEFAULT_REFERENCE_PORT as i32,
                DEFAULT_DISCOVERY_PORT as i32,
            ],
            false,
            String::new(),
            true,
            true,
            "Unknown".to_string(),
            String::new(),
            false,
        )
    }
}

impl NetworkSheetState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ports: [i32; 4],
        use_alternate_server: bool,
        alternate_server_address: String,
        automatic_update: bool,
        enable_upnp: bool,
        local_name: String,
        stored_nick: String,
        hide_no_official_league_notice: bool,
    ) -> Self {
        let nick = if stored_nick.is_empty() {
            local_name.clone()
        } else {
            stored_nick
        };
        Self {
            ports: std::array::from_fn(|index| {
                let id = NetworkPortId::ALL[index];
                NetworkPortState::from_config(ports[index], id.default_port())
            }),
            use_alternate_server,
            alternate_server_address,
            automatic_update,
            enable_upnp,
            local_name,
            nick,
            hide_no_official_league_notice,
        }
    }

    pub const fn port(&self, id: NetworkPortId) -> &NetworkPortState {
        &self.ports[id.index()]
    }

    pub fn port_mut(&mut self, id: NetworkPortId) -> &mut NetworkPortState {
        &mut self.ports[id.index()]
    }

    pub fn set_text(&mut self, field: NetworkTextField, value: String) {
        match field {
            NetworkTextField::Port(id) => {
                if let Ok(port) = value.trim().parse::<u16>() {
                    if port != 0 {
                        self.port_mut(id).port = port;
                    }
                }
            }
            NetworkTextField::AlternateServerAddress => self.alternate_server_address = value,
            NetworkTextField::LocalName => self.local_name = value,
            NetworkTextField::Nick => self.nick = value,
        }
    }

    pub fn stored_nick(&self) -> &str {
        if self.nick == self.local_name {
            ""
        } else {
            &self.nick
        }
    }

    pub fn validate_ports(&self) -> Result<(), NetworkValidationError> {
        let tcp = self.port(NetworkPortId::Tcp).config_value();
        let udp = self.port(NetworkPortId::Udp).config_value();
        let reference = self.port(NetworkPortId::Reference).config_value();
        let discovery = self.port(NetworkPortId::Discovery).config_value();
        if tcp > 0 && tcp == reference {
            return Err(NetworkValidationError::TcpReferenceConflict);
        }
        if udp > 0 && udp == discovery {
            return Err(NetworkValidationError::UdpDiscoveryConflict);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSheetLayout {
    pub port_controls: [IntRect; 4],
    pub port_checks: [IntRect; 4],
    pub port_edits: [IntRect; 4],
    pub alternate_check: IntRect,
    pub alternate_edit: IntRect,
    pub automatic_update_check: IntRect,
    pub upnp_check: IntRect,
    pub local_name_edit: IntRect,
    pub nick_edit: IntRect,
}

impl NetworkSheetLayout {
    pub fn from_sheet(sheet: IntRect, line_height: i32) -> Self {
        let margin_x = (sheet.w / 20).max(8);
        let margin_y = (sheet.h / 20).max(4);
        let inner = IntRect {
            x: sheet.x + margin_x,
            y: sheet.y + margin_y,
            w: sheet.w - margin_x * 2,
            h: sheet.h - margin_y * 2,
        };
        let gap = 12;
        let column_w = (inner.w - gap) / 2;
        let port_h = (line_height * 2 + 12).max(44);
        let port_controls = std::array::from_fn(|index| {
            let column = index % 2;
            let row = index / 2;
            IntRect {
                x: inner.x + column as i32 * (column_w + gap),
                y: inner.y + row as i32 * (port_h + 4),
                w: column_w,
                h: port_h,
            }
        });
        let port_checks = std::array::from_fn(|index| IntRect {
            x: port_controls[index].x,
            y: port_controls[index].y + line_height + 4,
            w: (column_w / 3).max(line_height),
            h: line_height,
        });
        let port_edits = std::array::from_fn(|index| IntRect {
            x: port_checks[index].x + port_checks[index].w + 4,
            y: port_checks[index].y - 2,
            w: port_controls[index].w - port_checks[index].w - 4,
            h: line_height + 4,
        });
        let below_ports = inner.y + 2 * (port_h + 4) + 4;
        let row_h = line_height + 8;
        let alternate_check = IntRect {
            x: inner.x,
            y: below_ports,
            w: column_w,
            h: line_height,
        };
        let alternate_edit = IntRect {
            x: inner.x + column_w,
            y: below_ports - 2,
            w: inner.w - column_w,
            h: line_height + 4,
        };
        let automatic_update_check = IntRect {
            x: inner.x,
            y: below_ports + row_h,
            w: inner.w,
            h: line_height,
        };
        let upnp_check = IntRect {
            x: inner.x,
            y: below_ports + row_h * 2,
            w: inner.w,
            h: line_height,
        };
        let local_name_edit = IntRect {
            x: inner.x,
            y: below_ports + row_h * 3,
            w: column_w,
            h: line_height + 4,
        };
        let nick_edit = IntRect {
            x: inner.x + column_w + gap,
            y: local_name_edit.y,
            w: column_w,
            h: line_height + 4,
        };
        Self {
            port_controls,
            port_checks,
            port_edits,
            alternate_check,
            alternate_edit,
            automatic_update_check,
            upnp_check,
            local_name_edit,
            nick_edit,
        }
    }

    pub const fn port_check(&self, id: NetworkPortId) -> IntRect {
        self.port_checks[id.index()]
    }

    pub const fn port_edit(&self, id: NetworkPortId) -> IntRect {
        self.port_edits[id.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkSheetHit {
    PortCheck(NetworkPortId),
    Text(NetworkTextField),
    Checkbox(NetworkCheckboxId),
}

pub fn network_sheet_hit_test(
    layout: &NetworkSheetLayout,
    state: &NetworkSheetState,
    point: GuiPoint,
) -> Option<NetworkSheetHit> {
    let contains = |rect: IntRect| {
        let x = point.x.floor() as i32;
        let y = point.y.floor() as i32;
        x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
    };
    for id in NetworkPortId::ALL {
        if contains(layout.port_check(id)) {
            return Some(NetworkSheetHit::PortCheck(id));
        }
        if state.port(id).enabled && contains(layout.port_edit(id)) {
            return Some(NetworkSheetHit::Text(NetworkTextField::Port(id)));
        }
    }
    if contains(layout.alternate_check) {
        return Some(NetworkSheetHit::Checkbox(
            NetworkCheckboxId::UseAlternateServer,
        ));
    }
    if state.use_alternate_server && contains(layout.alternate_edit) {
        return Some(NetworkSheetHit::Text(
            NetworkTextField::AlternateServerAddress,
        ));
    }
    if contains(layout.automatic_update_check) {
        return Some(NetworkSheetHit::Checkbox(
            NetworkCheckboxId::AutomaticUpdate,
        ));
    }
    if contains(layout.upnp_check) {
        return Some(NetworkSheetHit::Checkbox(NetworkCheckboxId::EnableUpnp));
    }
    if contains(layout.local_name_edit) {
        return Some(NetworkSheetHit::Text(NetworkTextField::LocalName));
    }
    if contains(layout.nick_edit) {
        return Some(NetworkSheetHit::Text(NetworkTextField::Nick));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_ports_display_defaults_but_save_zero() {
        let state = NetworkSheetState::new(
            [0, 0, 0, 0],
            false,
            String::new(),
            true,
            true,
            "Host".to_string(),
            String::new(),
            false,
        );
        for id in NetworkPortId::ALL {
            assert!(!state.port(id).enabled);
            assert_eq!(state.port(id).port, id.default_port());
            assert_eq!(state.port(id).config_value(), 0);
        }
        assert_eq!(state.nick, "Host");
        assert_eq!(state.stored_nick(), "");
    }

    #[test]
    fn port_validation_matches_saveconfig_order_and_zero_rules() {
        let mut state = NetworkSheetState::default();
        state.port_mut(NetworkPortId::Reference).port = DEFAULT_TCP_PORT;
        assert_eq!(
            state.validate_ports(),
            Err(NetworkValidationError::TcpReferenceConflict)
        );
        state.port_mut(NetworkPortId::Tcp).enabled = false;
        state.port_mut(NetworkPortId::Discovery).port = DEFAULT_UDP_PORT;
        assert_eq!(
            state.validate_ports(),
            Err(NetworkValidationError::UdpDiscoveryConflict)
        );
        state.port_mut(NetworkPortId::Udp).enabled = false;
        assert_eq!(state.validate_ports(), Ok(()));
    }

    #[test]
    fn network_layout_routes_all_native_controls() {
        let layout = NetworkSheetLayout::from_sheet(
            IntRect {
                x: 100,
                y: 80,
                w: 600,
                h: 400,
            },
            20,
        );
        let state = NetworkSheetState::default();
        for id in NetworkPortId::ALL {
            let check = layout.port_check(id);
            assert_eq!(
                network_sheet_hit_test(
                    &layout,
                    &state,
                    GuiPoint::new(check.x as f32, check.y as f32),
                ),
                Some(NetworkSheetHit::PortCheck(id))
            );
            let edit = layout.port_edit(id);
            assert_eq!(
                network_sheet_hit_test(
                    &layout,
                    &state,
                    GuiPoint::new(edit.x as f32, edit.y as f32),
                ),
                Some(NetworkSheetHit::Text(NetworkTextField::Port(id)))
            );
        }
    }

    #[test]
    fn disabled_port_and_alternate_server_edits_are_not_interactive() {
        let layout = NetworkSheetLayout::from_sheet(
            IntRect {
                x: 100,
                y: 80,
                w: 600,
                h: 400,
            },
            20,
        );
        let mut state = NetworkSheetState::new(
            [0, 0, 0, 0],
            false,
            "master.example".to_string(),
            true,
            true,
            "Host".to_string(),
            String::new(),
            false,
        );
        let center = |rect: IntRect| {
            GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
        };

        for id in NetworkPortId::ALL {
            assert_eq!(
                network_sheet_hit_test(&layout, &state, center(layout.port_edit(id))),
                None
            );
        }
        assert_eq!(
            network_sheet_hit_test(&layout, &state, center(layout.alternate_edit)),
            None
        );

        state.port_mut(NetworkPortId::Tcp).enabled = true;
        state.use_alternate_server = true;
        assert_eq!(
            network_sheet_hit_test(
                &layout,
                &state,
                center(layout.port_edit(NetworkPortId::Tcp))
            ),
            Some(NetworkSheetHit::Text(NetworkTextField::Port(
                NetworkPortId::Tcp
            )))
        );
        assert_eq!(
            network_sheet_hit_test(&layout, &state, center(layout.alternate_edit)),
            Some(NetworkSheetHit::Text(
                NetworkTextField::AlternateServerAddress
            ))
        );
    }
}
