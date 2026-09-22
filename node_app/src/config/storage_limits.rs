use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageLimits {
    /// Maximum number of results returned by prefix queries (default: 1000)
    #[serde(default = "default_max_prefix_results")]
    pub max_prefix_results: usize,
    /// Maximum number of results per page for paginated queries (default: 100)
    #[serde(default = "default_max_page_size")]
    pub max_page_size: usize,
    /// Rate limit for prefix queries per minute (default: 60)
    #[serde(default = "default_prefix_query_rate_limit")]
    pub prefix_query_rate_limit: u32,
    /// Maximum memory usage for prefix query results in MB (default: 50)
    #[serde(default = "default_max_query_memory_mb")]
    pub max_query_memory_mb: usize,
}

fn default_max_prefix_results() -> usize {
    1000
}

fn default_max_page_size() -> usize {
    100
}

fn default_prefix_query_rate_limit() -> u32 {
    60
}

fn default_max_query_memory_mb() -> usize {
    50
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_prefix_results: default_max_prefix_results(),
            max_page_size: default_max_page_size(),
            prefix_query_rate_limit: default_prefix_query_rate_limit(),
            max_query_memory_mb: default_max_query_memory_mb(),
        }
    }
}

impl StorageLimits {
    pub fn validate(&self) -> Result<(), eld_common::error::EldError> {
        use eld_common::validation::validate_positive_integer;

        // Validate max_prefix_results
        validate_positive_integer(
            self.max_prefix_results,
            Some(1),
            Some(100_000), // Maximum 100k results
            "max_prefix_results",
        )?;

        // Validate max_page_size
        validate_positive_integer(
            self.max_page_size,
            Some(1),
            Some(10_000), // Maximum 10k per page
            "max_page_size",
        )?;

        // Validate prefix_query_rate_limit
        validate_positive_integer(
            self.prefix_query_rate_limit as usize,
            Some(1),
            Some(10_000), // Maximum 10k queries per minute
            "prefix_query_rate_limit",
        )?;

        // Validate max_query_memory_mb
        validate_positive_integer(
            self.max_query_memory_mb,
            Some(1),
            Some(1000), // Maximum 1GB
            "max_query_memory_mb",
        )?;

        Ok(())
    }
}
