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

## redstone_adapter_mainnet.wasm

The real RedStone multi-feed adapter, used by `nav-oracle` O7c tests.

- Fetched read-only from Stellar mainnet on 15 Nov 2026 (contract
  `CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG`) with
  `stellar contract fetch` (stellar-cli 30.0.1).
- sha256 `f0ccdffc29931abed6d77eef359450b183584103a3665be1f080f5f712b1542e`
- Contract meta: soroban-sdk 25.3.1, source redstone-finance/redstone-public-contracts.
  RedStone code is BUSL-1.1; this binary is used only as a test fixture.

## redstone_write_usdc.hex / redstone_write_euroc.hex

Real signed RedStone payloads, copied from the `payload` argument of two
successful mainnet `write_prices` transactions on the adapter above (decoded
with `stellar xdr decode`):

| File | Tx hash | Ledger | Closed (unix s) | Feeds | Decoded values (3 signers) |
|---|---|---|---|---|---|
| `redstone_write_usdc.hex` | `4bed8f749f5d5d640e17599b7dcb6c5d051fd4b8c88cee2bd537dd0cd59d3a31` | 65493578 | 1794753932 | USDC, BTC | USDC 0.99975479 / 0.99975978 / 0.99975978 (signed 1794753928000 ms) |
| `redstone_write_euroc.hex` | `089303b59a2fecc42ed6f95d327a1bb4d7f88717d9cb9bbec0fa4bb70f9a69fa` | 65493639 | 1794754237 | SolvBTC.BBN_FUNDAMENTAL/USD, SolvBTC_FUNDAMENTAL/USD, XLM, EUROC | EUROC 1.09853605 ×3 (signed 1794754230000 ms) |

Updater (RedStone relayer): `GASNOA72CECDUZ5GEUK6WFINSASEG6R3WYZB2DE2CGDU7YI7GC2QPSFX`.
Tests replay them at the original ledger times; the adapter verifies the
signer signatures itself (payload must be within -3 / +1 min of ledger time).
