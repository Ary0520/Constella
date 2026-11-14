"""
Constella AI Thesis Engine package.
"""

from src.explainer import generate_explanation
from src.models import (
    AssetCandidate,
    CanonicalAdmin,
    CanonicalConstraints,
    CanonicalEligibility,
    CanonicalPolicy,
    CanonicalSleeve,
    DiffItem,
    ExplanationReport,
    OptimizationResult,
    PolicyDraft,
    RedemptionLadder,
    SignedPolicyEnvelope,
    SimulationReport,
    SleeveType,
    StressScenario,
)
from src.optimizer import optimize_portfolio
from src.parser import parse_thesis
from src.retriever import AssetRegistry, retrieve_candidates
from src.signer import (
    assemble_canonical_policy,
    compute_policy_hash,
    prepare_signing_envelope,
    to_canonical_json,
)
from src.simulator import simulate_portfolio

__all__ = [
    "AssetCandidate",
    "AssetRegistry",
    "CanonicalAdmin",
    "CanonicalConstraints",
    "CanonicalEligibility",
    "CanonicalPolicy",
    "CanonicalSleeve",
    "DiffItem",
    "ExplanationReport",
    "OptimizationResult",
    "PolicyDraft",
    "RedemptionLadder",
    "SignedPolicyEnvelope",
    "SimulationReport",
    "SleeveType",
    "StressScenario",
    "assemble_canonical_policy",
    "compute_policy_hash",
    "generate_explanation",
    "optimize_portfolio",
    "parse_thesis",
    "prepare_signing_envelope",
    "retrieve_candidates",
    "simulate_portfolio",
    "to_canonical_json",
]
