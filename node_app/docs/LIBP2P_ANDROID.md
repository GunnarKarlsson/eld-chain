 have a Rust server running Libp2p for p2p gossipsub. Is there a library for Kotlin/Java/Android I can use to have an Android device listen to the gossipsub?
AI-generated answer
Short answer
Yes. Use the JVM implementation of libp2p (jvm-libp2p) on Android, or bridge to your Rust node via a websocket/HTTP relay or a small native helper.

Options
jvm-libp2p — a Java/Kotlin implementation of libp2p that supports common transports and pubsub (gossipsub). This is the most direct approach for Kotlin/Android.
js-libp2p via WebRTC or WebView — run a JS libp2p peer in a WebView (or Node) and connect to your Rust node if JS fits your stack.
Bridge/Relay — run a trusted relay (websocket/http or a libp2p relay node) that both the Rust server and Android client can use if mobile environment restrictions prevent direct P2P.
Native helper — embed a tiny Rust/native libp2p process and talk JNI/aidl to it if you need parity with your Rust behavior.
Quick Kotlin sketch (conceptual)
This is a minimal conceptual outline showing the flow (not a full program):

kotlin

// create libp2p Host, add transports, and gossipsub behaviour
val host = /* build jvm-libp2p host with TCP/WebSocket transports and identity */
val gossipsub = /* create gossipsub behaviour, subscribe to topic */

// start host and listen
host.start().join()

// publish/handle messages
gossipsub.subscribe("my-topic") { msg ->
    // handle incoming message bytes
}
gossipsub.publish("my-topic", "hello".toByteArray())
Tips & Alternatives
On Android, prefer WebSocket transport or WebRTC if NAT/firewall traversal is problematic.
Mind battery, background restrictions, and network permissions (foreground service / Wi‑Fi/data).
If jvm-libp2p lacks a needed feature, consider a small Rust-based helper process and communicate via binder/JNI or a local socket.
Next steps
Try jvm-libp2p in a simple Kotlin JVM app first, then port to Android.
If you hit missing transport/features, evaluate a relay approach or embed native code.
If you want, tell me which transports (TCP/WebSocket/WebRTC) you need and I can suggest a concrete dependency and a tighter example.