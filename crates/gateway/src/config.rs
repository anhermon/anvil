/// Configuration for the WebSocket gateway.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Port to listen on. Defaults to `9000`.
    pub port: u16,
    /// Maximum number of broadcast events buffered in the channel.
    /// Lagging clients will miss events rather than block the agent.
    pub event_buffer: usize,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 9000,
            event_buffer: 256,
        }
    }
}
