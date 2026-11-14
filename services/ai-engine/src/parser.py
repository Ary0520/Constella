"""
Parser module for the Constella AI Thesis Engine.
Converts plain-English investment statements into typed, validated PolicyDraft schemas.
Enforces closed universe awareness and conservative defaults for ambiguous inputs.
"""

import re
from typing import Dict, List, Set
from src.models import PolicyDraft


# Known unregistered tickers and cryptos outside the closed Stellar asset registry
UNREGISTERED_SYMBOLS = {
    "TSLA", "TESLA", "BTC", "BITCOIN", "ETH", "ETHEREUM",
    "SOL", "SOLANA", "AAPL", "APPLE", "AMZN", "AMAZON",
    "GOOG", "GOOGLE", "NVDA", "NVIDIA", "MSFT", "MICROSOFT"
}


def parse_thesis(
    thesis: str,
    jurisdiction: str = "non-US",
    investor_class: str = "retail"
) -> PolicyDraft:
    """
    Parse a plain-English investment thesis into a typed PolicyDraft.
    
    Offline deterministic execution:
    1. Extracts currency exposures and liquidity reserve requirements.
    2. Identifies target objectives and risk orientation.
    3. Flags unsupported assets outside the closed registry in ambiguities.
    4. Applies conservative fallback parameters and confidence penalties for vague inputs.
    """
    raw_text = thesis.strip()
    text_lower = raw_text.lower()
    ambiguities: List[str] = []
    confidence: Dict[str, float] = {}

    # Check for prompt vagueness (< 5 words or very short)
    is_vague = len(raw_text.split()) < 5

    # 1. Detect unsupported assets (Closed Universe Rule)
    # Check for known unregistered tickers/names
    for symbol in UNREGISTERED_SYMBOLS:
        pattern = rf"\b{re.escape(symbol.lower())}\b"
        if re.search(pattern, text_lower):
            ambiguities.append(
                f"Requested asset '{symbol}' is not part of the verified Stellar asset registry (Closed Universe Rule)."
            )
            confidence["unregistered_asset_detected"] = 0.20

    # 2. Extract percentage allocations
    # Matches patterns like '70% dollar T-bill funds', '20% euro T-bills', '10% instant liquidity'
    pct_matches = re.findall(
        r"(\d+(?:\.\d+)?)\s*%\s*(?:in\s+)?([a-zA-Z\$-]+(?:\s+[a-zA-Z-]+)*)",
        raw_text,
        re.IGNORECASE
    )

    currency_exposure: Dict[str, float] = {}
    min_liquidity_sleeve: float = 0.10
    has_explicit_liquidity = False

    for pct_str, context in pct_matches:
        pct = float(pct_str) / 100.0
        ctx_lower = context.lower()

        # Check for liquidity sleeve
        if any(term in ctx_lower for term in ["liquidity", "instant liquidity", "cash", "reserve", "buffer"]):
            min_liquidity_sleeve = pct
            has_explicit_liquidity = True
            confidence["min_liquidity_sleeve"] = 0.95
        # Check for USD / dollar exposure
        elif any(term in ctx_lower for term in ["dollar", "usd", "$", "us t-bill"]):
            currency_exposure["USD"] = pct
        # Check for EUR / euro exposure
        elif any(term in ctx_lower for term in ["euro", "eur", "€"]):
            currency_exposure["EUR"] = pct

    # Fallback for currency exposure if not parsed
    if currency_exposure:
        confidence["currency_exposure"] = 0.95
    else:
        # Default to USD if no currency mentioned
        if not any(curr in text_lower for curr in ["euro", "eur", "gbp"]):
            currency_exposure["USD"] = 1.00
            confidence["currency_exposure"] = 0.40 if is_vague else 0.70
            ambiguities.append("No explicit currency distribution specified; defaulted to USD.")
        else:
            currency_exposure["EUR"] = 1.00
            confidence["currency_exposure"] = 0.40 if is_vague else 0.70

    # Fallback for liquidity sleeve if not explicitly extracted
    if not has_explicit_liquidity:
        if any(term in text_lower for term in ["runway", "safe", "treasury"]):
            min_liquidity_sleeve = 0.10
            confidence["min_liquidity_sleeve"] = 0.85
        else:
            min_liquidity_sleeve = 0.10  # conservative floor >= 0.05
            confidence["min_liquidity_sleeve"] = 0.50 if is_vague else 0.70
            if is_vague:
                ambiguities.append("No explicit liquidity floor requested; defaulted to 10% instant liquidity sleeve.")

    # 3. Determine Risk Tolerance
    if any(term in text_lower for term in ["safe", "runway", "capital preservation", "conservative", "t-bill"]):
        risk_tolerance = "conservative"
        confidence["risk_tolerance"] = 0.95
    elif any(term in text_lower for term in ["equities", "equity", "tech", "growth", "high yield", "aggressive"]):
        risk_tolerance = "growth"
        confidence["risk_tolerance"] = 0.85
    elif any(term in text_lower for term in ["moderate", "balanced"]):
        risk_tolerance = "moderate"
        confidence["risk_tolerance"] = 0.85
    else:
        # Conservative fallback for ambiguous requests
        risk_tolerance = "conservative"
        confidence["risk_tolerance"] = 0.45 if is_vague else 0.70
        ambiguities.append("Ambiguous risk orientation; defaulted to conservative risk tolerance.")

    # 4. Objectives extraction
    objectives: List[str] = []
    if "runway" in text_lower:
        objectives.append("preserve capital and extend runway")
    if "payroll" in text_lower:
        objectives.append("match operational payroll obligations")
    if "liquidity" in text_lower or has_explicit_liquidity:
        objectives.append("maintain instant liquidity reserve")
    if "t-bill" in text_lower or "treasury" in text_lower:
        objectives.append("earn sovereign t-bill yield")
    if "yield" in text_lower:
        objectives.append("generate sustainable yield")
    if "equities" in text_lower or "tech" in text_lower:
        objectives.append("equity growth exposure")

    if not objectives:
        objectives.append("capital preservation and short-term liquidity")
        confidence["objectives"] = 0.40
    else:
        confidence["objectives"] = 0.90 if not is_vague else 0.55

    # 5. Exclusions
    exclusions: List[str] = []
    if risk_tolerance == "conservative":
        exclusions.extend(["equity", "illiquid"])
    if "no crypto" in text_lower:
        exclusions.append("crypto")
    if "no lending" in text_lower:
        exclusions.append("lending")

    # 6. Yield orientation
    if any(term in text_lower for term in ["high yield", "maximize"]):
        target_yield_orientation = "maximize"
    elif any(term in text_lower for term in ["safe", "runway", "capital preservation"]):
        target_yield_orientation = "secondary"
    else:
        target_yield_orientation = "secondary" if is_vague else "none"

    # Liquidity horizon hours
    liquidity_horizon_hours = 24
    confidence["liquidity_horizon_hours"] = 0.50 if is_vague else 0.90
    if is_vague:
        ambiguities.append("Prompt is brief; defaulted liquidity horizon to 24 hours.")

    return PolicyDraft(
        objectives=objectives,
        currency_exposure=currency_exposure,
        liquidity_horizon_hours=liquidity_horizon_hours,
        max_issuer_weight=0.30,
        max_single_asset_weight=0.25,
        risk_tolerance=risk_tolerance,
        exclusions=exclusions,
        min_liquidity_sleeve=min_liquidity_sleeve,
        target_yield_orientation=target_yield_orientation,
        investor_class=investor_class,
        jurisdiction=jurisdiction,
        confidence=confidence,
        ambiguities=ambiguities,
    )
