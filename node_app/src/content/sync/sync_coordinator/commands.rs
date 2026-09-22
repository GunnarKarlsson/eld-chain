use super::SyncMsg;

#[derive(Debug)]
pub(crate) enum P2pCommand {
    BroadcastAnnounce { key: String },
    BroadcastContentRequest { key: String },
    BroadcastContentResponse { key: String, content: String },
    SubscribeToTopic { topic: String },
    PublishToTopic { topic: String, msg: SyncMsg },
}
