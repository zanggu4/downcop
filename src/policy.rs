//! What the engine should do with traffic in each direction.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Action {
    /// Leave traffic alone.
    #[default]
    Pass,
    /// Drop every packet.
    Block,
    /// Shape traffic to at most this many bytes per second.
    Limit(u64),
}

impl Action {
    /// Whether this action requires capturing packets.
    pub fn intervenes(self) -> bool {
        !matches!(self, Action::Pass)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Policy {
    /// Inbound traffic.
    pub download: Action,
    /// Outbound traffic.
    pub upload: Action,
    /// Leave local network (LAN, link-local, multicast) traffic untouched.
    pub lan_exclude: bool,
}

impl Policy {
    #[cfg(test)]
    pub fn is_passthrough(&self) -> bool {
        !self.download.intervenes() && !self.upload.intervenes()
    }
}
