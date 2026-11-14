"""
Thesis Engine Step 6: Sign (signer.py)
Assembles canonical policy specifications, performs deterministic RFC 8785 canonical
JSON serialization, and computes the 32-byte SHA-256 hash for Soroban on-chain anchoring.
"""

from datetime import datetime, timezone
import hashlib
import json
from typing import List, Optional

from src.models import (
    AssetCandidate,
    CanonicalAdmin,
    CanonicalConstraints,
    CanonicalEligibility,
    CanonicalPolicy,
    CanonicalSleeve,
    OptimizationResult,
    PolicyDraft,
    SignedPolicyEnvelope,
    SleeveType,
)


def assemble_canonical_policy(
    policy_draft: PolicyDraft,
    opt_result: OptimizationResult,
    candidates: List[AssetCandidate],
    basket_name: str = "cRUNWAY",
    admin_address: str = "GBADMIN1234567890",
    multisig_signers: Optional[List[str]] = None,
    multisig_threshold: int = 1,
    numeraire: str = "USDC",
    created_at: Optional[str] = None,
) -> CanonicalPolicy:
    """
    Assembles the final, immutable CanonicalPolicy representation from
    the parsed policy draft and optimization results.
    """
    candidate_map = {c.asset_id: c for c in candidates}
    sleeves: List[CanonicalSleeve] = []

    # Map allocated assets (target_weight_bps > 0) to CanonicalSleeve entries
    for asset_id, weight_bps in opt_result.weights_bps.items():
        if weight_bps > 0:
            candidate = candidate_map.get(asset_id)
            if candidate is not None:
                contract_address = candidate.contract_address
                sleeve_type_str = (
                    candidate.sleeve_type.value
                    if hasattr(candidate.sleeve_type, "value")
                    else str(candidate.sleeve_type)
                )
            else:
                contract_address = ""
                sleeve_type_str = "cash"

            # Cash sleeves use tighter rebalance bands (200 bps); others use default 500 bps
            band_bps = 200 if sleeve_type_str in ("cash", SleeveType.CASH_STABLE) else 500

            sleeves.append(
                CanonicalSleeve(
                    asset_id=asset_id,
                    contract_address=contract_address,
                    target_weight_bps=weight_bps,
                    band_bps=band_bps,
                    sleeve_type=sleeve_type_str,
                )
            )

    # Sort sleeves deterministically by asset_id
    sleeves.sort(key=lambda s: s.asset_id)

    constraints = CanonicalConstraints(
        max_issuer_weight_bps=int(round(policy_draft.max_issuer_weight * 10000)),
        max_single_asset_weight_bps=int(round(policy_draft.max_single_asset_weight * 10000)),
        min_liquidity_sleeve_bps=int(round(policy_draft.min_liquidity_sleeve * 10000)),
        max_slippage_bps=50,
        rebalance_band_bps=500,
    )

    eligibility = CanonicalEligibility(
        jurisdiction=policy_draft.jurisdiction,
        investor_class=policy_draft.investor_class,
        share_class="open",
    )

    signers = multisig_signers if multisig_signers is not None else [admin_address]
    admin = CanonicalAdmin(
        admin_address=admin_address,
        multisig_threshold=multisig_threshold,
        multisig_signers=signers,
    )

    policy_kwargs = {
        "schema_version": "1.0",
        "basket_name": basket_name,
        "numeraire": numeraire,
        "sleeves": sleeves,
        "constraints": constraints,
        "eligibility": eligibility,
        "admin": admin,
    }
    if created_at is not None:
        policy_kwargs["created_at"] = created_at

    return CanonicalPolicy(**policy_kwargs)


def to_canonical_json(policy: CanonicalPolicy) -> str:
    """
    Serializes a CanonicalPolicy object into deterministic RFC 8785 canonical JSON:
    - Lexicographically sorted keys at all nested levels
    - Compact separators (',', ':') with no whitespace
    - Deterministic UTF-8 string encoding
    """
    data = policy.model_dump(mode="json")
    return json.dumps(data, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def compute_policy_hash(policy: CanonicalPolicy) -> tuple[str, bytes]:
    """
    Computes the SHA-256 hash of the canonical JSON policy representation.
    Returns (hex_string_64, raw_bytes_32) matching Soroban BytesN<32>.
    """
    canonical_json = to_canonical_json(policy)
    hasher = hashlib.sha256(canonical_json.encode("utf-8"))
    return hasher.hexdigest(), hasher.digest()


def prepare_signing_envelope(policy: CanonicalPolicy) -> SignedPolicyEnvelope:
    """
    Bundles the canonical policy, canonical JSON string, and SHA-256 hash
    into a SignedPolicyEnvelope ready for client signing and on-chain anchoring.
    """
    canonical_json = to_canonical_json(policy)
    hex_digest, raw_bytes = compute_policy_hash(policy)
    return SignedPolicyEnvelope(
        policy=policy,
        canonical_json=canonical_json,
        policy_hash_hex=hex_digest,
        policy_hash_bytes=raw_bytes,
    )
