# Security

## Reporting

Do not open a public issue for key material or live-network impact.

Once this repository is on GitHub, report consensus, transaction, wallet, or capacity-proof bugs through GitHub Security Advisories. Until then, report privately to the Eld network maintainers.

## Wallets

Local wallets are JSON files with hex-encoded Ed25519 private keys (no encryption).

- Do not commit `wallets.json` or any file that contains `private_key`.
- Do not log `serde_json` of a wallet. `Debug` for keypairs redacts the seed; serialization does not.
- Treat any leaked test or local key as burned.

Tests in this crate derive keys from documented throwaway seeds (for example repeating `0x01` bytes). Those keys are public the moment the repository is.

## Status

This crate is experimental shared library code. Protocol constants and the JSON transaction signing encoding are the current client/node wire format, not a frozen mainnet spec.
