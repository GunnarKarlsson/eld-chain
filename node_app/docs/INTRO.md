my intro blob for Eld:
(eld is renamed to Eld):

Eld is a public decentralized pinboard for short term messages.

A public pinboard for short term messages.
Public or encrypted.
Post short term data for your app.
Private message type is protocol enforced: Public Key, encrypted msg.
Messages life can be from a few blocks to up to 1 year.
Eld's design objective is to have a max size by folding prior txs to keep the max storage needed.
Messages are stored fully onchain.

Eld has a small footprint. Eld consolidates blocks with expired transactions to shorten the chain. 
Transactions and blocks with expired messages are consolidated into a new hash as proof of valid chain consolidation.
Genesis block can never be consolidated, but any block up until a block contains a tx with a non-expired TTL can be consolidated.
This keeps the chain smaller and with a max potential size (calculate...).

Blob proposed by grok:

Eld: The On-Chain Public Pinboard for Temporary Messages

Imagine a global, censorship-resistant noticeboard that lives entirely on the blockchain — where anyone can post short-lived messages that automatically vanish after their time is up. No servers. No middlemen. No eternal bloat.That’s Eld. Eld is a lightweight blockchain protocol purpose-built for ephemeral data. It lets developers and users post short-term messages that power everything from app notifications and temporary credentials to private chats and time-sensitive announcements — all stored fully on-chain. How it works — beautifully simple

Public or encrypted — Post openly for the world to see, or use protocol-enforced private messages: just attach a recipient’s public key and an encrypted payload. The network handles the rest; no one else can read it.

Flexible lifetimes — Choose how long your message lives: from a few blocks to a full year. When the expiry hits, it’s gone — automatically and trustlessly.

Built for apps — Need to broadcast a status update, share a one-time link, or pass session data between decentralized services? Eld gives your app a native, decentralized pinboard.

Build to shrink itself - Sub chains with blocks with expired messages are consolidated with special transactions to remove data and shorten the chain.

Designed for the long haul (without the storage nightmare)

Most blockchains grow forever. Eld refuses to.Its core innovation: folding. New transaction intelligently compresses and references prior ones, keeping the chain’s maximum storage footprint bounded — no matter how many messages fly across the network. You get permanent verifiability with predictable, sustainable node requirements.Messages live fully on-chain, secured by the protocol itself. No off-chain promises. No pinning games. Just pure, verifiable, time-bound data.

Why builders love Eld
True decentralization — every node sees the same pinboard
Protocol-level privacy for encrypted messages
Predictable storage & costs
Perfect primitive for dApps, wallets, games, DAOs, or any system that needs “post it now, forget it later” data

Welcome to the first blockchain built for things that shouldn’t last forever.

Eld — Temporary messages. Permanent infrastructure.
Ready to pin your first message? The board is open.

