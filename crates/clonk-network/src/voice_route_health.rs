//! Authenticated round-trip evidence for a droppable media route. Control
//! connectivity and successful queue insertion do not prove media delivery.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ring::rand::{SecureRandom as _, SystemRandom};

use crate::voice::{
    encode_authenticated_voice_packet, VoiceMediaCipher, VoicePacket, VoiceRouteCookie,
};
use crate::{ClientId, ReliableUdpSessionHandle};

const PROBE_INTERVAL: Duration = Duration::from_millis(500);
const CONFIRMATION_LIFETIME: Duration = Duration::from_millis(1_500);
const MIN_REPLY_INTERVAL: Duration = Duration::from_millis(100);
const MAX_ROUTES: usize = 64;
const PROBES_PER_PASS: usize = 4;
const MAX_PENDING_PROBES: usize = 3;

struct RouteHealth {
    cookie: VoiceRouteCookie,
    pending: VecDeque<([u8; 16], Instant)>,
    last_probe_at: Option<Instant>,
    confirmed_at: Option<Instant>,
    last_reply_at: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct VoiceRouteHealth {
    routes: BTreeMap<SocketAddr, RouteHealth>,
}

impl VoiceRouteHealth {
    fn route(&mut self, peer: SocketAddr, cookie: VoiceRouteCookie) -> Option<&mut RouteHealth> {
        if self
            .routes
            .get(&peer)
            .is_some_and(|route| route.cookie != cookie)
        {
            self.routes.remove(&peer);
        }
        if !self.routes.contains_key(&peer) && self.routes.len() >= MAX_ROUTES {
            return None;
        }
        Some(self.routes.entry(peer).or_insert(RouteHealth {
            cookie,
            pending: VecDeque::with_capacity(MAX_PENDING_PROBES),
            last_probe_at: None,
            confirmed_at: None,
            last_reply_at: None,
        }))
    }

    pub(crate) fn confirmed(
        &self,
        peer: SocketAddr,
        cookie: VoiceRouteCookie,
        now: Instant,
    ) -> bool {
        self.routes.get(&peer).is_some_and(|route| {
            route.cookie == cookie
                && route.confirmed_at.is_some_and(|confirmed| {
                    now.saturating_duration_since(confirmed) < CONFIRMATION_LIFETIME
                })
        })
    }

    fn begin_probe(
        &mut self,
        peer: SocketAddr,
        cookie: VoiceRouteCookie,
        now: Instant,
    ) -> Option<[u8; 16]> {
        let route = self.route(peer, cookie)?;
        if route
            .last_probe_at
            .is_some_and(|sent| now.saturating_duration_since(sent) < PROBE_INTERVAL)
        {
            return None;
        }
        let mut nonce = [0; 16];
        SystemRandom::new().fill(&mut nonce).ok()?;
        route
            .pending
            .retain(|(_, sent)| now.saturating_duration_since(*sent) < CONFIRMATION_LIFETIME);
        if route.pending.len() == MAX_PENDING_PROBES {
            route.pending.pop_front();
        }
        route.pending.push_back((nonce, now));
        route.last_probe_at = Some(now);
        Some(nonce)
    }

    fn acknowledge(
        &mut self,
        peer: SocketAddr,
        cookie: VoiceRouteCookie,
        nonce: [u8; 16],
        now: Instant,
    ) {
        let Some(route) = self.route(peer, cookie) else {
            return;
        };
        if let Some(index) = route.pending.iter().position(|(expected, sent)| {
            *expected == nonce && now.saturating_duration_since(*sent) < CONFIRMATION_LIFETIME
        }) {
            route.pending.remove(index);
            route.confirmed_at = Some(now);
        }
    }

    /// Probe only admitted routes, with a bounded budget below control/media.
    /// Oldest unanswered routes go first so a full socket queue cannot starve
    /// later peers forever. Rekeying and route retirement erase old evidence.
    pub(crate) fn poll(
        &mut self,
        routes: &[(ClientId, SocketAddr, VoiceMediaCipher)],
        udp: Option<&ReliableUdpSessionHandle>,
        now: Instant,
    ) {
        self.routes.retain(|peer, health| {
            routes
                .iter()
                .any(|(_, current, cipher)| peer == current && health.cookie == cipher.cookie())
        });
        let Some(udp) = udp else {
            return;
        };
        let mut candidates = routes.iter().collect::<Vec<_>>();
        candidates.sort_by_key(|(_, peer, _)| {
            self.routes.get(peer).and_then(|route| route.last_probe_at)
        });
        let mut sent_count = 0;
        for (_, peer, cipher) in candidates {
            if sent_count == PROBES_PER_PASS {
                break;
            }
            let Some(nonce) = self.begin_probe(*peer, cipher.cookie(), now) else {
                continue;
            };
            let sent = encode_authenticated_voice_packet(cipher, &VoicePacket::Probe(nonce))
                .ok()
                .is_some_and(|wire| udp.try_send_voice_media(*peer, wire));
            if !sent {
                if let Some(route) = self.routes.get_mut(peer) {
                    route.pending.retain(|(pending, _)| *pending != nonce);
                    route.last_probe_at = None;
                }
                break;
            }
            sent_count += 1;
        }
    }

    /// Called only after the session authenticates the packet and its route.
    pub(crate) fn receive_control(
        &mut self,
        peer: SocketAddr,
        send_cipher: &VoiceMediaCipher,
        packet: VoicePacket,
        udp: Option<&ReliableUdpSessionHandle>,
        now: Instant,
    ) {
        match packet {
            VoicePacket::ProbeAck(nonce) => {
                self.acknowledge(peer, send_cipher.cookie(), nonce, now)
            }
            VoicePacket::Probe(nonce) => {
                let Some(route) = self.route(peer, send_cipher.cookie()) else {
                    return;
                };
                if route
                    .last_reply_at
                    .is_some_and(|last| now.saturating_duration_since(last) < MIN_REPLY_INTERVAL)
                {
                    return;
                }
                route.last_reply_at = Some(now);
                if let Some(udp) = udp {
                    if let Ok(wire) = encode_authenticated_voice_packet(
                        send_cipher,
                        &VoicePacket::ProbeAck(nonce),
                    ) {
                        let _ = udp.try_send_voice_media(peer, wire);
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn confirm_for_test(
        &mut self,
        peer: SocketAddr,
        cookie: VoiceRouteCookie,
        now: Instant,
    ) {
        let nonce = self.begin_probe(peer, cookie, now).unwrap();
        self.acknowledge(peer, cookie, nonce, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_cannot_confirm_a_foreign_route_or_amplify_replayed_requests() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 40_000));
        let other = SocketAddr::from(([127, 0, 0, 1], 40_001));
        let cookie = VoiceRouteCookie::from_bytes([7; 16]);
        let cipher = VoiceMediaCipher::from_parts(cookie, [8; 32]);
        let now = Instant::now();
        let mut health = VoiceRouteHealth::default();
        let nonce = health.begin_probe(peer, cookie, now).unwrap();
        health.acknowledge(other, cookie, nonce, now);
        assert!(!health.confirmed(other, cookie, now));
        let mut tampered = nonce;
        tampered[0] ^= 1;
        health.acknowledge(peer, cookie, tampered, now);
        assert!(!health.confirmed(peer, cookie, now));
        let (udp, mut queued) = ReliableUdpSessionHandle::test_voice_queue();
        for _ in 0..100 {
            health.receive_control(peer, &cipher, VoicePacket::Probe(nonce), Some(&udp), now);
        }
        assert!(queued.try_recv().is_ok());
        assert!(
            queued.try_recv().is_err(),
            "authenticated replays still get a bounded reply rate"
        );
        health.receive_control(peer, &cipher, VoicePacket::ProbeAck(nonce), Some(&udp), now);
        assert!(health.confirmed(peer, cookie, now));
        assert!(
            queued.try_recv().is_err(),
            "acknowledgements cannot trigger a reply loop"
        );
        let new_cookie = VoiceRouteCookie::from_bytes([9; 16]);
        let _new_nonce = health.begin_probe(peer, new_cookie, now).unwrap();
        health.acknowledge(peer, new_cookie, nonce, now);
        assert!(!health.confirmed(peer, new_cookie, now));
    }

    #[test]
    fn a_delayed_ack_survives_a_new_probe_without_extending_replay_validity() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 40_000));
        let cookie = VoiceRouteCookie::from_bytes([7; 16]);
        let now = Instant::now();
        let mut health = VoiceRouteHealth::default();
        let first = health.begin_probe(peer, cookie, now).unwrap();
        let _second = health
            .begin_probe(peer, cookie, now + Duration::from_millis(500))
            .unwrap();
        let ack_at = now + Duration::from_millis(700);
        health.acknowledge(peer, cookie, first, ack_at);
        assert!(
            health.confirmed(peer, cookie, ack_at),
            "a slower valid route must still be usable"
        );
        let expired = ack_at + CONFIRMATION_LIFETIME;
        health.acknowledge(peer, cookie, first, expired);
        assert!(
            !health.confirmed(peer, cookie, expired),
            "replaying an old ack cannot suppress fallback"
        );
    }

    #[test]
    fn an_acknowledgement_does_not_reset_the_probe_rate_budget() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 40_000));
        let cookie = VoiceRouteCookie::from_bytes([7; 16]);
        let now = Instant::now();
        let mut health = VoiceRouteHealth::default();
        let nonce = health.begin_probe(peer, cookie, now).unwrap();
        health.acknowledge(peer, cookie, nonce, now + Duration::from_millis(10));
        assert!(health.confirmed(peer, cookie, now + Duration::from_millis(50)));
        assert!(
            health
                .begin_probe(peer, cookie, now + Duration::from_millis(50))
                .is_none(),
            "successful probes must remain limited to two per second"
        );
    }
}
