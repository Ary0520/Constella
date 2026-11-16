"""
Asset Registry and Candidate Retrieval Module for Constella AI Thesis Engine.
Enforces the Closed Universe Rule: zero hallucinated or unregistered assets.
"""

from typing import Dict, List, Optional

from src import eligibility
from src.models import AssetCandidate, PolicyDraft
from src.registry_sync import candidates_from_entries, load_onchain_entries


class AssetRegistry:
    """
    The asset universe for one engine run, built from the ON-CHAIN registry
    (audit A3): `AssetRegistry.from_chain(...)` in production. There is no
    built-in asset list - assets the chain does not list do not exist.
    """

    def __init__(self, assets: List[AssetCandidate]):
        self._assets = {a.asset_id: a for a in assets}

    @classmethod
    def from_entries(cls, entries: List[Dict]) -> "AssetRegistry":
        candidates, _skipped = candidates_from_entries(entries)
        return cls(candidates)

    @classmethod
    def from_chain(cls, rpc_url: str, network_passphrase: str, registry_id: str, source_account: str) -> "AssetRegistry":
        return cls.from_entries(load_onchain_entries(rpc_url, network_passphrase, registry_id, source_account))

    def get_asset(self, asset_id: str) -> Optional[AssetCandidate]:
        return self._assets.get(asset_id)

    def list_assets(self) -> List[AssetCandidate]:
        return list(self._assets.values())

    def eligibility(self, jurisdiction: str, investor_class: str) -> Dict[str, str]:
        """asset_id -> reason for every asset this user may NOT hold."""
        out = {}
        for a in self._assets.values():
            ok, why = eligibility.check(a, jurisdiction, investor_class)
            if not ok:
                out[a.asset_id] = why
        return out


def retrieve_candidates(policy: PolicyDraft, registry: AssetRegistry) -> List[AssetCandidate]:
    """
    Retrieve candidate assets from the registry matching policy constraints.
    Enforces:
    1-2. Eligibility: on-chain issuer rules + Constella class policy (D-B)
    3. Exclusions (sleeves or specific asset IDs excluded)
    4. Target currency alignment
    """
    # 1-2. Eligibility (audit A3 / decision D-B): issuer rules from the
    # on-chain registry AND Constella's class policy (retail = permissionless).
    eligible = eligibility.eligible_assets(registry.list_assets(), policy.jurisdiction, policy.investor_class)

    # 3. Filter by exclusions
    exclusions_lower = {e.strip().lower() for e in policy.exclusions}
    eligible = [
        a for a in eligible
        if a.sleeve_type.value.lower() not in exclusions_lower
        and a.sleeve_type.name.lower() not in exclusions_lower
        and a.asset_id.lower() not in exclusions_lower
    ]

    # 4. Target currency matching (if specified in currency_exposure)
    if policy.currency_exposure:
        target_currencies = {c.strip().upper() for c, w in policy.currency_exposure.items() if w > 0}
        # Keep assets matching target currencies; if none match, keep all eligible
        matching = [a for a in eligible if a.currency.upper() in target_currencies]
        if matching:
            eligible = matching

    return eligible
