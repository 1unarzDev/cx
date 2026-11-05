//! Task replies for network observation and neighbor probes.
use super::*;

impl App {
    pub(in crate::ui) fn apply_network_reply(&mut self, reply: Reply, value: Value) {
        match reply.op {
            Operation::Network => {
                let selection = self.network_selection();
                let mut value = value;
                value["_authenticated_at"] = serde_json::json!(transport::now());
                let candidates = self
                    .network
                    .get(&reply.device)
                    .map(|v| (v["candidates"].clone(), v["candidates_observed_at"].clone()));
                self.network.insert(reply.device, value);
                if let Some((candidates, observed)) = candidates {
                    self.network.get_mut(&reply.device).unwrap()["candidates"] = candidates;
                    self.network.get_mut(&reply.device).unwrap()["candidates_observed_at"] =
                        observed;
                }
                self.restore_network_selection(selection);
            }
            Operation::NetworkCandidates => {
                let selected = self.network_selection();
                self.network
                    .entry(reply.device)
                    .or_insert_with(|| serde_json::json!({}))["candidates"] =
                    value["candidates"].clone();
                self.network.get_mut(&reply.device).unwrap()["candidates_observed_at"] =
                    value["observed_at"].clone();
                if let Some(candidates) = self
                    .network
                    .get_mut(&reply.device)
                    .and_then(|v| v["candidates"].as_array_mut())
                {
                    for candidate in candidates {
                        let key = (
                            reply.device,
                            candidate["address"].as_str().unwrap_or("").to_owned(),
                            candidate["interface"].as_str().map(str::to_owned),
                        );
                        if let Some(evidence) = self.neighbor_evidence.get(&key).filter(|v| {
                            transport::now().saturating_sub(v["observed_at"].as_u64().unwrap_or(0))
                                <= 90
                        }) {
                            candidate["ssh"] = evidence["ssh"].clone();
                            candidate["observed_at"] = evidence["observed_at"].clone();
                        }
                    }
                }
                self.queue_neighbor_checks();
                self.pump_neighbor_checks();
                self.restore_network_selection(selected);
            }
            Operation::ProbeCandidate { address, interface } => {
                if let Some(candidates) = self
                    .network
                    .get_mut(&reply.device)
                    .and_then(|v| v["candidates"].as_array_mut())
                {
                    if let Some(candidate) = candidates.iter_mut().find(|c| {
                        c["address"].as_str() == Some(&address)
                            && c["interface"].as_str() == interface.as_deref()
                    }) {
                        candidate["ssh"] = value["ssh"].clone();
                        candidate["observed_at"] = value["observed_at"].clone();
                    }
                }
            }
            _ => {}
        }
    }
}
