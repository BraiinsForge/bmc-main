// Copyright (C) 2026  Braiins Systems s.r.o.

//! Per-runtime limits for host-side resources spawned on behalf of a widget.

/// Caps for background resources owned by a single [`crate::WasmWidgetRuntime`].
///
/// These defaults are intentionally conservative because widgets run in an
/// embedded environment. Hosts may override them through [`crate::RuntimeConfig`]
/// for trusted workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeResourceLimits {
    /// Maximum total fetch slots (`host_fetch` in flight + delayed fetch queue).
    pub max_fetches: usize,
    /// Maximum active WebSocket connections.
    pub max_websockets: usize,
    /// Maximum active TCP/TLS sockets combined.
    pub max_sockets: usize,
    /// Maximum active mDNS browse sessions.
    pub max_mdns_browses: usize,
    /// Maximum active mDNS registrations.
    pub max_mdns_registrations: usize,
    /// Maximum active SSDP searches.
    pub max_ssdp_searches: usize,
    /// Maximum active UDP broadcast sessions.
    pub max_udp_broadcasts: usize,
    /// Maximum active HTTP listeners.
    pub max_http_listeners: usize,
}

impl Default for RuntimeResourceLimits {
    fn default() -> Self {
        Self {
            max_fetches: 16,
            max_websockets: 4,
            max_sockets: 8,
            max_mdns_browses: 4,
            max_mdns_registrations: 4,
            max_ssdp_searches: 4,
            max_udp_broadcasts: 4,
            max_http_listeners: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeResourceLimits;

    #[test]
    fn defaults_are_non_zero() {
        let limits = RuntimeResourceLimits::default();

        assert!(limits.max_fetches > 0);
        assert!(limits.max_websockets > 0);
        assert!(limits.max_sockets > 0);
        assert!(limits.max_mdns_browses > 0);
        assert!(limits.max_mdns_registrations > 0);
        assert!(limits.max_ssdp_searches > 0);
        assert!(limits.max_udp_broadcasts > 0);
        assert!(limits.max_http_listeners > 0);
    }
}
