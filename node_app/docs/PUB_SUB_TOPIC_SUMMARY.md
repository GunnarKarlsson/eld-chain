PUB SUB TOPIC SUMMARY

Android app:

Subscribes: eld-storage-challenge-topic-{providerId}, eld-content-sync
Publishes: eld-content-sync-response
Does not subscribe to: eld-content-sync-response

Node:

Always subscribed (at startup):
eld-content-sync — receives:
ContentSyncHeartbeat messages
Announce messages
ContentRequest messages
ContentResponse messages
ContentInventoryRequest messages
ContentInventoryResponse messages
eld-content-sync-response — receives:
ContentSyncHeartbeatResponse messages
Dynamically subscribed (conditionally):
eld-storage-challenge-topic-{provider_id} — subscribed when:
The node is a registered storage provider
Receives Heartbeat messages from validators
Receives CapacityChallenge messages
eld-storage-proof-topic-{provider_id} — subscribed when:
The node acts as a storage validator and sends challenges
Receives CapacityChallengeResponse messages from storage providers
Topics the Node Publishes To:
eld-content-sync — publishes:
ContentSyncHeartbeat (every 5 seconds)
Announce (when content is announced)
ContentRequest (when requesting content)
ContentResponse (when responding to content requests)
ContentInventoryResponse (when responding to inventory requests)
eld-storage-challenge-topic-{provider_id} — publishes:
Heartbeat (every 5 seconds to each registered storage provider)
eld-storage-proof-topic-{provider_id} — publishes:
CapacityChallengeResponse (when a storage provider responds to a challenge)
Note: The node subscribes to eld-content-sync-response but does not publish to it. Storage providers (like the Android app) publish ContentSyncHeartbeatResponse to this topic.