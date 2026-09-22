use crate::api::pagination::{PaginatedResult, PrefixQueryOptions};
use eld_common::cado::{CadoBody, CadoPath};
use eld_common::error::EldError;
use eld_common::validation::safe_deserialize_cado_data;
use tracing::{info, warn};

use super::super::RocksDBStorage;

impl RocksDBStorage {
    /// Secure version of get_cado_paths_by_prefix with pagination and rate limiting
    pub fn get_cado_paths_by_prefix_secure(
        &self,
        prefix: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<CadoPath>, EldError> {
        // Validate options
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        // Check rate limiting
        if !self.rate_limiter.is_allowed(&options.client_id, prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Log large prefix queries for monitoring
        if prefix.len() < 10 {
            warn!(
                "Large prefix query detected: '{}' (length: {})",
                prefix,
                prefix.len()
            );
        }

        let mut paths = Vec::new();
        let mut total_processed = 0;
        let mut estimated_memory_usage = 0;
        let prefix_bytes = prefix.as_bytes();

        // Use path_index_cf to find all paths with matching prefix
        let iter = self.db.iterator_cf(
            self.path_index_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (path_bytes, _) = item.map_err(|e| EldError::StorageError {
                operation: "read_path_index_cf".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path_str =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "path_utf8".to_string(),
                    value: format!("{path_bytes:?}"),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;

            // Stop if we've moved past our prefix
            if !path_str.starts_with(prefix) {
                break;
            }

            total_processed += 1;

            // Check memory usage limit
            estimated_memory_usage += path_str.len();
            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for prefix query '{}': {} bytes (limit: {} bytes)",
                    prefix, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            // Check result limit
            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for prefix query '{}': {} results (limit: {})",
                    prefix, total_processed, options.max_results
                );
                break;
            }

            // Apply pagination
            if total_processed > options.pagination.offset() {
                if paths.len() < options.pagination.page_size {
                    // Convert path string to CadoPath
                    let cado_path = CadoPath::parse(&path_str)?;
                    paths.push(cado_path);
                } else {
                    // We've filled the page
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + paths.len();

        info!(
            "Prefix query '{}' returned {} results (processed: {}, memory: {} bytes)",
            prefix,
            paths.len(),
            total_processed,
            estimated_memory_usage
        );

        Ok(PaginatedResult::new(
            paths,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }

    /// Secure version of get_cados_by_prefix with pagination and rate limiting
    pub fn get_cados_by_prefix_secure(
        &self,
        prefix: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<CadoBody>, EldError> {
        // Validate options
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        // Check rate limiting
        if !self.rate_limiter.is_allowed(&options.client_id, prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Log large prefix queries for monitoring
        if prefix.len() < 10 {
            warn!(
                "Large prefix query detected: '{}' (length: {})",
                prefix,
                prefix.len()
            );
        }

        let mut results = Vec::new();
        let mut total_processed = 0;
        let mut estimated_memory_usage = 0;
        let prefix_bytes = prefix.as_bytes();

        // Use path_index_cf to find all CADOs with matching prefix
        let iter = self.db.iterator_cf(
            self.path_index_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (path_bytes, key_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "read_path_index_cf".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "path_utf8".to_string(),
                    value: format!("{path_bytes:?}"),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;

            // Stop if we've moved past our prefix
            if !path.starts_with(prefix) {
                break;
            }

            total_processed += 1;

            // Check memory usage limit
            estimated_memory_usage += path.len() + key_bytes.len();
            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for prefix query '{}': {} bytes (limit: {} bytes)",
                    prefix, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            // Check result limit
            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for prefix query '{}': {} results (limit: {})",
                    prefix, total_processed, options.max_results
                );
                break;
            }

            // Apply pagination
            if total_processed > options.pagination.offset() {
                if results.len() < options.pagination.page_size {
                    // Get and deserialize the CADO
                    if let Some(value) =
                        self.db.get_cf(self.cado_cf()?, &key_bytes).map_err(|e| {
                            EldError::StorageError {
                                operation: "read_cado_cf".to_string(),
                                details: format!("Failed to read cado_cf: {e}"),
                            }
                        })?
                    {
                        estimated_memory_usage += value.len();
                        if estimated_memory_usage > options.max_memory_bytes {
                            warn!(
                                "Memory limit exceeded after deserializing CADO for prefix '{}'",
                                prefix
                            );
                            break;
                        }

                        let cado: CadoBody = safe_deserialize_cado_data(
                            &value,
                            &format!("CadoBody in prefix query for {prefix}"),
                        )
                        .map_err(|e| EldError::StorageError {
                            operation: "deserialize_cado_type".to_string(),
                            details: format!("Failed to deserialize CadoBody: {e}"),
                        })?;
                        results.push(cado);
                    }
                } else {
                    // We've filled the page
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + results.len();

        info!(
            "Prefix query '{}' returned {} CADOs (processed: {}, memory: {} bytes)",
            prefix,
            results.len(),
            total_processed,
            estimated_memory_usage
        );

        Ok(PaginatedResult::new(
            results,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }
    pub fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        // Validate prefix for security issues
        Self::validate_cado_prefix_security(prefix)?;

        // Check rate limiting with default client ID
        if !self.rate_limiter.is_allowed("default", prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limit_check".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Use secure version with default options
        let options = PrefixQueryOptions::default();
        let result = self.get_cado_paths_by_prefix_secure(prefix, options)?;
        Ok(result.items)
    }
    pub fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        // Validate prefix for security issues
        Self::validate_cado_prefix_security(prefix)?;

        // Check rate limiting with default client ID
        if !self.rate_limiter.is_allowed("default", prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limit_check".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Use secure version with default options
        let options = PrefixQueryOptions::default();
        let result = self.get_cados_by_prefix_secure(prefix, options)?;
        Ok(result.items)
    }
}
