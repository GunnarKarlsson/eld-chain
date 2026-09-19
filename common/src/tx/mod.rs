//! Transaction wire types, payloads, and signing.

pub(crate) mod envelope;
pub(crate) mod event;
pub(crate) mod parts;
pub(crate) mod payload;
pub(crate) mod payloads;

pub use envelope::{validate_message_and_build_post_message_tx, Tx};
pub use event::create_event_attribute;
pub use parts::{HasAmount, HasSender, TxAmount, TxPublicKey, TxSig};
pub use payload::{Payload, PayloadInner, TxType};
pub use payloads::{
    build_signed_post_message_user_request, calculate_message_content_key,
    canonicalize_post_message_tags, post_message_id_from_signing_bytes,
    validate_post_message_content_type, validate_post_message_user_signature, AddNamespaceTx,
    PostMessageTx, PostMessageUserRequest, PostMessageUserRequestInput, RegisterCapacityTx,
    StakeTx, TransferTx, UnregisterCapacityTx, UnstakeTx, UpdateCapacityMerkleRootTx,
    VerifiedProofTx, POST_MESSAGE_ALLOWED_CONTENT_TYPES, POST_MESSAGE_MAX_TAGS,
    POST_MESSAGE_MAX_TAG_UTF8_BYTES,
};

#[cfg(test)]
mod tests;
