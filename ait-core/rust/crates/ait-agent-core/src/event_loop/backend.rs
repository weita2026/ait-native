use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventLoopBackend {
    LinuxEpoll,
    PortablePoll,
}

impl AgentEventLoopBackend {
    pub fn current_platform_default() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::LinuxEpoll
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::PortablePoll
        }
    }

    pub fn is_epoll(self) -> bool {
        matches!(self, Self::LinuxEpoll)
    }

    pub fn supports_many_workers(self) -> bool {
        matches!(self, Self::LinuxEpoll)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LinuxEpoll => "linux_epoll",
            Self::PortablePoll => "portable_poll",
        }
    }

    pub fn from_label(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "linux_epoll" | "epoll" => Some(Self::LinuxEpoll),
            "portable_poll" | "poll" | "portable" => Some(Self::PortablePoll),
            _ => None,
        }
    }
}
