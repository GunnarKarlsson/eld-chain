use eld_common::error::EldError;
use serde::{Deserialize, Serialize};

/// Pagination parameters for prefix queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    /// Page number (0-based)
    pub page: usize,
    /// Number of items per page
    pub page_size: usize,
    /// Optional cursor for pagination
    pub cursor: Option<String>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: 0,
            page_size: 100,
            cursor: None,
        }
    }
}

impl PaginationParams {
    /// Validate pagination parameters
    pub fn validate(&self, max_page_size: usize) -> Result<(), EldError> {
        if self.page_size > max_page_size {
            return Err(EldError::ValidationError {
                field: "page size".to_string(),
                value: self.page_size.to_string(),
                details: format!(
                    "Page size {} exceeds maximum allowed size {}",
                    self.page_size, max_page_size
                ),
            });
        }

        if self.page_size == 0 {
            return Err(EldError::ValidationError {
                field: "page size".to_string(),
                value: "0".to_string(),
                details: "Page size must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Calculate offset from page number and page size
    pub fn offset(&self) -> usize {
        self.page * self.page_size
    }
}

/// Paginated result for prefix queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResult<T> {
    /// Items in the current page
    pub items: Vec<T>,
    /// Total number of items (if known)
    pub total: Option<usize>,
    /// Current page number
    pub page: usize,
    /// Number of items per page
    pub page_size: usize,
    /// Total number of pages (if total is known)
    pub total_pages: Option<usize>,
    /// Cursor for the next page
    pub next_cursor: Option<String>,
    /// Whether there are more pages
    pub has_more: bool,
}

impl<T> PaginatedResult<T> {
    /// Create a new paginated result
    pub fn new(items: Vec<T>, page: usize, page_size: usize, has_more: bool) -> Self {
        let total = None; // We don't know the total for prefix queries
        let total_pages = None; // We don't know the total pages
        let next_cursor = if has_more {
            Some(format!("{}:{}", page + 1, page_size))
        } else {
            None
        };

        Self {
            items,
            total,
            page,
            page_size,
            total_pages,
            next_cursor,
            has_more,
        }
    }
}

#[cfg(test)]
impl<T> PaginatedResult<T> {
    pub fn is_first_page(&self) -> bool {
        self.page == 0
    }

    pub fn is_last_page(&self) -> bool {
        !self.has_more
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

/// Query options for prefix queries
#[derive(Debug, Clone)]
pub struct PrefixQueryOptions {
    /// Pagination parameters
    pub pagination: PaginationParams,
    /// Maximum number of results to return (safety limit)
    pub max_results: usize,
    /// Maximum memory usage in bytes
    pub max_memory_bytes: usize,
    /// Client identifier for rate limiting
    pub client_id: String,
}

impl Default for PrefixQueryOptions {
    fn default() -> Self {
        Self {
            pagination: PaginationParams::default(),
            max_results: 1000,
            max_memory_bytes: 50 * 1024 * 1024, // 50MB
            client_id: "unknown".to_string(),
        }
    }
}

impl PrefixQueryOptions {
    /// Set pagination parameter
    pub fn with_pagination(mut self, pagination: PaginationParams) -> Self {
        self.pagination = pagination;
        self
    }

    /// Validate the options
    pub fn validate(&self) -> Result<(), EldError> {
        self.pagination.validate(self.max_results)?;

        if self.max_results == 0 {
            return Err(EldError::ValidationError {
                field: "max results".to_string(),
                value: "0".to_string(),
                details: "Max results must be greater than 0".to_string(),
            });
        }

        if self.max_memory_bytes == 0 {
            return Err(EldError::ValidationError {
                field: "max memory bytes".to_string(),
                value: "0".to_string(),
                details: "Max memory bytes must be greater than 0".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
impl PrefixQueryOptions {
    pub fn with_limits(max_results: usize, max_memory_bytes: usize, client_id: String) -> Self {
        Self {
            pagination: PaginationParams::default(),
            max_results,
            max_memory_bytes,
            client_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination_params_validation() {
        let params = PaginationParams {
            page: 0,
            page_size: 50,
            cursor: None,
        };

        assert!(params.validate(100).is_ok());

        let invalid_params = PaginationParams {
            page: 0,
            page_size: 150,
            cursor: None,
        };

        assert!(invalid_params.validate(100).is_err());
    }

    #[test]
    fn test_paginated_result() {
        let items = vec![1, 2, 3, 4, 5];
        let result = PaginatedResult::new(items, 0, 10, true);

        assert_eq!(result.page, 0);
        assert_eq!(result.page_size, 10);
        assert_eq!(result.item_count(), 5);
        assert!(result.has_more);
        assert!(result.next_cursor.is_some());
        assert!(result.is_first_page());
        assert!(!result.is_last_page());
    }

    #[test]
    fn test_prefix_query_options() {
        let options =
            PrefixQueryOptions::with_limits(1000, 50 * 1024 * 1024, "test_client".to_string());

        assert_eq!(options.max_results, 1000);
        assert_eq!(options.max_memory_bytes, 50 * 1024 * 1024);
        assert_eq!(options.client_id, "test_client");
        assert!(options.validate().is_ok());
    }
}
