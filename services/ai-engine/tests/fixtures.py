"""
Test fixtures in the exact shape the ON-CHAIN registry returns
(`scval.to_native(list_assets())`: AssetEntry fields, Address objects).

Addresses: USDC / EURC are the verified mainnet SACs and BLEND_USDC_SUPPLY is
the verified Blend FixedV2 pool (docs/build-spec/references/ORACLE-SOURCES.md).
FIXTURE: gated assets use deterministic test addresses (their mainnet IDs are
not verified yet). Issuer rules follow docs/build-spec/ai-engine/REGISTRY.md.
FIXTURE_YIELDS are indicative test numbers, NOT market data; production
yields are unknown (0) until a live source exists.
"""
import hashlib

from stellar_sdk import Address, StrKey

from src.retriever import AssetRegistry

USDC_MAINNET = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"
EURC_MAINNET = "CDTKPWPLOURQA2SGTKTUQOWRCBZEORB4BWBOMJ3D3ZTQQSGE5F6JBQLV"
BLEND_FIXED_V2_POOL = "CAJJZSGMMM3PD7N33TAPHGBUGTB43OC73HVIK2L2G6BNGGGYOSSYBXBD"


def fixture_address(name: str) -> str:
    return StrKey.encode_contract(hashlib.sha256(f"constella-fixture:{name}".encode()).digest())


def entry(asset_id, issuer, sleeve_code, addr, permissionless=True, tier=0, blocked=(), eligible=(), classes=(), active=True):
    return {
        "contract_address": Address(addr),
        "asset_id": asset_id,
        "issuer": issuer,
        "sleeve_type": sleeve_code,
        "liquidity_tier": tier,
        "permissionless": permissionless,
        "oracle_sources": [f"{asset_id}_A", f"{asset_id}_B"],
        "eligible_jurisdictions": list(eligible),
        "blocked_jurisdictions": list(blocked),
        "investor_classes_required": list(classes),
        "is_active": active,
        "added_at": 0,
    }


def fixture_entries():
    return [
        entry("USDC", "CIRCLE", 0, USDC_MAINNET),
        entry("EURC", "CIRCLE", 0, EURC_MAINNET),
        entry("BLEND_USDC_SUPPLY", "BLEND", 4, BLEND_FIXED_V2_POOL, tier=1),
        entry("FRANKLIN_BENJI", "FRANKLIN_TEMPLETON", 1, fixture_address("BENJI"), permissionless=False, tier=1),
        entry("ONDO_USDY", "ONDO", 3, fixture_address("USDY"), permissionless=False, tier=1, blocked=["US"]),
        entry("SPIKO_USD", "SPIKO", 1, fixture_address("SPIKO_USD"), permissionless=False, tier=1, blocked=["US"]),
        entry("SPIKO_EUR", "SPIKO", 2, fixture_address("SPIKO_EUR"), permissionless=False, tier=1, blocked=["US"]),
        entry("JH_JQQQ", "JANUS_HENDERSON", 5, fixture_address("JQQQ"), permissionless=False, tier=2, blocked=["US"],
              classes=["professional"]),
    ]


FIXTURE_YIELDS = {"BLEND_USDC_SUPPLY": 2.4, "FRANKLIN_BENJI": 4.0, "ONDO_USDY": 4.0, "SPIKO_USD": 4.0, "SPIKO_EUR": 2.5}


def fixture_registry(entries=None, yields=FIXTURE_YIELDS) -> AssetRegistry:
    reg = AssetRegistry.from_entries(entries if entries is not None else fixture_entries())
    for a in reg.list_assets():
        a.current_yield_pct = yields.get(a.asset_id, 0.0)
    return reg
