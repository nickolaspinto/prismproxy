use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

/// Per-proxy counters and histograms exposed at /metrics (Prometheus text format).
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub requests_2xx: AtomicU64,
    pub requests_4xx: AtomicU64,
    pub requests_5xx: AtomicU64,
    pub requests_blocked: AtomicU64,
    /// Sum of all response times in milliseconds (for computing mean)
    pub response_time_ms_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record(&self, status: u16, elapsed_ms: u64, blocked: bool) {
        self.requests_total.fetch_add(1, Relaxed);
        self.response_time_ms_total.fetch_add(elapsed_ms, Relaxed);
        if blocked {
            self.requests_blocked.fetch_add(1, Relaxed);
            return;
        }
        match status {
            200..=299 => self.requests_2xx.fetch_add(1, Relaxed),
            400..=499 => self.requests_4xx.fetch_add(1, Relaxed),
            500..=599 => self.requests_5xx.fetch_add(1, Relaxed),
            _ => 0,
        };
    }

    /// Render Prometheus text format.
    pub fn render(&self) -> String {
        let total = self.requests_total.load(Relaxed);
        let elapsed = self.response_time_ms_total.load(Relaxed);
        let mean_ms = if total > 0 { elapsed / total } else { 0 };

        format!(
            "# HELP prismproxy_requests_total Total requests handled\n\
             # TYPE prismproxy_requests_total counter\n\
             prismproxy_requests_total {total}\n\
             # HELP prismproxy_requests_2xx 2xx responses\n\
             # TYPE prismproxy_requests_2xx counter\n\
             prismproxy_requests_2xx {}\n\
             # HELP prismproxy_requests_4xx 4xx responses\n\
             # TYPE prismproxy_requests_4xx counter\n\
             prismproxy_requests_4xx {}\n\
             # HELP prismproxy_requests_5xx 5xx responses\n\
             # TYPE prismproxy_requests_5xx counter\n\
             prismproxy_requests_5xx {}\n\
             # HELP prismproxy_requests_blocked Requests blocked by plugins\n\
             # TYPE prismproxy_requests_blocked counter\n\
             prismproxy_requests_blocked {}\n\
             # HELP prismproxy_response_time_ms_mean Mean response time in ms\n\
             # TYPE prismproxy_response_time_ms_mean gauge\n\
             prismproxy_response_time_ms_mean {mean_ms}\n",
            self.requests_2xx.load(Relaxed),
            self.requests_4xx.load(Relaxed),
            self.requests_5xx.load(Relaxed),
            self.requests_blocked.load(Relaxed),
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            requests_blocked: AtomicU64::new(0),
            response_time_ms_total: AtomicU64::new(0),
        }
    }
}
