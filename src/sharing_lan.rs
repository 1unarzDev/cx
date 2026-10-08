//! Existing-LAN sharing plans never replace connection profiles or robot addresses.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::Ipv4Addr};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipient {
    pub id: String,
    pub address: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Vec<Ipv4Addr>,
    /// Fresh end-to-end evidence that this recipient already exits through
    /// the sharing host (robot HTTPS plus matching gateway NAT counters).
    #[serde(default)]
    pub egress_via_host_verified_at: Option<u64>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GatewayAction {
    Keep {
        id: String,
        gateway: Ipv4Addr,
    },
    /// A trial route must be independently rolled back; the original default
    /// and all addresses/local routes remain in place.
    TrialRoute {
        id: String,
        via: Ipv4Addr,
        preserve_gateway: Ipv4Addr,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub observed_at: u64,
    pub downstream: String,
    pub upstream: String,
    pub gateway: Ipv4Addr,
    pub recipients: Vec<Recipient>,
    pub mesh_addresses: Vec<Ipv4Addr>,
    pub forwarding_enabled: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub observation: Observation,
    pub nft_rules: String,
    pub required_gateway_changes: Vec<String>,
    pub gateway_actions: Vec<GatewayAction>,
    pub required_dns_configuration: Vec<String>,
    pub preserves_addresses_and_profiles: bool,
    pub ready_for_apply: bool,
    pub verification: Vec<String>,
}
fn iface(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 15
        && s != "lo"
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn unicast(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && !ip.is_link_local()
}
pub fn preview(id: &str, obs: Observation, now: u64) -> Result<Plan> {
    ensure!(
        !id.is_empty()
            && id.len() <= 32
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid sharing id"
    );
    ensure!(
        now >= obs.observed_at && now - obs.observed_at <= 30,
        "refresh downstream evidence"
    );
    ensure!(
        iface(&obs.downstream) && iface(&obs.upstream) && obs.downstream != obs.upstream,
        "invalid sharing interfaces"
    );
    ensure!(
        unicast(obs.gateway)
            && (1..=32).contains(&obs.recipients.len())
            && (1..=128).contains(&obs.mesh_addresses.len()),
        "invalid sharing scope"
    );
    let mut ips = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for r in &obs.recipients {
        ensure!(
            !r.id.is_empty()
                && r.id.len() <= 256
                && ids.insert(&r.id)
                && unicast(r.address)
                && ips.insert(r.address)
                && r.address != obs.gateway
                && !obs.mesh_addresses.contains(&r.address)
                && r.dns.len() <= 8
                && r.dns.iter().copied().all(unicast),
            "invalid or overlapping recipient"
        );
    }
    ensure!(
        obs.mesh_addresses.iter().copied().all(unicast),
        "invalid mesh destination"
    );
    let sources = obs
        .recipients
        .iter()
        .map(|r| r.address.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mesh = obs
        .mesh_addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    // Dedicated tables; policy accept never claims to supersede another firewall.
    // Reject new reverse SSH only. Established replies and robotics traffic are preserved.
    let nft_rules = format!("table inet cx_{id} {{\n set robots {{ type ipv4_addr; elements = {{ {sources} }} }}\n set mesh {{ type ipv4_addr; elements = {{ {mesh} }} }}\n chain input {{ type filter hook input priority -10; policy accept;\n  iifname \"{down}\" ip saddr @robots tcp dport 22 ct state new reject with tcp reset;\n }}\n chain forward {{ type filter hook forward priority -10; policy accept;\n  iifname \"{down}\" ip saddr @robots ip daddr @mesh tcp dport 22 ct state new reject with tcp reset;\n  iifname \"{down}\" oifname \"{up}\" ip saddr @robots counter accept;\n  iifname \"{up}\" oifname \"{down}\" ip daddr @robots ct state established,related counter accept;\n }}\n}}\ntable ip cx_{id}_nat {{\n chain postrouting {{ type nat hook postrouting priority srcnat; policy accept;\n  oifname \"{up}\" ip saddr {{ {sources} }} counter masquerade;\n }}\n}}\n", down=obs.downstream,up=obs.upstream);
    let gateway_actions = obs
        .recipients
        .iter()
        .map(|r| {
            let verified = r
                .egress_via_host_verified_at
                .is_some_and(|at| now >= at && now - at <= 30);
            if r.gateway == obs.gateway || verified {
                GatewayAction::Keep {
                    id: r.id.clone(),
                    gateway: r.gateway,
                }
            } else {
                GatewayAction::TrialRoute {
                    id: r.id.clone(),
                    via: obs.gateway,
                    preserve_gateway: r.gateway,
                }
            }
        })
        .collect::<Vec<_>>();
    let required_gateway_changes = gateway_actions
        .iter()
        .filter_map(|a| match a {
            GatewayAction::TrialRoute { id, .. } => Some(id.clone()),
            GatewayAction::Keep { .. } => None,
        })
        .collect();
    let required_dns_configuration = obs
        .recipients
        .iter()
        .filter(|r| r.dns.is_empty())
        .map(|r| r.id.clone())
        .collect::<Vec<_>>();
    // IPv6 reverse access, other sshd ports and pre-existing rules need inspection.
    // This planner deliberately cannot attest enforcement from IPv4 inputs alone.
    Ok(Plan { id:id.into(), observation:obs, nft_rules, required_gateway_changes, gateway_actions,
        required_dns_configuration, preserves_addresses_and_profiles:true, ready_for_apply:false,
        verification: vec!["Verify the proposed gateway is already assigned to the sharing host; never adopt an occupied router address".into(),
            "Inspect IPv6, all SSH listener ports, existing firewall and effective upstream policy".into(),
            "Acquire privilege and owned transaction lock; persist rollback and schedule host-local watchdog before apply".into(),
            "Verify each robot's DNS and HTTPS plus tranquility NAT counters and preserved SSH/robot routes".into(),
            "Verify reverse SSH denial in IPv4 and IPv6 before reporting enforcement".into()] })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> Observation {
        Observation {
            observed_at: 100,
            downstream: "enp63s0".into(),
            upstream: "CloudflareWARP".into(),
            gateway: "192.168.0.2".parse().unwrap(),
            recipients: vec![Recipient {
                id: "blastoise".into(),
                address: "192.168.0.147".parse().unwrap(),
                gateway: "192.168.0.1".parse().unwrap(),
                dns: vec![],
                egress_via_host_verified_at: None,
            }],
            mesh_addresses: vec!["100.96.0.12".parse().unwrap()],
            forwarding_enabled: true,
        }
    }
    #[test]
    fn host_nat_does_not_claim_internet_without_robot_gateway_dns_and_verification() {
        let p = preview("robot_lan", observation(), 100).unwrap();
        assert!(p.preserves_addresses_and_profiles && !p.ready_for_apply);
        assert_eq!(p.required_gateway_changes, ["blastoise"]);
        assert_eq!(p.required_dns_configuration, ["blastoise"]);
        assert!(p.nft_rules.contains("ct state new reject"));
        assert!(p.nft_rules.contains("ct state established,related"));
        assert!(!p.nft_rules.contains("flush ruleset") && !p.nft_rules.contains("policy drop"));
    }
    #[test]
    fn stale_injected_and_overlapping_inputs_fail_closed() {
        assert!(preview("robot_lan", observation(), 131).is_err());
        let mut o = observation();
        o.upstream = "eth0\"; flush ruleset".into();
        assert!(preview("robot_lan", o, 100).is_err());
        let mut o = observation();
        o.recipients[0].address = o.mesh_addresses[0];
        assert!(preview("robot_lan", o, 100).is_err());
        assert!(preview("bad; id", observation(), 100).is_err());
    }
    #[test]
    fn working_indirect_gateway_is_reused_only_with_fresh_end_to_end_evidence() {
        let mut o = observation();
        o.recipients[0].egress_via_host_verified_at = Some(100);
        let p = preview("robot_lan", o.clone(), 100).unwrap();
        assert!(p.required_gateway_changes.is_empty());
        assert!(
            matches!(&p.gateway_actions[0], GatewayAction::Keep { gateway, .. } if *gateway == "192.168.0.1".parse::<Ipv4Addr>().unwrap())
        );
        o.recipients[0].egress_via_host_verified_at = Some(101);
        assert_eq!(
            preview("robot_lan", o.clone(), 100)
                .unwrap()
                .required_gateway_changes,
            ["blastoise"]
        );
        o.recipients[0].egress_via_host_verified_at = Some(69);
        let p = preview("robot_lan", o, 100).unwrap();
        assert!(matches!(
            p.gateway_actions[0],
            GatewayAction::TrialRoute { .. }
        ));
    }
}
