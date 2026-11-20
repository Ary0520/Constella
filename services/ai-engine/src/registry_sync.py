"""
Read the asset universe from the ON-CHAIN Constella registry (audit A3).

The chain is the only source of truth for which assets exist, their contract
addresses, issuers, sleeve types, liquidity tiers, permissionless flag, issuer
eligibility rules and active status. The engine adds display text only
(`ASSET_METADATA`): an on-chain asset without a description is skipped (its
currency is needed for matching), and a description without an on-chain
entry describes nothing.
"""
from __future__ import annotations

from typing import Dict, List, Tuple

from src.models import AssetCandidate, SleeveType

# constella_registry::SleeveType discriminants (contracts/registry/src/lib.rs).
SLEEVE_BY_CODE = {
    0: SleeveType.CASH_STABLE,
    1: SleeveType.USD_GOV_FUNDS,
    2: SleeveType.EUR_SOVEREIGN,
    3: SleeveType.YIELD_DOLLAR,
    4: SleeveType.LENDING,
    5: SleeveType.EQUITY,
    6: SleeveType.XLM,
}

# Display metadata only - no addresses, eligibility or yields (those are on
# chain, or unknown until a live source exists). Keys = registry asset_id.
ASSET_METADATA: Dict[str, Dict] = {
    "USDC": {"display_name": "USD Coin", "currency": "USD",
             "tags": ["cash", "stablecoin", "usd", "circle", "instant", "numeraire", "liquidity"],
             "description": "Circle USD Coin on Stellar. Permissionless, instant liquidity."},
    "EURC": {"display_name": "Euro Coin", "currency": "EUR",
             "tags": ["cash", "stablecoin", "eur", "euro", "circle", "instant", "payroll", "liquidity"],
             "description": "Circle Euro Coin on Stellar. Permissionless, instant liquidity."},
    "BLEND_USDC_SUPPLY": {"display_name": "Blend USDC Supply", "currency": "USD",
                          "tags": ["lending", "defi", "usd", "blend", "yield", "variable"],
                          "description": "USDC supplied to an official-factory Blend v2 pool; variable interest; withdrawals depend on pool liquidity."},
    "FRANKLIN_BENJI": {"display_name": "Franklin OnChain U.S. Government Money Fund (BENJI)", "currency": "USD",
                       "tags": ["treasury", "t-bill", "usd", "benji", "franklin", "government", "money-market", "yield"],
                       "description": "Tokenized US government money market fund (KYC by the issuer)."},
    "ONDO_USDY": {"display_name": "Ondo USDY", "currency": "USD",
                  "tags": ["treasury", "yield-dollar", "usd", "ondo", "yield"],
                  "description": "Yield-bearing dollar backed by short-term Treasuries; not for US persons."},
    "SPIKO_USD": {"display_name": "Spiko USD T-bills", "currency": "USD",
                  "tags": ["treasury", "t-bill", "usd", "spiko", "sovereign", "yield"],
                  "description": "Spiko tokenized USD short-term Treasury bills (KYC by the issuer)."},
    "SPIKO_EUR": {"display_name": "Spiko EUR T-bills", "currency": "EUR",
                  "tags": ["treasury", "t-bill", "eur", "euro", "spiko", "sovereign", "payroll", "yield"],
                  "description": "Spiko tokenized EUR short-term sovereign T-bills (KYC by the issuer)."},
    "JH_JQQQ": {"display_name": "Janus Henderson Anemoy JQQQ (Nasdaq-100)", "currency": "USD",
                "tags": ["equity", "tech", "nasdaq", "growth", "jqqq"],
                "description": "Tokenized Nasdaq-100 fund for non-US professional investors."},
}


class RegistryError(ValueError):
    pass


def candidates_from_entries(entries: List[Dict], metadata: Dict[str, Dict] = ASSET_METADATA) -> Tuple[List[AssetCandidate], List[str]]:
    """Convert on-chain `AssetEntry` values (as decoded by `scval.to_native`)
    into candidates. Returns (candidates, skipped asset_ids). Inactive assets
    are kept (flagged) so the eligibility engine can explain why they are out."""
    out: List[AssetCandidate] = []
    skipped: List[str] = []
    for e in entries:
        asset_id = str(e["asset_id"])
        meta = metadata.get(asset_id)
        if meta is None:
            skipped.append(asset_id)
            continue
        code = int(e["sleeve_type"])
        if code not in SLEEVE_BY_CODE:
            raise RegistryError(f"{asset_id}: unknown on-chain sleeve type {code}")
        addr = e["contract_address"]
        out.append(AssetCandidate(
            asset_id=asset_id,
            display_name=meta["display_name"],
            issuer=str(e["issuer"]),
            sleeve_type=SLEEVE_BY_CODE[code],
            currency=meta["currency"],
            liquidity_tier=int(e["liquidity_tier"]),
            permissionless=bool(e["permissionless"]),
            blocked_jurisdictions=[str(j) for j in e["blocked_jurisdictions"]],
            eligible_jurisdictions=[str(j) for j in e["eligible_jurisdictions"]],
            issuer_investor_classes=[str(c) for c in e["investor_classes_required"]],
            is_active=bool(e["is_active"]),
            contract_address=getattr(addr, "address", str(addr)),
            tags=list(meta["tags"]),
            description=meta["description"],
        ))
    return out, skipped


def load_onchain_entries(rpc_url: str, network_passphrase: str, registry_id: str, source_account: str) -> List[Dict]:
    """Read-only simulation of `registry.list_assets()`. `source_account` is
    any existing account (simulation needs one; nothing is signed or sent)."""
    from stellar_sdk import SorobanServer, TransactionBuilder, scval, xdr

    server = SorobanServer(rpc_url)
    account = server.load_account(source_account)
    tx = (TransactionBuilder(account, network_passphrase, base_fee=100)
          .append_invoke_contract_function_op(registry_id, "list_assets", []).set_timeout(30).build())
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RegistryError(f"list_assets simulation failed: {sim.error}")
    return list(scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr)))
