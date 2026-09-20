//! HTTP REST: node app (`AppApi`), pinboard/namespace JSON DTOs, and the dev faucet.

mod app;
pub mod faucet;
pub mod namespace;
pub mod pinboard;

pub use app::{ApiErrorResponse, AppApi};
pub use namespace::{
    NamespaceListItem, NamespaceListPagination, NamespaceListResponse,
    NamespaceNotRegisteredResponse, NamespaceRegisteredResponse,
};
pub use pinboard::{PinboardMessageParams, PostMessageSubmitRequest, PostMessageSubmitResponse};
