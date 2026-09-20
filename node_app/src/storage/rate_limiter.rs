use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::warn;

/// Rate limiter for prefix queries to prevent DoS attacks
pub struct PrefixQueryRateLimiter {
    requests: Mutex<HashMap<String, Vec<Instant>>>,
    max_requests_per_minute: u32,
}

impl PrefixQueryRateLimiter {
    pub fn new(max_requests_per_minute: u32) -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            max_requests_per_minute,
        }
    }

    /// Check if a prefix query is allowed for the given client
    pub fn is_allowed(&self, client_id: &str, prefix: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let key = format!("{client_id}:{prefix}");

        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);

        // Clean up old requests
        if let Some(timestamps) = requests.get_mut(&key) {
            timestamps.retain(|&timestamp| timestamp > one_minute_ago);

            // Check if we're under the limit
            if timestamps.len() < self.max_requests_per_minute as usize {
                timestamps.push(now);
                true
            } else {
                warn!(
                    "Rate limit exceeded for client {} with prefix {}: {} requests in the last minute",
                    client_id, prefix, timestamps.len()
                );
                false
            }
        } else {
            // First request for this client/prefix combination
            requests.insert(key, vec![now]);
            true
        }
    }
}

#[cfg(test)]
impl PrefixQueryRateLimiter {
    pub fn get_request_count(&self, client_id: &str, prefix: &str) -> usize {
        let requests = self.requests.lock().unwrap();
        let key = format!("{client_id}:{prefix}");

        if let Some(timestamps) = requests.get(&key) {
            let now = Instant::now();
            let one_minute_ago = now - Duration::from_secs(60);
            timestamps
                .iter()
                .filter(|&&timestamp| timestamp > one_minute_ago)
                .count()
        } else {
            0
        }
    }

    pub fn cleanup(&self) {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);

        let mut to_remove = Vec::new();
        let mut to_update = Vec::new();

        for (key, timestamps) in requests.iter() {
            let active_requests: Vec<Instant> = timestamps
                .iter()
                .filter(|&&timestamp| timestamp > one_minute_ago)
                .cloned()
                .collect();

            if active_requests.is_empty() {
                to_remove.push(key.clone());
            } else {
                to_update.push((key.clone(), active_requests));
            }
        }

        for (key, active_requests) in to_update {
            requests.insert(key, active_requests);
        }

        for key in &to_remove {
            requests.remove(key);
        }

        tracing::info!(
            "Rate limiter cleanup: removed {} inactive entries",
            to_remove.len()
        );
    }
}

impl Default for PrefixQueryRateLimiter {
    fn default() -> Self {
        Self::new(60) // Default: 60 requests per minute
    }
}
