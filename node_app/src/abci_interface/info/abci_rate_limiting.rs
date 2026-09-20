use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use abci::types::{RequestQuery, ResponseQuery};
use eld_common::constants::{abci_query, ELD_CODESPACE};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Rate limit multiplier for easy adjustment of all limits
const RATE_LIMIT_MULTIPLIER: u32 = 1000;

/// Query types for ABCI rate limiting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbciQueryType {
    CadoList,  // get_cados_by_prefix - most expensive
    SingleKey, // get_account_by_address - cheap
    Other,     // other queries
}

impl AbciQueryType {
    /// Get the global rate limit for this query type
    pub fn get_global_rate_limit(&self) -> u32 {
        match self {
            AbciQueryType::CadoList => 10 * RATE_LIMIT_MULTIPLIER, // Most expensive
            AbciQueryType::SingleKey => 100 * RATE_LIMIT_MULTIPLIER, // Cheap
            AbciQueryType::Other => 50 * RATE_LIMIT_MULTIPLIER,    // Default
        }
    }

    /// Get the result limit for this query type
    pub fn get_result_limit(&self) -> usize {
        match self {
            AbciQueryType::CadoList => 100 * RATE_LIMIT_MULTIPLIER as usize,
            AbciQueryType::SingleKey => 1, // Single key queries always return 1 result
            AbciQueryType::Other => 30 * RATE_LIMIT_MULTIPLIER as usize,
        }
    }
}

impl std::fmt::Display for AbciQueryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbciQueryType::CadoList => write!(f, "{}", abci_query::CADO_LIST),
            AbciQueryType::SingleKey => write!(f, "{}", abci_query::SINGLE_KEY),
            AbciQueryType::Other => write!(f, "{}", abci_query::OTHER),
        }
    }
}

/// Global rate limiter for ABCI queries
pub struct GlobalAbciRateLimiter {
    cado_list_requests: AtomicU32,
    single_key_requests: AtomicU32,
    other_requests: AtomicU32,
    last_reset: AtomicU64, // timestamp
}

impl GlobalAbciRateLimiter {
    pub fn new() -> Self {
        Self {
            cado_list_requests: AtomicU32::new(0),
            single_key_requests: AtomicU32::new(0),
            other_requests: AtomicU32::new(0),
            last_reset: AtomicU64::new(0),
        }
    }

    fn classify_query_type(path: &str) -> AbciQueryType {
        match path {
            abci_query::CADO_LIST | abci_query::CADO_PATHS | abci_query::PINBOARD_FEED => {
                AbciQueryType::CadoList
            }
            abci_query::CADO
            | abci_query::STAKING_ACCOUNT
            | abci_query::CAPACITY_PROVIDER
            | abci_query::PINBOARD => AbciQueryType::SingleKey,
            _ => AbciQueryType::Other,
        }
    }

    /// Check global rate limit for a query request.
    pub fn check_rate_limit(&self, query_request: &RequestQuery) -> Option<ResponseQuery> {
        let query_type = Self::classify_query_type(&query_request.path);
        self.check_rate_limit_by_type(query_type)
    }

    /// Check global rate limit for a query type.
    pub fn check_rate_limit_by_type(&self, query_type: AbciQueryType) -> Option<ResponseQuery> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let window_start = now - (now % 60); // 1-minute windows

        // Reset counters if we're in a new minute
        if self.last_reset.load(Ordering::Relaxed) < window_start {
            self.reset_counters();
            self.last_reset.store(window_start, Ordering::Relaxed);
        }

        // Check and increment counter
        let limit = query_type.get_global_rate_limit();
        let current = match query_type {
            AbciQueryType::CadoList => {
                let current = self.cado_list_requests.fetch_add(1, Ordering::Relaxed);
                if current >= limit {
                    warn!(
                        "Global rate limit exceeded for cado_list: {} requests per minute",
                        limit
                    );
                    return Some(ResponseQuery {
                        code: 100,
                        codespace: ELD_CODESPACE.to_string(),
                        log: format!(
                            "Global rate limit exceeded for cado_list: {limit} requests per minute"
                        ),
                        info: format!(
                            "Global rate limit exceeded for cado_list: {limit} requests per minute"
                        ),
                        ..Default::default()
                    });
                }
                current + 1
            }
            AbciQueryType::SingleKey => {
                let current = self.single_key_requests.fetch_add(1, Ordering::Relaxed);
                if current >= limit {
                    warn!(
                        "Global rate limit exceeded for single_key: {} requests per minute",
                        limit
                    );
                    return Some(ResponseQuery {
                        code: 100,
                        codespace: ELD_CODESPACE.to_string(),
                        log: format!(
                            "Global rate limit exceeded for single_key: {limit} requests per minute"
                        ),
                        info: format!(
                            "Global rate limit exceeded for single_key: {limit} requests per minute"
                        ),
                        ..Default::default()
                    });
                }
                current + 1
            }
            AbciQueryType::Other => {
                let current = self.other_requests.fetch_add(1, Ordering::Relaxed);
                if current >= limit {
                    warn!(
                        "Global rate limit exceeded for other queries: {} requests per minute",
                        limit
                    );
                    return Some(ResponseQuery {
                        code: 100,
                        codespace: ELD_CODESPACE.to_string(),
                        log: format!(
                            "Global rate limit exceeded for other queries: {limit} requests per minute"
                        ),
                        info: format!(
                            "Global rate limit exceeded for other queries: {limit} requests per minute"
                        ),
                        ..Default::default()
                    });
                }
                current + 1
            }
        };

        info!(
            "Global rate limit check passed for {}: {}/{} requests",
            query_type, current, limit
        );
        None
    }

    /// Reset all counters
    fn reset_counters(&self) {
        self.cado_list_requests.store(0, Ordering::Relaxed);
        self.single_key_requests.store(0, Ordering::Relaxed);
        self.other_requests.store(0, Ordering::Relaxed);
        info!("Global ABCI rate limiter counters reset");
    }
}

impl Default for GlobalAbciRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Result limiter for ABCI queries
pub struct ResultLimiter {
    total_results_returned: AtomicU32,
    last_reset: AtomicU64,
}

impl ResultLimiter {
    pub fn new() -> Self {
        Self {
            total_results_returned: AtomicU32::new(0),
            last_reset: AtomicU64::new(0),
        }
    }

    /// Check if the result count is within limits
    pub fn check_result_limit(
        &self,
        result_count: usize,
        query_type: AbciQueryType,
    ) -> Option<ResponseQuery> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let window_start = now - (now % 60);

        // Reset counter if we're in a new minute
        if self.last_reset.load(Ordering::Relaxed) < window_start {
            self.total_results_returned.store(0, Ordering::Relaxed);
            self.last_reset.store(window_start, Ordering::Relaxed);
        }

        // Check per-query type result limit first
        let query_limit = query_type.get_result_limit();
        if result_count > query_limit {
            warn!(
                "Result limit exceeded for {}: {} results returned (limit: {})",
                query_type, result_count, query_limit
            );
            return Some(ResponseQuery {
                code: 101,
                codespace: ELD_CODESPACE.to_string(),
                log: format!(
                    "Result limit exceeded for {query_type}: {result_count} results returned (limit: {query_limit})"
                ),
                info: format!(
                    "Result limit exceeded for {query_type}: {result_count} results returned (limit: {query_limit})"
                ),
                ..Default::default()
            });
        }

        // Check global result limit (1000 results per minute)
        const GLOBAL_RESULT_LIMIT: u32 = 1000 * RATE_LIMIT_MULTIPLIER;
        let current = self.total_results_returned.load(Ordering::Relaxed);
        if current + result_count as u32 > GLOBAL_RESULT_LIMIT {
            warn!(
                "Global result limit exceeded: {} results returned in the last minute (limit: {})",
                current + result_count as u32,
                GLOBAL_RESULT_LIMIT
            );
            return Some(ResponseQuery {
                code: 101,
                codespace: ELD_CODESPACE.to_string(),
                log: format!("Global result limit exceeded: {} results returned in the last minute (limit: {})", current + result_count as u32, GLOBAL_RESULT_LIMIT),
                info: format!("Global result limit exceeded: {} results returned in the last minute (limit: {})", current + result_count as u32, GLOBAL_RESULT_LIMIT),
                ..Default::default()
            });
        }

        // Increment the counter only if both checks pass
        self.total_results_returned
            .fetch_add(result_count as u32, Ordering::Relaxed);

        info!(
            "Result limit check passed for {}: {} results (query limit: {}, global total: {})",
            query_type,
            result_count,
            query_limit,
            current + result_count as u32
        );
        None
    }
}

impl Default for ResultLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Query-based rate limiter for ABCI queries
pub struct QueryBasedRateLimiter {
    requests: Mutex<HashMap<String, Vec<Instant>>>,
}

impl QueryBasedRateLimiter {
    pub fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
        }
    }

    /// Check rate limit for a specific query
    pub fn check_rate_limit(
        &self,
        query_request: &abci::types::RequestQuery,
    ) -> Option<ResponseQuery> {
        let query_hash = self.hash_query(query_request);
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);

        // Get or create entry for this query
        let mut requests = match self.requests.lock() {
            Ok(requests) => requests,
            Err(e) => {
                warn!("Failed to lock requests: {}", e);
                return Some(ResponseQuery {
                    code: 102,
                    codespace: ELD_CODESPACE.to_string(),
                    log: format!("Failed to lock requests: {e}"),
                    info: format!("Failed to lock requests: {e}"),
                    ..Default::default()
                });
            }
        };

        let timestamps = requests.entry(query_hash.clone()).or_insert_with(Vec::new);

        // Remove old timestamps
        timestamps.retain(|&ts| ts > one_minute_ago);

        // Check limit based on query type
        let query_type = self.classify_query_type(&query_request.path);
        let limit = self.get_limit_for_query_type(&query_type);

        if timestamps.len() >= limit {
            warn!(
                "Query rate limit exceeded for query '{}': {} requests per minute",
                query_hash, limit
            );
            return Some(ResponseQuery {
                code: 102,
                codespace: ELD_CODESPACE.to_string(),
                log: format!(
                    "Query rate limit exceeded for query '{query_hash}': {limit} requests per minute"
                ),
                info: format!(
                    "Query rate limit exceeded for query '{query_hash}': {limit} requests per minute"
                ),
                ..Default::default()
            });
        }

        timestamps.push(now);

        info!(
            "Query rate limit check passed for '{}': {}/{} requests",
            query_hash,
            timestamps.len(),
            limit
        );
        None
    }

    /// Hash the query to identify similar queries
    fn hash_query(&self, query: &abci::types::RequestQuery) -> String {
        let mut hasher = Sha256::new();
        hasher.update(query.path.as_bytes());
        hasher.update(&query.data);
        format!("{:x}", hasher.finalize())
    }

    /// Classify the query type based on the path
    fn classify_query_type(&self, path: &str) -> AbciQueryType {
        match path {
            abci_query::CADO_LIST | abci_query::CADO_PATHS | abci_query::PINBOARD_FEED => {
                AbciQueryType::CadoList
            }
            abci_query::CADO
            | abci_query::STAKING_ACCOUNT
            | abci_query::CAPACITY_PROVIDER
            | abci_query::PINBOARD => AbciQueryType::SingleKey,
            _ => AbciQueryType::Other,
        }
    }

    /// Get the rate limit for a query type
    fn get_limit_for_query_type(&self, query_type: &AbciQueryType) -> usize {
        match query_type {
            AbciQueryType::CadoList => 5 * RATE_LIMIT_MULTIPLIER as usize, // Most expensive - limit repeated identical queries
            AbciQueryType::SingleKey => 20 * RATE_LIMIT_MULTIPLIER as usize, // Cheap
            AbciQueryType::Other => 15 * RATE_LIMIT_MULTIPLIER as usize,   // Default
        }
    }
}

impl Default for QueryBasedRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_rate_limiter_basic() {
        let limiter = GlobalAbciRateLimiter::new();

        // Should allow first 10 * RATE_LIMIT_MULTIPLIER requests
        for i in 0..(10 * RATE_LIMIT_MULTIPLIER) {
            assert!(
                limiter
                    .check_rate_limit_by_type(AbciQueryType::CadoList)
                    .is_none(),
                "Request {i} should be allowed"
            );
        }

        // Next request should be denied
        assert!(limiter
            .check_rate_limit_by_type(AbciQueryType::CadoList)
            .is_some());
    }

    #[test]
    fn test_global_rate_limiter_different_types() {
        let limiter = GlobalAbciRateLimiter::new();

        // Should be limited for one query type
        for i in 0..(10 * RATE_LIMIT_MULTIPLIER) {
            assert!(
                limiter
                    .check_rate_limit_by_type(AbciQueryType::CadoList)
                    .is_none(),
                "Request {i} should be allowed"
            );
        }
        assert!(limiter
            .check_rate_limit_by_type(AbciQueryType::CadoList)
            .is_some());

        // Should still be allowed for different query type
        assert!(limiter
            .check_rate_limit_by_type(AbciQueryType::SingleKey)
            .is_none());
    }

    #[test]
    fn test_query_type_limits() {
        assert_eq!(
            AbciQueryType::CadoList.get_global_rate_limit(),
            10 * RATE_LIMIT_MULTIPLIER
        );
        assert_eq!(
            AbciQueryType::SingleKey.get_global_rate_limit(),
            100 * RATE_LIMIT_MULTIPLIER
        );
        assert_eq!(
            AbciQueryType::Other.get_global_rate_limit(),
            50 * RATE_LIMIT_MULTIPLIER
        );
    }

    #[test]
    fn test_query_type_result_limits() {
        assert_eq!(
            AbciQueryType::CadoList.get_result_limit(),
            100 * RATE_LIMIT_MULTIPLIER as usize
        );
        assert_eq!(AbciQueryType::SingleKey.get_result_limit(), 1);
        assert_eq!(
            AbciQueryType::Other.get_result_limit(),
            30 * RATE_LIMIT_MULTIPLIER as usize
        );
    }

    #[test]
    fn test_result_limiter_basic() {
        let limiter = ResultLimiter::new();

        // Should allow results within limit
        assert!(limiter
            .check_result_limit(50 * RATE_LIMIT_MULTIPLIER as usize, AbciQueryType::CadoList)
            .is_none());
        assert!(limiter
            .check_result_limit(10 * RATE_LIMIT_MULTIPLIER as usize, AbciQueryType::Other)
            .is_none());

        // Should deny results exceeding query type limit
        assert!(limiter
            .check_result_limit(
                150 * RATE_LIMIT_MULTIPLIER as usize,
                AbciQueryType::CadoList
            )
            .is_some()); // Over 100 * RATE_LIMIT_MULTIPLIER limit
        assert!(limiter
            .check_result_limit(60 * RATE_LIMIT_MULTIPLIER as usize, AbciQueryType::Other)
            .is_some()); // Over 30 * RATE_LIMIT_MULTIPLIER limit
    }

    #[test]
    fn test_result_limiter_global_limit() {
        let limiter = ResultLimiter::new();

        // Should allow results up to global limit (but within per-query limits)
        assert!(limiter
            .check_result_limit(
                100 * RATE_LIMIT_MULTIPLIER as usize,
                AbciQueryType::CadoList
            )
            .is_none()); // At the limit
        assert!(limiter
            .check_result_limit(30 * RATE_LIMIT_MULTIPLIER as usize, AbciQueryType::Other)
            .is_none()); // At the limit

        // Should deny when exceeding global limit
        assert!(limiter
            .check_result_limit(851 * RATE_LIMIT_MULTIPLIER as usize, AbciQueryType::Other)
            .is_some()); // Over 1000 * RATE_LIMIT_MULTIPLIER global limit
    }

    #[test]
    fn test_time_window_logic() {
        let limiter = GlobalAbciRateLimiter::new();

        // Test that the time window logic works correctly
        // This test verifies that the <= comparison handles edge cases properly

        // First request should work
        assert!(limiter
            .check_rate_limit_by_type(AbciQueryType::CadoList)
            .is_none());

        // Simulate a scenario where last_reset might be greater than window_start
        // (This could happen with clock adjustments, though rare)
        // The <= logic should handle this gracefully

        // Multiple requests should work within the same minute
        for i in 0..5 {
            assert!(
                limiter
                    .check_rate_limit_by_type(AbciQueryType::CadoList)
                    .is_none(),
                "Request {i} should be allowed"
            );
        }
    }

    #[test]
    fn test_query_based_rate_limiter_basic() {
        use abci::types::RequestQuery;

        let limiter = QueryBasedRateLimiter::new();

        // Create a test query
        let query = RequestQuery {
            path: abci_query::CADO_LIST.to_string(),
            data: b"test_prefix".to_vec(),
            height: 0,
            prove: false,
        };

        // Should allow first 5 * RATE_LIMIT_MULTIPLIER requests (limit for cado_list)
        for i in 0..(5 * RATE_LIMIT_MULTIPLIER) {
            assert!(
                limiter.check_rate_limit(&query).is_none(),
                "Request {i} should be allowed"
            );
        }

        // Next request should be denied
        assert!(limiter.check_rate_limit(&query).is_some());
    }

    #[test]
    fn test_query_based_rate_limiter_different_queries() {
        use abci::types::RequestQuery;

        let limiter = QueryBasedRateLimiter::new();

        // Create different queries
        let query1 = RequestQuery {
            path: abci_query::CADO_LIST.to_string(),
            data: b"prefix1".to_vec(),
            height: 0,
            prove: false,
        };

        let query2 = RequestQuery {
            path: abci_query::CADO_LIST.to_string(),
            data: b"prefix2".to_vec(),
            height: 0,
            prove: false,
        };

        // Should be limited for one query
        for i in 0..(5 * RATE_LIMIT_MULTIPLIER) {
            assert!(
                limiter.check_rate_limit(&query1).is_none(),
                "Request {i} should be allowed"
            );
        }
        assert!(limiter.check_rate_limit(&query1).is_some());

        // Should still be allowed for different query
        assert!(limiter.check_rate_limit(&query2).is_none());
    }

    #[test]
    fn test_query_based_rate_limiter_hash_consistency() {
        use abci::types::RequestQuery;

        let limiter = QueryBasedRateLimiter::new();

        // Create identical queries
        let query1 = RequestQuery {
            path: abci_query::CADO_LIST.to_string(),
            data: b"same_prefix".to_vec(),
            height: 0,
            prove: false,
        };

        let query2 = RequestQuery {
            path: abci_query::CADO_LIST.to_string(),
            data: b"same_prefix".to_vec(),
            height: 0,
            prove: false,
        };

        // Should be treated as the same query
        let initial_requests = 2 * RATE_LIMIT_MULTIPLIER;
        for _ in 0..initial_requests {
            assert!(limiter.check_rate_limit(&query1).is_none());
            assert!(limiter.check_rate_limit(&query2).is_none());
        }

        // Both should count towards the same limit
        let remaining_requests = RATE_LIMIT_MULTIPLIER;
        for i in 0..remaining_requests {
            assert!(
                limiter.check_rate_limit(&query1).is_none(),
                "Request {i} should be allowed"
            );
        }
        assert!(limiter.check_rate_limit(&query1).is_some()); // Over the limit
    }
}
