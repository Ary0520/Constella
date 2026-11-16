"""Run by localnet_registry_e2e.sh: read the REAL on-chain registry and check
the engine's view of it (decoding of real contract data, D-B eligibility)."""
import os
import sys

from src.models import PolicyDraft
from src.retriever import AssetRegistry, retrieve_candidates

E = os.environ
reg = AssetRegistry.from_chain(E["RPC"], E["PASS"], E["REGISTRY"], E["VIEWER"])
assets = {a.asset_id: a for a in reg.list_assets()}
print("on-chain assets seen by the engine:", sorted(assets))
fail = []
if set(assets) != {"USDC", "BLEND_USDC_SUPPLY", "FRANKLIN_BENJI"}:
    fail.append(f"expected USDC, BLEND_USDC_SUPPLY, FRANKLIN_BENJI (MYSTERY skipped): {sorted(assets)}")
if assets.get("USDC") and assets["USDC"].contract_address != E["USDC"]:
    fail.append("USDC address not the deployed SAC")
if assets.get("BLEND_USDC_SUPPLY") and (assets["BLEND_USDC_SUPPLY"].sleeve_type.value != "lending" or assets["BLEND_USDC_SUPPLY"].is_active):
    fail.append("BLEND entry: sleeve or deactivation not decoded")
if assets.get("FRANKLIN_BENJI") and (assets["FRANKLIN_BENJI"].permissionless or assets["FRANKLIN_BENJI"].blocked_jurisdictions != ["CN"]):
    fail.append("BENJI issuer rules not decoded")
retail = {c.asset_id for c in retrieve_candidates(PolicyDraft(jurisdiction="EU", investor_class="retail", currency_exposure={}), reg)}
pro = {c.asset_id for c in retrieve_candidates(PolicyDraft(jurisdiction="EU", investor_class="professional", currency_exposure={}), reg)}
pro_cn = {c.asset_id for c in retrieve_candidates(PolicyDraft(jurisdiction="CN", investor_class="professional", currency_exposure={}), reg)}
print("retail EU:", sorted(retail), "| professional EU:", sorted(pro), "| professional CN:", sorted(pro_cn))
if retail != {"USDC"}:
    fail.append(f"retail must get USDC only (BLEND deactivated, BENJI gated): {retail}")
if pro != {"USDC", "FRANKLIN_BENJI"}:
    fail.append(f"professional EU: {pro}")
if pro_cn != {"USDC"}:
    fail.append(f"issuer blocks CN: {pro_cn}")
if fail:
    print("E2E FAIL:", *fail, sep="\n  ")
    sys.exit(1)
print("REGISTRY E2E PASS")
