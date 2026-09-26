//! The two CPUs' conversation: every byte either wrote to the four ports.

use crate::recording::apu::{ApuEventKind, ApuEvents};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortMessage {
    pub frame: u64,
    pub spc_cycle: u64,
    /// The S-CPU wrote it (a command); otherwise the SPC700 did (a reply).
    pub from_cpu: bool,
    pub port: u8,
    pub value: u8,
}

/// A frame's port writes, both ways, in order.
pub fn port_messages(e: &ApuEvents) -> Vec<PortMessage> {
    e.events
        .iter()
        .filter_map(|x| {
            let (from_cpu, port) = match x.kind {
                ApuEventKind::CpuPort => (true, x.address & 3),
                ApuEventKind::SpcIo if (4..=7).contains(&x.address) => (false, x.address - 4),
                _ => return None,
            };
            Some(PortMessage {
                frame: e.frame,
                spc_cycle: x.spc_cycle,
                from_cpu,
                port,
                value: x.value,
            })
        })
        .collect()
}
