# Test fixtures

## reflector_pulse_mainnet.wasm

The real Reflector Pulse oracle contract, used by `nav-oracle` O7b tests so
the Reflector integration is exercised against production code, not a stand-in.

- Fetched read-only from Stellar mainnet on 15 Nov 2026 with stellar-cli 30.0.1:
  `stellar contract fetch --id <ID> --rpc-url https://mainnet.sorobanrpc.com --network-passphrase "Public Global Stellar Network ; September 2015"`
- Both `CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN` (CEX/DEX feed)
  and `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` (Stellar pubnet
  feed) run this exact binary.
- sha256 `25fc306065cbb9b05461db78ec259802cb916053ba5b3ab911b0a72dd1a81439`
- Contract meta: soroban-sdk 28.0.0, protocol 28, source github:reflector-network/reflector-contract (MIT).

Re-fetch and compare the hash before each release; if Reflector upgrades its
contracts, update this fixture and re-run the O7b tests.
