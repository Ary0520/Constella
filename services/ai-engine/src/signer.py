"""
Thesis Engine Step 6: Sign (signer.py)
Assembles the canonical policy, serializes it as canonical JSON and computes the
32-byte SHA-256 hash anchored on chain by `bind_policy`.

OPEN-ISSUES #3 (fixed 24 Nov 2026):
* the policy names the REAL owner - the account that signs `register_policy`
  (a G account or a C smart account; never a placeholder, never a muxed W
  address, which cannot sign);
* the on-chain `bind_policy` terms are derived from the canonical policy
  (`onchain_terms`), so what is hashed is exactly what is enforced;
* canonical JSON is RFC 8785 (JCS) compatible because floats are refused: for
  integers, strings, booleans and null with ASCII keys, sorted keys plus
  compact separators produce the JCS serialization.
"""

from datetime import datetime, timezone
import hashlib
import json
from typing import Dict, List, Optional

from stellar_sdk import StrKey

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


# Execution limits written into every policy (and enforced on chain from it).
MAX_SLIPPAGE_BPS = 50
REBALANCE_BAND_BPS = 200


def validate_owner(address: str) -> str:
    """A policy owner must be able to sign: a G account or a C contract account."""
    if not isinstance(address, str) or not (
        StrKey.is_valid_ed25519_public_key(address) or StrKey.is_valid_contract(address)
    ):
        raise ValueError(f"invalid policy owner address: {address!r}")
    return address


def assemble_canonical_policy(
    policy_draft: PolicyDraft,
    opt_result: OptimizationResult,
    candidates: List[AssetCandidate],
    basket_name: str = "cRUNWAY",
    *,
    owner: str,
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
            if candidate is None:
                # Never sign an asset the user is not eligible for (fail closed).
                raise ValueError(f"cannot sign {asset_id}: not an eligible candidate")
            contract_address = candidate.contract_address
            sleeve_type_str = (
                candidate.sleeve_type.value
                if hasattr(candidate.sleeve_type, "value")
                else str(candidate.sleeve_type)
            )
            # Decision D-A / A1b: the vault currency is declared on chain as the
            # "numeraire" sleeve (exempt from the concentration caps there too).
            if opt_result.numeraire and asset_id == opt_result.numeraire:
                sleeve_type_str = "numeraire"

            # Cash sleeves use tighter rebalance bands (200 bps); others use default 500 bps
            band_bps = 200 if sleeve_type_str in ("cash", "numeraire", SleeveType.CASH_STABLE) else 500

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
        max_slippage_bps=MAX_SLIPPAGE_BPS,
        rebalance_band_bps=REBALANCE_BAND_BPS,
    )

    eligibility = CanonicalEligibility(
        jurisdiction=policy_draft.jurisdiction,
        investor_class=policy_draft.investor_class,
        share_class="open",
    )

    validate_owner(owner)
    signers = [validate_owner(a) for a in multisig_signers] if multisig_signers is not None else [owner]
    admin = CanonicalAdmin(
        admin_address=owner,
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


def onchain_terms(policy: CanonicalPolicy) -> Dict:
    """The exact `bind_policy` arguments enforced on chain for this policy."""
    c = policy.constraints
    sleeves = [{"asset_id": s.asset_id, "target_weight_bps": s.target_weight_bps,
                "min_band_bps": max(0, s.target_weight_bps - s.band_bps),
                "max_band_bps": min(10_000, s.target_weight_bps + s.band_bps), "sleeve_type": s.sleeve_type}
               for s in policy.sleeves]
    return {"min_liquidity_sleeve_bps": c.min_liquidity_sleeve_bps, "max_single_asset_bps": c.max_single_asset_weight_bps,
            "max_issuer_bps": c.max_issuer_weight_bps, "max_slippage_bps": c.max_slippage_bps,
            "rebalance_band_bps": c.rebalance_band_bps, "sleeves": sleeves}


def _jcs_safe(value, path: str = "$") -> None:
    if isinstance(value, float):
        raise ValueError(f"canonical policy must not contain floats ({path})")
    if isinstance(value, dict):
        for k, v in value.items():
            if not isinstance(k, str) or not k.isascii():
                raise ValueError(f"canonical policy keys must be ASCII strings ({path})")
            _jcs_safe(v, f"{path}.{k}")
    elif isinstance(value, list):
        for i, v in enumerate(value):
            _jcs_safe(v, f"{path}[{i}]")


def to_canonical_json(policy: CanonicalPolicy) -> str:
    """
    Canonical JSON (RFC 8785 compatible, see module doc): sorted keys at every
    level, compact separators, UTF-8, and no floats (refused, never rounded).
    """
    data = policy.model_dump(mode="json", warnings=False)
    _jcs_safe(data)
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
