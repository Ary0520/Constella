# Blend v2 contracts (real mainnet WASMs)

Fetched 16 Nov 2026 with `stellar contract fetch` (stellar-cli 30.0.1) from
`mainnet.sorobanrpc.com`. IDs from the official
[blend-utils `mainnet.contracts.json`](https://github.com/blend-capital/blend-utils) (commit `b05242d`).
Each hash was checked three ways: the bytes on mainnet, Blend's published `wasm_v2/` build in
blend-utils, and the `testnet.contracts.json` hash list (testnet runs the same code).

| File | Mainnet contract | sha256 | Also matches |
|---|---|---|---|
| `blend_pool_v2_fixed_mainnet.wasm` | FixedV2 `CAJJZSGMMM3PD7N33TAPHGBUGTB43OC73HVIK2L2G6BNGGGYOSSYBXBD` | `a41fc53d6753b6c04eb15b021c55052366a4c8e0e21bc72700f461264ec1350e` | YieldBloxV2 `CCCCIQSD…OGYFS` (same code), `wasm_v2/pool.wasm`, testnet `lendingPoolV2` |
| `blend_backstop_v2_mainnet.wasm` | backstopV2 `CAQQR5SWBXKIGZKPBZDH3KM5GQ5GUTPKB7JAFCINLZBC5WXPJKRG3IM7` | `c1f4502a757e25c611f5a159bc1ab0eef64085adac6c68123dca66e87faffbc2` | `wasm_v2/backstop.wasm`, testnet `backstopV2` |
| `blend_pool_factory_v2_mainnet.wasm` | poolFactoryV2 `CDSYOAVXFY7SM5S64IZPPPYB4GVGGLMQVFREPSQQEZVIWXX5R23G4QSU` | `31328050548831f63d2b72e37bcfd0bb7371b7907135755dbe09ed434d755ca9` | `wasm_v2/pool_factory.wasm`, testnet `poolFactoryV2` |
| `blend_emitter_mainnet.wasm` | emitter `CCOQM6S7ICIUWA225O5PSJWUBEMXGFSSW2PQFO6FP4DQEKMS5DASRGRR` | `438a5528cff17ede6fe515f095c43c5f15727af17d006971485e52462e7e7b89` | mainnet + testnet `emitter` hash |
| `blend_comet_mainnet.wasm` | comet BLND:USDC `CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM` | `8abc28913035c07411ed5d134e6bfeab4723d97ddd4d1a22a0605d35c94d1a36` | mainnet + testnet `comet` hash |

Note: the `comet.wasm` checked into `blend-contracts-v2` (sha256 `5d721fa5…`) is a different
build from the deployed one; we use the deployed one.

Source read for the integration (blend-contracts-v2 `0bf9b74`, 26 Oct 2026): `pool/src/contract.rs`,
`pool/src/pool/{actions,submit,reserve,user,gulp,status,config,bad_debt}.rs`, `test-suites/src/*`.
