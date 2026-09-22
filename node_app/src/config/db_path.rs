const DEFAULT_DB_PATH: &str = "./data/rocksdb";
const ENV_DB_PATH: &str = "ELD_DB_PATH";

/// Resolve the database path with fallback priority:
/// 1) CLI argument, 2) environment variable, 3) default path
pub fn resolve_db_path(cli_db_path: Option<String>) -> String {
    cli_db_path
        .or_else(|| std::env::var(ENV_DB_PATH).ok())
        .unwrap_or_else(|| DEFAULT_DB_PATH.to_string())
}
