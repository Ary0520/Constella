# Test fixtures: real Soroswap contracts

Fetched read-only from Stellar mainnet on 15 Nov 2026 with stellar-cli 30.0.1
(`stellar contract fetch --id <ID> --rpc-url https://mainnet.sorobanrpc.com ...`),
used by the W1 rebalancing tests so swaps run through production DEX code.

| File | Mainnet contract | sha256 |
|---|---|---|
| `soroswap_router_mainnet.wasm` | router `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` | `4c3db3ebd2d6a2ab23de1f622eaabb39501539b4611b68622ec4e47f76c4ba07` |
| `soroswap_factory_mainnet.wasm` | factory `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` | `5db738b05d9148128a240b0e2c1cb935c2805192bf98a579421aacda364c8dae` |
| `soroswap_pair_mainnet.wasm` | USDC/EURC pair `CC7CDFY2VGDODJ7WPO3JIK2MXLOAXL4LRQCC43UJDBAIJ4SVFO3HNPOC` | `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e` (= Soroswap's published `pair_wasm_hash`, `soroswap/core` `public/mainnet.contracts.json`) |

All built with soroban-sdk 20.2.0 (source github.com/soroswap/core; OtterSec audit 2024-02-22).
