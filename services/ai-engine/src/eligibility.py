"""
Eligibility engine (audit A3, owner decision D-B, 17 Nov 2026).

An asset may be proposed to a user only if BOTH say yes:
  1. the issuer's rules, recorded ON CHAIN in the asset registry
     (blocked / eligible jurisdictions, required investor classes, active flag);
  2. Constella's own class policy (COMPLIANCE.md): retail users get
     permissionless assets only, until Phase 4 (licensed partner / MoR).
Comprehensively sanctioned jurisdictions get nothing at all.

Ambiguity fails closed: e.g. an issuer allow-list of jurisdictions cannot be
satisfied by the generic "non-US" answer. This is engineering of the owner's
decision, not legal advice; counsel confirms before mainnet.
"""
from __future__ import annotations

from typing import List, Tuple

from src.models import AssetCandidate

# OFAC comprehensive (country-wide) sanctions programs. Kept explicit and
# short on purpose; counsel must confirm the list before mainnet.
SANCTIONED_JURISDICTIONS = frozenset({"CU", "IR", "KP"})

# Investor-class ranks. Registry symbols may use "accredited" / "qib".
_CLASS_RANK = {"retail": 0, "professional": 1, "accredited": 1, "institutional": 2, "qib": 2}


def _norm(code: str) -> str:
    return code.strip().upper()


def class_rank(investor_class: str) -> int:
    try:
        return _CLASS_RANK[investor_class.strip().lower()]
    except KeyError:
        raise ValueError(f"unknown investor class {investor_class!r}") from None


def check(asset: AssetCandidate, jurisdiction: str, investor_class: str) -> Tuple[bool, str]:
    """(eligible, reason). `reason` is empty when eligible."""
    jur = _norm(jurisdiction)
    rank = class_rank(investor_class)
    if jur in SANCTIONED_JURISDICTIONS:
        return False, f"{jur} is a sanctioned jurisdiction"
    if not asset.is_active:
        return False, "asset is not active in the on-chain registry"
    # Constella policy (D-B): retail = permissionless only.
    if rank == 0 and not asset.permissionless:
        return False, "retail users get permissionless assets only (until Phase 4)"
    # Issuer rules (on chain).
    if any(_norm(b) == jur for b in asset.blocked_jurisdictions):
        return False, f"the issuer does not allow holders in {jur}"
    if asset.eligible_jurisdictions and not any(_norm(e) == jur for e in asset.eligible_jurisdictions):
        return False, f"the issuer allows only {', '.join(asset.eligible_jurisdictions)}"
    if asset.investor_classes_required:
        needed = min(class_rank(c) for c in asset.investor_classes_required)
        if rank < needed:
            return False, f"the issuer requires a {'/'.join(asset.investor_classes_required)} investor"
    return True, ""


def eligible_assets(assets: List[AssetCandidate], jurisdiction: str, investor_class: str) -> List[AssetCandidate]:
    return [a for a in assets if check(a, jurisdiction, investor_class)[0]]
